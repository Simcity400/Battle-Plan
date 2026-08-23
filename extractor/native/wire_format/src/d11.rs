//! d11 `production_live` — per-country stockpiles, production lines, the civil
//! construction queue, production licences and the convoy bank.
//!
//! Derived from `world_fold.rs:17934`-`:18575`
//! (`fold_production`), its `parse_production_trailers` chain parser and the ten
// @world-fold-authority domain=11 functions=fold_production lines=17934-18575 sha256=196584a7f9b89e2a3bcfd56c98f5b92acfcce5ddbeb34995021888e6db1e14bb closure=732f5526bd6b32ea3d5a8f10ae32fb79ca9d326bacc289fd4725e105a332e03c
// @leaf-source-authority crate=d11_grammar root=extractor/native/d11_grammar files=8 sha256=cd0e1eed4d8767af23c3d745c2ff369f1fa78e514c9a77ab4f6ef9acdfc54631
//! per-segment parsers it dispatches to, plus the collector reads that produce
//! every gate (`extractor/native/collector/src/attach_log.rs`, cited per field).
//! Native offsets and scale proofs are retained as
//! `domain-11::Deployment queues`, `domain-11::Production-line efficiency`, and
//! `domain-11::Stockpiles, production lines, and construction` in
//! `extractor/documents/technical_evidence.json`. Where that evidence and the
//! fold disagree, the fold wins (see `construction_queue`).
//!
//! # Payload grammar
//!
//! A 40-byte header (`world_fold.rs::fold_production`):
//!
//! | offset | width | field |
//! |---|---|---|
//! | `+0`  | u64 | `elapsed_wall_ms` |
//! | `+8`  | u32 | `country_count` |
//! | `+12` | u32 | `changed_country_count` |
//! | `+16` | u32 | `status_flags` |
//! | `+20` | u32 | `scale_raw` (written by the collector, **never read by the fold**) |
//! | `+24` | u32 | `stockpile_count` |
//! | `+28` | u32 | `military_count` |
//! | `+32` | u32 | `naval_count` |
//! | `+36` | u32 | `construction_count` |
//!
//! then three fixed-row tables in this order, and then a chain of ten optional
//! self-sized trailer segments:
//!
//! * `stockpile_count` × 16-byte rows — `tag_db_id` u32, `variant_id` u32,
//!   `amount` i64 `/100_000` (`world_fold.rs::fold_production`).
//! * `military_count + naval_count` × 32-byte rows — `tag_db_id` u32, `kind` u32,
//!   `line_id` i32, `active_factories` i32, `cost` i32 `/100_000`, `speed` i32
//!   `/100_000`, `produced` i32 `/100_000`, `amount` i32 **raw**
//!   (`world_fold.rs::fold_production`). The military rows come first, then the naval rows;
//!   `kind` (not the position) is what the fold reads, and `kind == 2` is naval
//!   (`world_fold.rs::fold_production`).
//! * `construction_count` × 32-byte rows — `tag_db_id` u32, `line_id` u32,
//!   `template_id` u32 (`u32::MAX` = absent), `active_factories` i32, `cost` i32
//!   `/100_000`, `produced` i32 `/100_000`, `priority` u32, then 4 reserved bytes
//!   the fold does not read (`world_fold.rs::fold_production`, packed at
//!   `attach_log.rs:68076`-`:68101`).
//!
//! **The scales are per field and three of them are different.**
//! `PRODUCTION_SCALE = 100_000` covers the stockpile amount and the line
//! cost/speed/produced (`:12553`, `:12576`-`:12578`) and the construction
//! cost/produced (`:12672`-`:12673`). The three PEF1 efficiency channels use a
//! **function-local `EFFICIENCY_SCALE = 10_000_000`** (`:12631`) — reading them
//! at `PRODUCTION_SCALE` is a silent 100× error. The line `amount` at `+28` is
//! emitted **raw, unscaled** (`:12592`) even though its neighbours at `+16`,
//! `+20` and `+24` are all scaled; `active_factories` and the construction
//! `priority` are raw too.
//!
//! # Trailer chain (`world_fold.rs::parse_production_trailers`)
//!
//! Each segment starts with its 4-byte magic; a magic that is not one of the ten
//! refuses the whole payload (`world_fold.rs::parse_production_trailers`), and a repeated magic is a hard error.
//! Nine of the ten carry a 20-byte header of `magic, version u16, header_len u16,
//! segment_len u32, row_count u32, reserved u32`; `PLIC` has no `segment_len`
//! and computes its length from two counts, `PCV1` has a 28-byte header with
//! three counts, and `PDQ1` carries a nested codec payload and **must be last**
//! (`world_fold.rs::parse_production_trailers`).
//!
//! | magic | rows | joins onto | carries |
//! |---|---|---|---|
//! | `PLIC` v1/v2 | per-country counts + 4- or 16-byte items | country | licence variant ids; v2 adds `visible_order`, `creator`, `origin` |
//! | `PVJ1` v1 | 16 | line | `equipment_variant_id` |
//! | `PFR1` v1 | 16 | line | `requested_factories`, range `0..=100_000` |
//! | `PPR1` v1 | 16 | line | `priority`, range `0..=4095` — the engine's line ordinal at line+0x44 |
//! | `PLS1` v1 | 16 | line | `line_subtype`, only `(1,1) (2,2) (2,3) (1,4)` |
//! | `PEF1` v1 | 36 | line | `efficiency`, `efficiency_cap`, `efficiency_growth`, all i64 `/10_000_000` |
//! | `PMI1` v1 | 16 | line | `mio_id`; `0` is a **proven zero CID**, folded to Null (`world_fold.rs::fold_production`) |
//! | `PCP1` v1 | 8 | country | the three completeness bits |
//! | `PCV1` v1 | 40/16/8 | country | the convoy bank, its variants and its priority reservations |
//! | `PDQ1` v1 | nested | country | the military deployment queue — **stop-reported, see below** |
//!
//! Every line join is **all-or-nothing per frame**: the collector returns early
//! and drops the whole segment the moment one line cannot supply the value
//! (`attach_log.rs:68980`-`:68985` is the pattern). So a missing segment means
//! "not captured for any line this hour", never "captured for some".
//!
//! # Four-state semantics
//!
//! * **A missing line join is `Unknown` for every line in the frame.** The
//!   collector's own words: `equipment_variant_id`'s `None` "means the live
//!   producer could not prove the join, never an empty/default id"
//!   (`attach_log.rs:72719`-`:72721`); `requested_factories`' `None` "means the
//!   field could not be validated" (`:72729`); `efficiency`'s `None` "keeps PEF1
//!   absent for every line in the frame" (`:72732`). That is record-state contract's *applicable,
//!   not captured*. The capture exercises it: hour 60832893 of `deepwar_t1`
//!   carries 147 production lines and no `PEF1` segment.
//! * **`mio_id` is the one `Null`.** `Some(None)` is "a proven zero CID"
//!   (`attach_log.rs:72722`-`:72724`) — the line genuinely has no industrial
//!   organisation — and the fold writes `Value::Null` for it (`:12651`-`:12655`).
//!   `None` is unreadable and suppresses the segment, which is `Unknown`. So the
//!   field is the only one that declares both, and it is never `Absent`.
//! * **The three PCP1 completeness bits are measured booleans**, each exactly
//!   `Option::is_some()` over a collector vector (`attach_log.rs:68787`-`:68795`),
//!   so `false` is a real measurement ("this vector was unreadable this hour").
//!   Without the `PCP1` segment they were not captured at all: `Unknown`.
//! * **`template_id == u32::MAX` is `Unknown`.** The collector writes the
//!   sentinel for `template_id: None` (`attach_log.rs:68081`), and that `None`
//!   covers a non-heap sub-pointer, an unreadable id and an out-of-range id
//!   alike (`:85351`-`:85356`) — unreadable, not "no template". A null
//!   sub-pointer would argue for `Null`, so this is the recipe's tie-break, taken
//!   towards `Unknown` because `Null` asserts something the read cannot support.
//!   Unexercised in `deepwar_t1` (0 of 67,848 construction rows).
//! * **`cache_valid` and `assigned` are `Unknown` on an unreadable convoy bank.**
//!   `PCV1` forces every field of an incomplete country row to zero
//!   (`world_fold.rs::parse_country_convoy_bank_segment`), so those zeros are padding, not
//!   measurements, and the fold never emits them (`world_fold.rs::fold_production`).
//! * **Zero is a value everywhere else.** A country with an empty but readable
//!   stockpile has `stockpile_complete = true` and no rows; a line with no
//!   factories assigned reads `active_factories = 0` with every gate set.
//!
//! # Values dissolved because the fold enforces them (the d24/d62 rule)
//!
//! * `convoys_owned_raw` — `PCV1` rejects any row whose `owned_raw` is not the
//!   (saturating, `OWNED_RAW_CAP`-clamped) sum of that country's variant amounts
//!   (`world_fold.rs:27732`-`:27738`). Every summand is non-negative and every
//!   step re-clamps, so the sum is order-independent and reproducible from the
//!   carried variant rows.
//! * `convoys_count` — `owned_raw / 100_000` (`world_fold.rs::fold_production`).
//! * `convoys_available` — rejected unless it equals `owned_raw / 100_000 -
//!   assigned` (`world_fold.rs::parse_country_convoy_bank_segment`); the replay layer re-checks the same law
//!   (`replay_core/src/world.rs:19841`).
//! * The `PCV1` `cache_verified` bit — rejected unless equal to `cache_valid`
//!   (`:27720`) — and the `classifier_exact` bit, which is required set on every
//!   complete row (`:27719`), so it is constant over everything this schema can
//!   hold. Both are d62's witness-flag case.
//! * The `PCV1` variant `equip_type`, rejected unless `70` (`:27646`).
//! * `stockpile_total`, `military_factories` and `naval_factories` are sums the
//!   fold computes over the rows (`:12568`, `:12660`, `:12663`); they are not on
//!   the wire at all and are recomputed, not carried.
//!
//! # Shapes, and the order the fold's arrays carry
//!
//! Every collection here is a `KeyedSet`. Three of them the fold stores as
//! **ordered arrays**, and for those a keyed set is only lossless if the array
//! order is a total function of carried fields **on every payload the fold
//! accepts** — not merely on the payloads this capture happens to contain. Two
//! of the three need an explicit ordinal for that; the third does not.
//!
//! * `military_lines` / `naval_lines` (`world_fold.rs::fold_production`) — the
//!   array is the base table's row order within a `(tag_db_id, kind)` run. The
//!   `PPR1` join's `priority` is the engine's own line ordinal at line+0x44
//!   (`attach_log.rs:72727`) and it *does* rank that order: it is unique within
//!   `(tag_db_id, kind)` in every country-kind-hour of `deepwar_t1` and the wire
//!   order is ascending `priority` in every one of them (base wire order is
//!   **not** ascending `line_id` — 13,782 country-kind-hours differ — so
//!   `line_id` alone would not have recovered it). But `PPR1` is optional, and a
//!   frame with line rows and no `PPR1` is a frame the fold accepts, on which
//!   `priority` is `Unknown` and the order would be gone. So the row also
//!   carries `line_ordinal`, its **dense** position in its `(tag_db_id, kind)`
//!   run — always concrete, and the order witness in every case. `priority`
//!   stays beside it as the engine's own value: it is *sparse* (12,884
//!   country-kind-hours are not a dense `0..n-1` permutation), so the two are
//!   different measurements, and the fold enforces no relation between them.
//!   The key stays the fold's own `(tag_db_id, kind, line_id)` (`:12581`), which
//!   the fold does enforce, so a line keeps its identity while its ordinal moves.
//! * `construction_queue` (`:12667`-`:12689`) — the fold pushes rows in wire
//!   order and enforces **no key at all**, so two rows sharing every identity
//!   word are a frame the recorder accepts and this collection must hold. The
//!   key is therefore `(tag_db_id, construction_ordinal, line_id)`, where the
//!   ordinal is the row's dense position within its country's run of the base
//!   table: unique by construction, so nothing is ever refused, and leading the
//!   `line_id` so the keyed set's own order **is** the fold's array order. The
//!   `+24` word the fold calls `priority` is a value field: the collector sets
//!   it to `k / CONSTRUCTION_STRIDE`, the *sparse* source-slot index that skips
//!   unreadable slots (`attach_log.rs:85364`), so its gaps (2,048 country-hours)
//!   are data about what was skipped rather than an ordering to re-derive.
//! * `production_licenses` — no ordinal needed: v2 carries `visible_order`
//!   explicitly and is required to equal the item's position (`:27975`), and v1's
//!   ids are sorted ascending by the fold (`:27995`), which is the keyed set's
//!   own order. Both orders are recoverable on every payload the fold accepts.
//!
//! `convoy_variants` is a fourth ordered array (`:13017`). Its key is
//! fold-enforced — `PCV1` rejects a duplicate `variant_id` inside a complete
//! country's run (`:27733`) — but nothing constrains the run's ORDER, so it
//! carries `convoy_variant_ordinal` for the same reason the other two do. Over
//! `deepwar_t1` the ordinal is redundant with key order in all 9,528 non-empty
//! runs (they are ascending by `variant_id`), which
//! [`FoldOutcome::unordered_variant_runs`] reports so that redundancy stays
//! visible; it is carried anyway, because "this stream happens to be sorted" is
//! not a property of the payloads the fold accepts.
//!
//! `stockpile` is a JSON **map** in the fold (`:12567`), so it has no order to
//! lose, and `countries` / `convoy_bank` are one row per country.
//!
//! # `convoy_priority_reservations`: ordered occurrence list
//!
//! `PCV1`'s reservation run is a pointer vector the fold emits as an ordered
//! array of `{priority, count}` (`world_fold.rs::fold_production`). Its only identity
//! candidate, `priority`, happens to be unique within each captured country-hour
//! but is not required unique by the parser. **Its order is load-bearing and is
//! not a function of any carried field**: across `deepwar_t1`, 2,633 consecutive
//! samples of a country's reservation list are a *pure reorder* of the identical
//! `{priority, count}` multiset (e.g. tag 29 at hour 60832407 carries
//! `[(2,0),(4,7),(5,0),(3,0)]` and at 60832417 `[(2,0),(3,0),(4,7),(5,0)]`).
//! A keyed set stores those two hours identically and emits no delta, which is
//! exactly the fidelity loss collection-shape contract's ordered shapes prevent. `priority` is not
//! fold-enforced unique — `PCV1` enforces uniqueness for variants and checks only
//! `count >= 0` for reservations — so this is an `OrderedOccurrenceList`, not an
//! `OrderedKeyedList`. Keying by source index would make identity positional,
//! which every reorder then rewrites, and a positional index is not the "stable
//! identity that persists across hours" a keyed set requires.
//!
//! This module still **parses** the reservation run — the fold's exact total law
//! checks every `count` against `owned_raw / 100_000`
//! (`world_fold.rs::parse_country_convoy_bank_segment`), so refusing to
//! parse it would accept payloads the recorder refuses — and
//! [`FoldOutcome::reservation_rows`] accounts for it. The port maps every row,
//! including `tag_db_id`, retaining source order within each country and
//! flattening country groups in the fold's `BTreeMap` tag order. The
//! convoy *variant* run has no such problem: it is sorted by `variant_id` in
//! every country-hour of the capture and shows **zero** pure reorders, so its
//! order is recoverable from its own key.
//!
//! # STOP: `PDQ1` / `military.deployment_queue` is not portable
//!
//! The final trailer carries a three-level tree (country → conveyor → line →
//! equipment) whose rows include **variable-length UTF-8 strings**: the
//! conveyor's `name` and `role`, and each equipment row's `archetype`
//! (`lanes/country_economy_production_stockpile_live/src/deployment_queue.rs:58`,
//! `:63`, `:35`; encoded at `:197`-`:198` and `:230`). schema-kind contract has no string kind, and
//! content-identity contract carries content symbols as "manifest ids as varints, never text" — but the
//! conveyor `name` is
//! a player-authored division name, so it is not a content symbol at all and no
//! manifest rule will ever cover it. Ordering is *not* the blocker here: every
//! level carries a dense `ordinal` the decoder already requires to equal its
//! position. This module therefore parses `PDQ1` **structurally** — enough to
//! enforce the five cross-checks the fold makes against it and to account for its
//! bytes — and maps none of it. See the deliverable's stop-report.
//!
//! # No derived field (derived-field contract)
//!
//! Nothing here changes deterministically with the clock: costs, efficiencies and
//! stockpiles all move on their own. The mappers still take `frame_tick` so every
//! domain's boundary has one shape.

use std::collections::{BTreeMap, BTreeSet};

use crate::body::{CollectionState, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Quant, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 11;

pub const COUNTRIES: u32 = 1;
pub const STOCKPILE: u32 = 2;
pub const PRODUCTION_LINES: u32 = 3;
pub const CONSTRUCTION_QUEUE: u32 = 4;
pub const PRODUCTION_LICENSES: u32 = 5;
pub const CONVOY_BANK: u32 = 6;
pub const CONVOY_VARIANTS: u32 = 7;
pub const CONVOY_PRIORITY_RESERVATIONS: u32 = 8;
/// Historical name retained for callers that knew id 8 while it was reserved.
/// The id now names the implemented collection.
pub const RESERVED_CONVOY_RESERVATIONS: u32 = CONVOY_PRIORITY_RESERVATIONS;

/// Field ids are stable and append-only; schema-identity contract forbids reusing an id for a
/// different field. The same id names the same concept in every collection that
/// carries it (d26's convention).
pub mod field {
    // Keys.
    pub const TAG_DB_ID: u32 = 1;
    pub const VARIANT_ID: u32 = 2;
    pub const KIND: u32 = 3;
    pub const LINE_ID: u32 = 4;
    pub const CONSTRUCTION_ID: u32 = 5;
    /// The construction row's `+24` word — the collector's *sparse* source-slot
    /// index (`attach_log.rs:85364`), which skips unreadable slots. A value
    /// field: the skips are data, and it is not what orders the queue here.
    pub const CONSTRUCTION_PRIORITY: u32 = 6;
    /// The construction row's **dense** position within its country's run of the
    /// base table — a key component, and the order witness that exists on every
    /// payload the fold accepts. See the module docs.
    pub const CONSTRUCTION_ORDINAL: u32 = 7;

    // `countries`.
    pub const STOCKPILE_COMPLETE: u32 = 10;
    pub const MILITARY_LINES_COMPLETE: u32 = 11;
    pub const NAVAL_LINES_COMPLETE: u32 = 12;
    pub const LICENSE_DETAIL: u32 = 13;

    /// The `stockpile` amount, as the fold projects it: `raw / 100_000` and
    /// nothing else (`world_fold.rs::fold_production`).
    pub const AMOUNT: u32 = 20;
    /// The convoy variant's **raw i64**. The fold emits `amount_raw` beside its
    /// `f64` projection (`world_fold.rs::fold_production`), and `PCV1` accepts
    /// raws past 2^53, which no `f64` can hold — so the raw is what is carried
    /// and the projection is recomputed.
    pub const CONVOY_AMOUNT_RAW: u32 = 21;
    /// The convoy variant's dense position within its country's `PCV1` run. The
    /// fold stores the run as an ordered array (`world_fold.rs::fold_production`) and
    /// enforces nothing about its order, so this is the order witness — the same
    /// device `line_ordinal` and `construction_ordinal` use.
    pub const CONVOY_VARIANT_ORDINAL: u32 = 22;

    // `production_lines` and `construction_queue`.
    pub const ACTIVE_FACTORIES: u32 = 30;
    pub const COST: u32 = 31;
    pub const SPEED: u32 = 32;
    pub const PRODUCED: u32 = 33;
    pub const LINE_AMOUNT: u32 = 34;
    pub const EQUIPMENT_VARIANT_ID: u32 = 35;
    pub const REQUESTED_FACTORIES: u32 = 36;
    pub const LINE_PRIORITY: u32 = 37;
    pub const LINE_SUBTYPE: u32 = 38;
    pub const EFFICIENCY: u32 = 39;
    pub const EFFICIENCY_CAP: u32 = 40;
    pub const EFFICIENCY_GROWTH: u32 = 41;
    pub const MIO_ID: u32 = 42;
    /// The line's **dense** position within its `(tag_db_id, kind)` run of the
    /// base table. Required and always concrete — unlike `LINE_PRIORITY`, which
    /// is `Unknown` whenever the frame carries no `PPR1` segment.
    pub const LINE_ORDINAL: u32 = 43;

    pub const TEMPLATE_ID: u32 = 50;

    // `production_licenses`.
    pub const VISIBLE_ORDER: u32 = 60;
    pub const CREATOR_TAG_DB_ID: u32 = 61;
    pub const ORIGIN_TAG_DB_ID: u32 = 62;

    // `convoy_bank`.
    pub const CONVOYS_COMPLETE: u32 = 70;
    pub const RESERVATIONS_COMPLETE: u32 = 71;
    pub const CACHE_VALID: u32 = 72;
    pub const ASSIGNED: u32 = 73;

    /// Id 80 was reserved for this field before the collection could be ported;
    /// schema-identity contract makes that assignment permanent.
    pub const RESERVATION_COUNT: u32 = 80;
    pub const RESERVATION_PRIORITY: u32 = 81;
    /// Historical name retained now that the reserved field is implemented.
    pub const RESERVED_RESERVATION_COUNT: u32 = RESERVATION_COUNT;
}

// --- Payload constants, from `world_fold.rs::fold_production` -------------------

/// `world_fold.rs::fold_production` — `need(payload, 40, "production header")`.
pub const HEADER_LEN: usize = d11_grammar::BASE_HEADER_LEN;
pub const STOCKPILE_ROW_LEN: usize = d11_grammar::STOCKPILE_ROW_LEN;
pub const LINE_ROW_LEN: usize = d11_grammar::LINE_ROW_LEN;
pub const CONSTRUCTION_ROW_LEN: usize = d11_grammar::CONSTRUCTION_ROW_LEN;

/// `PRODUCTION_SCALE`, applied to the stockpile amount (`:12553`), the line
/// cost/speed/produced (`:12576`-`:12578`) and the construction cost/produced
/// (`:12672`-`:12673`).
pub const PRODUCTION_SCALE: f64 = 100_000.0;
/// The **function-local** `EFFICIENCY_SCALE` at `world_fold.rs:12631`, applied to
/// the three PEF1 channels and to nothing else. 100× `PRODUCTION_SCALE`.
pub const EFFICIENCY_SCALE: f64 = 10_000_000.0;

/// `attach_log.rs:68081` — the sentinel the collector writes for
/// `template_id: None`. `world_fold.rs::fold_production` reads it as "omit the key".
pub const TEMPLATE_ID_ABSENT: u32 = u32::MAX;

/// `world_fold.rs::fold_production` — the only naval kind. Everything else folds military.
pub const KIND_NAVAL: u32 = 2;
pub const KIND_MILITARY: u32 = 1;

/// The three `PCP1` bits (`world_fold.rs::fold_production`).
pub const PCP1_STOCKPILE: u32 = 1;
pub const PCP1_MILITARY_LINES: u32 = 1 << 1;
pub const PCP1_NAVAL_LINES: u32 = 1 << 2;
/// `world_fold.rs:27835` — any other bit refuses the segment.
pub const PCP1_KNOWN_FLAGS: u32 = PCP1_STOCKPILE | PCP1_MILITARY_LINES | PCP1_NAVAL_LINES;

/// `world_fold.rs:27598`.
pub const PCV1_KNOWN_FLAGS: u32 = 0b1_1111;
pub const PCV1_COMPLETE: u32 = 1;
pub const PCV1_RESERVATIONS_COMPLETE: u32 = 1 << 1;
pub const PCV1_CACHE_VALID: u32 = 1 << 2;
pub const PCV1_CACHE_VERIFIED: u32 = 1 << 3;
pub const PCV1_CLASSIFIER_EXACT: u32 = 1 << 4;
/// `world_fold.rs:27600`.
pub const PCV1_OWNED_RAW_CAP: i64 = 0x53E2_D620_FB40;
/// `world_fold.rs:27599`. The integer denominator of the convoy bank totals.
pub const PCV1_SCALE: i64 = 100_000;
/// `world_fold.rs::parse_country_convoy_bank_segment` — the only accepted convoy variant equipment type.
pub const PCV1_EQUIP_TYPE: u32 = 70;

/// `world_fold.rs::parse_production_requested_factories_segment`.
pub const MAX_REQUESTED_FACTORIES: i32 = 100_000;
/// `world_fold.rs:28371`.
pub const MAX_LINE_PRIORITY: i32 = 4095;
/// `world_fold.rs::parse_production_license_segment` — a licence variant id is `1..2^24`.
pub const MAX_LICENSE_VARIANT_ID: u32 = 1 << 24;

/// A production-licence creator/origin `tag_db_id`.
///
/// Every copy of this check — producer, this parser, the payload authority and
/// both folds — spelled it `> i32::MAX`, which is ONE SHORT of the id rule: it
/// admits `i32::MAX` itself, and that is the vector SIZE ceiling, so it is the
/// first value no loaded database can index. A torn `0x7fffffff` was therefore
/// published and folded as a genuine creator. The family bound is
/// `build_profile::MAX_COMPACT_OWNER_TAG_DB_ID` = `i32::MAX - 1`; mirrored here
/// because d11 is not admitted to the production-codec dependency surface.
pub const MAX_PRODUCTION_LICENSE_TAG_DB_ID: u32 = (i32::MAX as u32) - 1;

/// Test-visible aliases retained while the nested layout lives in the leaf.
#[cfg(test)]
const PDQ1_CODEC_VERSION: u32 = d11_grammar::deployment::SchemaVersion::V2.wire();
#[cfg(test)]
const PDQ1_CONVEYOR_ID_TYPE: u32 = d11_grammar::deployment::CONVEYOR_ID_TYPE;
#[cfg(test)]
const PDQ1_LINE_ID_TYPE: u32 = d11_grammar::deployment::LINE_ID_TYPE;
#[cfg(test)]
const PDQ1_TEMPLATE_ID_TYPE: u32 = d11_grammar::deployment::TEMPLATE_ID_TYPE;
#[cfg(test)]
const PDQ1_EQUIPMENT_ID_TYPE: u32 = d11_grammar::deployment::EQUIPMENT_ID_TYPE;
const PDQ1_MAX_STRING_BYTES: usize = 16 * 1024;

/// The `line_subtype` codes of the `PLS1` join (`world_fold.rs::parse_production_line_subtype_segment`).
/// A mutually exclusive set, so record-state contract's Enum rule applies and the overlap the fold
/// rejects becomes inexpressible.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum LineSubtype {
    Military = 0,
    Naval = 1,
    ShipRefit = 2,
    RailwayGun = 3,
}

impl LineSubtype {
    pub const CARDINALITY: u32 = 4;

    /// `world_fold.rs::parse_production_line_subtype_segment`: only these four `(kind, subtype)` pairs
    /// exist; anything else refuses the whole segment.
    pub fn from_wire(kind: u32, subtype: u32) -> Option<Self> {
        match (kind, subtype) {
            (1, 1) => Some(Self::Military),
            (2, 2) => Some(Self::Naval),
            (2, 3) => Some(Self::ShipRefit),
            (1, 4) => Some(Self::RailwayGun),
            _ => None,
        }
    }

    pub fn code(self) -> u32 {
        self as u32
    }
}

/// Which licence detail the frame's `PLIC` segment carried for one country.
/// Mutually exclusive by construction — one segment, one version — so the three
/// states collapse into one enum rather than two booleans whose fourth
/// combination means nothing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum LicenseDetail {
    /// No `PLIC` row for this country: the fold writes `production_licenses:
    /// Null` (`world_fold.rs::fold_production`). Absent is not empty.
    NotMeasured = 0,
    /// `PLIC` v1: ids only, sorted ascending by the fold (`:27995`), and
    /// `production_license_variants` is omitted entirely (`:12933`).
    IdsOnly = 1,
    /// `PLIC` v2: ids in manager order with creator/origin identities.
    IdsAndVariants = 2,
}

impl LicenseDetail {
    pub const CARDINALITY: u32 = 3;

    pub fn code(self) -> u32 {
        self as u32
    }
}

pub fn schema() -> Schema {
    use field as f;
    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![
            Collection {
                collection_id: COUNTRIES,
                shape: Shape::KeyedSet,
                fields: vec![
                    // A country tag db id is content-defined (content-identity contract) — a manifest
                    // must resolve it for it to keep its meaning under another
                    // mod set. d63 and d26 key their countries the same way.
                    // A ContentId **key**, the heavier codec-measurement contract retrofit class.
                    Field::new(f::TAG_DB_ID, Kind::ContentId).key(0),
                    // `PCP1` bit 0/1/2. A measured boolean: `false` says the
                    // collector could not read that vector this hour
                    // (`attach_log.rs:68787`). Unknown when the segment is
                    // missing, which is a different fact and a different bit.
                    Field::new(f::STOCKPILE_COMPLETE, Kind::Boolean).unknown_capable(),
                    Field::new(f::MILITARY_LINES_COMPLETE, Kind::Boolean).unknown_capable(),
                    Field::new(f::NAVAL_LINES_COMPLETE, Kind::Boolean).unknown_capable(),
                    // Always determinable from the frame: the country is in the
                    // `PLIC` map or it is not, and the segment has one version.
                    Field::new(f::LICENSE_DETAIL, Kind::Enum)
                        .enum_card(LicenseDetail::CARDINALITY),
                ],
            },
            Collection {
                collection_id: STOCKPILE,
                shape: Shape::KeyedSet,
                fields: vec![
                    Field::new(f::TAG_DB_ID, Kind::ContentId).key(0),
                    // An equipment variant id, `ContentId` for d26's reason
                    // (`d26.rs:226`-`:239`): the replay layer cannot say what a
                    // variant *is* without a content-side catalog, and under
                    // another mod set the same number names other equipment.
                    Field::new(f::VARIANT_ID, Kind::ContentId).key(1),
                    // `Fixed(5)`, not `Exact`: an i64 over a 100_000 denominator
                    // is a decimal fixed-point source, so `fixed(5)` is the
                    // source's own grid rather than a precision budget, and it
                    // recovers the raw integer the dossier requires
                    // ("Preserve signed i64 raw values"). d24's rule; the escape
                    // rule (`value::escapes`) makes any value the grid cannot
                    // hold keep its raw f64, so the choice cannot lose data.
                    Field::new(f::AMOUNT, Kind::Continuous).quant(Quant::Fixed(5)),
                ],
            },
            Collection {
                collection_id: PRODUCTION_LINES,
                shape: Shape::KeyedSet,
                fields: vec![
                    Field::new(f::TAG_DB_ID, Kind::ContentId).key(0),
                    // An engine discriminator, stable in every campaign and every
                    // mod set and resolved by no manifest: `Unsigned`, exactly as
                    // d20 classes `div_type`.
                    Field::new(f::KIND, Kind::Unsigned).key(1),
                    // A runtime instance identity. `Signed` because the fold
                    // reads it with `read_i32` (`world_fold.rs:12574`) and
                    // rejects no sign.
                    Field::new(f::LINE_ID, Kind::Signed).key(2),
                    // Raw i32, no scale (`world_fold.rs::fold_production`). `Signed`
                    // because the fold range-checks nothing here.
                    Field::new(f::ACTIVE_FACTORIES, Kind::Signed),
                    Field::new(f::COST, Kind::Continuous).quant(Quant::Fixed(5)),
                    Field::new(f::SPEED, Kind::Continuous).quant(Quant::Fixed(5)),
                    Field::new(f::PRODUCED, Kind::Continuous).quant(Quant::Fixed(5)),
                    // The trap field: `+28` is emitted **raw** (`:12592`) while
                    // its three neighbours are `/100_000`. `Signed`, not
                    // `Continuous`.
                    Field::new(f::LINE_AMOUNT, Kind::Signed),
                    // `PVJ1`. Unknown-capable and nothing else: the join is
                    // all-or-nothing per frame and its absence is "could not
                    // prove the join", never "no variant".
                    Field::new(f::EQUIPMENT_VARIANT_ID, Kind::ContentId).unknown_capable(),
                    // `PFR1`, range-checked `0..=100_000` by the fold.
                    Field::new(f::REQUESTED_FACTORIES, Kind::Unsigned).unknown_capable(),
                    // `PPR1`, range-checked `0..=4095`. The engine's own line
                    // ordinal at line+0x44 — sparse, and Unknown without the
                    // segment. See the module docs on shapes.
                    Field::new(f::LINE_PRIORITY, Kind::Unsigned).unknown_capable(),
                    // The dense base-table position within this row's
                    // `(tag_db_id, kind)` run. Required and always concrete:
                    // it is what makes the fold's array order recoverable on a
                    // frame that carries lines and no `PPR1`, which the fold
                    // accepts.
                    Field::new(f::LINE_ORDINAL, Kind::Unsigned),
                    Field::new(f::LINE_SUBTYPE, Kind::Enum)
                        .enum_card(LineSubtype::CARDINALITY)
                        .unknown_capable(),
                    // `/10_000_000`, not `/100_000` (`world_fold.rs::fold_production`).
                    Field::new(f::EFFICIENCY, Kind::Continuous)
                        .quant(Quant::Fixed(7))
                        .unknown_capable(),
                    Field::new(f::EFFICIENCY_CAP, Kind::Continuous)
                        .quant(Quant::Fixed(7))
                        .unknown_capable(),
                    Field::new(f::EFFICIENCY_GROWTH, Kind::Continuous)
                        .quant(Quant::Fixed(7))
                        .unknown_capable(),
                    // The only field in the domain that reaches both empty
                    // states: a proven zero CID is Null, a suppressed segment is
                    // Unknown. An MIO is mod-defined content, so `ContentId`.
                    Field::new(f::MIO_ID, Kind::ContentId)
                        .nullable()
                        .unknown_capable(),
                ],
            },
            Collection {
                collection_id: CONSTRUCTION_QUEUE,
                shape: Shape::KeyedSet,
                fields: vec![
                    Field::new(f::TAG_DB_ID, Kind::ContentId).key(0),
                    // The dense per-country position in the base table. It leads
                    // `line_id` so the keyed set's own order is the fold's array
                    // order, and it is unique by construction, so no frame the
                    // fold accepts is ever refused for a key collision — which
                    // matters here because the fold enforces no construction key
                    // at all (`world_fold.rs::fold_production`).
                    Field::new(f::CONSTRUCTION_ORDINAL, Kind::Unsigned).key(1),
                    // `read_u32` (`world_fold.rs:12669`), so `Unsigned`.
                    Field::new(f::CONSTRUCTION_ID, Kind::Unsigned).key(2),
                    // The `+24` word: the collector's SPARSE source-slot index
                    // (`attach_log.rs:85364`). A value, not the ordering.
                    Field::new(f::CONSTRUCTION_PRIORITY, Kind::Unsigned),
                    // A construction template is mod-defined content.
                    Field::new(f::TEMPLATE_ID, Kind::ContentId).unknown_capable(),
                    Field::new(f::ACTIVE_FACTORIES, Kind::Signed),
                    Field::new(f::COST, Kind::Continuous).quant(Quant::Fixed(5)),
                    Field::new(f::PRODUCED, Kind::Continuous).quant(Quant::Fixed(5)),
                ],
            },
            Collection {
                collection_id: PRODUCTION_LICENSES,
                shape: Shape::KeyedSet,
                fields: vec![
                    Field::new(f::TAG_DB_ID, Kind::ContentId).key(0),
                    Field::new(f::VARIANT_ID, Kind::ContentId).key(1),
                    // v1 carries none of the three: the item is 4 bytes wide and
                    // physically cannot hold them (`world_fold.rs::parse_production_license_segment`), which
                    // is the recipe's legacy-row Unknown.
                    Field::new(f::VISIBLE_ORDER, Kind::Unsigned).unknown_capable(),
                    Field::new(f::CREATOR_TAG_DB_ID, Kind::ContentId).unknown_capable(),
                    // `attach_log.rs:69023`, verbatim: "`origin_tag_db_id == 0`
                    // means the engine's optional origin override is absent."
                    // That is record-state contract's null — the relation genuinely points at
                    // nothing — and it is safe from record-state contract's sentinel ban because
                    // `0` is not a country: every d11 parser rejects a zero tag.
                    // The fold emits the raw `0` (`world_fold.rs::fold_production`), which
                    // decides value-vs-nothing and nothing else.
                    Field::new(f::ORIGIN_TAG_DB_ID, Kind::ContentId)
                        .nullable()
                        .unknown_capable(),
                ],
            },
            Collection {
                collection_id: CONVOY_BANK,
                shape: Shape::KeyedSet,
                fields: vec![
                    Field::new(f::TAG_DB_ID, Kind::ContentId).key(0),
                    // Both witnesses are emitted unconditionally by the fold
                    // (`world_fold.rs::fold_production`), including for an
                    // unreadable country, so both are always concrete.
                    Field::new(f::CONVOYS_COMPLETE, Kind::Boolean),
                    Field::new(f::RESERVATIONS_COMPLETE, Kind::Boolean),
                    // Emitted only past the `!complete` early return (`:13006`).
                    Field::new(f::CACHE_VALID, Kind::Boolean).unknown_capable(),
                    // `assigned >= 0` is enforced on every row that carries one
                    // (`:27739`), so `Unsigned`.
                    Field::new(f::ASSIGNED, Kind::Unsigned).unknown_capable(),
                ],
            },
            Collection {
                collection_id: CONVOY_VARIANTS,
                shape: Shape::KeyedSet,
                fields: vec![
                    Field::new(f::TAG_DB_ID, Kind::ContentId).key(0),
                    Field::new(f::VARIANT_ID, Kind::ContentId).key(1),
                    // The fold emits `amount_raw` (exact i64) beside its `f64`
                    // projection (`world_fold.rs::fold_production`). `PCV1` only
                    // requires `amount_raw >= 0` (`:27646`) and the total law
                    // clamps the SUM at `OWNED_RAW_CAP` (`:27736`), so a single
                    // variant may legally carry a raw past 2^53 — which no
                    // `f64` can hold and no `Continuous` field could recover.
                    // The raw is carried; `amount` is recomputed from it, which
                    // is the d24 rule applied to the pair.
                    Field::new(f::CONVOY_AMOUNT_RAW, Kind::Unsigned),
                    // The run's order. `PCV1` enforces `variant_id` uniqueness
                    // within a complete country's run (`world_fold.rs::parse_country_convoy_bank_segment`),
                    // so the key is fold-enforced and the ordinal is a value —
                    // but the run's ORDER is enforced nowhere, so it has to be
                    // carried rather than inferred from the key.
                    Field::new(f::CONVOY_VARIANT_ORDINAL, Kind::Unsigned),
                ],
            },
            Collection {
                collection_id: CONVOY_PRIORITY_RESERVATIONS,
                // The fold stores each country's pointer-vector order
                // (`world_fold.rs::fold_production`). `priority` is not proven
                // unique by `PCV1`, so all three words are values and duplicate
                // rows remain legitimate occurrences.
                shape: Shape::OrderedOccurrenceList,
                fields: vec![
                    Field::new(f::TAG_DB_ID, Kind::ContentId),
                    // `PCV1` reads an i32 and places no range constraint on it
                    // (`world_fold.rs::parse_country_convoy_bank_segment`).
                    Field::new(f::RESERVATION_PRIORITY, Kind::Signed),
                    // Production rejects `count < 0` before the row reaches the
                    // fold (`world_fold.rs::parse_country_convoy_bank_segment`; mirrored by this
                    // module's `parse_convoy_bank_segment_leaf`), so the complete
                    // accepted range is exactly `0..=i32::MAX`.
                    Field::new(f::RESERVATION_COUNT, Kind::Unsigned),
                ],
            },
        ],
        event_part: None,
    }
}

/// The eight collections, resolved once so no caller re-derives the order.
#[derive(Clone, Debug)]
pub struct Collections {
    pub countries: Collection,
    pub stockpile: Collection,
    pub lines: Collection,
    pub construction: Collection,
    pub licenses: Collection,
    pub convoy_bank: Collection,
    pub convoy_variants: Collection,
    pub convoy_reservations: Collection,
}

impl Collections {
    /// Resolve by collection id, never by position.
    pub fn from_schema(schema: &Schema) -> core::result::Result<Self, String> {
        let find = |id: u32| -> core::result::Result<Collection, String> {
            schema
                .collections
                .iter()
                .find(|c| c.collection_id == id)
                .cloned()
                .ok_or_else(|| format!("production_live schema has no collection {id}"))
        };
        Ok(Self {
            countries: find(COUNTRIES)?,
            stockpile: find(STOCKPILE)?,
            lines: find(PRODUCTION_LINES)?,
            construction: find(CONSTRUCTION_QUEUE)?,
            licenses: find(PRODUCTION_LICENSES)?,
            convoy_bank: find(CONVOY_BANK)?,
            convoy_variants: find(CONVOY_VARIANTS)?,
            convoy_reservations: find(CONVOY_PRIORITY_RESERVATIONS)?,
        })
    }

    /// In `schema()` order, for the block encoders.
    pub fn as_slice(&self) -> [&Collection; 8] {
        [
            &self.countries,
            &self.stockpile,
            &self.lines,
            &self.construction,
            &self.licenses,
            &self.convoy_bank,
            &self.convoy_variants,
            &self.convoy_reservations,
        ]
    }

    /// The collection name of each slot, in the same order.
    pub const NAMES: [&'static str; 8] = [
        "countries",
        "stockpile",
        "production_lines",
        "construction_queue",
        "production_licenses",
        "convoy_bank",
        "convoy_variants",
        "convoy_priority_reservations",
    ];
}

impl Default for Collections {
    fn default() -> Self {
        Self::from_schema(&schema()).expect("the built-in schema has all eight collections")
    }
}

// --- The payload header ------------------------------------------------------

/// The 40-byte payload header (`world_fold.rs::fold_production`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub country_count: u32,
    pub changed_country_count: u32,
    pub status_flags: u32,
    /// `+20`. The collector writes `COMPACT_PRODUCTION_SCALE_RAW`
    /// (`attach_log.rs:68036`); **the fold never reads it** and uses its own
    /// `PRODUCTION_SCALE` constant, so this is carried for callers and enforced
    /// nowhere — mirroring the fold. See the deliverable's open questions.
    pub scale_raw: u32,
    pub stockpile_count: u32,
    pub military_count: u32,
    pub naval_count: u32,
    pub construction_count: u32,
}

impl PayloadHeader {
    pub fn line_count(&self) -> u64 {
        self.military_count as u64 + self.naval_count as u64
    }
}

/// The three base tables and the remaining trailer bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Tables<'a> {
    pub stockpile: &'a [u8],
    pub lines: &'a [u8],
    pub construction: &'a [u8],
    pub trailers: &'a [u8],
}

/// Read the header alone, applying only the check the fold applies **before** it
/// looks at the status flags (`world_fold.rs::fold_production`).
///
/// The split is not cosmetic. `fold_production` checks the payload length against
/// 40, then the failure status, and only then the row counts against `base_len`
/// (`world_fold.rs::fold_production` before `world_fold.rs::fold_production`). A harness that validated `base_len` first would
/// refuse a malformed failure-status record the recorder accepts and skips — a
/// stream the recorder reads and this arm does not.
pub fn split_header(payload: &[u8]) -> core::result::Result<PayloadHeader, String> {
    let header = d11_grammar::read_base_header(payload)
        .map_err(|violation| format_grammar_violation(payload, violation, None))?;
    Ok(PayloadHeader {
        elapsed_wall_ms: header.elapsed_wall_ms,
        country_count: header.country_count,
        changed_country_count: header.changed_rows,
        status_flags: header.status_flags,
        scale_raw: header.scale_raw,
        stockpile_count: header.stockpile_count,
        military_count: header.military_count,
        naval_count: header.naval_count,
        construction_count: header.construction_count,
    })
}

/// Split a d11 DATA payload into its header, its three base tables and the
/// trailer chain, applying the fold's structural checks
/// (`world_fold.rs::fold_production`).
///
/// Call only after the status check has passed — see [`split_header`].
///
/// `base_len` is **not** an exact length: the trailer chain follows it, and
/// `world_fold.rs::fold_production` requires only `payload.len() >= base_len`.
pub fn split_payload(payload: &[u8]) -> core::result::Result<(PayloadHeader, Tables<'_>), String> {
    let header = split_header(payload)?;
    let grammar_header = d11_grammar::read_base_header(payload)
        .map_err(|violation| format_grammar_violation(payload, violation, None))?;
    let layout = d11_grammar::base_layout(payload, &grammar_header)
        .map_err(|violation| format_grammar_violation(payload, violation, None))?;
    Ok((
        header,
        Tables {
            stockpile: &payload[layout.stockpiles.start..layout.stockpiles.end],
            lines: &payload[layout.lines.start..layout.lines.end],
            construction: &payload[layout.construction.start..layout.construction.end],
            trailers: &payload[layout.base_len..],
        },
    ))
}

// --- Trailers ----------------------------------------------------------------

/// The `(tag_db_id, kind, line_id)` identity every line join is keyed by
/// (`world_fold.rs::fold_production`).
pub type LineKey = (u32, u32, i32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Efficiency {
    pub efficiency_raw: i64,
    pub efficiency_cap_raw: i64,
    pub efficiency_growth_raw: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LicenseItem {
    pub variant_id: u32,
    /// `None` on a v1 segment, whose 4-byte item cannot carry it.
    pub visible_order: Option<u32>,
    pub creator_tag_db_id: Option<u32>,
    pub origin_tag_db_id: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LicenseSet {
    pub detail: LicenseDetail,
    pub items: Vec<LicenseItem>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConvoyVariant {
    pub variant_id: u32,
    pub amount_raw: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConvoyReservation {
    pub priority: i32,
    pub count: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConvoyBank {
    pub complete: bool,
    pub reservations_complete: bool,
    pub cache_valid: bool,
    pub assigned: i32,
    pub variants: Vec<ConvoyVariant>,
    pub reservations: Vec<ConvoyReservation>,
}

/// What the `PDQ1` structural scan retained. No conveyor, line or equipment row
/// is materialised — see the module docs' stop-report — but every identity and
/// count the fold cross-checks is, and so is the byte accounting the report
/// needs.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeploymentShape {
    pub elapsed_wall_ms: u64,
    pub changed_country_rows: u32,
    pub status_flags: u32,
    pub tags: Vec<u32>,
    pub complete_countries: u32,
    pub conveyor_rows: u32,
    pub line_rows: u32,
    pub equipment_rows: u32,
    /// Bytes of UTF-8 payload, excluding the four-byte length prefixes. The
    /// quantity schema-kind contract has no kind for.
    pub string_bytes: usize,
    /// Total bytes of the whole `PDQ1` segment, header included.
    pub segment_len: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Trailers {
    pub licenses_by_tag: Option<BTreeMap<u32, LicenseSet>>,
    pub variant_by_line: Option<BTreeMap<LineKey, u32>>,
    pub requested_factories_by_line: Option<BTreeMap<LineKey, i32>>,
    pub priority_by_line: Option<BTreeMap<LineKey, i32>>,
    pub subtype_by_line: Option<BTreeMap<LineKey, LineSubtype>>,
    pub efficiency_by_line: Option<BTreeMap<LineKey, Efficiency>>,
    pub mio_by_line: Option<BTreeMap<LineKey, u32>>,
    pub completeness_by_tag: Option<BTreeMap<u32, u32>>,
    pub convoy_bank_by_tag: Option<BTreeMap<u32, ConvoyBank>>,
    pub deployment: Option<DeploymentShape>,
}

/// The line joins, borrowed for one row's mapping.
#[derive(Clone, Copy, Debug, Default)]
pub struct LineJoins<'a> {
    pub variant: Option<&'a BTreeMap<LineKey, u32>>,
    pub requested_factories: Option<&'a BTreeMap<LineKey, i32>>,
    pub priority: Option<&'a BTreeMap<LineKey, i32>>,
    pub subtype: Option<&'a BTreeMap<LineKey, LineSubtype>>,
    pub efficiency: Option<&'a BTreeMap<LineKey, Efficiency>>,
    pub mio: Option<&'a BTreeMap<LineKey, u32>>,
}

impl Trailers {
    pub fn line_joins(&self) -> LineJoins<'_> {
        LineJoins {
            variant: self.variant_by_line.as_ref(),
            requested_factories: self.requested_factories_by_line.as_ref(),
            priority: self.priority_by_line.as_ref(),
            subtype: self.subtype_by_line.as_ref(),
            efficiency: self.efficiency_by_line.as_ref(),
            mio: self.mio_by_line.as_ref(),
        }
    }
}

/// The two status predicates the `PDQ1` segment check needs
/// (`world_fold.rs::parse_production_deployment_queue_segment`).
///
/// They are function pointers rather than calls because they live in
/// `compact_contract` and d11 is not admitted to the production-codec
/// dependency surface. Copying the bit masks in here is exactly the drift the
/// recipe forbids.
#[derive(Clone, Copy)]
pub struct StatusPredicates {
    pub unknown_flags: fn(u32) -> u32,
    pub is_failure: fn(u32) -> bool,
}

impl StatusPredicates {
    /// For callers and tests with no compact stream in hand: no bit is unknown
    /// and no bit is a failure. Never use this on a real stream.
    pub const PERMISSIVE: Self = Self {
        unknown_flags: |_| 0,
        is_failure: |_| false,
    };
}

/// Parse the trailer chain, mirroring `parse_production_trailers`
/// (`world_fold.rs::parse_production_trailers`): a strict segment chain, one of each magic at
/// most, an unknown magic refusing the whole payload, and `PDQ1` last.
pub fn parse_trailers(
    trailers: &[u8],
    status: StatusPredicates,
) -> core::result::Result<Trailers, String> {
    let mut out = Trailers::default();
    let mut cursor = d11_grammar::TrailerCursor::for_trailer_slice();
    loop {
        let step = cursor
            .next(trailers)
            .map_err(|violation| format_grammar_violation(trailers, violation, None))?;
        let d11_grammar::TrailerStep::Magic { trailer, .. } = step else {
            return Ok(out);
        };
        let preview = cursor.peek_header(trailers).ok();
        match trailer {
            d11_grammar::Trailer::Plic => {
                let body = cursor
                    .read_body_recoverable(trailers)
                    .map_err(|violation| format_grammar_violation(trailers, violation, preview))?;
                let d11_grammar::TrailerBody::Plic(body) = body else {
                    unreachable!()
                };
                let (rows, resume) = parse_license_segment_leaf(trailers, body)?;
                out.licenses_by_tag = Some(rows);
                cursor = resume.resume();
            }
            d11_grammar::Trailer::Pvj1 => {
                let body = read_fixed_body(trailers, cursor, preview, d11_grammar::Trailer::Pvj1)?;
                let (rows, resume) = parse_pvj1_segment_leaf(trailers, body)?;
                out.variant_by_line = Some(rows);
                cursor = resume.resume();
            }
            d11_grammar::Trailer::Pfr1 => {
                let body = read_fixed_body(trailers, cursor, preview, d11_grammar::Trailer::Pfr1)?;
                let (rows, resume) = parse_pfr1_segment_leaf(trailers, body)?;
                out.requested_factories_by_line = Some(rows);
                cursor = resume.resume();
            }
            d11_grammar::Trailer::Ppr1 => {
                let body = read_fixed_body(trailers, cursor, preview, d11_grammar::Trailer::Ppr1)?;
                let (rows, resume) = parse_ppr1_segment_leaf(trailers, body)?;
                out.priority_by_line = Some(rows);
                cursor = resume.resume();
            }
            d11_grammar::Trailer::Pls1 => {
                let body = read_fixed_body(trailers, cursor, preview, d11_grammar::Trailer::Pls1)?;
                let (rows, resume) = parse_pls1_segment_leaf(trailers, body)?;
                out.subtype_by_line = Some(rows);
                cursor = resume.resume();
            }
            d11_grammar::Trailer::Pef1 => {
                let body = read_fixed_body(trailers, cursor, preview, d11_grammar::Trailer::Pef1)?;
                let (rows, resume) = parse_pef1_segment_leaf(trailers, body)?;
                out.efficiency_by_line = Some(rows);
                cursor = resume.resume();
            }
            d11_grammar::Trailer::Pmi1 => {
                let body = read_fixed_body(trailers, cursor, preview, d11_grammar::Trailer::Pmi1)?;
                let (rows, resume) = parse_pmi1_segment_leaf(trailers, body)?;
                out.mio_by_line = Some(rows);
                cursor = resume.resume();
            }
            d11_grammar::Trailer::Pcp1 => {
                let body = read_fixed_body(trailers, cursor, preview, d11_grammar::Trailer::Pcp1)?;
                let (rows, resume) = parse_pcp1_segment_leaf(trailers, body)?;
                out.completeness_by_tag = Some(rows);
                cursor = resume.resume();
            }
            d11_grammar::Trailer::Pcv1 => {
                let body = cursor
                    .read_body(trailers)
                    .map_err(|violation| format_grammar_violation(trailers, violation, preview))?;
                let d11_grammar::TrailerBody::Pcv1(body) = body else {
                    unreachable!()
                };
                let (rows, resume) = parse_convoy_bank_segment_leaf(trailers, body)?;
                out.convoy_bank_by_tag = Some(rows);
                cursor = resume.resume();
            }
            d11_grammar::Trailer::Pdq1 => {
                let body = cursor
                    .read_body(trailers)
                    .map_err(|violation| format_grammar_violation(trailers, violation, preview))?;
                let d11_grammar::TrailerBody::Pdq1(body) = body else {
                    unreachable!()
                };
                let (shape, resume) = parse_deployment_segment_leaf(trailers, body, status)?;
                // `world_fold.rs::parse_production_trailers`.
                if resume.end() != trailers.len() {
                    return Err("PDQ1 must be the final D11 trailer".into());
                }
                out.deployment = Some(shape);
                cursor = resume.resume();
            }
        }
    }
}

fn format_header_shape(payload: &[u8], preview: d11_grammar::HeaderPreview) -> String {
    let available = payload.len().saturating_sub(preview.at);
    match preview.trailer {
        d11_grammar::Trailer::Plic => format!(
            "production license trailer header shape invalid (version {}, header_len {}, reserved {})",
            preview.version,
            preview.header_len,
            preview.reserved()
        ),
        d11_grammar::Trailer::Pdq1 => format!(
            "PDQ1 header shape invalid (version {}, header_len {}, segment_len {}, codec_payload_len {}, reserved {}, available {available})",
            preview.version,
            preview.header_len,
            preview.words[0],
            preview.words[1],
            preview.reserved()
        ),
        trailer => format!(
            "{} header shape invalid (version {}, header_len {}, segment_len {}, expected {}, reserved {}, available {available})",
            trailer.label(),
            preview.version,
            preview.header_len,
            preview.words[0],
            preview.computed_len(),
            preview.reserved()
        ),
    }
}

fn format_grammar_violation(
    payload: &[u8],
    violation: d11_grammar::Violation,
    preview: Option<d11_grammar::HeaderPreview>,
) -> String {
    use d11_grammar::{ArithmeticOp, CallerDefect, Section, ViolationKind};
    match violation.kind {
        ViolationKind::Need {
            section,
            needed,
            available,
        } => match section {
            Section::BaseHeader => format!(
                "production payload {} shorter than the {HEADER_LEN}-byte header",
                payload.len()
            ),
            Section::BaseStockpiles | Section::BaseLines | Section::BaseConstruction => {
                let base_len = d11_grammar::read_base_header(payload)
                    .ok()
                    .and_then(|header| d11_grammar::derive_base_layout(&header).ok())
                    .map_or(needed, |layout| layout.base_len);
                format!("production len {available} < base {base_len}")
            }
            Section::TrailerMagic => "production trailer magic".to_string(),
            Section::TrailerHeader(trailer) => {
                let available = payload.len().saturating_sub(violation.at);
                match trailer {
                    d11_grammar::Trailer::Plic => format!(
                        "production license trailer is {available} byte(s), expected at least {}",
                        trailer.header_len()
                    ),
                    d11_grammar::Trailer::Pvj1
                    | d11_grammar::Trailer::Pfr1
                    | d11_grammar::Trailer::Ppr1
                    | d11_grammar::Trailer::Pls1
                    | d11_grammar::Trailer::Pef1
                    | d11_grammar::Trailer::Pmi1
                    | d11_grammar::Trailer::Pcp1
                    | d11_grammar::Trailer::Pcv1
                    | d11_grammar::Trailer::Pdq1 => format!(
                        "{} trailer is {available} byte(s), expected at least {}",
                        trailer.label(),
                        trailer.header_len()
                    ),
                }
            }
            Section::TrailerBody(d11_grammar::Trailer::Plic) => {
                let expected = preview.map_or(needed.saturating_sub(violation.at), |value| {
                    value.computed_len()
                });
                format!(
                    "production license trailer len {} < {expected}",
                    payload.len().saturating_sub(violation.at)
                )
            }
            Section::TrailerBody(_) => preview
                .map(|value| format_header_shape(payload, value))
                .unwrap_or_else(|| format!("production len {available} < base {needed}")),
        },
        ViolationKind::UnknownTrailerMagic { .. } => {
            "production trailing bytes are not a known D11 trailer".to_string()
        }
        ViolationKind::DuplicateTrailer { trailer, .. } => {
            format!("production has duplicate {} trailer", trailer.label())
        }
        ViolationKind::UnknownVersion { .. }
        | ViolationKind::HeaderLen { .. }
        | ViolationKind::ReservedWord { .. }
        | ViolationKind::DeclaredExtent { .. } => preview
            .map(|value| format_header_shape(payload, value))
            .unwrap_or_else(|| "production trailer header shape invalid".to_string()),
        ViolationKind::PlicItemCount {
            declared_items,
            summed_items,
        } => format!(
            "production license id total {summed_items} != id_rows {declared_items}"
        ),
        ViolationKind::ArithmeticOverflow {
            section,
            operation,
            right,
            ..
        } => match (section, operation, right) {
            (Section::BaseStockpiles, ArithmeticOp::Multiply, _) => {
                "production stockpile_count overflows".to_string()
            }
            (Section::BaseLines, ArithmeticOp::Add, _) => {
                "production line counts overflow".to_string()
            }
            (Section::BaseLines, ArithmeticOp::Multiply, _) => {
                "production line_count overflows".to_string()
            }
            (Section::BaseConstruction, ArithmeticOp::Multiply, _) => {
                "production construction_count overflows".to_string()
            }
            (Section::BaseStockpiles | Section::BaseLines | Section::BaseConstruction, _, _) => {
                "production base length overflows".to_string()
            }
            (Section::TrailerBody(d11_grammar::Trailer::Plic), _, _) => {
                "production license trailer size overflow".to_string()
            }
            (Section::TrailerBody(d11_grammar::Trailer::Pcv1), _, _)
            | (Section::TrailerBody(d11_grammar::Trailer::Pdq1), _, _) => {
                let trailer = match section {
                    Section::TrailerBody(value) => value,
                    _ => unreachable!(),
                };
                format!("{} segment size overflow", trailer.label())
            }
            (Section::TrailerBody(_), _, _) | (Section::TrailerMagic, _, _)
            | (Section::TrailerHeader(_), _, _) | (Section::BaseHeader, _, _) => {
                "production segment size overflow".to_string()
            }
        },
        ViolationKind::CallerDefect(defect) => match defect {
            CallerDefect::BodyNotDelimited => {
                "production trailer body was not delimited".to_string()
            }
            CallerDefect::BlockNotConsumed {
                owner,
                block,
                remaining,
            } => format!(
                "production caller did not consume {remaining} row(s) from {owner:?} {}",
                block.name()
            ),
        },
    }
}

fn read_fixed_body(
    payload: &[u8],
    cursor: d11_grammar::TrailerCursor,
    preview: Option<d11_grammar::HeaderPreview>,
    expected: d11_grammar::Trailer,
) -> core::result::Result<d11_grammar::FixedCursor, String> {
    let body = cursor
        .read_body(payload)
        .map_err(|violation| format_grammar_violation(payload, violation, preview))?;
    let (actual, body) = match body {
        d11_grammar::TrailerBody::Pvj1(body) => (d11_grammar::Trailer::Pvj1, body),
        d11_grammar::TrailerBody::Pfr1(body) => (d11_grammar::Trailer::Pfr1, body),
        d11_grammar::TrailerBody::Ppr1(body) => (d11_grammar::Trailer::Ppr1, body),
        d11_grammar::TrailerBody::Pls1(body) => (d11_grammar::Trailer::Pls1, body),
        d11_grammar::TrailerBody::Pef1(body) => (d11_grammar::Trailer::Pef1, body),
        d11_grammar::TrailerBody::Pmi1(body) => (d11_grammar::Trailer::Pmi1, body),
        d11_grammar::TrailerBody::Pcp1(body) => (d11_grammar::Trailer::Pcp1, body),
        d11_grammar::TrailerBody::Plic(_)
        | d11_grammar::TrailerBody::Pcv1(_)
        | d11_grammar::TrailerBody::Pdq1(_) => unreachable!(),
    };
    debug_assert_eq!(actual, expected);
    Ok(body)
}

fn parse_line_value_segment_leaf<T>(
    payload: &[u8],
    body: d11_grammar::FixedCursor,
    read: impl Fn(&[u8], usize) -> core::result::Result<(d11_grammar::LineKey, T), d11_grammar::Violation>,
    check: impl Fn(usize, T) -> core::result::Result<T, String>,
) -> core::result::Result<(BTreeMap<LineKey, T>, d11_grammar::TrailerResume), String> {
    let magic = body.header().trailer.label();
    let mut out = BTreeMap::new();
    let mut rows = body.rows();
    while let d11_grammar::RowStep::Row {
        index: row_index,
        at,
    } = rows.next()
    {
        let (key, value) = read(payload, at)
            .map_err(|violation| format_grammar_violation(payload, violation, None))?;
        if key.tag_db_id == 0 {
            return Err(format!("{magic} row {row_index} has zero tag_db_id"));
        }
        if key.kind != KIND_MILITARY && key.kind != KIND_NAVAL {
            return Err(format!("{magic} row {row_index} has invalid kind {}", key.kind));
        }
        let value = check(row_index, value)?;
        if out
            .insert((key.tag_db_id, key.kind, key.line_id), value)
            .is_some()
        {
            return Err(format!(
                "{magic} duplicates tag {} kind {} line_id {}",
                key.tag_db_id, key.kind, key.line_id
            ));
        }
    }
    let resume = rows
        .into_end()
        .map_err(|violation| format_grammar_violation(payload, violation, None))?
        .next;
    Ok((out, resume))
}

fn parse_pvj1_segment_leaf(
    payload: &[u8],
    body: d11_grammar::FixedCursor,
) -> core::result::Result<(BTreeMap<LineKey, u32>, d11_grammar::TrailerResume), String> {
    parse_line_value_segment_leaf(
        payload,
        body,
        |payload, at| d11_grammar::pvj_row_at(payload, at).map(|row| (row.key, row.equipment_variant_id)),
        |row_index, value| {
            if value == 0 || value > i32::MAX as u32 {
                return Err(format!(
                    "PVJ1 row {row_index} has invalid equipment_variant_id {value}"
                ));
            }
            Ok(value)
        },
    )
}

fn parse_pfr1_segment_leaf(
    payload: &[u8],
    body: d11_grammar::FixedCursor,
) -> core::result::Result<(BTreeMap<LineKey, i32>, d11_grammar::TrailerResume), String> {
    parse_line_value_segment_leaf(
        payload,
        body,
        |payload, at| d11_grammar::pfr_row_at(payload, at).map(|row| (row.key, row.requested_factories)),
        |row_index, value| {
            if !(0..=MAX_REQUESTED_FACTORIES).contains(&value) {
                return Err(format!(
                    "PFR1 row {row_index} has invalid requested_factories {value}"
                ));
            }
            Ok(value)
        },
    )
}

fn parse_ppr1_segment_leaf(
    payload: &[u8],
    body: d11_grammar::FixedCursor,
) -> core::result::Result<(BTreeMap<LineKey, i32>, d11_grammar::TrailerResume), String> {
    parse_line_value_segment_leaf(
        payload,
        body,
        |payload, at| d11_grammar::ppr_row_at(payload, at).map(|row| (row.key, row.priority)),
        |row_index, value| {
            if !(0..=MAX_LINE_PRIORITY).contains(&value) {
                return Err(format!("PPR1 row {row_index} has invalid priority {value}"));
            }
            Ok(value)
        },
    )
}

fn parse_pmi1_segment_leaf(
    payload: &[u8],
    body: d11_grammar::FixedCursor,
) -> core::result::Result<(BTreeMap<LineKey, u32>, d11_grammar::TrailerResume), String> {
    parse_line_value_segment_leaf(
        payload,
        body,
        |payload, at| d11_grammar::pmi_row_at(payload, at).map(|row| (row.key, row.mio_id)),
        |_, value| Ok(value),
    )
}

fn parse_pls1_segment_leaf(
    payload: &[u8],
    body: d11_grammar::FixedCursor,
) -> core::result::Result<
    (BTreeMap<LineKey, LineSubtype>, d11_grammar::TrailerResume),
    String,
> {
    let mut out = BTreeMap::new();
    let mut rows = body.rows();
    while let d11_grammar::RowStep::Row {
        index: row_index,
        at,
    } = rows.next()
    {
        let row = d11_grammar::pls_row_at(payload, at)
            .map_err(|violation| format_grammar_violation(payload, violation, None))?;
        let key = row.key;
        if key.tag_db_id == 0 {
            return Err(format!("PLS1 row {row_index} has zero tag_db_id"));
        }
        let subtype = LineSubtype::from_wire(key.kind, row.subtype).ok_or_else(|| {
            format!(
                "PLS1 row {row_index} has incoherent kind/subtype ({}, {})",
                key.kind, row.subtype
            )
        })?;
        if out
            .insert((key.tag_db_id, key.kind, key.line_id), subtype)
            .is_some()
        {
            return Err(format!(
                "PLS1 duplicates tag {} kind {} line_id {}",
                key.tag_db_id, key.kind, key.line_id
            ));
        }
    }
    let resume = rows
        .into_end()
        .map_err(|violation| format_grammar_violation(payload, violation, None))?
        .next;
    Ok((out, resume))
}

fn parse_pef1_segment_leaf(
    payload: &[u8],
    body: d11_grammar::FixedCursor,
) -> core::result::Result<
    (BTreeMap<LineKey, Efficiency>, d11_grammar::TrailerResume),
    String,
> {
    let mut out = BTreeMap::new();
    let mut rows = body.rows();
    while let d11_grammar::RowStep::Row {
        index: row_index,
        at,
    } = rows.next()
    {
        let row = d11_grammar::pef_row_at(payload, at)
            .map_err(|violation| format_grammar_violation(payload, violation, None))?;
        let key = row.key;
        if key.tag_db_id == 0 || (key.kind != KIND_MILITARY && key.kind != KIND_NAVAL) {
            return Err(format!(
                "PEF1 row {row_index} has invalid tag/kind ({}, {})",
                key.tag_db_id, key.kind
            ));
        }
        if row.efficiency_raw < 0
            || row.efficiency_cap_raw < 0
            || row.efficiency_growth_raw < 0
        {
            return Err(format!(
                "PEF1 row {row_index} has negative efficiency values ({}, {}, {})",
                row.efficiency_raw, row.efficiency_cap_raw, row.efficiency_growth_raw
            ));
        }
        let values = Efficiency {
            efficiency_raw: row.efficiency_raw,
            efficiency_cap_raw: row.efficiency_cap_raw,
            efficiency_growth_raw: row.efficiency_growth_raw,
        };
        if out
            .insert((key.tag_db_id, key.kind, key.line_id), values)
            .is_some()
        {
            return Err(format!(
                "PEF1 duplicates tag {} kind {} line_id {}",
                key.tag_db_id, key.kind, key.line_id
            ));
        }
    }
    let resume = rows
        .into_end()
        .map_err(|violation| format_grammar_violation(payload, violation, None))?
        .next;
    Ok((out, resume))
}

fn parse_pcp1_segment_leaf(
    payload: &[u8],
    body: d11_grammar::FixedCursor,
) -> core::result::Result<(BTreeMap<u32, u32>, d11_grammar::TrailerResume), String> {
    let mut out = BTreeMap::new();
    let mut rows = body.rows();
    while let d11_grammar::RowStep::Row {
        index: row_index,
        at,
    } = rows.next()
    {
        let row = d11_grammar::pcp_row_at(payload, at)
            .map_err(|violation| format_grammar_violation(payload, violation, None))?;
        if row.tag_db_id == 0 {
            return Err(format!("PCP1 row {row_index} has zero tag_db_id"));
        }
        if row.flags & !PCP1_KNOWN_FLAGS != 0 {
            return Err(format!(
                "PCP1 row {row_index} has unknown flags {:#x}",
                row.flags
            ));
        }
        if out.insert(row.tag_db_id, row.flags).is_some() {
            return Err(format!("PCP1 duplicates tag_db_id {}", row.tag_db_id));
        }
    }
    let resume = rows
        .into_end()
        .map_err(|violation| format_grammar_violation(payload, violation, None))?
        .next;
    Ok((out, resume))
}

fn parse_license_segment_leaf(
    payload: &[u8],
    body: d11_grammar::PlicCursor,
) -> core::result::Result<
    (BTreeMap<u32, LicenseSet>, d11_grammar::TrailerResume),
    String,
> {
    let header = *body.header();
    let version = header.version;
    let detail = match version {
        d11_grammar::PlicVersion::V1 => LicenseDetail::IdsOnly,
        d11_grammar::PlicVersion::V2 => LicenseDetail::IdsAndVariants,
    };
    let mut counts = Vec::with_capacity(header.country_count);
    let mut seen_country = BTreeSet::new();
    let mut countries = body.countries();
    while let d11_grammar::RowStep::Row { at, .. } = countries.next() {
        let row = d11_grammar::plic_country_row_at(payload, at)
            .map_err(|violation| format_grammar_violation(payload, violation, None))?;
        if row.tag_db_id == 0 {
            return Err("production license country row has zero tag_db_id".into());
        }
        if !seen_country.insert(row.tag_db_id) {
            return Err(format!(
                "production license trailer duplicates tag_db_id {}",
                row.tag_db_id
            ));
        }
        counts.push((row.tag_db_id, row.count as usize));
    }
    let countries_end = countries
        .into_end()
        .map_err(|violation| format_grammar_violation(payload, violation, None))?;
    if header.summed_item_count != header.item_count {
        return Err(format!(
            "production license id total {} != id_rows {}",
            header.summed_item_count, header.item_count
        ));
    }
    let mut items = countries_end.next.items();
    let mut out = BTreeMap::new();
    for (tag_db_id, count) in counts {
        let mut license_items = Vec::with_capacity(count);
        let mut seen_ids = BTreeSet::new();
        for expected_order in 0..count {
            let d11_grammar::RowStep::Row { at, .. } = items.next() else {
                unreachable!()
            };
            let (variant_id, visible_order, creator, origin) = match version {
                d11_grammar::PlicVersion::V1 => {
                    let row = d11_grammar::plic_v1_item_at(payload, at)
                        .map_err(|violation| format_grammar_violation(payload, violation, None))?;
                    (row.variant_id, None, None, None)
                }
                d11_grammar::PlicVersion::V2 => {
                    let row = d11_grammar::plic_v2_item_at(payload, at)
                        .map_err(|violation| format_grammar_violation(payload, violation, None))?;
                    (
                        row.variant_id,
                        Some(row.visible_order),
                        Some(row.creator_tag_db_id),
                        Some(row.origin_tag_db_id),
                    )
                }
            };
            if variant_id == 0 || variant_id >= MAX_LICENSE_VARIANT_ID {
                return Err(format!(
                    "production license id {variant_id} for tag {tag_db_id} out of range"
                ));
            }
            if !seen_ids.insert(variant_id) {
                return Err(format!(
                    "production license tag {tag_db_id} duplicates variant_id {variant_id}"
                ));
            }
            if let (Some(order), Some(creator), Some(origin)) =
                (visible_order, creator, origin)
            {
                if order != expected_order as u32
                    || creator == 0
                    || creator > MAX_PRODUCTION_LICENSE_TAG_DB_ID
                    || origin > MAX_PRODUCTION_LICENSE_TAG_DB_ID
                {
                    return Err(format!(
                        "production license v2 tag {tag_db_id} row {expected_order} has invalid order/creator/origin ({order}, {creator}, {origin})"
                    ));
                }
            }
            license_items.push(LicenseItem {
                variant_id,
                visible_order,
                creator_tag_db_id: creator,
                origin_tag_db_id: origin,
            });
        }
        if version == d11_grammar::PlicVersion::V1 {
            license_items.sort_unstable_by_key(|item| item.variant_id);
        }
        out.insert(
            tag_db_id,
            LicenseSet {
                detail,
                items: license_items,
            },
        );
    }
    let resume = items
        .into_end()
        .map_err(|violation| format_grammar_violation(payload, violation, None))?
        .next;
    Ok((out, resume))
}

fn parse_convoy_bank_segment_leaf(
    payload: &[u8],
    body: d11_grammar::PcvCursor,
) -> core::result::Result<
    (BTreeMap<u32, ConvoyBank>, d11_grammar::TrailerResume),
    String,
> {
    let header = *body.header();
    let mut all_variants = Vec::with_capacity(header.variant_count);
    for row in header.variant_rows(payload) {
        let (row_index, row) =
            row.map_err(|violation| format_grammar_violation(payload, violation, None))?;
        if row.variant_id == 0 || row.equip_type != PCV1_EQUIP_TYPE || row.amount_raw < 0 {
            return Err(format!(
                "PCV1 variant row {row_index} invalid id/type/amount ({}/{}/{})",
                row.variant_id, row.equip_type, row.amount_raw
            ));
        }
        all_variants.push(ConvoyVariant {
            variant_id: row.variant_id,
            amount_raw: row.amount_raw,
        });
    }
    let mut all_reservations = Vec::with_capacity(header.reservation_count);
    for row in header.reservation_rows(payload) {
        let (row_index, row) =
            row.map_err(|violation| format_grammar_violation(payload, violation, None))?;
        if row.count < 0 {
            return Err(format!(
                "PCV1 reservation row {row_index} has negative count"
            ));
        }
        all_reservations.push(ConvoyReservation {
            priority: row.priority,
            count: row.count,
        });
    }

    let mut rows_by_tag = BTreeMap::new();
    let mut countries = body.countries();
    loop {
        let (row_index, row, noncanonical_ranges) = match countries
            .next(payload)
            .map_err(|violation| format_grammar_violation(payload, violation, None))?
        {
            d11_grammar::PcvCountryStep::Row { index, row, .. } => (index, row, false),
            d11_grammar::PcvCountryStep::RecoverableRow { index, row, .. } => {
                (index, row, true)
            }
            d11_grammar::PcvCountryStep::RecoverableFinal { .. } => {
                return Err("PCV1 ranges do not cover all rows".into());
            }
            d11_grammar::PcvCountryStep::End(_) => break,
        };
        if row.tag_db_id == 0 || row.flags & !PCV1_KNOWN_FLAGS != 0 {
            return Err(format!("PCV1 country row {row_index} invalid tag/flags"));
        }
        if noncanonical_ranges {
            return Err(format!(
                "PCV1 country row {row_index} has noncanonical ranges"
            ));
        }
        let variant_start = row.variant_start as usize;
        let variant_len = row.variant_len as usize;
        let reservation_start = row.reservation_start as usize;
        let reservation_len = row.reservation_len as usize;
        let complete = row.flags & PCV1_COMPLETE != 0;
        let reservations_complete = row.flags & PCV1_RESERVATIONS_COMPLETE != 0;
        let cache_valid = row.flags & PCV1_CACHE_VALID != 0;
        let cache_verified = row.flags & PCV1_CACHE_VERIFIED != 0;
        let classifier_exact = row.flags & PCV1_CLASSIFIER_EXACT != 0;
        if !complete {
            if row.flags != 0
                || row.owned_raw != 0
                || row.assigned != 0
                || row.available != 0
                || variant_len != 0
                || reservation_len != 0
            {
                return Err(format!(
                    "PCV1 unreadable country row {row_index} carries data"
                ));
            }
        } else if !classifier_exact
            || cache_valid != cache_verified
            || (!reservations_complete && reservation_len != 0)
        {
            return Err(format!(
                "PCV1 country row {row_index} has inconsistent witnesses"
            ));
        }
        let variants = all_variants[variant_start..variant_start + variant_len].to_vec();
        let reservations =
            all_reservations[reservation_start..reservation_start + reservation_len].to_vec();
        if complete {
            let mut seen = BTreeSet::new();
            let sum_raw = variants.iter().try_fold(0i64, |sum, variant| {
                if !seen.insert(variant.variant_id) {
                    return None;
                }
                Some(
                    sum.saturating_add(variant.amount_raw)
                        .min(PCV1_OWNED_RAW_CAP),
                )
            });
            if sum_raw != Some(row.owned_raw)
                || row.assigned < 0
                || i64::from(row.assigned) > row.owned_raw / PCV1_SCALE
                || i64::from(row.available)
                    != row.owned_raw / PCV1_SCALE - i64::from(row.assigned)
                || reservations
                    .iter()
                    .any(|reservation| i64::from(reservation.count) > row.owned_raw / PCV1_SCALE)
            {
                return Err(format!(
                    "PCV1 country row {row_index} fails exact total law"
                ));
            }
        }
        if rows_by_tag
            .insert(
                row.tag_db_id,
                ConvoyBank {
                    complete,
                    reservations_complete,
                    cache_valid,
                    assigned: row.assigned,
                    variants,
                    reservations,
                },
            )
            .is_some()
        {
            return Err(format!("PCV1 duplicates tag_db_id {}", row.tag_db_id));
        }
    }
    let countries_end = match countries
        .into_end()
        .map_err(|violation| format_grammar_violation(payload, violation, None))?
    {
        d11_grammar::PcvCountryEnd::Complete(end) => end,
        d11_grammar::PcvCountryEnd::Recoverable { .. } => {
            return Err("PCV1 ranges do not cover all rows".into());
        }
    };
    // Values were read through header views; discard both owned blocks so omission stays a CallerDefect.
    let variants_end = countries_end.next.variants().discard_all();
    let reservations_end = variants_end.next.reservations().discard_all();
    Ok((rows_by_tag, reservations_end.next))
}

fn parse_deployment_segment_leaf(
    payload: &[u8],
    body: d11_grammar::PdqCursor,
    status: StatusPredicates,
) -> core::result::Result<(DeploymentShape, d11_grammar::TrailerResume), String> {
    let span = body.inner_payload_span();
    let codec_payload = &payload[span.start..span.end];
    let codec_preview = d11_grammar::deployment::read_header_preview(codec_payload)
        .map_err(format_deployment_violation)?;
    let required = d11_grammar::deployment::SchemaVersion::V2.wire();
    let floor = d11_grammar::deployment::SchemaVersion::V1.wire();
    if codec_preview.schema_version != required {
        return Err(format!(
            "PDQ1 requires deployment codec v{required}, got v{} (codec floor v{floor})",
            codec_preview.schema_version
        ));
    }
    let mut shape = scan_deployment_codec_leaf(codec_payload)?;
    shape.segment_len = body.header().extent.declared_len;
    let unknown = (status.unknown_flags)(shape.status_flags);
    if unknown != 0 {
        return Err(format!(
            "PDQ1 deployment carrier has unknown status flags {unknown:#x}"
        ));
    }
    if (status.is_failure)(shape.status_flags) && shape.complete_countries > 0 {
        return Err("PDQ1 failed deployment carrier contains a complete country queue".into());
    }
    // The nested leaf consumed the carrier; discard the outer block so omission stays a CallerDefect.
    let resume = body.inner_payload().discard_all().next;
    Ok((shape, resume))
}

fn format_deployment_violation(violation: d11_grammar::deployment::Violation) -> String {
    use d11_grammar::deployment::{Block, DqCallerDefect as CallerDefect, DqSection as Section, DqViolationKind as ViolationKind};
    match violation.kind {
        ViolationKind::Need {
            section,
            needed,
            available,
        } => {
            let label = match section {
                Section::Header => "deployment header",
                Section::Country => "deployment country row",
                Section::Conveyor => "deployment conveyor row",
                Section::ConveyorName => "deployment conveyor name",
                Section::ConveyorRole => "deployment conveyor role",
                Section::Line => "deployment line row",
                Section::Equipment => "deployment equipment row",
                Section::Archetype => "deployment archetype",
            };
            if matches!(
                section,
                Section::ConveyorName | Section::ConveyorRole | Section::Archetype
            ) {
                let declared_len = needed
                    .saturating_sub(violation.at)
                    .saturating_sub(d11_grammar::deployment::STRING_PREFIX_LEN);
                if declared_len > PDQ1_MAX_STRING_BYTES {
                    return format!(
                        "{label} length {declared_len} exceeds {PDQ1_MAX_STRING_BYTES}"
                    );
                }
            }
            if section == Section::Header {
                "deployment header".to_string()
            } else {
                format!("{label}: need {needed} bytes, have {available}")
            }
        }
        ViolationKind::UnknownSchema { version, .. } => format!(
            "PDQ1 requires deployment codec v{}, got v{version} (codec floor v{})",
            d11_grammar::deployment::SchemaVersion::V2.wire(),
            d11_grammar::deployment::SchemaVersion::V1.wire()
        ),
        ViolationKind::HeaderReserved { value, .. } => {
            format!("deployment invalid reserved {value}")
        }
        ViolationKind::CountCannotFit {
            block,
            remaining,
            minimum_row_len,
            ..
        } => {
            let label = match block {
                Block::Countries => "deployment country row",
                Block::Conveyors => "deployment conveyor row",
                Block::Lines => "deployment line row",
                Block::Equipment => "deployment equipment row",
            };
            format!(
                "{label}: need {} bytes, have {}",
                violation.at.saturating_add(minimum_row_len),
                violation.at.saturating_add(remaining)
            )
        }
        ViolationKind::TrailingBytes {
            consumed,
            available,
        } => format!("deployment trailing bytes: consumed {consumed} of {available}"),
        ViolationKind::ArithmeticOverflow { .. } => "deployment count overflow".to_string(),
        ViolationKind::CallerDefect(CallerDefect::BlockNotConsumed { block, remaining }) => {
            format!("deployment caller did not consume {remaining} {block:?} row(s)")
        }
    }
}

fn deployment_string_len(
    payload: &[u8],
    span: d11_grammar::Span,
    label: &str,
) -> core::result::Result<usize, String> {
    if span.len > PDQ1_MAX_STRING_BYTES {
        return Err(format!(
            "{label} length {} exceeds {PDQ1_MAX_STRING_BYTES}",
            span.len
        ));
    }
    core::str::from_utf8(&payload[span.start..span.end])
        .map_err(|_| format!("{label} is not UTF-8"))?;
    Ok(span.len)
}

fn scan_deployment_codec_leaf(
    payload: &[u8],
) -> core::result::Result<DeploymentShape, String> {
    use d11_grammar::deployment::{CountryFlag, EquipmentFlag, LineFlag};

    let country_cursor = d11_grammar::deployment::walk(payload)
        .map_err(format_deployment_violation)?;
    let header = *country_cursor.header();
    if header.changed_country_rows > header.country_count {
        return Err("deployment changed country count exceeds country count".into());
    }
    let mut country_cursor = country_cursor
        .precheck_declared_counts(payload)
        .map_err(format_deployment_violation)?;
    let mut shape = DeploymentShape {
        elapsed_wall_ms: header.elapsed_wall_ms,
        changed_country_rows: header.changed_country_rows,
        status_flags: header.status_flags,
        ..DeploymentShape::default()
    };
    let mut tags = BTreeSet::new();
    let mut conveyor_ids = BTreeSet::new();
    let mut line_ids = BTreeSet::new();
    loop {
        let (country_index, row, conveyors_block) = match country_cursor
            .next(payload)
            .map_err(format_deployment_violation)?
        {
            d11_grammar::deployment::CountryStep::Country {
                index,
                row,
                conveyors,
            } => (index, row, conveyors),
            d11_grammar::deployment::CountryStep::End(end) => {
                shape.conveyor_rows = end.totals.conveyors;
                shape.line_rows = end.totals.lines;
                shape.equipment_rows = end.totals.equipment;
                break;
            }
        };
        if row.tag_db_id == 0 || !tags.insert(row.tag_db_id) {
            return Err(format!(
                "deployment country {country_index} invalid/duplicate tag {}",
                row.tag_db_id
            ));
        }
        if row.flags & !d11_grammar::deployment::COUNTRY_KNOWN_FLAGS != 0 || row.reserved != 0 {
            return Err(format!(
                "deployment country {} invalid flags/reserved",
                row.tag_db_id
            ));
        }
        let queue_complete = row.flags & CountryFlag::QueueComplete.bit() != 0;
        if !queue_complete && row.conveyor_count != 0 {
            return Err(format!(
                "deployment unknown country {} carries conveyors",
                row.tag_db_id
            ));
        }
        if queue_complete {
            shape.complete_countries += 1;
        }
        shape.tags.push(row.tag_db_id);
        if row.conveyor_count > header.declared_conveyors.saturating_sub(shape.conveyor_rows) {
            return Err(format!(
                "deployment country {} conveyor count exceeds declared remainder",
                row.tag_db_id
            ));
        }
        let mut conveyors = conveyors_block
            .open(payload)
            .map_err(format_deployment_violation)?;
        while conveyors.remaining() != 0 {
            let fixed = conveyors.next(payload).map_err(format_deployment_violation)?;
            let row = fixed.row;
            if row.ordinal != fixed.index as u32
                || row.conveyor_id == 0
                || row.id_type != d11_grammar::deployment::CONVEYOR_ID_TYPE
                || !conveyor_ids.insert(row.conveyor_id)
                || row.template_id == 0
                || row.template_id_type != d11_grammar::deployment::TEMPLATE_ID_TYPE
                || row.amount < 0
                || row.flags & !d11_grammar::deployment::CONVEYOR_KNOWN_FLAGS != 0
            {
                return Err(format!(
                    "deployment invalid conveyor identity/body {}",
                    row.conveyor_id
                ));
            }
            let name = fixed
                .name
                .read_name(payload)
                .map_err(format_deployment_violation)?;
            shape.string_bytes +=
                deployment_string_len(payload, name.name, "deployment conveyor name")?;
            let role = name
                .role
                .read_role(payload)
                .map_err(format_deployment_violation)?;
            shape.string_bytes +=
                deployment_string_len(payload, role.row.role, "deployment conveyor role")?;
            shape.conveyor_rows = shape
                .conveyor_rows
                .checked_add(1)
                .ok_or_else(|| "deployment conveyor count overflow".to_string())?;
            if row.line_count > header.declared_lines.saturating_sub(shape.line_rows) {
                return Err(format!(
                    "deployment conveyor {} line count exceeds declared remainder",
                    row.conveyor_id
                ));
            }
            let mut lines = role.lines.open(payload).map_err(format_deployment_violation)?;
            while lines.remaining() != 0 {
                let fixed = lines.next(payload).map_err(format_deployment_violation)?;
                let row = fixed.row;
                if row.ordinal != fixed.index as u32
                    || row.line_id == 0
                    || row.id_type != d11_grammar::deployment::LINE_ID_TYPE
                    || !line_ids.insert(row.line_id)
                    || row.amount < 0
                    || row.flags & !d11_grammar::deployment::LINE_KNOWN_FLAGS != 0
                    || row.reserved != 0
                    || (row.flags & LineFlag::NameOrder.bit() == 0 && row.name_order != 0)
                {
                    return Err(format!("deployment invalid line identity/body {}", row.line_id));
                }
                check_optional_float(
                    row.flags,
                    LineFlag::Training.bit(),
                    f64::from_bits(row.training_bits),
                    "training",
                )?;
                check_optional_float(
                    row.flags,
                    LineFlag::MaxTraining.bit(),
                    f64::from_bits(row.max_training_bits),
                    "max_training",
                )?;
                for (flag, value, label) in [
                    (LineFlag::ManpowerHave.bit(), row.manpower_have, "manpower_have"),
                    (LineFlag::ManpowerNeed.bit(), row.manpower_need, "manpower_need"),
                    (LineFlag::MaxManpower.bit(), row.max_manpower, "max_manpower"),
                ] {
                    if row.flags & flag != 0 {
                        if value < 0 {
                            return Err(format!("deployment {label} is negative"));
                        }
                    } else if value != 0 {
                        return Err(format!(
                            "deployment {label} payload present without flag"
                        ));
                    }
                }
                shape.line_rows = shape
                    .line_rows
                    .checked_add(1)
                    .ok_or_else(|| "deployment line count overflow".to_string())?;
                if row.equipment_count
                    > header.declared_equipment.saturating_sub(shape.equipment_rows)
                {
                    return Err(format!(
                        "deployment line {} equipment count exceeds declared remainder",
                        row.line_id
                    ));
                }
                let mut equipment = fixed
                    .equipment
                    .open(payload)
                    .map_err(format_deployment_violation)?;
                while equipment.remaining() != 0 {
                    let item = equipment.next(payload).map_err(format_deployment_violation)?;
                    let row = item.row;
                    if let Some(span) = row.archetype {
                        shape.string_bytes +=
                            deployment_string_len(payload, span, "deployment archetype")?;
                    }
                    if row.ordinal != item.index as u32
                        || row.equipment_id == 0
                        || row.id_type != d11_grammar::deployment::EQUIPMENT_ID_TYPE
                        || row.flags & !d11_grammar::deployment::EQUIPMENT_KNOWN_FLAGS != 0
                        || !f64::from_bits(row.amount_bits).is_finite()
                        || f64::from_bits(row.amount_bits) < 0.0
                    {
                        return Err(format!(
                            "deployment invalid equipment row {}:{}",
                            row.equipment_id, row.id_type
                        ));
                    }
                    if row.flags & EquipmentFlag::Archetype.bit() == 0 && row.archetype.is_some() {
                        unreachable!();
                    }
                    shape.equipment_rows = shape
                        .equipment_rows
                        .checked_add(1)
                        .ok_or_else(|| "deployment equipment count overflow".to_string())?;
                    equipment = item.next;
                }
                lines = equipment
                    .into_end()
                    .map_err(format_deployment_violation)?;
            }
            conveyors = lines.into_end().map_err(format_deployment_violation)?;
        }
        country_cursor = conveyors
            .into_end()
            .map_err(format_deployment_violation)?;
    }
    if (shape.conveyor_rows, shape.line_rows, shape.equipment_rows)
        != (
            header.declared_conveyors,
            header.declared_lines,
            header.declared_equipment,
        )
    {
        return Err(format!(
            "deployment declared/observed count mismatch: declared {}/{}/{}, observed {}/{}/{}",
            header.declared_conveyors,
            header.declared_lines,
            header.declared_equipment,
            shape.conveyor_rows,
            shape.line_rows,
            shape.equipment_rows
        ));
    }
    Ok(shape)
}

fn check_optional_float(
    flags: u32,
    flag: u32,
    value: f64,
    label: &str,
) -> core::result::Result<(), String> {
    if flags & flag != 0 {
        if !value.is_finite() || value < 0.0 {
            return Err(format!("deployment {label} is invalid"));
        }
    } else if value.to_bits() != 0 {
        return Err(format!("deployment {label} payload present without flag"));
    }
    Ok(())
}

// --- Key extractors ----------------------------------------------------------

/// `world_fold.rs::fold_production`.
pub fn key_from_stockpile_row(row: &[u8]) -> core::result::Result<Key, String> {
    check_row_len(row, STOCKPILE_ROW_LEN, "stockpile")?;
    let row = d11_grammar::stockpile_row_at(row, 0)
        .map_err(|violation| format_grammar_violation(row, violation, None))?;
    Ok(key_from_stockpile_value(row))
}

fn key_from_stockpile_value(row: d11_grammar::StockpileRow) -> Key {
    Key(vec![row.tag_db_id as i128, row.variant_id as i128])
}

/// `world_fold.rs::fold_production`.
pub fn key_from_line_row(row: &[u8]) -> core::result::Result<Key, String> {
    let (tag, kind, line_id) = line_key_of_row(row)?;
    Ok(Key(vec![tag as i128, kind as i128, line_id as i128]))
}

fn key_from_line_value(row: d11_grammar::LineRow) -> Key {
    Key(vec![
        row.tag_db_id as i128,
        row.kind as i128,
        row.line_id as i128,
    ])
}

/// The `(tag, kind, line_id)` triple, with the fold's kind gate. The base table
/// itself does **not** reject a kind outside `{1, 2}` — `world_fold.rs::fold_production`
/// folds everything that is not `2` as military — so no kind check belongs here.
pub fn line_key_of_row(row: &[u8]) -> core::result::Result<LineKey, String> {
    check_row_len(row, LINE_ROW_LEN, "production line")?;
    let row = d11_grammar::line_row_at(row, 0)
        .map_err(|violation| format_grammar_violation(row, violation, None))?;
    Ok((row.tag_db_id, row.kind, row.line_id))
}

/// The `(tag_db_id, construction_ordinal, line_id)` construction key.
///
/// `ordinal` is the row's dense position within its country's run of the base
/// table, which the caller counts — [`fold_payload`] is the one that does, and
/// any other caller must count it the same way. The fold enforces no
/// construction key of its own (`world_fold.rs::fold_production`), so the ordinal
/// is what makes every frame the fold accepts expressible here; see the module
/// docs.
pub fn key_from_construction_row(row: &[u8], ordinal: u32) -> core::result::Result<Key, String> {
    check_row_len(row, CONSTRUCTION_ROW_LEN, "construction")?;
    let row = d11_grammar::construction_row_at(row, 0)
        .map_err(|violation| format_grammar_violation(row, violation, None))?;
    Ok(key_from_construction_value(row, ordinal))
}

fn key_from_construction_value(row: d11_grammar::ConstructionRow, ordinal: u32) -> Key {
    Key(vec![
        row.tag_db_id as i128,
        ordinal as i128,
        row.line_id as i128,
    ])
}

// --- Boundary mappers --------------------------------------------------------

/// One compact row of the `stockpile` table, as the collector emits it today,
/// onto its stored Record. `row` is the raw 16-byte row slice (the caller has
/// already consumed the table header via [`split_payload`]). `frame_tick` is derived-field contract's
/// derived-field input; d11 declares no derived field, so it is taken and
/// ignored — every domain's mapper has the same shape.
pub fn record_from_stockpile_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    check_row_len(row, STOCKPILE_ROW_LEN, "stockpile")?;
    let row = d11_grammar::stockpile_row_at(row, 0)
        .map_err(|violation| format_grammar_violation(row, violation, None))?;
    record_from_stockpile_value(collection, row)
}

fn record_from_stockpile_value(
    collection: &Collection,
    row: d11_grammar::StockpileRow,
) -> core::result::Result<Record, String> {
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            // `world_fold.rs::fold_production`.
            field::AMOUNT => Value::Continuous(row.amount_raw as f64 / PRODUCTION_SCALE),
            other => return Err(format!("stockpile has no mapping for field id {other}")),
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// One compact row of the `production_lines` table plus the frame's line joins.
///
/// `joins` is not an extra input so much as the rest of the row: the fold reads
/// exactly these six maps for exactly this key and **errors the whole frame** if
/// a present join has no entry for a base line (`world_fold.rs::fold_production`),
/// so the lookup and its failure mode belong at this boundary, not in a caller.
///
/// `ordinal` is the row's dense position within its `(tag_db_id, kind)` run of
/// the base table — the fold's array order, and the only witness of it on a
/// frame that carries no `PPR1` segment. [`fold_payload`] counts it; any other
/// caller must count it the same way.
pub fn record_from_line_row(
    collection: &Collection,
    row: &[u8],
    joins: LineJoins<'_>,
    ordinal: u32,
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    let base_row = d11_grammar::line_row_at(row, 0)
        .map_err(|violation| format_grammar_violation(row, violation, None))?;
    record_from_line_value(collection, base_row, joins, ordinal)
}

fn record_from_line_value(
    collection: &Collection,
    base_row: d11_grammar::LineRow,
    joins: LineJoins<'_>,
    ordinal: u32,
) -> core::result::Result<Record, String> {
    let key = (base_row.tag_db_id, base_row.kind, base_row.line_id);
    let (tag, kind, line_id) = key;

    // `world_fold.rs::fold_production`:
    // a present join that does not cover this line is a hard error, not a hole.
    let lookup = |present: bool, found: bool, magic: &str| -> core::result::Result<(), String> {
        if present && !found {
            return Err(format!(
                "{magic} has no value for production line tag {tag} kind {kind} line_id {line_id}"
            ));
        }
        Ok(())
    };
    let variant = joins.variant.map(|m| m.get(&key)).unwrap_or(None);
    lookup(joins.variant.is_some(), variant.is_some(), "PVJ1")?;
    let requested = joins
        .requested_factories
        .map(|m| m.get(&key))
        .unwrap_or(None);
    lookup(
        joins.requested_factories.is_some(),
        requested.is_some(),
        "PFR1",
    )?;
    let priority = joins.priority.map(|m| m.get(&key)).unwrap_or(None);
    lookup(joins.priority.is_some(), priority.is_some(), "PPR1")?;
    let subtype = joins.subtype.map(|m| m.get(&key)).unwrap_or(None);
    lookup(joins.subtype.is_some(), subtype.is_some(), "PLS1")?;
    let efficiency = joins.efficiency.map(|m| m.get(&key)).unwrap_or(None);
    lookup(joins.efficiency.is_some(), efficiency.is_some(), "PEF1")?;
    let mio = joins.mio.map(|m| m.get(&key)).unwrap_or(None);
    lookup(joins.mio.is_some(), mio.is_some(), "PMI1")?;

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::ACTIVE_FACTORIES => Value::Signed(base_row.active_factories as i64),
            field::COST => Value::Continuous(base_row.cost_raw as f64 / PRODUCTION_SCALE),
            field::SPEED => Value::Continuous(base_row.speed_raw as f64 / PRODUCTION_SCALE),
            field::PRODUCED => Value::Continuous(base_row.produced_raw as f64 / PRODUCTION_SCALE),
            // Raw: `world_fold.rs::fold_production` inserts the i32 with no division.
            field::LINE_AMOUNT => Value::Signed(base_row.amount as i64),
            field::EQUIPMENT_VARIANT_ID => match variant {
                Some(id) => Value::ContentId(*id as u64),
                None => Value::Unknown,
            },
            field::REQUESTED_FACTORIES => match requested {
                Some(v) => Value::Unsigned(*v as u64),
                None => Value::Unknown,
            },
            field::LINE_PRIORITY => match priority {
                Some(v) => Value::Unsigned(*v as u64),
                None => Value::Unknown,
            },
            field::LINE_ORDINAL => Value::Unsigned(ordinal as u64),
            field::LINE_SUBTYPE => match subtype {
                Some(v) => Value::Enum(v.code()),
                None => Value::Unknown,
            },
            field::EFFICIENCY => match efficiency {
                Some(v) => Value::Continuous(v.efficiency_raw as f64 / EFFICIENCY_SCALE),
                None => Value::Unknown,
            },
            field::EFFICIENCY_CAP => match efficiency {
                Some(v) => Value::Continuous(v.efficiency_cap_raw as f64 / EFFICIENCY_SCALE),
                None => Value::Unknown,
            },
            field::EFFICIENCY_GROWTH => match efficiency {
                Some(v) => Value::Continuous(v.efficiency_growth_raw as f64 / EFFICIENCY_SCALE),
                None => Value::Unknown,
            },
            // `world_fold.rs::fold_production`: zero is a proven-null CID.
            field::MIO_ID => match mio {
                Some(0) => Value::Null,
                Some(id) => Value::ContentId(*id as u64),
                None => Value::Unknown,
            },
            other => {
                return Err(format!(
                    "production_lines has no mapping for field id {other}"
                ))
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// One compact row of the `construction_queue` table
/// (`world_fold.rs::fold_production`).
pub fn record_from_construction_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    check_row_len(row, CONSTRUCTION_ROW_LEN, "construction")?;
    let row = d11_grammar::construction_row_at(row, 0)
        .map_err(|violation| format_grammar_violation(row, violation, None))?;
    record_from_construction_value(collection, row)
}

fn record_from_construction_value(
    collection: &Collection,
    row: d11_grammar::ConstructionRow,
) -> core::result::Result<Record, String> {
    let template_id = row.template_id;
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            // `world_fold.rs::fold_production` omits the key on the sentinel; the collector
            // writes it for an unreadable template subobject
            // (`attach_log.rs:85351`), which is record-state contract's unknown.
            field::TEMPLATE_ID => {
                if template_id == TEMPLATE_ID_ABSENT {
                    Value::Unknown
                } else {
                    Value::ContentId(template_id as u64)
                }
            }
            field::ACTIVE_FACTORIES => Value::Signed(row.active_factories as i64),
            field::COST => Value::Continuous(row.cost_raw as f64 / PRODUCTION_SCALE),
            field::PRODUCED => Value::Continuous(row.produced_raw as f64 / PRODUCTION_SCALE),
            // `world_fold.rs::fold_production` reads `+24` raw. Sparse, so it is data about
            // which source slots were skipped, not the queue's ordering.
            field::CONSTRUCTION_PRIORITY => Value::Unsigned(row.priority as u64),
            other => {
                return Err(format!(
                    "construction_queue has no mapping for field id {other}"
                ))
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// One country's row of the `countries` collection.
///
/// `completeness` is that country's `PCP1` flag word, or `None` when the frame
/// carried no `PCP1` segment — the fold leaves all three witnesses off the JSON
/// in that case (`world_fold.rs::fold_production`), which is record-state contract's
/// unknown, not a false.
pub fn record_from_country(
    collection: &Collection,
    completeness: Option<u32>,
    license_detail: LicenseDetail,
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    let bit = |mask: u32| match completeness {
        Some(flags) => Value::Bool(flags & mask != 0),
        None => Value::Unknown,
    };
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::STOCKPILE_COMPLETE => bit(PCP1_STOCKPILE),
            field::MILITARY_LINES_COMPLETE => bit(PCP1_MILITARY_LINES),
            field::NAVAL_LINES_COMPLETE => bit(PCP1_NAVAL_LINES),
            field::LICENSE_DETAIL => Value::Enum(license_detail.code()),
            other => return Err(format!("countries has no mapping for field id {other}")),
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// One `PLIC` item (`world_fold.rs::fold_production`).
pub fn record_from_license_item(
    collection: &Collection,
    item: &LicenseItem,
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::VISIBLE_ORDER => match item.visible_order {
                Some(v) => Value::Unsigned(v as u64),
                None => Value::Unknown,
            },
            field::CREATOR_TAG_DB_ID => match item.creator_tag_db_id {
                Some(v) => Value::ContentId(v as u64),
                None => Value::Unknown,
            },
            // `attach_log.rs:69023`: zero means "the engine's optional origin
            // override is absent", which is record-state contract's null and not a country id.
            // d53 reads it the same way (`world_fold.rs::project_production_license_agreements`: `if origin == 0
            // { creator } else { origin }`).
            field::ORIGIN_TAG_DB_ID => match item.origin_tag_db_id {
                Some(0) => Value::Null,
                Some(v) => Value::ContentId(v as u64),
                None => Value::Unknown,
            },
            other => {
                return Err(format!(
                    "production_licenses has no mapping for field id {other}"
                ))
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// One `PCV1` country row (`world_fold.rs::fold_production`).
pub fn record_from_convoy_bank(
    collection: &Collection,
    bank: &ConvoyBank,
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::CONVOYS_COMPLETE => Value::Bool(bank.complete),
            field::RESERVATIONS_COMPLETE => Value::Bool(bank.reservations_complete),
            field::CACHE_VALID => {
                if bank.complete {
                    Value::Bool(bank.cache_valid)
                } else {
                    Value::Unknown
                }
            }
            field::ASSIGNED => {
                if bank.complete {
                    Value::Unsigned(bank.assigned as u64)
                } else {
                    Value::Unknown
                }
            }
            other => return Err(format!("convoy_bank has no mapping for field id {other}")),
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// One `PCV1` variant row (`world_fold.rs::fold_production`).
///
/// The raw i64 is what is carried: `PCV1` accepts a variant raw past `2^53`
/// whenever the country's clamped total still satisfies the exact total law, and
/// the fold keeps that raw exactly. `amount` is [`convoy_amount`] of it.
///
/// `ordinal` is the row's position within its country's `PCV1` run — the order
/// the fold's array carries and that nothing in the payload constrains.
pub fn record_from_convoy_variant(
    collection: &Collection,
    variant: &ConvoyVariant,
    ordinal: u32,
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    // `world_fold.rs::parse_country_convoy_bank_segment` refuses a negative raw, so the cast is total.
    if variant.amount_raw < 0 {
        return Err(format!(
            "convoy variant {} has negative amount_raw {}",
            variant.variant_id, variant.amount_raw
        ));
    }
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::CONVOY_AMOUNT_RAW => Value::Unsigned(variant.amount_raw as u64),
            field::CONVOY_VARIANT_ORDINAL => Value::Unsigned(ordinal as u64),
            other => {
                return Err(format!(
                    "convoy_variants has no mapping for field id {other}"
                ))
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// One `PCV1` priority reservation in the source order the fold preserves
/// (`world_fold.rs::fold_production`).
pub fn record_from_convoy_reservation(
    collection: &Collection,
    tag_db_id: u32,
    reservation: &ConvoyReservation,
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    if collection.shape != Shape::OrderedOccurrenceList {
        return Err("convoy reservations collection is not an ordered occurrence list".into());
    }
    if tag_db_id == 0 {
        return Err("convoy reservation tag_db_id is zero".into());
    }
    if reservation.count < 0 {
        return Err(format!(
            "convoy reservation count {} is negative",
            reservation.count
        ));
    }
    let mut values = Vec::with_capacity(collection.fields.len());
    for field in collection.value_fields() {
        let value = match field.field_id {
            field::TAG_DB_ID => Value::ContentId(tag_db_id as u64),
            field::RESERVATION_PRIORITY => Value::Signed(reservation.priority as i64),
            field::RESERVATION_COUNT => Value::Unsigned(reservation.count as u64),
            other => {
                return Err(format!(
                    "convoy_priority_reservations has no mapping for field id {other}"
                ))
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// The fold's `amount` projection of a carried convoy variant raw
/// (`world_fold.rs::fold_production`). Recomputed rather than carried — it is a function of
/// state the record already holds.
pub fn convoy_amount(amount_raw: i64) -> f64 {
    amount_raw as f64 / PRODUCTION_SCALE
}

// --- The running state -------------------------------------------------------

/// The eight reconstructed collections.
///
/// d11 is **full-replace per country, not per frame**: `fold_production` rebuilds
/// `per_tag` from scratch each frame (`world_fold.rs::fold_production`) but writes it into
/// `self.country_mut(tag)` (`world_fold.rs::fold_production`), so a country the frame does not mention
/// keeps the production block it had. In `deepwar_t1` a frame touches a median of
/// two countries out of a roster of 356, so the accumulator — not the frame — is
/// the state this domain holds.
#[derive(Clone, Debug, Default)]
pub struct State {
    pub countries: KeyedState,
    pub stockpile: KeyedState,
    pub lines: KeyedState,
    pub construction: KeyedState,
    pub licenses: KeyedState,
    pub convoy_bank: KeyedState,
    pub convoy_variants: KeyedState,
    /// The fold owns one ordered reservation array per country. The map fixes the
    /// inter-country traversal to the fold's tag order; each value retains that
    /// country's source order and legitimate duplicates.
    pub convoy_reservations: BTreeMap<u32, Vec<Record>>,
}

impl State {
    pub fn new() -> Self {
        Self::default()
    }

    /// In `schema()` order, for the block encoders.
    pub fn as_slice(&self) -> [&KeyedState; 7] {
        [
            &self.countries,
            &self.stockpile,
            &self.lines,
            &self.construction,
            &self.licenses,
            &self.convoy_bank,
            &self.convoy_variants,
        ]
    }

    pub fn to_vec(&self) -> Vec<KeyedState> {
        self.as_slice().into_iter().cloned().collect()
    }

    pub fn rows(&self) -> usize {
        self.as_slice().into_iter().map(|c| c.len()).sum::<usize>()
            + self
                .convoy_reservations
                .values()
                .map(Vec::len)
                .sum::<usize>()
    }

    /// All collection states in schema order, preserving the ordered shape.
    pub fn states(&self) -> Vec<CollectionState> {
        let mut states: Vec<CollectionState> = self
            .as_slice()
            .into_iter()
            .cloned()
            .map(CollectionState::Keyed)
            .collect();
        states.push(CollectionState::OrderedOccurrence(
            self.convoy_reservations
                .values()
                .flat_map(|rows| rows.iter().cloned())
                .collect(),
        ));
        states
    }
}

/// What one accepted d11 DATA record did to the accumulator.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FoldOutcome {
    /// The countries whose production block this frame replaced — the fold's
    /// `per_tag` key set.
    pub touched_countries: usize,
    /// The countries whose convoy bank this frame replaced (`PCV1`'s key set),
    /// which is a separate write path onto a separate leaf.
    pub touched_convoy_countries: usize,
    /// `world_fold.rs::fold_production`: `country_count` exceeds the keyed set. The fold
    /// badges the hour and folds the rest, so this is a count, not an exclusion.
    pub uncovered_countries: u32,
    /// Bytes the `PDQ1` segment occupied, for the stop-report's accounting.
    pub deployment_segment_len: usize,
    /// `PCV1` reservation rows the frame carried. Parsed, checked and mapped; the
    /// count remains useful for shipping-byte accounting.
    pub reservation_rows: usize,
    /// `PCV1` variant runs whose `variant_id`s are not ascending — that is, runs
    /// whose order the `convoy_variant_ordinal` field is *load-bearing* for
    /// rather than merely redundant with key order. Zero over all 9,528
    /// non-empty runs of `deepwar_t1`; reported so the cost of carrying the
    /// ordinal stays visible, not because a non-zero value is a defect.
    pub unordered_variant_runs: usize,
}

/// Fold one d11 DATA payload, exactly as `fold_production` does from `:12511`
/// onward.
///
/// The caller must already have excluded failure-status records: the fold returns
/// at `:12508` before reaching any of this, retaining the accumulator. That is
/// d11's **only** fold-nothing path — there is no delta grammar, no
/// delta-before-baseline and no rowless-partial case.
///
/// On `Err` the accumulator is left exactly as it was: every check runs and every
/// row is built before the first mutation, mirroring a fold whose own errors all
/// precede its first write to `self`.
pub fn fold_payload(
    state: &mut State,
    collections: &Collections,
    payload: &[u8],
    status: StatusPredicates,
) -> core::result::Result<FoldOutcome, String> {
    let (header, tables) = split_payload(payload)?;
    let grammar_header = d11_grammar::read_base_header(payload)
        .map_err(|violation| format_grammar_violation(payload, violation, None))?;
    let layout = d11_grammar::base_layout(payload, &grammar_header)
        .map_err(|violation| format_grammar_violation(payload, violation, None))?;
    let trailers = parse_trailers(tables.trailers, status)?;

    // `world_fold.rs::fold_production`: PCP1 seeds `per_tag` and must be bijective
    // with the declared country count.
    let mut per_tag: BTreeSet<u32> = BTreeSet::new();
    if let Some(completeness) = trailers.completeness_by_tag.as_ref() {
        if completeness.len() != header.country_count as usize {
            return Err(format!(
                "PCP1 country coverage is not bijective: trailer_rows={} country_count={}",
                completeness.len(),
                header.country_count
            ));
        }
        per_tag.extend(completeness.keys().copied());
    }

    // Stockpile rows. `world_fold.rs::fold_production`: a duplicate `(tag, variant)` is a
    // torn read and refuses the frame.
    let mut stockpile_rows: Vec<(Key, Record)> = Vec::new();
    let mut seen_stockpile: BTreeSet<(u32, u32)> = BTreeSet::new();
    for row in layout.stockpile_rows(payload) {
        let (row_index, row) =
            row.map_err(|violation| format_grammar_violation(payload, violation, None))?;
        let tag = row.tag_db_id;
        let variant_id = row.variant_id;
        if !seen_stockpile.insert((tag, variant_id)) {
            return Err(format!(
                "production stockpile row {row_index} duplicates variant {variant_id} for tag {tag}"
            ));
        }
        per_tag.insert(tag);
        stockpile_rows.push((
            key_from_stockpile_value(row),
            record_from_stockpile_value(&collections.stockpile, row)?,
        ));
    }

    // Line rows. `world_fold.rs::fold_production`: a duplicate `(tag, kind, line_id)` is a
    // hard error.
    let joins = trailers.line_joins();
    let mut line_rows: Vec<(Key, Record)> = Vec::new();
    let mut seen_lines: BTreeSet<LineKey> = BTreeSet::new();
    // The dense position within each `(tag_db_id, kind)` run, counted the way
    // the fold builds its per-country arrays: `accum.military_lines.push(..)` /
    // `accum.naval_lines.push(..)` in base-table order (`:12659`, `:12662`).
    let mut line_ordinals: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for row in layout.line_rows(payload) {
        let (row_index, row) =
            row.map_err(|violation| format_grammar_violation(payload, violation, None))?;
        let key = (row.tag_db_id, row.kind, row.line_id);
        if !seen_lines.insert(key) {
            return Err(format!(
                "production line row {row_index} duplicates tag {} kind {} line_id {}",
                key.0, key.1, key.2
            ));
        }
        per_tag.insert(key.0);
        // `kind == 2` is naval and everything else is military (`:12658`), so
        // the run a row belongs to is that split, not the raw `kind` word.
        let run = (key.0, if key.1 == KIND_NAVAL { KIND_NAVAL } else { KIND_MILITARY });
        let ordinal = line_ordinals.entry(run).or_insert(0);
        let this_ordinal = *ordinal;
        *ordinal = ordinal
            .checked_add(1)
            .ok_or("production line ordinal overflows")?;
        line_rows.push((
            key_from_line_value(row),
            record_from_line_value(&collections.lines, row, joins, this_ordinal)?,
        ));
    }

    // Construction rows. The fold enforces NO key here (`:12667`-`:12689`) — it
    // appends every row to the country's array — so the key carries a dense
    // per-country ordinal counted here. It is unique by construction, which is
    // what makes every frame the fold accepts expressible: two rows sharing
    // every identity word land under two ordinals rather than colliding.
    let mut construction_rows: Vec<(Key, Record)> = Vec::new();
    let mut construction_ordinals: BTreeMap<u32, u32> = BTreeMap::new();
    for row in layout.construction_rows(payload) {
        let (_, row) =
            row.map_err(|violation| format_grammar_violation(payload, violation, None))?;
        let tag = row.tag_db_id;
        per_tag.insert(tag);
        let ordinal = construction_ordinals.entry(tag).or_insert(0);
        let this_ordinal = *ordinal;
        *ordinal = ordinal
            .checked_add(1)
            .ok_or("production construction ordinal overflows")?;
        construction_rows.push((
            key_from_construction_value(row, this_ordinal),
            record_from_construction_value(&collections.construction, row)?,
        ));
    }

    // `world_fold.rs::fold_production`: every present line join must be a bijection
    // over the base line keys.
    check_line_coverage("PVJ1", trailers.variant_by_line.as_ref(), &seen_lines)?;
    check_line_coverage(
        "PFR1",
        trailers.requested_factories_by_line.as_ref(),
        &seen_lines,
    )?;
    check_line_coverage("PPR1", trailers.priority_by_line.as_ref(), &seen_lines)?;
    check_line_coverage("PLS1", trailers.subtype_by_line.as_ref(), &seen_lines)?;
    check_line_coverage("PEF1", trailers.efficiency_by_line.as_ref(), &seen_lines)?;
    check_line_coverage("PMI1", trailers.mio_by_line.as_ref(), &seen_lines)?;

    // `world_fold.rs::fold_production`: PLIC adds its countries to `per_tag`.
    if let Some(licenses) = trailers.licenses_by_tag.as_ref() {
        per_tag.extend(licenses.keys().copied());
    }

    // `world_fold.rs::fold_production`: PCP1 tag bijection and the two
    // rows-without-a-witness checks.
    if let Some(completeness) = trailers.completeness_by_tag.as_ref() {
        if per_tag.len() != completeness.len() || !per_tag.iter().all(|t| completeness.contains_key(t))
        {
            return Err(format!(
                "PCP1 tag coverage is not bijective: trailer_rows={} keyed_countries={}",
                completeness.len(),
                per_tag.len()
            ));
        }
        for tag in seen_stockpile.iter().map(|(tag, _)| tag) {
            if completeness.get(tag).copied().unwrap_or(0) & PCP1_STOCKPILE == 0 {
                return Err(format!(
                    "PCP1 marks stockpile unreadable for tag {tag} with base stockpile rows"
                ));
            }
        }
        for &(tag, kind, _) in &seen_lines {
            let required = if kind == KIND_NAVAL {
                PCP1_NAVAL_LINES
            } else {
                PCP1_MILITARY_LINES
            };
            if completeness.get(&tag).copied().unwrap_or(0) & required == 0 {
                return Err(format!(
                    "PCP1 marks production line kind {kind} unreadable for tag {tag} with base rows"
                ));
            }
        }
    }

    // `world_fold.rs::fold_production`: the five-way PDQ1 cross-check.
    let mut deployment_segment_len = 0usize;
    if let Some(deployment) = trailers.deployment.as_ref() {
        deployment_segment_len = deployment.segment_len;
        if deployment.elapsed_wall_ms != header.elapsed_wall_ms {
            return Err(format!(
                "PDQ1 elapsed_wall_ms {} != D11 elapsed_wall_ms {}",
                deployment.elapsed_wall_ms, header.elapsed_wall_ms
            ));
        }
        if deployment.tags.len() != header.country_count as usize {
            return Err(format!(
                "PDQ1 country coverage is not bijective: carrier_rows={} country_count={}",
                deployment.tags.len(),
                header.country_count
            ));
        }
        if deployment.changed_country_rows != header.country_count {
            return Err(format!(
                "PDQ1 changed_country_rows {} != D11 country_count {}",
                deployment.changed_country_rows, header.country_count
            ));
        }
        if deployment.changed_country_rows != header.changed_country_count {
            return Err(format!(
                "PDQ1 changed_country_rows {} != D11 changed_rows {}",
                deployment.changed_country_rows, header.changed_country_count
            ));
        }
        if deployment.status_flags != header.status_flags {
            return Err(format!(
                "PDQ1 status_flags {:#x} != D11 status_flags {:#x}",
                deployment.status_flags, header.status_flags
            ));
        }
        let deployment_tags: BTreeSet<u32> = deployment.tags.iter().copied().collect();
        if deployment_tags.len() != deployment.tags.len() {
            return Err("PDQ1 contains duplicate country tags".into());
        }
        match trailers.completeness_by_tag.as_ref() {
            Some(completeness) => {
                let completeness_tags: BTreeSet<u32> = completeness.keys().copied().collect();
                if deployment_tags != completeness_tags {
                    return Err(
                        "PDQ1 country tags are not a bijection over PCP1 country tags".into(),
                    );
                }
            }
            None => {
                if !per_tag.iter().all(|tag| deployment_tags.contains(tag)) {
                    return Err("PDQ1 country tags do not cover every keyed D11 country".into());
                }
            }
        }
    }

    // Everything that can fail has failed. From here the accumulator is written.
    let uncovered_countries = header.country_count.saturating_sub(per_tag.len() as u32);

    // Full replace, per touched country, in every production-side collection.
    retain_untouched(&mut state.countries, &per_tag);
    retain_untouched(&mut state.stockpile, &per_tag);
    retain_untouched(&mut state.lines, &per_tag);
    retain_untouched(&mut state.construction, &per_tag);
    retain_untouched(&mut state.licenses, &per_tag);

    for &tag in &per_tag {
        let completeness = trailers
            .completeness_by_tag
            .as_ref()
            .and_then(|m| m.get(&tag).copied());
        let detail = trailers
            .licenses_by_tag
            .as_ref()
            .and_then(|m| m.get(&tag))
            .map_or(LicenseDetail::NotMeasured, |set| set.detail);
        state.countries.insert(
            Key(vec![tag as i128]),
            record_from_country(&collections.countries, completeness, detail, 0)?,
        );
    }
    for (key, record) in stockpile_rows {
        state.stockpile.insert(key, record);
    }
    for (key, record) in line_rows {
        state.lines.insert(key, record);
    }
    for (key, record) in construction_rows {
        state.construction.insert(key, record);
    }
    if let Some(licenses) = trailers.licenses_by_tag.as_ref() {
        for (&tag, set) in licenses {
            for item in &set.items {
                state.licenses.insert(
                    Key(vec![tag as i128, item.variant_id as i128]),
                    record_from_license_item(&collections.licenses, item, 0)?,
                );
            }
        }
    }

    // `world_fold.rs::fold_production`: a separate write path with its own key set.
    // Note `world_fold.rs::fold_production` removes every convoy leaf BEFORE the completeness
    // witnesses go back, so an unreadable country's previous rows are erased.
    let mut touched_convoy_countries = 0usize;
    let mut reservation_rows = 0usize;
    let mut unordered_variant_runs = 0usize;
    if let Some(banks) = trailers.convoy_bank_by_tag.as_ref() {
        let convoy_tags: BTreeSet<u32> = banks.keys().copied().collect();
        touched_convoy_countries = convoy_tags.len();
        retain_untouched(&mut state.convoy_bank, &convoy_tags);
        retain_untouched(&mut state.convoy_variants, &convoy_tags);
        state
            .convoy_reservations
            .retain(|tag, _| !convoy_tags.contains(tag));
        for (&tag, bank) in banks {
            state.convoy_bank.insert(
                Key(vec![tag as i128]),
                record_from_convoy_bank(&collections.convoy_bank, bank, 0)?,
            );
            if bank
                .variants
                .windows(2)
                .any(|pair| pair[0].variant_id > pair[1].variant_id)
            {
                unordered_variant_runs += 1;
            }
            for (ordinal, variant) in bank.variants.iter().enumerate() {
                let ordinal =
                    u32::try_from(ordinal).map_err(|_| "convoy variant ordinal overflows")?;
                state.convoy_variants.insert(
                    Key(vec![tag as i128, variant.variant_id as i128]),
                    record_from_convoy_variant(&collections.convoy_variants, variant, ordinal, 0)?,
                );
            }
            reservation_rows += bank.reservations.len();
            let reservations = bank
                .reservations
                .iter()
                .map(|reservation| {
                    record_from_convoy_reservation(
                        &collections.convoy_reservations,
                        tag,
                        reservation,
                        0,
                    )
                })
                .collect::<core::result::Result<Vec<_>, _>>()?;
            if !reservations.is_empty() {
                state.convoy_reservations.insert(tag, reservations);
            }
        }
    }

    Ok(FoldOutcome {
        touched_countries: per_tag.len(),
        touched_convoy_countries,
        uncovered_countries,
        deployment_segment_len,
        reservation_rows,
        unordered_variant_runs,
    })
}

/// Drop every row of `state` whose leading key component (the `tag_db_id` in all
/// eight collections) is one this frame replaces.
fn retain_untouched(state: &mut KeyedState, touched: &BTreeSet<u32>) {
    if touched.is_empty() {
        return;
    }
    state.retain(|key, _| {
        u32::try_from(key.first()).map_or(true, |tag| !touched.contains(&tag))
    });
}

fn check_line_coverage<T>(
    magic: &str,
    join: Option<&BTreeMap<LineKey, T>>,
    base: &BTreeSet<LineKey>,
) -> core::result::Result<(), String> {
    if let Some(join) = join {
        if join.len() != base.len() || !join.keys().all(|key| base.contains(key)) {
            return Err(format!(
                "{magic} line coverage is not bijective: trailer_rows={} base_lines={}",
                join.len(),
                base.len()
            ));
        }
    }
    Ok(())
}

fn check_row_len(row: &[u8], expected: usize, what: &str) -> core::result::Result<(), String> {
    if row.len() != expected {
        return Err(format!(
            "production {what} row length {} != {expected}",
            row.len()
        ));
    }
    Ok(())
}

#[cfg(test)]
fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{
        apply_shaped, collection_stored_eq, decode_shaped_baseline, decode_shaped_delta, diff,
        diff_shaped, encode_shaped_baseline, encode_shaped_delta,
    };
    use crate::TickGrid;

    fn grid() -> TickGrid {
        TickGrid::new(3).expect("3 is a legal grid")
    }

    fn cols() -> Collections {
        Collections::default()
    }

    fn index_of(c: &Collection, id: u32) -> usize {
        c.value_fields()
            .position(|f| f.field_id == id)
            .unwrap_or_else(|| panic!("field {id} is in collection {}", c.collection_id))
    }

    // --- raw row builders, packed the way the collector packs them -----------

    /// `attach_log.rs:68045`-`:68055`.
    fn stockpile_row(tag: u32, variant_id: u32, amount_raw: i64) -> Vec<u8> {
        let mut row = Vec::with_capacity(STOCKPILE_ROW_LEN);
        row.extend_from_slice(&tag.to_le_bytes());
        row.extend_from_slice(&variant_id.to_le_bytes());
        row.extend_from_slice(&amount_raw.to_le_bytes());
        assert_eq!(row.len(), STOCKPILE_ROW_LEN);
        row
    }

    /// `attach_log.rs:69081`-`:69109`.
    #[allow(clippy::too_many_arguments)]
    fn line_row(
        tag: u32,
        kind: u32,
        line_id: i32,
        active_factories: i32,
        cost_raw: i32,
        speed_raw: i32,
        produced_raw: i32,
        amount: i32,
    ) -> Vec<u8> {
        let mut row = Vec::with_capacity(LINE_ROW_LEN);
        row.extend_from_slice(&tag.to_le_bytes());
        row.extend_from_slice(&kind.to_le_bytes());
        row.extend_from_slice(&line_id.to_le_bytes());
        row.extend_from_slice(&active_factories.to_le_bytes());
        row.extend_from_slice(&cost_raw.to_le_bytes());
        row.extend_from_slice(&speed_raw.to_le_bytes());
        row.extend_from_slice(&produced_raw.to_le_bytes());
        row.extend_from_slice(&amount.to_le_bytes());
        assert_eq!(row.len(), LINE_ROW_LEN);
        row
    }

    /// `attach_log.rs:68076`-`:68101`, including the trailing reserved word.
    fn construction_row(
        tag: u32,
        line_id: u32,
        template_id: u32,
        active_factories: i32,
        cost_raw: i32,
        produced_raw: i32,
        priority: u32,
    ) -> Vec<u8> {
        let mut row = Vec::with_capacity(CONSTRUCTION_ROW_LEN);
        row.extend_from_slice(&tag.to_le_bytes());
        row.extend_from_slice(&line_id.to_le_bytes());
        row.extend_from_slice(&template_id.to_le_bytes());
        row.extend_from_slice(&active_factories.to_le_bytes());
        row.extend_from_slice(&cost_raw.to_le_bytes());
        row.extend_from_slice(&produced_raw.to_le_bytes());
        row.extend_from_slice(&priority.to_le_bytes());
        row.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(row.len(), CONSTRUCTION_ROW_LEN);
        row
    }

    /// The 20-byte self-sized segment header nine of the ten trailers share.
    fn segment(magic: &[u8; 4], row_len: usize, rows: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(magic);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&((20 + rows.len() * row_len) as u32).to_le_bytes());
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        for row in rows {
            assert_eq!(row.len(), row_len);
            out.extend_from_slice(row);
        }
        out
    }

    fn line_join_row(tag: u32, kind: u32, line_id: i32, value: u32) -> Vec<u8> {
        let mut row = Vec::new();
        row.extend_from_slice(&tag.to_le_bytes());
        row.extend_from_slice(&kind.to_le_bytes());
        row.extend_from_slice(&line_id.to_le_bytes());
        row.extend_from_slice(&value.to_le_bytes());
        row
    }

    fn efficiency_row(tag: u32, kind: u32, line_id: i32, raws: [i64; 3]) -> Vec<u8> {
        let mut row = Vec::new();
        row.extend_from_slice(&tag.to_le_bytes());
        row.extend_from_slice(&kind.to_le_bytes());
        row.extend_from_slice(&line_id.to_le_bytes());
        for v in raws {
            row.extend_from_slice(&v.to_le_bytes());
        }
        row
    }

    fn pcp1_row(tag: u32, flags: u32) -> Vec<u8> {
        let mut row = Vec::new();
        row.extend_from_slice(&tag.to_le_bytes());
        row.extend_from_slice(&flags.to_le_bytes());
        row
    }

    /// `world_fold.rs::parse_production_license_segment`. Items are `(variant_id, visible_order, creator, origin)`.
    fn plic(version: u16, countries: &[(u32, Vec<(u32, u32, u32, u32)>)]) -> Vec<u8> {
        let item_len = if version == 1 { 4 } else { 16 };
        let id_rows: usize = countries.iter().map(|(_, ids)| ids.len()).sum();
        let mut out = Vec::new();
        out.extend_from_slice(b"PLIC");
        out.extend_from_slice(&version.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&(countries.len() as u32).to_le_bytes());
        out.extend_from_slice(&(id_rows as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        for (tag, ids) in countries {
            out.extend_from_slice(&tag.to_le_bytes());
            out.extend_from_slice(&(ids.len() as u32).to_le_bytes());
        }
        for (_, ids) in countries {
            for (variant_id, order, creator, origin) in ids {
                out.extend_from_slice(&variant_id.to_le_bytes());
                // The item width follows `item_len`, not `version == 2`, so an
                // unsupported version still produces a self-consistent segment
                // and the parser's version gate is what refuses it.
                if item_len == 16 {
                    out.extend_from_slice(&order.to_le_bytes());
                    out.extend_from_slice(&creator.to_le_bytes());
                    out.extend_from_slice(&origin.to_le_bytes());
                }
            }
        }
        assert_eq!(out.len(), 20 + countries.len() * 8 + id_rows * item_len);
        out
    }

    struct ConvoyCountry {
        tag: u32,
        flags: u32,
        assigned: i32,
        variants: Vec<(u32, i64)>,
        reservations: Vec<(i32, i32)>,
    }

    /// `world_fold.rs::parse_country_convoy_bank_segment`. Derives `owned_raw` and `available` the way the
    /// fold requires them, so a well-formed segment is easy to write in a test.
    fn pcv1(countries: &[ConvoyCountry]) -> Vec<u8> {
        let variant_count: usize = countries.iter().map(|c| c.variants.len()).sum();
        let reservation_count: usize = countries.iter().map(|c| c.reservations.len()).sum();
        let segment_len = 28 + countries.len() * 40 + variant_count * 16 + reservation_count * 8;
        let mut out = Vec::new();
        out.extend_from_slice(b"PCV1");
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&28u16.to_le_bytes());
        out.extend_from_slice(&(segment_len as u32).to_le_bytes());
        out.extend_from_slice(&(countries.len() as u32).to_le_bytes());
        out.extend_from_slice(&(variant_count as u32).to_le_bytes());
        out.extend_from_slice(&(reservation_count as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        let mut variant_start = 0u32;
        let mut reservation_start = 0u32;
        for c in countries {
            let owned_raw: i64 = c.variants.iter().fold(0i64, |sum, (_, a)| {
                sum.saturating_add(*a).min(PCV1_OWNED_RAW_CAP)
            });
            let available = (owned_raw / PCV1_SCALE - i64::from(c.assigned)) as i32;
            out.extend_from_slice(&c.tag.to_le_bytes());
            out.extend_from_slice(&c.flags.to_le_bytes());
            out.extend_from_slice(&owned_raw.to_le_bytes());
            out.extend_from_slice(&c.assigned.to_le_bytes());
            out.extend_from_slice(&available.to_le_bytes());
            out.extend_from_slice(&variant_start.to_le_bytes());
            out.extend_from_slice(&(c.variants.len() as u32).to_le_bytes());
            out.extend_from_slice(&reservation_start.to_le_bytes());
            out.extend_from_slice(&(c.reservations.len() as u32).to_le_bytes());
            variant_start += c.variants.len() as u32;
            reservation_start += c.reservations.len() as u32;
        }
        for c in countries {
            for (variant_id, amount_raw) in &c.variants {
                out.extend_from_slice(&variant_id.to_le_bytes());
                out.extend_from_slice(&PCV1_EQUIP_TYPE.to_le_bytes());
                out.extend_from_slice(&amount_raw.to_le_bytes());
            }
        }
        for c in countries {
            for (priority, count) in &c.reservations {
                out.extend_from_slice(&priority.to_le_bytes());
                out.extend_from_slice(&count.to_le_bytes());
            }
        }
        assert_eq!(out.len(), segment_len);
        out
    }

    fn pdq_string(out: &mut Vec<u8>, value: &str) {
        out.extend_from_slice(&(value.len() as u32).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
    }

    /// A `PDQ1` segment with one conveyor, one line and one equipment row per
    /// complete country — enough to exercise every gate the structural scan
    /// enforces and the three strings the stop-report is about.
    fn pdq1(elapsed_wall_ms: u64, status_flags: u32, countries: &[(u32, bool)]) -> Vec<u8> {
        let complete = countries.iter().filter(|(_, c)| *c).count() as u32;
        let mut codec = Vec::new();
        codec.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
        codec.extend_from_slice(&(countries.len() as u32).to_le_bytes());
        codec.extend_from_slice(&(countries.len() as u32).to_le_bytes());
        codec.extend_from_slice(&status_flags.to_le_bytes());
        codec.extend_from_slice(&PDQ1_CODEC_VERSION.to_le_bytes());
        codec.extend_from_slice(&complete.to_le_bytes());
        codec.extend_from_slice(&complete.to_le_bytes());
        codec.extend_from_slice(&complete.to_le_bytes());
        codec.extend_from_slice(&0u32.to_le_bytes());
        let mut next_id = 1u32;
        for (tag, queue_complete) in countries {
            codec.extend_from_slice(&tag.to_le_bytes());
            codec.extend_from_slice(&u32::from(*queue_complete).to_le_bytes());
            codec.extend_from_slice(&u32::from(*queue_complete).to_le_bytes());
            codec.extend_from_slice(&0u32.to_le_bytes());
            if !queue_complete {
                continue;
            }
            let conveyor_id = next_id;
            next_id += 1;
            codec.extend_from_slice(&0u32.to_le_bytes()); // ordinal
            codec.extend_from_slice(&conveyor_id.to_le_bytes());
            codec.extend_from_slice(&PDQ1_CONVEYOR_ID_TYPE.to_le_bytes());
            codec.extend_from_slice(&7u32.to_le_bytes()); // template_id
            codec.extend_from_slice(&PDQ1_TEMPLATE_ID_TYPE.to_le_bytes());
            codec.extend_from_slice(&3i32.to_le_bytes()); // amount
            codec.extend_from_slice(&11u32.to_le_bytes()); // location
            codec.extend_from_slice(&0u32.to_le_bytes()); // priority
            codec.extend_from_slice(&0u32.to_le_bytes()); // flags
            codec.extend_from_slice(&1u32.to_le_bytes()); // line_count
            pdq_string(&mut codec, "1. Infanterie-Division");
            pdq_string(&mut codec, "role_infantry");
            let line_id = next_id;
            next_id += 1;
            codec.extend_from_slice(&0u32.to_le_bytes()); // ordinal
            codec.extend_from_slice(&line_id.to_le_bytes());
            codec.extend_from_slice(&PDQ1_LINE_ID_TYPE.to_le_bytes());
            codec.extend_from_slice(&0i32.to_le_bytes()); // name_order
            codec.extend_from_slice(&2i32.to_le_bytes()); // amount
            codec.extend_from_slice(&0u32.to_le_bytes()); // flags
            codec.extend_from_slice(&0u64.to_le_bytes()); // training
            codec.extend_from_slice(&0u64.to_le_bytes()); // max_training
            codec.extend_from_slice(&0i64.to_le_bytes()); // manpower_have
            codec.extend_from_slice(&0i64.to_le_bytes()); // manpower_need
            codec.extend_from_slice(&0i64.to_le_bytes()); // max_manpower
            codec.extend_from_slice(&1u32.to_le_bytes()); // equipment_count
            codec.extend_from_slice(&0u32.to_le_bytes()); // reserved
            codec.extend_from_slice(&0u32.to_le_bytes()); // ordinal
            codec.extend_from_slice(&99u32.to_le_bytes()); // equipment_id
            codec.extend_from_slice(&PDQ1_EQUIPMENT_ID_TYPE.to_le_bytes());
            codec.extend_from_slice(&1u32.to_le_bytes()); // archetype present
            codec.extend_from_slice(&1.0f64.to_bits().to_le_bytes());
            pdq_string(&mut codec, "infantry_equipment");
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"PDQ1");
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&((20 + codec.len()) as u32).to_le_bytes());
        out.extend_from_slice(&(codec.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&codec);
        out
    }

    struct Frame {
        elapsed_wall_ms: u64,
        country_count: u32,
        changed_country_count: Option<u32>,
        status_flags: u32,
        stockpile: Vec<Vec<u8>>,
        lines: Vec<Vec<u8>>,
        construction: Vec<Vec<u8>>,
        trailers: Vec<u8>,
    }

    impl Frame {
        fn new() -> Self {
            Self {
                elapsed_wall_ms: 0,
                country_count: 0,
                changed_country_count: None,
                status_flags: 0,
                stockpile: Vec::new(),
                lines: Vec::new(),
                construction: Vec::new(),
                trailers: Vec::new(),
            }
        }

        fn bytes(&self) -> Vec<u8> {
            let naval = self
                .lines
                .iter()
                .filter(|row| u32_at(row, 4) == KIND_NAVAL)
                .count() as u32;
            let military = self.lines.len() as u32 - naval;
            let mut out = Vec::new();
            out.extend_from_slice(&self.elapsed_wall_ms.to_le_bytes());
            out.extend_from_slice(&self.country_count.to_le_bytes());
            out.extend_from_slice(
                &self
                    .changed_country_count
                    .unwrap_or(self.country_count)
                    .to_le_bytes(),
            );
            out.extend_from_slice(&self.status_flags.to_le_bytes());
            out.extend_from_slice(&100_000u32.to_le_bytes());
            out.extend_from_slice(&(self.stockpile.len() as u32).to_le_bytes());
            out.extend_from_slice(&military.to_le_bytes());
            out.extend_from_slice(&naval.to_le_bytes());
            out.extend_from_slice(&(self.construction.len() as u32).to_le_bytes());
            for row in self.stockpile.iter() {
                out.extend_from_slice(row);
            }
            // Military rows first, then naval — the collector's order
            // (`attach_log.rs:69057`-`:69072`).
            for row in self.lines.iter().filter(|r| u32_at(r, 4) != KIND_NAVAL) {
                out.extend_from_slice(row);
            }
            for row in self.lines.iter().filter(|r| u32_at(r, 4) == KIND_NAVAL) {
                out.extend_from_slice(row);
            }
            for row in self.construction.iter() {
                out.extend_from_slice(row);
            }
            out.extend_from_slice(&self.trailers);
            out
        }
    }

    /// One country, every collection non-empty, every join present.
    fn full_frame() -> Frame {
        let mut frame = Frame::new();
        frame.elapsed_wall_ms = 4242;
        frame.country_count = 1;
        frame.stockpile = vec![stockpile_row(7, 500, 12_345_678)];
        frame.lines = vec![
            line_row(7, KIND_MILITARY, 3, 5, 150_000, 25_000, 900_000, 1),
            line_row(7, KIND_NAVAL, 4, 2, 250_000, 12_500, 100_000, -1),
        ];
        frame.construction = vec![construction_row(7, 90, 31, 4, 2_000_000, 500_000, 0)];
        let mut trailers = Vec::new();
        trailers.extend(plic(2, &[(7, vec![(11, 0, 9, 8)])]));
        trailers.extend(segment(
            b"PVJ1",
            16,
            &[
                line_join_row(7, KIND_MILITARY, 3, 500),
                line_join_row(7, KIND_NAVAL, 4, 501),
            ],
        ));
        trailers.extend(segment(
            b"PFR1",
            16,
            &[
                line_join_row(7, KIND_MILITARY, 3, 6),
                line_join_row(7, KIND_NAVAL, 4, 3),
            ],
        ));
        trailers.extend(segment(
            b"PPR1",
            16,
            &[
                line_join_row(7, KIND_MILITARY, 3, 0),
                line_join_row(7, KIND_NAVAL, 4, 1),
            ],
        ));
        trailers.extend(segment(
            b"PLS1",
            16,
            &[
                line_join_row(7, KIND_MILITARY, 3, 1),
                line_join_row(7, KIND_NAVAL, 4, 3),
            ],
        ));
        trailers.extend(segment(
            b"PEF1",
            36,
            &[
                efficiency_row(7, KIND_MILITARY, 3, [5_000_000, 10_000_000, 12_345]),
                efficiency_row(7, KIND_NAVAL, 4, [1, 2, 3]),
            ],
        ));
        trailers.extend(segment(
            b"PMI1",
            16,
            &[
                line_join_row(7, KIND_MILITARY, 3, 77),
                line_join_row(7, KIND_NAVAL, 4, 0),
            ],
        ));
        trailers.extend(segment(b"PCP1", 8, &[pcp1_row(7, PCP1_KNOWN_FLAGS)]));
        trailers.extend(pcv1(&[ConvoyCountry {
            tag: 7,
            flags: PCV1_KNOWN_FLAGS,
            assigned: 2,
            variants: vec![(60, 300_000), (61, 200_000)],
            reservations: vec![(1, 3), (2, 1)],
        }]));
        trailers.extend(pdq1(4242, 0, &[(7, true)]));
        frame.trailers = trailers;
        frame
    }

    fn full_trailers() -> Trailers {
        let payload = full_frame().bytes();
        let (_, tables) = split_payload(&payload).expect("splits");
        parse_trailers(tables.trailers, StatusPredicates::PERMISSIVE).expect("parses")
    }

    fn fold_one(state: &mut State, frame: &Frame) -> FoldOutcome {
        fold_payload(state, &cols(), &frame.bytes(), StatusPredicates::PERMISSIVE)
            .expect("the frame folds")
    }

    fn reservation_rows(state: &State) -> Vec<Record> {
        match state.states().pop().expect("the reservation collection exists") {
            CollectionState::OrderedOccurrence(rows) => rows,
            _ => panic!("the reservation collection is ordered-occurrence state"),
        }
    }

    // --- step 8.1: the schema validates -------------------------------------

    #[test]
    fn the_schema_is_valid() {
        schema()
            .validate(grid().log2())
            .expect("d11 schema is well-formed");
    }

    #[test]
    fn the_reservation_collection_and_field_ids_are_literal_pinned() {
        assert_eq!(CONVOY_PRIORITY_RESERVATIONS, 8);
        assert_eq!(RESERVED_CONVOY_RESERVATIONS, 8);
        assert_eq!(field::RESERVATION_COUNT, 80);
        assert_eq!(field::RESERVED_RESERVATION_COUNT, 80);
        assert_eq!(field::RESERVATION_PRIORITY, 81);
    }

    #[test]
    fn the_collections_resolve_by_id_not_position() {
        let c = cols();
        assert_eq!(c.countries.collection_id, COUNTRIES);
        assert_eq!(c.stockpile.collection_id, STOCKPILE);
        assert_eq!(c.lines.collection_id, PRODUCTION_LINES);
        assert_eq!(c.construction.collection_id, CONSTRUCTION_QUEUE);
        assert_eq!(c.licenses.collection_id, PRODUCTION_LICENSES);
        assert_eq!(c.convoy_bank.collection_id, CONVOY_BANK);
        assert_eq!(c.convoy_variants.collection_id, CONVOY_VARIANTS);
        assert_eq!(
            c.convoy_reservations.collection_id,
            CONVOY_PRIORITY_RESERVATIONS
        );
        assert_eq!(c.convoy_reservations.shape, Shape::OrderedOccurrenceList);
        assert_eq!(schema().collections.len(), 8);
        let mut shuffled = schema();
        shuffled.collections.reverse();
        let c = Collections::from_schema(&shuffled).expect("resolves");
        assert_eq!(c.countries.collection_id, COUNTRIES);
        assert_eq!(c.convoy_variants.collection_id, CONVOY_VARIANTS);
        let mut short = schema();
        short.collections.retain(|c| c.collection_id != STOCKPILE);
        assert!(Collections::from_schema(&short).is_err());
    }

    // --- step 8.5: shape and key --------------------------------------------

    #[test]
    fn every_collection_has_the_shape_and_key_the_fold_requires() {
        let c = cols();
        for collection in c.as_slice().into_iter().take(7) {
            assert_eq!(
                collection.shape,
                Shape::KeyedSet,
                "collection {} must be a keyed set",
                collection.collection_id
            );
            for k in collection.key_fields() {
                assert!(k.kind.legal_as_key(), "field {} as key", k.field_id);
                assert!(!k.optional && !k.nullable && !k.unknown_capable);
            }
        }
        assert_eq!(
            c.convoy_reservations.shape,
            Shape::OrderedOccurrenceList
        );
        assert!(c.convoy_reservations.key_fields().is_empty());
        assert_eq!(
            c.convoy_reservations
                .fields
                .iter()
                .map(|field| (field.field_id, field.kind))
                .collect::<Vec<_>>(),
            vec![
                (field::TAG_DB_ID, Kind::ContentId),
                (field::RESERVATION_PRIORITY, Kind::Signed),
                (field::RESERVATION_COUNT, Kind::Unsigned),
            ],
            "the occurrence row's full field list pins the signedness decisions"
        );
        let keys = |collection: &Collection| -> Vec<(u32, Kind)> {
            collection
                .key_fields()
                .into_iter()
                .map(|f| (f.field_id, f.kind))
                .collect()
        };
        assert_eq!(
            keys(&c.countries),
            vec![(field::TAG_DB_ID, Kind::ContentId)],
            "world_fold.rs:12936 keys the production block by country"
        );
        assert_eq!(
            keys(&c.stockpile),
            vec![
                (field::TAG_DB_ID, Kind::ContentId),
                (field::VARIANT_ID, Kind::ContentId)
            ],
            "world_fold.rs:12541 keys the stockpile map by variant, per country"
        );
        assert_eq!(
            keys(&c.lines),
            vec![
                (field::TAG_DB_ID, Kind::ContentId),
                (field::KIND, Kind::Unsigned),
                (field::LINE_ID, Kind::Signed)
            ],
            "world_fold.rs:12560 keys lines by (tag, kind, line_id)"
        );
        assert_eq!(
            keys(&c.construction),
            vec![
                (field::TAG_DB_ID, Kind::ContentId),
                (field::CONSTRUCTION_ORDINAL, Kind::Unsigned),
                (field::CONSTRUCTION_ID, Kind::Unsigned)
            ],
            "the dense ordinal leads, because the fold enforces no construction key"
        );
        assert_eq!(
            keys(&c.licenses),
            vec![
                (field::TAG_DB_ID, Kind::ContentId),
                (field::VARIANT_ID, Kind::ContentId)
            ]
        );
        assert_eq!(
            keys(&c.convoy_bank),
            vec![(field::TAG_DB_ID, Kind::ContentId)]
        );
        assert_eq!(
            keys(&c.convoy_variants),
            vec![
                (field::TAG_DB_ID, Kind::ContentId),
                (field::VARIANT_ID, Kind::ContentId)
            ]
        );
    }

    #[test]
    fn the_key_extractors_agree_with_the_folds_key() {
        assert_eq!(
            key_from_stockpile_row(&stockpile_row(7, 500, 1)).expect("keys"),
            Key(vec![7, 500])
        );
        assert_eq!(
            key_from_line_row(&line_row(7, 2, -3, 0, 0, 0, 0, 0)).expect("keys"),
            Key(vec![7, 2, -3]),
            "a negative line_id is a legal signed key component"
        );
        assert_eq!(
            key_from_construction_row(&construction_row(7, 90, 0, 0, 0, 0, 4), 2).expect("keys"),
            Key(vec![7, 2, 90]),
            "the dense ordinal leads the line_id, so key order IS the fold's array order"
        );
        for len in [0usize, 15, 17] {
            assert!(key_from_stockpile_row(&vec![0u8; len]).is_err(), "len {len}");
        }
        for len in [0usize, 31, 33] {
            assert!(key_from_line_row(&vec![0u8; len]).is_err(), "len {len}");
            assert!(
                key_from_construction_row(&vec![0u8; len], 0).is_err(),
                "len {len}"
            );
        }
    }

    // --- step 8.3: declared capabilities, and only those ---------------------

    #[test]
    fn no_field_declares_a_state_its_mapper_cannot_emit() {
        // d11 declares no `optional` anywhere: every field applies to every row
        // of its collection, so nothing is Absent and the presence bitmap costs
        // zero bytes.
        let c = cols();
        for collection in c.as_slice() {
            assert_eq!(
                collection.optional_count(),
                0,
                "collection {} declares an optional field",
                collection.collection_id
            );
        }
        let nullable: Vec<(u32, u32)> = c
            .as_slice()
            .into_iter()
            .flat_map(|collection| {
                collection
                    .value_fields()
                    .filter(|f| f.nullable)
                    .map(|f| (collection.collection_id, f.field_id))
            })
            .collect();
        assert_eq!(
            nullable,
            vec![
                (PRODUCTION_LINES, field::MIO_ID),
                (PRODUCTION_LICENSES, field::ORIGIN_TAG_DB_ID)
            ],
            "the two relations that can genuinely point at nothing, and no others"
        );
        let unknown: Vec<(u32, u32)> = c
            .as_slice()
            .into_iter()
            .flat_map(|collection| {
                collection
                    .value_fields()
                    .filter(|f| f.unknown_capable)
                    .map(|f| (collection.collection_id, f.field_id))
            })
            .collect();
        assert_eq!(
            unknown,
            vec![
                (COUNTRIES, field::STOCKPILE_COMPLETE),
                (COUNTRIES, field::MILITARY_LINES_COMPLETE),
                (COUNTRIES, field::NAVAL_LINES_COMPLETE),
                (PRODUCTION_LINES, field::EQUIPMENT_VARIANT_ID),
                (PRODUCTION_LINES, field::REQUESTED_FACTORIES),
                (PRODUCTION_LINES, field::LINE_PRIORITY),
                (PRODUCTION_LINES, field::LINE_SUBTYPE),
                (PRODUCTION_LINES, field::EFFICIENCY),
                (PRODUCTION_LINES, field::EFFICIENCY_CAP),
                (PRODUCTION_LINES, field::EFFICIENCY_GROWTH),
                (PRODUCTION_LINES, field::MIO_ID),
                (CONSTRUCTION_QUEUE, field::TEMPLATE_ID),
                (PRODUCTION_LICENSES, field::VISIBLE_ORDER),
                (PRODUCTION_LICENSES, field::CREATOR_TAG_DB_ID),
                (PRODUCTION_LICENSES, field::ORIGIN_TAG_DB_ID),
                (CONVOY_BANK, field::CACHE_VALID),
                (CONVOY_BANK, field::ASSIGNED),
            ],
            "every unknown-capable field, and nothing else"
        );
    }

    #[test]
    fn no_field_is_derived() {
        // derived-field contract derived fields are clock-linear integers; nothing in d11 changes
        // deterministically with the clock, so `frame_tick` never enters a value.
        for collection in cols().as_slice() {
            assert!(collection.fields.iter().all(|f| f.derived.is_none()));
        }
    }

    // --- step 8.6: every field's offset, with scale as the special case ------

    #[test]
    fn the_stockpile_amount_uses_the_100_000_scale() {
        let c = cols().stockpile;
        for (raw, expected) in [
            (100_000i64, 1.0f64),
            (-4_100_000, -41.0),
            (0, 0.0),
            (1, 0.000_01),
            (26_969_240_784, 269_692.407_84),
        ] {
            let r = record_from_stockpile_row(&c, &stockpile_row(7, 500, raw), 0).expect("maps");
            assert_eq!(
                r.values[index_of(&c, field::AMOUNT)],
                Value::Continuous(expected),
                "raw {raw} at scale {PRODUCTION_SCALE}"
            );
        }
    }

    #[test]
    fn every_line_field_reads_its_own_offset_at_its_own_scale() {
        // Distinct words at every offset, so a transposition fails as loudly as
        // a wrong scale. Three fields are `/100_000`, two are raw, and the three
        // efficiency channels are `/10_000_000` — a 100x difference no
        // round-trip test would ever catch.
        let c = cols().lines;
        let row = line_row(7, KIND_MILITARY, 3, 11, 150_000, 25_000, 900_000, -1);
        let joins_owner = full_trailers();
        let r = record_from_line_row(&c, &row, joins_owner.line_joins(), 0, 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::ACTIVE_FACTORIES)],
            Value::Signed(11),
            "+12 is raw"
        );
        assert_eq!(
            r.values[index_of(&c, field::COST)],
            Value::Continuous(1.5),
            "+16 is /100_000"
        );
        assert_eq!(
            r.values[index_of(&c, field::SPEED)],
            Value::Continuous(0.25),
            "+20 is /100_000"
        );
        assert_eq!(
            r.values[index_of(&c, field::PRODUCED)],
            Value::Continuous(9.0),
            "+24 is /100_000"
        );
        assert_eq!(
            r.values[index_of(&c, field::LINE_AMOUNT)],
            Value::Signed(-1),
            "+28 is emitted RAW by world_fold.rs:12572, unlike its three neighbours"
        );
        assert_eq!(
            r.values[index_of(&c, field::EFFICIENCY)],
            Value::Continuous(0.5),
            "5_000_000 / 10_000_000 — reading it at PRODUCTION_SCALE gives 50.0"
        );
        assert_eq!(
            r.values[index_of(&c, field::EFFICIENCY_CAP)],
            Value::Continuous(1.0)
        );
        assert_eq!(
            r.values[index_of(&c, field::EFFICIENCY_GROWTH)],
            Value::Continuous(0.001_234_5)
        );
        assert_eq!(
            r.values[index_of(&c, field::EQUIPMENT_VARIANT_ID)],
            Value::ContentId(500)
        );
        assert_eq!(
            r.values[index_of(&c, field::REQUESTED_FACTORIES)],
            Value::Unsigned(6)
        );
        assert_eq!(
            r.values[index_of(&c, field::LINE_PRIORITY)],
            Value::Unsigned(0),
            "priority zero is a real ordinal, not an empty state"
        );
        assert_eq!(
            r.values[index_of(&c, field::LINE_SUBTYPE)],
            Value::Enum(LineSubtype::Military.code())
        );
        assert_eq!(r.values[index_of(&c, field::MIO_ID)], Value::ContentId(77));
    }

    #[test]
    fn every_construction_field_reads_its_own_offset_at_its_own_scale() {
        let c = cols().construction;
        let r = record_from_construction_row(
            &c,
            &construction_row(7, 90, 31, 4, 2_000_000, 500_000, 12),
            0,
        )
        .expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::TEMPLATE_ID)],
            Value::ContentId(31)
        );
        assert_eq!(
            r.values[index_of(&c, field::ACTIVE_FACTORIES)],
            Value::Signed(4)
        );
        assert_eq!(r.values[index_of(&c, field::COST)], Value::Continuous(20.0));
        assert_eq!(
            r.values[index_of(&c, field::PRODUCED)],
            Value::Continuous(5.0)
        );
        assert_eq!(
            r.values[index_of(&c, field::CONSTRUCTION_PRIORITY)],
            Value::Unsigned(12),
            "world_fold.rs:12654 reads +24 raw; it is the sparse source-slot index"
        );
        // And the key carries the DENSE ordinal, which the caller counts.
        assert_eq!(
            key_from_construction_row(&construction_row(7, 90, 31, 4, 2_000_000, 500_000, 12), 0)
                .expect("keys"),
            Key(vec![7, 0, 90])
        );
    }

    #[test]
    fn the_convoy_variant_carries_the_raw_i64_not_a_float() {
        // The fold emits `amount_raw` beside its `f64` projection, and PCV1
        // accepts a variant raw past 2^53 whenever the country's clamped total
        // still satisfies the exact total law. `Continuous` would collapse
        // 9_007_199_254_740_993 onto 9_007_199_254_740_992; `Unsigned` does not.
        let c = cols().convoy_variants;
        for raw in [
            0i64,
            115_700_000,
            9_007_199_254_740_992,
            9_007_199_254_740_993,
            i64::MAX,
        ] {
            let r = record_from_convoy_variant(
                &c,
                &ConvoyVariant {
                    variant_id: 60,
                    amount_raw: raw,
                },
                0,
                0,
            )
            .expect("maps");
            assert_eq!(
                r.values[index_of(&c, field::CONVOY_AMOUNT_RAW)],
                Value::Unsigned(raw as u64),
                "raw {raw} must survive exactly"
            );
            // And it round-trips through the codec, so the exactness is on the
            // wire and not just in the mapper.
            let mut bytes = Vec::new();
            r.encode(&c, &mut bytes).expect("encodes");
            let (back, _) = Record::decode(&c, &bytes, 0).expect("decodes");
            assert_eq!(back, r, "raw {raw}");
        }
        // The fold's `amount` projection stays derivable.
        assert_eq!(convoy_amount(115_700_000), 1157.0);
        assert_eq!(convoy_amount(0), 0.0);
        // A negative raw is refused rather than cast into a huge unsigned.
        assert!(record_from_convoy_variant(
            &c,
            &ConvoyVariant {
                variant_id: 60,
                amount_raw: -1,
            },
            0,
            0
        )
        .is_err());
    }

    #[test]
    fn a_variant_raw_past_2_pow_53_is_a_payload_the_fold_accepts() {
        // The reviewer's own input, as a whole segment: one variant whose raw
        // exceeds 2^53, a country total clamped to `OWNED_RAW_CAP`, and the
        // exact total law satisfied. The parser must accept it and the mapper
        // must carry the raw unchanged.
        const RAW: i64 = 9_007_199_254_740_993;
        let chain = pcv1(&[ConvoyCountry {
            tag: 7,
            flags: PCV1_KNOWN_FLAGS,
            assigned: 0,
            variants: vec![(60, RAW)],
            reservations: vec![],
        }]);
        let parsed = parse_trailers(&chain, StatusPredicates::PERMISSIVE)
            .expect("the fold's total law clamps the sum, so this segment is legal");
        let bank = &parsed.convoy_bank_by_tag.as_ref().expect("pcv1")[&7];
        assert_eq!(bank.variants[0].amount_raw, RAW);
        let c = cols().convoy_variants;
        let r = record_from_convoy_variant(&c, &bank.variants[0], 0, 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::CONVOY_AMOUNT_RAW)],
            Value::Unsigned(RAW as u64)
        );
        assert_ne!(
            RAW as f64 as i64,
            RAW,
            "the value this test exists for is one f64 cannot hold"
        );
    }

    // --- step 8.3: every declared state, through the mapper, from raw bytes ---

    #[test]
    fn a_real_zero_is_a_value_not_an_empty_state() {
        let c = cols();
        let r =
            record_from_stockpile_row(&c.stockpile, &stockpile_row(7, 500, 0), 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c.stockpile, field::AMOUNT)],
            Value::Continuous(0.0)
        );
        let trailers = full_trailers();
        let r = record_from_line_row(
            &c.lines,
            &line_row(7, KIND_MILITARY, 3, 0, 0, 0, 0, 0),
            trailers.line_joins(),
            0,
            0,
        )
        .expect("maps");
        for id in [field::ACTIVE_FACTORIES, field::LINE_AMOUNT] {
            assert_eq!(r.values[index_of(&c.lines, id)], Value::Signed(0));
        }
        for id in [field::COST, field::SPEED, field::PRODUCED] {
            assert_eq!(r.values[index_of(&c.lines, id)], Value::Continuous(0.0));
        }
        // A country whose three vectors were all READ and all empty: `true`,
        // three times. `false` means "unreadable", which is a different fact.
        let r = record_from_country(
            &c.countries,
            Some(PCP1_KNOWN_FLAGS),
            LicenseDetail::NotMeasured,
            0,
        )
        .expect("maps");
        for id in [
            field::STOCKPILE_COMPLETE,
            field::MILITARY_LINES_COMPLETE,
            field::NAVAL_LINES_COMPLETE,
        ] {
            assert_eq!(r.values[index_of(&c.countries, id)], Value::Bool(true));
        }
    }

    #[test]
    fn a_missing_line_join_is_unknown_for_every_line_in_the_frame() {
        // One join dropped at a time, over the whole join space — the state the
        // capture exercises for real at hour 60832893, where 147 lines carry no
        // PEF1 segment.
        let c = cols();
        let full = full_trailers();
        let cases: [(&str, &[u32]); 6] = [
            ("PVJ1", &[field::EQUIPMENT_VARIANT_ID]),
            ("PFR1", &[field::REQUESTED_FACTORIES]),
            ("PPR1", &[field::LINE_PRIORITY]),
            ("PLS1", &[field::LINE_SUBTYPE]),
            (
                "PEF1",
                &[
                    field::EFFICIENCY,
                    field::EFFICIENCY_CAP,
                    field::EFFICIENCY_GROWTH,
                ],
            ),
            ("PMI1", &[field::MIO_ID]),
        ];
        for (dropped, unknown_ids) in cases {
            let mut joins = full.line_joins();
            match dropped {
                "PVJ1" => joins.variant = None,
                "PFR1" => joins.requested_factories = None,
                "PPR1" => joins.priority = None,
                "PLS1" => joins.subtype = None,
                "PEF1" => joins.efficiency = None,
                "PMI1" => joins.mio = None,
                other => panic!("unhandled {other}"),
            }
            let r = record_from_line_row(
                &c.lines,
                &line_row(7, KIND_MILITARY, 3, 1, 1, 1, 1, 1),
                joins,
                0,
                0,
            )
            .expect("maps");
            for f in c.lines.value_fields() {
                assert_eq!(
                    r.values[index_of(&c.lines, f.field_id)] == Value::Unknown,
                    unknown_ids.contains(&f.field_id),
                    "field {} with {dropped} dropped",
                    f.field_id
                );
            }
        }
        // With every join dropped, exactly the eight join-fed fields are Unknown
        // and the five base fields still carry values.
        let r = record_from_line_row(
            &c.lines,
            &line_row(7, KIND_MILITARY, 3, 1, 1, 1, 1, 1),
            LineJoins::default(),
            0,
            0,
        )
        .expect("maps");
        assert_eq!(r.values.iter().filter(|v| **v == Value::Unknown).count(), 8);
        assert_eq!(
            r.values.len(),
            14,
            "8 join-fed fields, 5 base fields and the dense ordinal"
        );
        // And the ordinal is concrete even with every join gone — that is the
        // whole reason it exists.
        assert_eq!(
            r.values[index_of(&c.lines, field::LINE_ORDINAL)],
            Value::Unsigned(0)
        );
    }

    #[test]
    fn a_proven_zero_mio_is_null_and_a_missing_segment_is_unknown() {
        // The one field in the domain that reaches both empty states, and the
        // distinction the collector states outright (`attach_log.rs:72722`):
        // `Some(None)` is a proven zero CID, `None` is unreadable.
        let c = cols().lines;
        let trailers = full_trailers();
        let r = record_from_line_row(
            &c,
            &line_row(7, KIND_NAVAL, 4, 1, 1, 1, 1, 1),
            trailers.line_joins(),
            0,
            0,
        )
        .expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::MIO_ID)],
            Value::Null,
            "mio_id 0 folds to Null at world_fold.rs:12632"
        );
        let mut joins = trailers.line_joins();
        joins.mio = None;
        let r = record_from_line_row(&c, &line_row(7, KIND_NAVAL, 4, 1, 1, 1, 1, 1), joins, 0, 0)
            .expect("maps");
        assert_eq!(r.values[index_of(&c, field::MIO_ID)], Value::Unknown);
    }

    #[test]
    fn a_missing_pcp1_segment_is_unknown_not_false() {
        let c = cols().countries;
        let r = record_from_country(&c, None, LicenseDetail::NotMeasured, 0).expect("maps");
        for id in [
            field::STOCKPILE_COMPLETE,
            field::MILITARY_LINES_COMPLETE,
            field::NAVAL_LINES_COMPLETE,
        ] {
            assert_eq!(r.values[index_of(&c, id)], Value::Unknown, "field {id}");
        }
        // Every legal flag word, bit by bit: `false` is a measured "this vector
        // was unreadable", which is not the same fact as "not captured".
        for flags in 0..=PCP1_KNOWN_FLAGS {
            let r = record_from_country(&c, Some(flags), LicenseDetail::IdsOnly, 0).expect("maps");
            assert_eq!(
                r.values[index_of(&c, field::STOCKPILE_COMPLETE)],
                Value::Bool(flags & PCP1_STOCKPILE != 0)
            );
            assert_eq!(
                r.values[index_of(&c, field::MILITARY_LINES_COMPLETE)],
                Value::Bool(flags & PCP1_MILITARY_LINES != 0)
            );
            assert_eq!(
                r.values[index_of(&c, field::NAVAL_LINES_COMPLETE)],
                Value::Bool(flags & PCP1_NAVAL_LINES != 0)
            );
        }
    }

    #[test]
    fn a_legacy_v1_license_item_is_unknown_where_the_item_cannot_carry_a_value() {
        // The recipe's legacy-row rule: a v1 `PLIC` item is four bytes wide and
        // physically cannot carry order, creator or origin.
        let c = cols().licenses;
        let v1 = LicenseItem {
            variant_id: 11,
            visible_order: None,
            creator_tag_db_id: None,
            origin_tag_db_id: None,
        };
        let r = record_from_license_item(&c, &v1, 0).expect("maps");
        for id in [
            field::VISIBLE_ORDER,
            field::CREATOR_TAG_DB_ID,
            field::ORIGIN_TAG_DB_ID,
        ] {
            assert_eq!(r.values[index_of(&c, id)], Value::Unknown, "field {id}");
        }
        let v2 = LicenseItem {
            variant_id: 11,
            visible_order: Some(0),
            creator_tag_db_id: Some(9),
            origin_tag_db_id: Some(0),
        };
        let r = record_from_license_item(&c, &v2, 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::VISIBLE_ORDER)],
            Value::Unsigned(0),
            "visible_order zero is a real ordinal, not an empty state"
        );
        assert_eq!(
            r.values[index_of(&c, field::CREATOR_TAG_DB_ID)],
            Value::ContentId(9)
        );
        assert_eq!(
            r.values[index_of(&c, field::ORIGIN_TAG_DB_ID)],
            Value::Null,
            "attach_log.rs:68657: zero means the optional origin override is absent"
        );
        // And a non-zero origin is a real country id, so the two are distinct.
        let with_origin = LicenseItem {
            origin_tag_db_id: Some(5),
            ..v2
        };
        let r = record_from_license_item(&c, &with_origin, 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::ORIGIN_TAG_DB_ID)],
            Value::ContentId(5)
        );
        // Null, Unknown and a real id all survive the codec distinctly.
        for (item, expected) in [
            (v2, Value::Null),
            (with_origin, Value::ContentId(5)),
            (v1, Value::Unknown),
        ] {
            let record = record_from_license_item(&c, &item, 0).expect("maps");
            let mut bytes = Vec::new();
            record.encode(&c, &mut bytes).expect("encodes");
            let (back, _) = Record::decode(&c, &bytes, 0).expect("decodes");
            assert_eq!(back.values[index_of(&c, field::ORIGIN_TAG_DB_ID)], expected);
            let mut reencoded = Vec::new();
            back.encode(&c, &mut reencoded).expect("re-encodes");
            assert_eq!(reencoded, bytes);
        }
    }

    #[test]
    fn an_unreadable_convoy_bank_is_unknown_where_the_fold_emits_nothing() {
        let c = cols().convoy_bank;
        let unreadable = ConvoyBank {
            complete: false,
            reservations_complete: false,
            cache_valid: false,
            assigned: 0,
            variants: Vec::new(),
            reservations: Vec::new(),
        };
        let r = record_from_convoy_bank(&c, &unreadable, 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::CONVOYS_COMPLETE)],
            Value::Bool(false),
            "world_fold.rs:12980 emits this witness unconditionally"
        );
        assert_eq!(
            r.values[index_of(&c, field::RESERVATIONS_COMPLETE)],
            Value::Bool(false)
        );
        assert_eq!(r.values[index_of(&c, field::CACHE_VALID)], Value::Unknown);
        assert_eq!(r.values[index_of(&c, field::ASSIGNED)], Value::Unknown);

        let readable = ConvoyBank {
            complete: true,
            reservations_complete: true,
            cache_valid: false,
            assigned: 0,
            variants: Vec::new(),
            reservations: Vec::new(),
        };
        let r = record_from_convoy_bank(&c, &readable, 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::CACHE_VALID)],
            Value::Bool(false),
            "a readable bank with an invalid cache is a measured false"
        );
        assert_eq!(r.values[index_of(&c, field::ASSIGNED)], Value::Unsigned(0));
    }

    #[test]
    fn an_unreadable_construction_template_is_unknown() {
        let c = cols().construction;
        let r = record_from_construction_row(
            &c,
            &construction_row(7, 90, TEMPLATE_ID_ABSENT, 0, 0, 0, 0),
            0,
        )
        .expect("maps");
        assert_eq!(r.values[index_of(&c, field::TEMPLATE_ID)], Value::Unknown);
        // The sentinel's neighbour is a real id, so no off-by-one.
        let r =
            record_from_construction_row(&c, &construction_row(7, 90, u32::MAX - 1, 0, 0, 0, 0), 0)
                .expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::TEMPLATE_ID)],
            Value::ContentId((u32::MAX - 1) as u64)
        );
    }

    // --- step 8.2: byte-identical round trips --------------------------------

    #[test]
    fn every_collections_records_roundtrip_byte_for_byte() {
        let mut state = State::new();
        fold_one(&mut state, &full_frame());
        let c = cols();
        for (collection, rows) in c.as_slice().into_iter().zip(state.as_slice()) {
            assert!(
                !rows.is_empty(),
                "collection {} has no row to round-trip",
                collection.collection_id
            );
            for record in rows.values() {
                let mut bytes = Vec::new();
                record.encode(collection, &mut bytes).expect("encodes");
                let (back, pos) = Record::decode(collection, &bytes, 0).expect("decodes");
                assert_eq!(pos, bytes.len(), "consumes exactly its own bytes");
                assert_eq!(&back, record);
                let mut reencoded = Vec::new();
                back.encode(collection, &mut reencoded).expect("re-encodes");
                assert_eq!(
                    reencoded, bytes,
                    "collection {} decode/encode is byte-identical",
                    collection.collection_id
                );
            }
        }
    }

    #[test]
    fn every_empty_state_survives_a_roundtrip() {
        // A row in which every unknown-capable field is Unknown, and one in
        // which the single nullable field is Null, so both conditional bitmaps
        // carry bits.
        let c = cols().lines;
        let record = record_from_line_row(
            &c,
            &line_row(7, KIND_MILITARY, 3, 1, 2, 3, 4, 5),
            LineJoins::default(),
            0,
            0,
        )
        .expect("maps");
        let mut bytes = Vec::new();
        record.encode(&c, &mut bytes).expect("encodes");
        let (back, _) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(back, record);
        for id in [
            field::EQUIPMENT_VARIANT_ID,
            field::REQUESTED_FACTORIES,
            field::LINE_PRIORITY,
            field::LINE_SUBTYPE,
            field::EFFICIENCY,
            field::MIO_ID,
        ] {
            assert_eq!(back.values[index_of(&c, id)], Value::Unknown);
        }
        let trailers = full_trailers();
        let record = record_from_line_row(
            &c,
            &line_row(7, KIND_NAVAL, 4, 1, 2, 3, 4, 5),
            trailers.line_joins(),
            0,
            0,
        )
        .expect("maps");
        let mut bytes = Vec::new();
        record.encode(&c, &mut bytes).expect("encodes");
        let (back, _) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(back.values[index_of(&c, field::MIO_ID)], Value::Null);
        let mut reencoded = Vec::new();
        back.encode(&c, &mut reencoded).expect("re-encodes");
        assert_eq!(reencoded, bytes);
    }

    // --- step 8.4: quantization pays for itself ------------------------------

    #[test]
    fn fixed_point_quantization_saves_bytes_against_an_all_exact_variant() {
        // Every continuous field in d11 is a decimal fixed-point source at a
        // 10^k denominator, so `Fixed(k)` is the source's own grid. This is what
        // it costs against `Exact`'s flat eight bytes.
        let c = cols();
        let quantized = c.lines.clone();
        let mut exact = c.lines.clone();
        for f in exact.fields.iter_mut() {
            if matches!(f.kind, Kind::Continuous) {
                f.quant = Quant::Exact;
            }
        }
        assert!(
            quantized
                .fields
                .iter()
                .any(|f| matches!(f.quant, Quant::Fixed(5) | Quant::Fixed(7))),
            "the quantized variant must actually be quantized"
        );
        let trailers = full_trailers();
        let row = line_row(7, KIND_MILITARY, 3, 5, 150_000, 25_000, 900_000, 1);
        let mut q_bytes = Vec::new();
        record_from_line_row(&quantized, &row, trailers.line_joins(), 0, 0)
            .expect("maps")
            .encode(&quantized, &mut q_bytes)
            .expect("encodes");
        let mut e_bytes = Vec::new();
        record_from_line_row(&exact, &row, trailers.line_joins(), 0, 0)
            .expect("maps")
            .encode(&exact, &mut e_bytes)
            .expect("encodes");
        assert!(
            q_bytes.len() < e_bytes.len(),
            "quantized {} bytes must beat exact {} bytes",
            q_bytes.len(),
            e_bytes.len()
        );
    }

    #[test]
    fn fixed_point_recovers_the_raw_integer_the_dossier_requires() {
        // "Preserve signed i64 raw values": `fixed(5)` and `fixed(7)` at the
        // source's own denominators must round-trip the raw word, or the choice
        // has cost the consumer the thing it needs. The values are the capture's
        // observed extremes plus the i32 bounds.
        use crate::value::{dequantize, quantize};
        for raw in [
            0i64,
            1,
            -1,
            100_000,
            -4_100_000,
            26_969_240_784,
            2_147_483_647,
            -2_147_483_648,
            1_414_437_500,
        ] {
            let v = raw as f64 / PRODUCTION_SCALE;
            let q = quantize(v, Quant::Fixed(5))
                .unwrap_or_else(|| panic!("raw {raw} must not escape fixed(5)"));
            assert_eq!(q, raw, "fixed(5) must store the raw word for {raw}");
            assert_eq!(dequantize(q, Quant::Fixed(5)), v);
        }
        for raw in [0i64, 1, 12_700_000, 4_149_180, 51_507, 10_000_000] {
            let v = raw as f64 / EFFICIENCY_SCALE;
            let q = quantize(v, Quant::Fixed(7))
                .unwrap_or_else(|| panic!("raw {raw} must not escape fixed(7)"));
            assert_eq!(q, raw, "fixed(7) must store the raw word for {raw}");
            assert_eq!(dequantize(q, Quant::Fixed(7)), v);
        }
    }

    // --- split_payload and the trailer chain ---------------------------------

    #[test]
    fn split_payload_enforces_the_folds_structural_rules() {
        let frame = full_frame();
        let payload = frame.bytes();
        let (header, tables) = split_payload(&payload).expect("well-formed payload");
        assert_eq!(header.country_count, 1);
        assert_eq!(header.stockpile_count, 1);
        assert_eq!(header.military_count, 1);
        assert_eq!(header.naval_count, 1);
        assert_eq!(header.construction_count, 1);
        assert_eq!(header.scale_raw, 100_000);
        assert_eq!(tables.stockpile.len(), STOCKPILE_ROW_LEN);
        assert_eq!(tables.lines.len(), 2 * LINE_ROW_LEN);
        assert_eq!(tables.construction.len(), CONSTRUCTION_ROW_LEN);
        assert_eq!(tables.trailers, &frame.trailers[..]);

        for len in 0..HEADER_LEN {
            assert!(split_payload(&payload[..len]).is_err(), "len {len}");
            assert!(split_header(&payload[..len]).is_err(), "len {len}");
        }
        let base_len = HEADER_LEN + STOCKPILE_ROW_LEN + 2 * LINE_ROW_LEN + CONSTRUCTION_ROW_LEN;
        for len in HEADER_LEN..base_len {
            assert!(split_payload(&payload[..len]).is_err(), "len {len}");
        }
        // A cut exactly at `base_len` is a legal trailer-free payload.
        assert!(split_payload(&payload[..base_len]).is_ok());
        // Counts whose byte length overflows must be rejected, not wrapped.
        for offset in [24usize, 28, 32, 36] {
            let mut huge = payload.clone();
            huge[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            assert!(split_payload(&huge).is_err(), "count at +{offset}");
        }
    }

    #[test]
    fn the_header_is_readable_before_the_row_counts_are_checked() {
        // `world_fold.rs::fold_production` checks the failure status BEFORE `world_fold.rs::fold_production` checks
        // `base_len`, so a failure-status record whose counts do not add up is
        // skipped by the recorder rather than refused. A harness that validated
        // the counts first would refuse a stream the recorder reads.
        let mut frame = full_frame();
        frame.status_flags = 0x1;
        let mut payload = frame.bytes();
        payload[24..28].copy_from_slice(&9_999u32.to_le_bytes());
        assert!(split_header(&payload).is_ok(), "the header alone still reads");
        assert_eq!(split_header(&payload).expect("header").status_flags, 0x1);
        assert!(split_payload(&payload).is_err(), "but the tables do not");
    }

    #[test]
    fn the_trailer_chain_refuses_what_the_fold_refuses() {
        let payload = full_frame().bytes();
        let (_, tables) = split_payload(&payload).expect("splits");
        let good = tables.trailers.to_vec();
        assert!(parse_trailers(&good, StatusPredicates::PERMISSIVE).is_ok());

        // An unknown magic (`world_fold.rs::parse_production_trailers`).
        let mut unknown = good.clone();
        unknown[0] = b'X';
        assert!(parse_trailers(&unknown, StatusPredicates::PERMISSIVE).is_err());
        // A trailing magic with nothing behind it.
        let mut short = good.clone();
        short.extend_from_slice(b"PVJ");
        assert!(parse_trailers(&short, StatusPredicates::PERMISSIVE).is_err());
        // A duplicate segment.
        let mut duplicated = good.clone();
        duplicated.extend(segment(b"PCP1", 8, &[pcp1_row(7, 1)]));
        assert!(parse_trailers(&duplicated, StatusPredicates::PERMISSIVE).is_err());
        // PDQ1 not last (`:27514`).
        let mut after_pdq1 = good.clone();
        after_pdq1.extend(segment(b"PVJ1", 16, &[line_join_row(7, 1, 3, 1)]));
        assert!(parse_trailers(&after_pdq1, StatusPredicates::PERMISSIVE).is_err());
        // Truncation anywhere in the chain, and every single-byte corruption:
        // a diagnosis or a parse, never a panic.
        for cut in 1..good.len() {
            let _ = parse_trailers(&good[..cut], StatusPredicates::PERMISSIVE);
        }
        for i in 0..good.len() {
            let mut bad = good.clone();
            bad[i] ^= 0xFF;
            let _ = parse_trailers(&bad, StatusPredicates::PERMISSIVE);
        }
    }

    #[test]
    fn every_segment_header_gate_is_enforced() {
        for (offset, patch) in [
            (4usize, 2u16.to_le_bytes().to_vec()), // version
            (6, 21u16.to_le_bytes().to_vec()),     // header_len
            (8, 999u32.to_le_bytes().to_vec()),    // segment_len
            (16, 1u32.to_le_bytes().to_vec()),     // reserved
        ] {
            let mut chain = segment(b"PVJ1", 16, &[line_join_row(7, 1, 3, 5)]);
            chain[offset..offset + patch.len()].copy_from_slice(&patch);
            assert!(
                parse_trailers(&chain, StatusPredicates::PERMISSIVE).is_err(),
                "PVJ1 patched at +{offset}"
            );
        }
        // A row_count whose byte length overflows usize.
        let mut chain = segment(b"PVJ1", 16, &[line_join_row(7, 1, 3, 5)]);
        chain[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse_trailers(&chain, StatusPredicates::PERMISSIVE).is_err());
    }

    #[test]
    fn every_line_join_row_gate_is_enforced() {
        let cases: Vec<(&[u8; 4], Vec<u8>)> = vec![
            (b"PVJ1", line_join_row(0, 1, 3, 5)),           // zero tag
            (b"PVJ1", line_join_row(7, 3, 3, 5)),           // kind not 1 or 2
            (b"PVJ1", line_join_row(7, 1, 3, 0)),           // variant id 0
            (b"PVJ1", line_join_row(7, 1, 3, 0x8000_0000)), // variant id > i32::MAX
            (b"PFR1", line_join_row(7, 1, 3, 100_001)),     // requested out of range
            (b"PFR1", line_join_row(7, 1, 3, u32::MAX)),    // negative as i32
            (b"PPR1", line_join_row(7, 1, 3, 4096)),        // priority out of range
            (b"PMI1", line_join_row(0, 1, 3, 5)),           // zero tag
            (b"PLS1", line_join_row(7, 1, 3, 2)),           // military kind, naval subtype
            (b"PLS1", line_join_row(7, 2, 3, 4)),           // naval kind, railway subtype
            (b"PLS1", line_join_row(7, 1, 3, 0)),           // no such subtype
        ];
        for (magic, row) in cases {
            let chain = segment(magic, 16, &[row.clone()]);
            assert!(
                parse_trailers(&chain, StatusPredicates::PERMISSIVE).is_err(),
                "{} row {row:?} must be refused",
                core::str::from_utf8(magic).expect("ascii")
            );
        }
        // The four legal (kind, subtype) pairs, and only those.
        for kind in 0..4u32 {
            for subtype in 0..6u32 {
                let legal = matches!((kind, subtype), (1, 1) | (2, 2) | (2, 3) | (1, 4));
                assert_eq!(
                    LineSubtype::from_wire(kind, subtype).is_some(),
                    legal,
                    "({kind}, {subtype})"
                );
            }
        }
        assert_eq!(
            LineSubtype::from_wire(1, 1),
            Some(LineSubtype::Military)
        );
        assert_eq!(LineSubtype::from_wire(2, 2), Some(LineSubtype::Naval));
        assert_eq!(LineSubtype::from_wire(2, 3), Some(LineSubtype::ShipRefit));
        assert_eq!(LineSubtype::from_wire(1, 4), Some(LineSubtype::RailwayGun));
        // The boundary values that must be ACCEPTED, so no bound is off by one.
        for (magic, row) in [
            (b"PFR1", line_join_row(7, 1, 3, 100_000)),
            (b"PPR1", line_join_row(7, 1, 3, 4095)),
            (b"PMI1", line_join_row(7, 1, 3, 0)),
            (b"PVJ1", line_join_row(7, 2, 3, i32::MAX as u32)),
        ] {
            let chain = segment(magic, 16, &[row]);
            assert!(
                parse_trailers(&chain, StatusPredicates::PERMISSIVE).is_ok(),
                "{} boundary row must be accepted",
                core::str::from_utf8(magic).expect("ascii")
            );
        }
        // A duplicate line key inside one segment.
        let chain = segment(
            b"PVJ1",
            16,
            &[line_join_row(7, 1, 3, 5), line_join_row(7, 1, 3, 6)],
        );
        assert!(parse_trailers(&chain, StatusPredicates::PERMISSIVE).is_err());
    }

    #[test]
    fn the_efficiency_segment_refuses_negative_channels() {
        for channel in 0..3usize {
            let mut raws = [1i64; 3];
            raws[channel] = -1;
            let chain = segment(b"PEF1", 36, &[efficiency_row(7, 1, 3, raws)]);
            assert!(
                parse_trailers(&chain, StatusPredicates::PERMISSIVE).is_err(),
                "channel {channel}"
            );
        }
        let chain = segment(b"PEF1", 36, &[efficiency_row(7, 1, 3, [0; 3])]);
        assert!(parse_trailers(&chain, StatusPredicates::PERMISSIVE).is_ok());
    }

    #[test]
    fn the_completeness_segment_refuses_unknown_bits_and_duplicates() {
        for bit in 3..32u32 {
            let chain = segment(b"PCP1", 8, &[pcp1_row(7, 1 << bit)]);
            assert!(
                parse_trailers(&chain, StatusPredicates::PERMISSIVE).is_err(),
                "bit {bit}"
            );
        }
        let chain = segment(b"PCP1", 8, &[pcp1_row(0, 1)]);
        assert!(parse_trailers(&chain, StatusPredicates::PERMISSIVE).is_err());
        let chain = segment(b"PCP1", 8, &[pcp1_row(7, 1), pcp1_row(7, 2)]);
        assert!(parse_trailers(&chain, StatusPredicates::PERMISSIVE).is_err());
    }

    #[test]
    fn the_license_segment_parses_both_versions_and_refuses_the_folds_rejects() {
        let v2 = plic(2, &[(7, vec![(11, 0, 9, 0), (12, 1, 9, 5)])]);
        let parsed = parse_trailers(&v2, StatusPredicates::PERMISSIVE).expect("v2 parses");
        let set = &parsed.licenses_by_tag.as_ref().expect("plic")[&7];
        assert_eq!(set.detail, LicenseDetail::IdsAndVariants);
        assert_eq!(set.items.len(), 2);
        assert_eq!(set.items[0].visible_order, Some(0));
        assert_eq!(set.items[1].origin_tag_db_id, Some(5));

        // v1 has no order/creator/origin and the fold sorts its ids, so the
        // keyed set's own order is the fold's order.
        let v1 = plic(1, &[(7, vec![(12, 0, 0, 0), (11, 0, 0, 0)])]);
        let parsed = parse_trailers(&v1, StatusPredicates::PERMISSIVE).expect("v1 parses");
        let set = &parsed.licenses_by_tag.as_ref().expect("plic")[&7];
        assert_eq!(set.detail, LicenseDetail::IdsOnly);
        assert_eq!(
            set.items.iter().map(|i| i.variant_id).collect::<Vec<_>>(),
            vec![11, 12],
            "world_fold.rs:27899 sorts a v1 id set"
        );
        assert_eq!(set.items[0].visible_order, None);

        // The admitted side of the identity ceiling. Without this the refusal
        // probes below pass for any predicate that is merely tight ENOUGH, and
        // an off-by-one bound (`> i32::MAX`, which admits the size ceiling
        // no database can index) is only visible as the difference between
        // these two values.
        //
        // Origin ZERO stays legal throughout: it is the null-country sentinel,
        // not an out-of-range id.
        //
        // kills: restoring `> i32::MAX` in `parse_trailers`' creator/origin
        // check, narrowing `MAX_PRODUCTION_LICENSE_TAG_DB_ID` to a
        // content-scale ceiling, and refusing origin 0.
        for admitted_origin in [0u32, 2_048, 2_049, MAX_PRODUCTION_LICENSE_TAG_DB_ID] {
            let good = plic(2, &[(7, vec![(11, 0, 9, admitted_origin)])]);
            parse_trailers(&good, StatusPredicates::PERMISSIVE)
                .unwrap_or_else(|e| panic!("origin {admitted_origin} is in range: {e}"));
        }
        let max_creator = plic(
            2,
            &[(7, vec![(11, 0, MAX_PRODUCTION_LICENSE_TAG_DB_ID, 0)])],
        );
        parse_trailers(&max_creator, StatusPredicates::PERMISSIVE)
            .expect("the largest admissible creator id is in range");

        for bad in [
            plic(3, &[(7, vec![(11, 0, 9, 0)])]),                     // version
            plic(2, &[(0, vec![(11, 0, 9, 0)])]),                     // zero tag
            plic(2, &[(7, vec![(0, 0, 9, 0)])]),                      // zero variant
            plic(2, &[(7, vec![(MAX_LICENSE_VARIANT_ID, 0, 9, 0)])]), // variant out of range
            plic(2, &[(7, vec![(11, 1, 9, 0)])]),                     // order != position
            plic(2, &[(7, vec![(11, 0, 0, 0)])]),                     // creator zero
            // One past the id ceiling. `0x8000_0000` used to stand here, and
            // both the retired `> i32::MAX` predicate and the correct
            // `> MAX_PRODUCTION_LICENSE_TAG_DB_ID` one reject it -- so it could
            // not tell them apart, and restoring the off-by-one would have left
            // this green. `MAX + 1` IS `i32::MAX`, which is exactly the value
            // the retired predicate admitted.
            plic(
                2,
                &[(7, vec![(11, 0, 9, MAX_PRODUCTION_LICENSE_TAG_DB_ID + 1)])],
            ),
            plic(
                2,
                &[(
                    7,
                    vec![(11, 0, MAX_PRODUCTION_LICENSE_TAG_DB_ID + 1, 0)],
                )],
            ), // creator one past the ceiling
            plic(2, &[(7, vec![(11, 0, 9, u32::MAX)])]), // and the sign-bit range
            plic(2, &[(7, vec![(11, 0, 9, 0), (11, 1, 9, 0)])]),      // duplicate variant
            plic(2, &[(7, vec![(11, 0, 9, 0)]), (7, vec![])]),        // duplicate tag
        ] {
            assert!(parse_trailers(&bad, StatusPredicates::PERMISSIVE).is_err());
        }
    }

    #[test]
    fn the_convoy_bank_segment_enforces_the_exact_total_law() {
        let good = pcv1(&[ConvoyCountry {
            tag: 7,
            flags: PCV1_KNOWN_FLAGS,
            assigned: 2,
            variants: vec![(60, 300_000), (61, 200_000)],
            reservations: vec![(1, 3)],
        }]);
        let parsed = parse_trailers(&good, StatusPredicates::PERMISSIVE).expect("parses");
        let bank = &parsed.convoy_bank_by_tag.as_ref().expect("pcv1")[&7];
        assert!(bank.complete && bank.reservations_complete && bank.cache_valid);
        assert_eq!(bank.assigned, 2);
        assert_eq!(bank.variants.len(), 2);
        assert_eq!(bank.reservations.len(), 1);

        // `owned_raw` disagreeing with the variant sum: the law that makes it a
        // function of carried state rather than a field.
        let mut broken = good.clone();
        let owned_at = 28 + 8;
        broken[owned_at..owned_at + 8].copy_from_slice(&999_999i64.to_le_bytes());
        assert!(parse_trailers(&broken, StatusPredicates::PERMISSIVE).is_err());
        // `available` disagreeing with `owned/scale - assigned`.
        let mut broken = good.clone();
        let available_at = 28 + 20;
        broken[available_at..available_at + 4].copy_from_slice(&99i32.to_le_bytes());
        assert!(parse_trailers(&broken, StatusPredicates::PERMISSIVE).is_err());
        // `cache_verified != cache_valid`, the dissolved duplicate bit.
        for flags in [
            PCV1_COMPLETE | PCV1_CLASSIFIER_EXACT | PCV1_CACHE_VALID,
            PCV1_COMPLETE | PCV1_CLASSIFIER_EXACT | PCV1_CACHE_VERIFIED,
        ] {
            let chain = pcv1(&[ConvoyCountry {
                tag: 7,
                flags,
                assigned: 0,
                variants: vec![],
                reservations: vec![],
            }]);
            assert!(parse_trailers(&chain, StatusPredicates::PERMISSIVE).is_err());
        }
        // `classifier_exact` clear on a complete row, the dissolved constant bit.
        let chain = pcv1(&[ConvoyCountry {
            tag: 7,
            flags: PCV1_COMPLETE,
            assigned: 0,
            variants: vec![],
            reservations: vec![],
        }]);
        assert!(parse_trailers(&chain, StatusPredicates::PERMISSIVE).is_err());
        // An incomplete row carrying data.
        let chain = pcv1(&[ConvoyCountry {
            tag: 7,
            flags: 0,
            assigned: 0,
            variants: vec![(60, 100_000)],
            reservations: vec![],
        }]);
        assert!(parse_trailers(&chain, StatusPredicates::PERMISSIVE).is_err());
        // A clean incomplete row, which is legal and folds to Unknowns.
        let chain = pcv1(&[ConvoyCountry {
            tag: 7,
            flags: 0,
            assigned: 0,
            variants: vec![],
            reservations: vec![],
        }]);
        let parsed = parse_trailers(&chain, StatusPredicates::PERMISSIVE).expect("parses");
        assert!(!parsed.convoy_bank_by_tag.as_ref().expect("pcv1")[&7].complete);
    }

    #[test]
    fn the_convoy_variant_equip_type_is_a_constant_the_segment_enforces() {
        // d62's witness-flag case: `equip_type` can only ever be 70, so carrying
        // it would be a second copy of state the decoder must keep consistent.
        let mut chain = pcv1(&[ConvoyCountry {
            tag: 7,
            flags: PCV1_KNOWN_FLAGS,
            assigned: 0,
            variants: vec![(60, 100_000)],
            reservations: vec![],
        }]);
        let variant_at = 28 + 40 + 4;
        chain[variant_at..variant_at + 4].copy_from_slice(&71u32.to_le_bytes());
        assert!(parse_trailers(&chain, StatusPredicates::PERMISSIVE).is_err());
    }

    // --- the PDQ1 stop-report's boundary -------------------------------------

    #[test]
    fn the_deployment_segment_is_scanned_structurally_and_mapped_nowhere() {
        let payload = full_frame().bytes();
        let (_, tables) = split_payload(&payload).expect("splits");
        let trailers =
            parse_trailers(tables.trailers, StatusPredicates::PERMISSIVE).expect("parses");
        let shape = trailers.deployment.as_ref().expect("pdq1");
        assert_eq!(shape.tags, vec![7]);
        assert_eq!(shape.complete_countries, 1);
        assert_eq!(shape.conveyor_rows, 1);
        assert_eq!(shape.line_rows, 1);
        assert_eq!(shape.equipment_rows, 1);
        assert_eq!(
            shape.string_bytes,
            "1. Infanterie-Division".len() + "role_infantry".len() + "infantry_equipment".len(),
            "the bytes schema-kind contract has no kind for, which is why this collection stops"
        );
        // And nothing in the schema can hold any deployment-queue string.
        assert!(
            schema()
                .collections
                .iter()
                .all(|c| c.collection_id <= CONVOY_PRIORITY_RESERVATIONS),
            "no deployment-queue collection exists"
        );
    }

    #[test]
    fn the_convoy_reservation_run_is_parsed_checked_and_mapped_in_order() {
        // The reservation run is parsed (the exact
        // total law checks every `count` against `owned_raw / 100_000`, so
        // skipping it would accept payloads the recorder refuses), counted, and
        // retained by the ordered collection.
        let c = cols();
        let mut state = State::new();
        let outcome = fold_one(&mut state, &full_frame());
        assert_eq!(outcome.reservation_rows, 2);
        assert_eq!(state.convoy_reservations.len(), 1, "one country group");
        let reservations = &state.convoy_reservations[&7];
        assert_eq!(reservations.len(), 2);
        assert_eq!(
            reservations[0].values,
            vec![Value::ContentId(7), Value::Signed(1), Value::Unsigned(3)]
        );
        assert_eq!(
            reservations[1].values,
            vec![Value::ContentId(7), Value::Signed(2), Value::Unsigned(1)]
        );
        assert_eq!(c.convoy_reservations.fields.len(), 3);
        // A reservation whose count exceeds the country's convoy total is still
        // refused, which is what parsing it buys.
        let chain = pcv1(&[ConvoyCountry {
            tag: 7,
            flags: PCV1_KNOWN_FLAGS,
            assigned: 0,
            variants: vec![(60, 100_000)],
            reservations: vec![(1, 2)],
        }]);
        assert!(parse_trailers(&chain, StatusPredicates::PERMISSIVE).is_err());
    }

    #[test]
    fn inter_country_physical_order_is_not_part_of_reservation_state() {
        let mut frame = Frame::new();
        frame.trailers = pcv1(&[
            ConvoyCountry {
                tag: 29,
                flags: PCV1_KNOWN_FLAGS,
                assigned: 0,
                variants: vec![(60, 1_000_000)],
                reservations: vec![(4, 2), (3, 1)],
            },
            ConvoyCountry {
                tag: 7,
                flags: PCV1_KNOWN_FLAGS,
                assigned: 0,
                variants: vec![(61, 1_000_000)],
                reservations: vec![(2, 3), (1, 4)],
            },
        ]);
        let mut state = State::new();
        fold_one(&mut state, &frame);

        let rows = reservation_rows(&state);
        assert_eq!(
            rows.iter()
                .map(|record| record.values.clone())
                .collect::<Vec<_>>(),
            vec![
                vec![Value::ContentId(7), Value::Signed(2), Value::Unsigned(3)],
                vec![Value::ContentId(7), Value::Signed(1), Value::Unsigned(4)],
                vec![Value::ContentId(29), Value::Signed(4), Value::Unsigned(2)],
                vec![Value::ContentId(29), Value::Signed(3), Value::Unsigned(1)],
            ],
            "country groups flatten by tag while each country's source order survives"
        );
    }

    #[test]
    fn a_byte_identical_single_country_update_does_not_move_its_group() {
        let country_7 = ConvoyCountry {
            tag: 7,
            flags: PCV1_KNOWN_FLAGS,
            assigned: 0,
            variants: vec![(60, 1_000_000)],
            reservations: vec![(2, 3), (1, 4)],
        };
        let country_29 = ConvoyCountry {
            tag: 29,
            flags: PCV1_KNOWN_FLAGS,
            assigned: 0,
            variants: vec![(61, 1_000_000)],
            reservations: vec![(4, 2), (3, 1)],
        };
        let mut initial = Frame::new();
        initial.trailers = pcv1(&[country_7, country_29]);
        let mut state = State::new();
        fold_one(&mut state, &initial);
        let before = reservation_rows(&state);

        let mut update = Frame::new();
        update.trailers = pcv1(&[ConvoyCountry {
            tag: 7,
            flags: PCV1_KNOWN_FLAGS,
            assigned: 0,
            variants: vec![(60, 1_000_000)],
            reservations: vec![(2, 3), (1, 4)],
        }]);
        fold_one(&mut state, &update);

        assert_eq!(
            state
                .convoy_reservations
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            vec![7, 29]
        );
        assert_eq!(
            reservation_rows(&state),
            before,
            "replacing tag 7 must not move it behind untouched tag 29"
        );
    }

    #[test]
    fn the_line_array_order_survives_a_frame_with_no_ppr1_segment() {
        // The regression input: two military lines for one country in wire order
        // [20, 10] and no `PPR1`. `fold_production` accepts that frame and keeps
        // `[20, 10]`; reversing the two rows folds to a different array. The
        // dense `line_ordinal` is what keeps the two distinguishable, since
        // `line_priority` is Unknown without the trailer.
        let c = cols();
        let order_of = |rows: Vec<Vec<u8>>| -> Vec<(i128, u64)> {
            let mut frame = Frame::new();
            frame.country_count = 1;
            frame.lines = rows;
            frame.trailers = segment(b"PCP1", 8, &[pcp1_row(7, PCP1_KNOWN_FLAGS)]);
            let mut state = State::new();
            fold_payload(&mut state, &c, &frame.bytes(), StatusPredicates::PERMISSIVE)
                .expect("a frame with lines and no PPR1 is one the fold accepts");
            state
                .lines
                .iter()
                .map(|(key, record)| {
                    let ordinal =
                        match record.values[index_of(&c.lines, field::LINE_ORDINAL)] {
                            Value::Unsigned(v) => v,
                            other => panic!("line_ordinal must be concrete, got {other:?}"),
                        };
                    assert_eq!(
                        record.values[index_of(&c.lines, field::LINE_PRIORITY)],
                        Value::Unknown,
                        "without PPR1 the engine ordinal is not captured"
                    );
                    (key.0[2], ordinal)
                })
                .collect()
        };
        let forward = order_of(vec![
            line_row(7, KIND_MILITARY, 20, 1, 1, 1, 1, 1),
            line_row(7, KIND_MILITARY, 10, 1, 1, 1, 1, 1),
        ]);
        let reversed = order_of(vec![
            line_row(7, KIND_MILITARY, 10, 1, 1, 1, 1, 1),
            line_row(7, KIND_MILITARY, 20, 1, 1, 1, 1, 1),
        ]);
        assert_eq!(forward, vec![(10, 1), (20, 0)]);
        assert_eq!(reversed, vec![(10, 0), (20, 1)]);
        assert_ne!(
            forward, reversed,
            "the two frames the fold distinguishes must stay distinguishable here"
        );
        // The two kinds are counted independently, because the fold keeps two
        // arrays (`world_fold.rs::fold_production`).
        let mixed = order_of(vec![
            line_row(7, KIND_MILITARY, 20, 1, 1, 1, 1, 1),
            line_row(7, KIND_NAVAL, 30, 1, 1, 1, 1, 1),
            line_row(7, KIND_NAVAL, 31, 1, 1, 1, 1, 1),
        ]);
        assert_eq!(mixed, vec![(20, 0), (30, 0), (31, 1)]);
    }

    #[test]
    fn a_reordered_convoy_variant_run_stays_distinguishable() {
        // The regression input: `[(61, 100000), (60, 100000)]` is a complete
        // PCV1 country the fold accepts, and it preserves that array order. The
        // same two variants in the other order fold to a DIFFERENT array, so the
        // two frames must reach different stored states here. A keyed set alone
        // could not tell them apart, which is why the run's ordinal is carried.
        let c = cols();
        let fold_run = |variants: Vec<(u32, i64)>| -> (Vec<(i128, u64)>, usize) {
            let mut frame = Frame::new();
            frame.country_count = 1;
            frame.trailers = {
                let mut t = segment(b"PCP1", 8, &[pcp1_row(7, PCP1_KNOWN_FLAGS)]);
                t.extend(pcv1(&[ConvoyCountry {
                    tag: 7,
                    flags: PCV1_KNOWN_FLAGS,
                    assigned: 0,
                    variants,
                    reservations: vec![],
                }]));
                t
            };
            let mut state = State::new();
            let outcome =
                fold_payload(&mut state, &c, &frame.bytes(), StatusPredicates::PERMISSIVE)
                    .expect("both runs are legal payloads");
            let rows = state
                .convoy_variants
                .iter()
                .map(|(key, record)| {
                    let ordinal = match record.values
                        [index_of(&c.convoy_variants, field::CONVOY_VARIANT_ORDINAL)]
                    {
                        Value::Unsigned(v) => v,
                        other => panic!("the ordinal must be concrete, got {other:?}"),
                    };
                    (key.0[1], ordinal)
                })
                .collect();
            (rows, outcome.unordered_variant_runs)
        };
        let (ascending, ascending_flag) = fold_run(vec![(60, 100_000), (61, 100_000)]);
        let (descending, descending_flag) = fold_run(vec![(61, 100_000), (60, 100_000)]);
        assert_eq!(ascending, vec![(60, 0), (61, 1)]);
        assert_eq!(descending, vec![(60, 1), (61, 0)]);
        assert_ne!(
            ascending, descending,
            "the two frames the fold distinguishes must stay distinguishable here"
        );
        assert_eq!(ascending_flag, 0);
        assert_eq!(
            descending_flag, 1,
            "the counter reports that the ordinal was load-bearing for this run"
        );
        // And the two states really differ under the codec's own equality, not
        // just under this test's projection.
        let record_of = |variants: Vec<(u32, i64)>| -> KeyedState {
            let mut frame = Frame::new();
            frame.country_count = 1;
            frame.trailers = {
                let mut t = segment(b"PCP1", 8, &[pcp1_row(7, PCP1_KNOWN_FLAGS)]);
                t.extend(pcv1(&[ConvoyCountry {
                    tag: 7,
                    flags: PCV1_KNOWN_FLAGS,
                    assigned: 0,
                    variants,
                    reservations: vec![],
                }]));
                t
            };
            let mut state = State::new();
            fold_payload(&mut state, &c, &frame.bytes(), StatusPredicates::PERMISSIVE)
                .expect("folds");
            state.convoy_variants
        };
        let a = record_of(vec![(60, 100_000), (61, 100_000)]);
        let b = record_of(vec![(61, 100_000), (60, 100_000)]);
        assert!(
            !diff(&c.convoy_variants, &a, &b).is_empty(),
            "a reorder must produce a delta, not an UNCHANGED hour"
        );
    }

    #[test]
    fn every_dissolved_value_is_recomputable_from_the_reconstructed_state() {
        // The d24 rule's other half: a value left off the wire because the fold
        // enforces it as a function of carried state must actually be
        // recoverable from the state a reader reconstructs — not just from the
        // payload. Every one of these is checked against the frame it came from.
        let schema = schema();
        let c = cols();
        let mut state = State::new();
        fold_one(&mut state, &full_frame());
        let encoded = {
            let mut bytes = Vec::new();
            encode_shaped_baseline(&schema, &state.states(), &mut bytes).expect("encodes");
            let (decoded, _) =
                decode_shaped_baseline(&schema, &bytes, 0).expect("decodes");
            decoded
                .into_iter()
                .take(7)
                .map(|state| match state {
                    CollectionState::Keyed(state) => state,
                    _ => panic!("the first seven d11 collections are keyed"),
                })
                .collect::<Vec<_>>()
        };
        let variants = &encoded[6];
        let bank = &encoded[5];

        // `owned_raw` = the clamped sum of the country's variant raws
        // (`world_fold.rs::parse_country_convoy_bank_segment`). The frame carried 300_000 + 200_000.
        let owned_raw = variants
            .iter()
            .filter(|(key, _)| key.first() == 7)
            .map(|(_, record)| {
                match record.values[index_of(&c.convoy_variants, field::CONVOY_AMOUNT_RAW)] {
                    Value::Unsigned(v) => v as i64,
                    other => panic!("{other:?}"),
                }
            })
            .fold(0i64, |sum, raw| {
                sum.saturating_add(raw).min(PCV1_OWNED_RAW_CAP)
            });
        assert_eq!(owned_raw, 500_000);
        // `convoys_count` (`:13011`) and `available` (`:13014`, law at `:27741`).
        let assigned = match bank[&Key(vec![7])].values[index_of(&c.convoy_bank, field::ASSIGNED)] {
            Value::Unsigned(v) => v as i64,
            other => panic!("{other:?}"),
        };
        assert_eq!(owned_raw / PCV1_SCALE, 5, "convoys_count");
        assert_eq!(owned_raw / PCV1_SCALE - assigned, 3, "available");
        // The fold's per-variant `amount` projection (`:13027`).
        assert_eq!(convoy_amount(300_000), 3.0);

        // `stockpile_total` (`:12568`) over the reconstructed stockpile.
        let stockpile_total: f64 = encoded[1]
            .values()
            .map(
                |record| match record.values[index_of(&c.stockpile, field::AMOUNT)] {
                    Value::Continuous(v) => v,
                    other => panic!("{other:?}"),
                },
            )
            .sum();
        assert_eq!(stockpile_total, 123.456_78);

        // `military_factories` / `naval_factories` (`:12660`, `:12663`) —
        // partitioned by the fold's own `kind == 2` test, over the key.
        let factories = |naval: bool| -> i64 {
            encoded[2]
                .iter()
                .filter(|(key, _)| (key.0[1] == KIND_NAVAL as i128) == naval)
                .map(|(_, record)| {
                    match record.values[index_of(&c.lines, field::ACTIVE_FACTORIES)] {
                        Value::Signed(v) => v,
                        other => panic!("{other:?}"),
                    }
                })
                .sum()
        };
        assert_eq!(factories(false), 5, "military_factories");
        assert_eq!(factories(true), 2, "naval_factories");
    }

    #[test]
    fn a_pure_reservation_reorder_changes_the_ordered_collection() {
        // The regression input, from `deepwar_t1` tag 29 at hours 60832407
        // and 60832417: the same `{priority, count}` multiset in a different
        // source order. `fold_production` stores an ordered array and those two
        // hours differ; any keyed set over `(tag, priority)` stores them
        // identically. This test retains the counter-evidence that forced an
        // ordered occurrence list rather than allowing the implementation to
        // drift back to a keyed projection.
        let first = [(2i32, 0i32), (4, 7), (5, 0), (3, 0)];
        let second = [(2i32, 0i32), (3, 0), (4, 7), (5, 0)];
        assert_ne!(first, second, "the fold's arrays differ");
        let mut a: Vec<(i32, i32)> = first.to_vec();
        let mut b: Vec<(i32, i32)> = second.to_vec();
        a.sort_unstable();
        b.sort_unstable();
        assert_eq!(a, b, "but every keyed projection of them is identical");
        let mut folded = Vec::new();
        let mut state = State::new();
        for order in [first, second] {
            let chain = pcv1(&[ConvoyCountry {
                tag: 29,
                flags: PCV1_KNOWN_FLAGS,
                assigned: 0,
                variants: vec![(60, 1_000_000)],
                reservations: order.to_vec(),
            }]);
            let parsed = parse_trailers(&chain, StatusPredicates::PERMISSIVE).expect("parses");
            let bank = &parsed.convoy_bank_by_tag.as_ref().expect("pcv1")[&29];
            assert_eq!(
                bank.reservations
                    .iter()
                    .map(|r| (r.priority, r.count))
                    .collect::<Vec<_>>(),
                order.to_vec(),
                "the parser preserves source order"
            );
            let mut frame = Frame::new();
            frame.trailers = chain;
            fold_payload(&mut state, &cols(), &frame.bytes(), StatusPredicates::PERMISSIVE)
                .expect("folds");
            folded.push(state.states());
        }
        assert_ne!(
            folded[0][7], folded[1][7],
            "the two within-country orders remain distinct"
        );

        let schema = schema();
        let collection = &schema.collections[7];
        let delta = diff_shaped(collection, &folded[0][7], &folded[1][7])
            .expect("both states match the schema")
            .expect("a pure reorder is a change");
        let expected_rows = match &folded[1][7] {
            CollectionState::OrderedOccurrence(rows) => rows,
            _ => panic!("collection 8 is ordered-occurrence state"),
        };
        match &delta {
            crate::body::CollectionDelta::OrderedOccurrence(rows) => {
                assert_eq!(rows, expected_rows, "the replacement carries the second order")
            }
            other => panic!("pure reorder produced the wrong contribution: {other:?}"),
        }

        let mut deltas = vec![None; schema.collections.len()];
        deltas[7] = Some(delta.clone());
        let mut wire = Vec::new();
        encode_shaped_delta(&schema, &folded[0], &deltas, &mut wire)
            .expect("the pure reorder encodes");
        assert_eq!(wire[0], 0x80, "only collection 8's bitmap bit is set");
        let (decoded, end) =
            decode_shaped_delta(&schema, &folded[0], &wire, 0).expect("the delta decodes");
        assert_eq!(end, wire.len());
        assert_eq!(decoded[7].as_ref(), Some(&delta));
    }

    #[test]
    fn the_deployment_carrier_status_predicates_are_the_callers() {
        // `world_fold.rs::parse_production_deployment_queue_segment`, with the two `compact_contract`
        // predicates the reference library must not depend on.
        let mut frame = full_frame();
        frame.status_flags = 0x40;
        frame.stockpile.clear();
        frame.lines.clear();
        frame.construction.clear();
        frame.trailers = {
            let mut t = Vec::new();
            t.extend(segment(b"PCP1", 8, &[pcp1_row(7, PCP1_KNOWN_FLAGS)]));
            t.extend(pdq1(frame.elapsed_wall_ms, 0x40, &[(7, true)]));
            t
        };
        let payload = frame.bytes();
        let (_, tables) = split_payload(&payload).expect("splits");
        assert!(parse_trailers(
            tables.trailers,
            StatusPredicates {
                unknown_flags: |f| f & 0x40,
                is_failure: |_| false,
            }
        )
        .is_err());
        assert!(parse_trailers(
            tables.trailers,
            StatusPredicates {
                unknown_flags: |_| 0,
                is_failure: |_| true,
            }
        )
        .is_err());
        assert!(parse_trailers(tables.trailers, StatusPredicates::PERMISSIVE).is_ok());
    }

    #[test]
    fn the_five_way_deployment_cross_check_is_carried() {
        let mismatches: Vec<Box<dyn Fn(&mut Frame)>> = vec![
            Box::new(|f: &mut Frame| f.elapsed_wall_ms = 1),
            Box::new(|f: &mut Frame| f.country_count = 2),
            Box::new(|f: &mut Frame| f.changed_country_count = Some(0)),
            Box::new(|f: &mut Frame| f.status_flags = 0x400),
        ];
        for (i, patch) in mismatches.iter().enumerate() {
            let mut frame = full_frame();
            patch(&mut frame);
            let mut state = State::new();
            assert!(
                fold_payload(
                    &mut state,
                    &cols(),
                    &frame.bytes(),
                    StatusPredicates::PERMISSIVE
                )
                .is_err(),
                "cross-check {i} must refuse the frame"
            );
            assert_eq!(state.rows(), 0, "a refused frame leaves the state untouched");
        }
        // The carrier covering a different country than PCP1.
        let mut frame = full_frame();
        frame.stockpile.clear();
        frame.lines.clear();
        frame.construction.clear();
        frame.trailers = {
            let mut t = Vec::new();
            t.extend(segment(b"PCP1", 8, &[pcp1_row(7, PCP1_KNOWN_FLAGS)]));
            t.extend(pdq1(frame.elapsed_wall_ms, 0, &[(8, true)]));
            t
        };
        let mut state = State::new();
        assert!(fold_payload(
            &mut state,
            &cols(),
            &frame.bytes(),
            StatusPredicates::PERMISSIVE
        )
        .is_err());
    }

    // --- coverage bijections -------------------------------------------------

    #[test]
    fn a_present_join_that_does_not_cover_every_base_line_refuses_the_frame() {
        for magic in [b"PVJ1", b"PFR1", b"PPR1", b"PLS1", b"PMI1"] {
            let mut frame = full_frame();
            // Drop the naval line's join row from one segment, keeping the rest.
            let mut trailers = Vec::new();
            trailers.extend(plic(2, &[(7, vec![(11, 0, 9, 8)])]));
            for (m, rows) in [
                (
                    b"PVJ1",
                    vec![(3i32, 500u32, KIND_MILITARY), (4, 501, KIND_NAVAL)],
                ),
                (b"PFR1", vec![(3, 6, KIND_MILITARY), (4, 3, KIND_NAVAL)]),
                (b"PPR1", vec![(3, 0, KIND_MILITARY), (4, 1, KIND_NAVAL)]),
                (b"PLS1", vec![(3, 1, KIND_MILITARY), (4, 3, KIND_NAVAL)]),
                (b"PMI1", vec![(3, 77, KIND_MILITARY), (4, 0, KIND_NAVAL)]),
            ] {
                let rows: Vec<Vec<u8>> = rows
                    .into_iter()
                    .filter(|(line_id, _, _)| !(m == magic && *line_id == 4))
                    .map(|(line_id, value, kind)| line_join_row(7, kind, line_id, value))
                    .collect();
                trailers.extend(segment(m, 16, &rows));
            }
            trailers.extend(segment(
                b"PEF1",
                36,
                &[
                    efficiency_row(7, KIND_MILITARY, 3, [5_000_000, 10_000_000, 12_345]),
                    efficiency_row(7, KIND_NAVAL, 4, [1, 2, 3]),
                ],
            ));
            trailers.extend(segment(b"PCP1", 8, &[pcp1_row(7, PCP1_KNOWN_FLAGS)]));
            frame.trailers = trailers;
            let mut state = State::new();
            assert!(
                fold_payload(
                    &mut state,
                    &cols(),
                    &frame.bytes(),
                    StatusPredicates::PERMISSIVE
                )
                .is_err(),
                "{} missing a base line must refuse the frame",
                core::str::from_utf8(magic).expect("ascii")
            );
        }
        // A join carrying a line the base table does not: the other half of the
        // bijection (`world_fold.rs::fold_production`).
        let mut frame = full_frame();
        frame.trailers = {
            let mut t = Vec::new();
            t.extend(segment(
                b"PVJ1",
                16,
                &[
                    line_join_row(7, KIND_MILITARY, 3, 500),
                    line_join_row(7, KIND_NAVAL, 4, 501),
                    line_join_row(7, KIND_NAVAL, 5, 502),
                ],
            ));
            t.extend(segment(b"PCP1", 8, &[pcp1_row(7, PCP1_KNOWN_FLAGS)]));
            t
        };
        let mut state = State::new();
        assert!(fold_payload(
            &mut state,
            &cols(),
            &frame.bytes(),
            StatusPredicates::PERMISSIVE
        )
        .is_err());
    }

    #[test]
    fn the_pcp1_country_bijection_and_its_row_witnesses_are_enforced() {
        // Row count vs `country_count` (`world_fold.rs::fold_production`).
        let mut frame = full_frame();
        frame.country_count = 2;
        frame.changed_country_count = Some(2);
        let mut state = State::new();
        assert!(fold_payload(
            &mut state,
            &cols(),
            &frame.bytes(),
            StatusPredicates::PERMISSIVE
        )
        .is_err());
        // A keyed country the trailer does not name (`:12782`).
        let mut frame = full_frame();
        frame.lines.clear();
        frame.construction.clear();
        frame.stockpile.push(stockpile_row(8, 500, 1));
        frame.trailers = segment(b"PCP1", 8, &[pcp1_row(7, PCP1_KNOWN_FLAGS)]);
        let mut state = State::new();
        assert!(fold_payload(
            &mut state,
            &cols(),
            &frame.bytes(),
            StatusPredicates::PERMISSIVE
        )
        .is_err());
        // Stockpile rows under a country the trailer marks unreadable (`:12794`).
        let mut frame = full_frame();
        frame.lines.clear();
        frame.construction.clear();
        frame.trailers = segment(
            b"PCP1",
            8,
            &[pcp1_row(7, PCP1_MILITARY_LINES | PCP1_NAVAL_LINES)],
        );
        let mut state = State::new();
        assert!(fold_payload(
            &mut state,
            &cols(),
            &frame.bytes(),
            StatusPredicates::PERMISSIVE
        )
        .is_err());
        // Line rows under a country the trailer marks unreadable, per kind
        // (`:12802`).
        for (cleared, kind) in [
            (PCP1_MILITARY_LINES, KIND_MILITARY),
            (PCP1_NAVAL_LINES, KIND_NAVAL),
        ] {
            let mut frame = full_frame();
            frame.stockpile.clear();
            frame.construction.clear();
            frame.lines = vec![line_row(7, kind, 3, 1, 1, 1, 1, 1)];
            frame.trailers = segment(b"PCP1", 8, &[pcp1_row(7, PCP1_KNOWN_FLAGS & !cleared)]);
            let mut state = State::new();
            assert!(
                fold_payload(
                    &mut state,
                    &cols(),
                    &frame.bytes(),
                    StatusPredicates::PERMISSIVE
                )
                .is_err(),
                "kind {kind} with its witness clear"
            );
        }
    }

    #[test]
    fn a_duplicate_key_refuses_the_frame_in_every_keyed_table() {
        // `world_fold.rs:12562` and `:12581` refuse these outright. The
        // construction table's key is UNENFORCED by the fold, which pushes both
        // rows into an array; a keyed set cannot, so it refuses rather than
        // silently dropping one — and the capture shows the key is in fact
        // unique in all 903 hours and 67,848 rows.
        let mut frame = full_frame();
        frame.stockpile.push(stockpile_row(7, 500, 5));
        let mut state = State::new();
        assert!(fold_payload(
            &mut state,
            &cols(),
            &frame.bytes(),
            StatusPredicates::PERMISSIVE
        )
        .is_err());

        let mut frame = full_frame();
        frame
            .lines
            .push(line_row(7, KIND_MILITARY, 3, 1, 1, 1, 1, 1));
        let mut state = State::new();
        assert!(fold_payload(
            &mut state,
            &cols(),
            &frame.bytes(),
            StatusPredicates::PERMISSIVE
        )
        .is_err());

        // The construction table has NO key the fold enforces, so nothing about
        // it is refused — see the next test.
    }

    #[test]
    fn no_construction_frame_the_fold_accepts_is_ever_refused() {
        // `fold_production` enforces no construction key at all (`:12667`-
        // `:12689`): it appends every row to the country's array. The reviewer's
        // input — two rows sharing every identity word, differing only in their
        // values — must therefore be held, not refused. The dense ordinal is
        // what makes that possible.
        let c = cols();
        let mut frame = full_frame();
        frame.construction = vec![
            construction_row(7, 90, 31, 4, 2_000_000, 500_000, 0),
            construction_row(7, 90, 32, 1, 1_000_000, 250_000, 0),
        ];
        let mut state = State::new();
        fold_payload(&mut state, &c, &frame.bytes(), StatusPredicates::PERMISSIVE)
            .expect("the fold accepts this frame, so this boundary must too");
        assert_eq!(state.construction.len(), 2, "both rows are held");
        assert!(state.construction.contains_key(&Key(vec![7, 0, 90])));
        assert!(state.construction.contains_key(&Key(vec![7, 1, 90])));
        assert_eq!(
            state.construction[&Key(vec![7, 1, 90])].values
                [index_of(&c.construction, field::TEMPLATE_ID)],
            Value::ContentId(32),
            "the two rows keep their own field values"
        );
        // Both rows carry the same sparse `+24` word, which is data, not a key.
        for ordinal in [0i128, 1] {
            assert_eq!(
                state.construction[&Key(vec![7, ordinal, 90])].values
                    [index_of(&c.construction, field::CONSTRUCTION_PRIORITY)],
                Value::Unsigned(0)
            );
        }
        // The fold's array order IS the keyed set's own order, because the dense
        // ordinal leads the line_id.
        let mut frame = full_frame();
        frame.construction = vec![
            construction_row(7, 91, 31, 0, 0, 0, 5),
            construction_row(7, 90, 31, 0, 0, 0, 9),
            construction_row(8, 12, 31, 0, 0, 0, 0),
        ];
        frame.country_count = 2;
        frame.lines.clear();
        frame.stockpile.clear();
        frame.trailers = segment(
            b"PCP1",
            8,
            &[pcp1_row(7, PCP1_KNOWN_FLAGS), pcp1_row(8, PCP1_KNOWN_FLAGS)],
        );
        let mut state = State::new();
        fold_payload(&mut state, &c, &frame.bytes(), StatusPredicates::PERMISSIVE)
            .expect("folds");
        let order: Vec<(i128, i128)> = state
            .construction
            .keys()
            .map(|k| (k.0[0], k.0[2]))
            .collect();
        assert_eq!(
            order,
            vec![(7, 91), (7, 90), (8, 12)],
            "wire order within each country, even though 91 > 90"
        );
    }

    // --- the accumulator -----------------------------------------------------

    #[test]
    fn the_fold_replaces_per_country_and_leaves_untouched_countries_alone() {
        // The property that makes d11 an accumulator rather than a per-frame
        // snapshot (`world_fold.rs::fold_production` builds `per_tag` fresh, `world_fold.rs::fold_production`
        // writes it into one country).
        let c = cols();
        let mut state = State::new();
        let mut first = full_frame();
        first.country_count = 2;
        first.lines.clear();
        first.construction.clear();
        first.stockpile.push(stockpile_row(8, 600, 700_000));
        first.trailers = segment(
            b"PCP1",
            8,
            &[pcp1_row(7, PCP1_KNOWN_FLAGS), pcp1_row(8, PCP1_KNOWN_FLAGS)],
        );
        let outcome = fold_payload(
            &mut state,
            &c,
            &first.bytes(),
            StatusPredicates::PERMISSIVE,
        )
        .expect("folds");
        assert_eq!(outcome.touched_countries, 2);
        assert_eq!(state.stockpile.len(), 2);
        assert_eq!(state.countries.len(), 2);

        // A second frame touching only country 7, with a different stockpile.
        let mut second = Frame::new();
        second.elapsed_wall_ms = 5;
        second.country_count = 1;
        second.stockpile = vec![stockpile_row(7, 999, 1)];
        second.trailers = segment(b"PCP1", 8, &[pcp1_row(7, PCP1_KNOWN_FLAGS)]);
        let outcome = fold_payload(
            &mut state,
            &c,
            &second.bytes(),
            StatusPredicates::PERMISSIVE,
        )
        .expect("folds");
        assert_eq!(outcome.touched_countries, 1);
        assert!(
            state.stockpile.contains_key(&Key(vec![8, 600])),
            "an untouched country keeps its production block"
        );
        assert!(
            !state.stockpile.contains_key(&Key(vec![7, 500])),
            "a touched country's old rows are replaced, not merged"
        );
        assert!(state.stockpile.contains_key(&Key(vec![7, 999])));
        assert_eq!(state.stockpile.len(), 2);
        assert!(state.licenses.is_empty(), "the licence set went with the block");
    }

    #[test]
    fn the_convoy_bank_is_a_separate_write_path_with_its_own_key_set() {
        let c = cols();
        let mut state = State::new();
        fold_one(&mut state, &full_frame());
        assert_eq!(state.convoy_bank.len(), 1);
        assert_eq!(state.convoy_variants.len(), 2);

        // A frame that replaces country 7's production block but carries no
        // PCV1: `world_fold.rs::fold_production` is gated on the trailer, so the bank
        // survives.
        let mut frame = Frame::new();
        frame.country_count = 1;
        frame.stockpile = vec![stockpile_row(7, 1, 0)];
        frame.trailers = segment(b"PCP1", 8, &[pcp1_row(7, PCP1_KNOWN_FLAGS)]);
        let outcome = fold_payload(
            &mut state,
            &c,
            &frame.bytes(),
            StatusPredicates::PERMISSIVE,
        )
        .expect("folds");
        assert_eq!(outcome.touched_convoy_countries, 0);
        assert_eq!(state.convoy_bank.len(), 1);
        assert_eq!(state.convoy_variants.len(), 2);
    }

    #[test]
    fn an_incomplete_convoy_bank_erases_the_countrys_previous_rows() {
        // `world_fold.rs::fold_production` removes every convoy leaf before the
        // completeness witnesses go back and returns early on `!complete`.
        let c = cols();
        let mut state = State::new();
        fold_one(&mut state, &full_frame());
        assert_eq!(state.convoy_variants.len(), 2);

        let mut frame = Frame::new();
        frame.country_count = 1;
        frame.trailers = {
            let mut t = segment(b"PCP1", 8, &[pcp1_row(7, PCP1_KNOWN_FLAGS)]);
            t.extend(pcv1(&[ConvoyCountry {
                tag: 7,
                flags: 0,
                assigned: 0,
                variants: vec![],
                reservations: vec![],
            }]));
            t
        };
        let outcome =
            fold_payload(&mut state, &c, &frame.bytes(), StatusPredicates::PERMISSIVE)
                .expect("folds");
        assert!(state.convoy_variants.is_empty());
        assert_eq!(outcome.reservation_rows, 0);
        assert_eq!(state.convoy_bank.len(), 1);
        let record = &state.convoy_bank[&Key(vec![7])];
        assert_eq!(
            record.values[index_of(&c.convoy_bank, field::CONVOYS_COMPLETE)],
            Value::Bool(false)
        );
        assert_eq!(
            record.values[index_of(&c.convoy_bank, field::ASSIGNED)],
            Value::Unknown
        );
    }

    #[test]
    fn a_refused_frame_leaves_the_accumulator_exactly_as_it_was() {
        let c = cols();
        let mut state = State::new();
        fold_one(&mut state, &full_frame());
        let before = state.to_vec();
        let mut bad = full_frame();
        bad.stockpile.push(stockpile_row(7, 500, 5)); // duplicate key
        assert!(fold_payload(&mut state, &c, &bad.bytes(), StatusPredicates::PERMISSIVE).is_err());
        assert_eq!(state.to_vec(), before);
    }

    #[test]
    fn the_license_detail_enum_records_which_carrier_the_frame_had() {
        let c = cols();
        for (trailer, expected, rows) in [
            (Vec::new(), LicenseDetail::NotMeasured, 0usize),
            (
                plic(1, &[(7, vec![(11, 0, 0, 0)])]),
                LicenseDetail::IdsOnly,
                1,
            ),
            (
                plic(2, &[(7, vec![(11, 0, 9, 0)])]),
                LicenseDetail::IdsAndVariants,
                1,
            ),
            // A country present in PLIC with zero ids: measured-empty, which is
            // `[]` in the fold and NOT the null of NotMeasured.
            (plic(2, &[(7, vec![])]), LicenseDetail::IdsAndVariants, 0),
        ] {
            let mut frame = Frame::new();
            frame.country_count = 1;
            frame.stockpile = vec![stockpile_row(7, 1, 0)];
            frame.trailers = {
                let mut t = trailer.clone();
                t.extend(segment(b"PCP1", 8, &[pcp1_row(7, PCP1_KNOWN_FLAGS)]));
                t
            };
            let mut state = State::new();
            fold_payload(&mut state, &c, &frame.bytes(), StatusPredicates::PERMISSIVE)
                .expect("folds");
            assert_eq!(
                state.countries[&Key(vec![7])].values
                    [index_of(&c.countries, field::LICENSE_DETAIL)],
                Value::Enum(expected.code()),
                "detail {expected:?}"
            );
            assert_eq!(state.licenses.len(), rows);
        }
    }

    #[test]
    fn a_plic_only_country_is_still_a_keyed_country() {
        // `world_fold.rs::fold_production` adds PLIC's tags to `per_tag`, so a country with
        // no base rows at all still gets a production block.
        let c = cols();
        let mut frame = Frame::new();
        frame.country_count = 1;
        frame.trailers = {
            let mut t = plic(2, &[(7, vec![(11, 0, 9, 0)])]);
            t.extend(segment(b"PCP1", 8, &[pcp1_row(7, 0)]));
            t
        };
        let mut state = State::new();
        let outcome = fold_payload(&mut state, &c, &frame.bytes(), StatusPredicates::PERMISSIVE)
            .expect("folds");
        assert_eq!(outcome.touched_countries, 1);
        assert_eq!(state.countries.len(), 1);
        assert_eq!(state.licenses.len(), 1);
        assert_eq!(
            state.countries[&Key(vec![7])].values
                [index_of(&c.countries, field::STOCKPILE_COMPLETE)],
            Value::Bool(false),
            "PCP1 says the vector was unreadable, which is a measured false"
        );
    }

    #[test]
    fn a_frame_with_no_rows_and_no_trailers_touches_nothing() {
        // 55 of the 903 `deepwar_t1` hours are exactly this: an all-zero header
        // and not one trailer. It must not clear the accumulator.
        let c = cols();
        let mut state = State::new();
        fold_one(&mut state, &full_frame());
        let before = state.to_vec();
        let outcome = fold_payload(
            &mut state,
            &c,
            &Frame::new().bytes(),
            StatusPredicates::PERMISSIVE,
        )
        .expect("an empty frame is well-formed");
        assert_eq!(outcome.touched_countries, 0);
        assert_eq!(outcome.uncovered_countries, 0);
        assert_eq!(state.to_vec(), before);
    }

    #[test]
    fn an_uncovered_country_is_counted_not_excluded() {
        // `world_fold.rs::fold_production` badges the hour and folds the rest.
        let c = cols();
        let mut frame = Frame::new();
        frame.country_count = 3;
        frame.stockpile = vec![stockpile_row(7, 1, 0)];
        let mut state = State::new();
        let outcome = fold_payload(&mut state, &c, &frame.bytes(), StatusPredicates::PERMISSIVE)
            .expect("folds");
        assert_eq!(outcome.uncovered_countries, 2);
        assert_eq!(state.countries.len(), 1);
    }

    // --- the block grammar, end to end ---------------------------------------

    #[test]
    fn a_baseline_and_a_delta_replay_back_to_the_folded_state() {
        // The measurement bin's property, in miniature: encode a baseline, then
        // a stored-minimal delta, and decode both against the reader's own
        // reconstructed prior.
        let schema = schema();
        let c = cols();
        let mut state = State::new();
        fold_one(&mut state, &full_frame());
        let first = state.states();

        let mut bytes = Vec::new();
        encode_shaped_baseline(&schema, &first, &mut bytes).expect("baseline encodes");
        let (decoded, consumed) =
            decode_shaped_baseline(&schema, &bytes, 0).expect("decodes");
        assert_eq!(consumed, bytes.len());
        assert_eq!(decoded, first);

        // A second frame: country 7's block is replaced and country 8 arrives.
        let mut second = Frame::new();
        second.elapsed_wall_ms = 9;
        second.country_count = 2;
        second.stockpile = vec![stockpile_row(7, 500, 12_345_679), stockpile_row(8, 600, 5)];
        second.trailers = segment(
            b"PCP1",
            8,
            &[pcp1_row(7, PCP1_KNOWN_FLAGS), pcp1_row(8, PCP1_KNOWN_FLAGS)],
        );
        fold_payload(&mut state, &c, &second.bytes(), StatusPredicates::PERMISSIVE)
            .expect("folds");
        let next = state.states();

        let collections: Vec<&Collection> = c.as_slice().to_vec();
        let deltas: Vec<_> = collections
            .iter()
            .enumerate()
            .map(|(i, collection)| {
                diff_shaped(collection, &first[i], &next[i]).expect("states encode")
            })
            .collect();
        assert!(
            deltas.iter().any(Option::is_some),
            "the second frame moved something"
        );
        let mut delta_bytes = Vec::new();
        encode_shaped_delta(&schema, &first, &deltas, &mut delta_bytes)
            .expect("delta encodes");
        let (decoded_deltas, consumed) =
            decode_shaped_delta(&schema, &first, &delta_bytes, 0).expect("delta decodes");
        assert_eq!(consumed, delta_bytes.len());
        let mut replayed = first.clone();
        for (i, delta) in decoded_deltas.iter().enumerate() {
            if let Some(delta) = delta {
                apply_shaped(&mut replayed[i], delta).expect("matching shape");
            }
        }
        for (i, collection) in collections.iter().enumerate() {
            assert!(
                collection_stored_eq(collection, &replayed[i], &next[i])
                    .expect("states encode"),
                "collection {} is not stored-equal to the fold after replay",
                collection.collection_id
            );
        }
        // Country 7's line, construction and licence rows went away with its
        // replaced block; tombstones are what carry that.
        assert!(
            decoded_deltas
                .iter()
                .flatten()
                .any(|d| matches!(d, crate::body::CollectionDelta::Keyed(d) if !d.tombstones.is_empty())),
            "a replaced country's dropped rows must be tombstoned"
        );
    }

    // --- totality ------------------------------------------------------------

    #[test]
    fn the_mappers_refuse_a_collection_they_cannot_fill() {
        let c = cols();
        let trailers = full_trailers();
        let mut lines = c.lines.clone();
        lines.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_line_row(
            &lines,
            &line_row(7, 1, 3, 0, 0, 0, 0, 0),
            trailers.line_joins(),
            0,
            0
        )
        .is_err());
        let mut stockpile = c.stockpile.clone();
        stockpile.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_stockpile_row(&stockpile, &stockpile_row(7, 1, 0), 0).is_err());
        let mut construction = c.construction.clone();
        construction.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_construction_row(
            &construction,
            &construction_row(7, 1, 0, 0, 0, 0, 0),
            0
        )
        .is_err());
        let mut countries = c.countries.clone();
        countries.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_country(&countries, None, LicenseDetail::NotMeasured, 0).is_err());
        let mut licenses = c.licenses.clone();
        licenses.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_license_item(
            &licenses,
            &LicenseItem {
                variant_id: 1,
                visible_order: None,
                creator_tag_db_id: None,
                origin_tag_db_id: None,
            },
            0
        )
        .is_err());
    }

    #[test]
    fn fold_payload_never_panics_on_arbitrary_bytes() {
        // The public boundary reads a file from disk and must return a
        // diagnosis, never unwind.
        let c = cols();
        let mut xorshift = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            xorshift ^= xorshift << 13;
            xorshift ^= xorshift >> 7;
            xorshift ^= xorshift << 17;
            xorshift
        };
        for _ in 0..4_000 {
            let len = (next() as usize) % 500;
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                *byte = next() as u8;
            }
            let mut state = State::new();
            let _ = fold_payload(&mut state, &c, &bytes, StatusPredicates::PERMISSIVE);
            let _ = split_payload(&bytes);
            let _ = parse_trailers(&bytes, StatusPredicates::PERMISSIVE);
        }
        // Byte-by-byte corruption of a genuinely well-formed payload, and every
        // truncation of it.
        let good = full_frame().bytes();
        for i in 0..good.len() {
            let mut bad = good.clone();
            bad[i] ^= 0xFF;
            let mut state = State::new();
            let _ = fold_payload(&mut state, &c, &bad, StatusPredicates::PERMISSIVE);
        }
        for cut in 0..good.len() {
            let mut state = State::new();
            let _ = fold_payload(&mut state, &c, &good[..cut], StatusPredicates::PERMISSIVE);
        }
    }

    #[test]
    fn reservation_count_uses_the_parsers_complete_nonnegative_i32_range() {
        let collection = cols().convoy_reservations;
        let at_max = record_from_convoy_reservation(
            &collection,
            7,
            &ConvoyReservation { priority: i32::MIN, count: i32::MAX },
            0,
        )
        .expect("the parser accepts i32::MAX");
        assert_eq!(
            at_max.values,
            vec![
                Value::ContentId(7),
                Value::Signed(i32::MIN as i64),
                Value::Unsigned(i32::MAX as u64),
            ]
        );
        assert!(
            record_from_convoy_reservation(
                &collection,
                7,
                &ConvoyReservation { priority: 0, count: -1 },
                0,
            )
            .is_err(),
            "the mapper keeps the parser's count >= 0 boundary"
        );
    }
}
