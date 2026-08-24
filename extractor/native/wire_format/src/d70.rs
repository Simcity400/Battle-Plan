//! d70 `sunk_convoy_history_live` — the global sunk-convoy history table.
//!
//! Derived from `world_fold.rs:14632`-`:14692`
//! (`fold_sunk_convoy_history`), its two dispatch sites (`:4737` and `:4801`-`:4806`),
// @world-fold-authority domain=70 functions=fold_sunk_convoy_history lines=14632-14692 sha256=275d9ee0dc03e03b4c0f5f85f38d7da0f2cb6106a7021ef04e2dbbd21780b54a closure=bc647c0bf505cff94640654e13af4836ddc4320b62e697def79bd54dc40b9981
//! and the payload codec `extractor/native/compact_contract/src/naval_history.rs`,
//! which is where this domain's row grammar actually lives. There is no dossier
//! under `extractor/documents/domains/`, so those three are the whole basis.
//!
//! **Authority split.** Unlike the earlier waves, d70's fold decodes nothing
//! itself: it calls `compact_contract::naval_history::decode_payload` and then
//! makes five decisions over the decoded result. So the codec is authority for
//! offsets, lengths and structural validity (`naval_history.rs:266`-`:473`), and
//! the fold is authority for the accumulator semantics, the gates and the
//! refusals (`world_fold.rs::fold_sunk_convoy_history`). `wire_format`'s library half must
//! not depend on `compact_contract` (`Cargo.toml`), so this module mirrors the
//! codec's grammar with a `naval_history.rs:LINE` citation on every constant and
//! every check — and `bin/bench_real_d70.rs`, which *does* depend on it, runs
//! the real `decode_payload` beside this module's own decode on every record of
//! the capture and refuses the stream if the two ever disagree. That
//! cross-check, not the comments, is what keeps the mirror from drifting.
//!
//! # Payload grammar (`naval_history.rs:9`-`:24`, `:266`-`:421`)
//!
//! `SCH1`, three versions. v3 is the only production form:
//!
//! | offset | width | field | notes |
//! |---|---|---|---|
//! | `+0`  | u32 | `magic`           | `SCH1` (`:10`) |
//! | `+4`  | u16 | `schema_version`  | 1, 2 or 3 (`:16`-`:18`) |
//! | `+6`  | u16 | `header_len`      | 272 on v3, 32 on v1/v2 (`:19`-`:20`, `:292`) |
//! | `+8`  | u16 | `row_len`         | always 24 (`:21`, `:297`) |
//! | `+10` | u16 | `authority_raw`   | v3: 0 unavailable, 1 exact; v1/v2: reserved, must be 0 (`:303`, `:359`) |
//! | `+12` | u32 | `status_flags`    | the compact status word |
//! | `+16` | u64 | `elapsed_wall_ms` | timing only, no predicate |
//! | `+24` | u32 | `row_count`       | `<= 65_536` (`:366`) |
//! | `+28` | u32 | `observed_rows`   | must equal `row_count` (`:436`) |
//! | `+32` | u64 | `frame_hour`      | **v3 only** (`:312`) |
//! | `+40` | u64 | `source_sequence` | **v3 only**, non-zero (`:313`-`:316`) |
//! | `+48` | 7×32 | receipt hashes   | **v3 only** (`:317`-`:343`) |
//!
//! Row, 24 bytes (`:385`-`:399`):
//!
//! | offset | width | field | notes |
//! |---|---|---|---|
//! | `+0`  | i32 | `month`                    | engine month index, **signed** (`read_i32`, `:394`) |
//! | `+4`  | u32 | `convoys`                  | non-zero (`:444`) |
//! | `+8`  | u32 | `killer_country_tag_db_id` | non-zero (`:449`-`:458`) |
//! | `+12` | u32 | `owner_tag_db_id`          | non-zero (same loop) |
//! | `+16` | u64 | reserved tail              | must be zero (`:387`-`:392`) |
//!
//! **No scale factor anywhere.** Every field is a raw integer the codec reads
//! and the fold stores untouched; there is no `FIXED_SCALE`, no `/32768`, and
//! nothing continuous. **No strings anywhere.** `month` is the engine's own
//! month index (23,308…23,333 across the reference capture), not a date and not
//! a duration.
//!
//! # The headline: two typed inputs the payload does not carry
//!
//! `fold_sunk_convoy_history` takes `expected_frame_hour: Option<u64>` and
//! `expected_source_sequence: Option<u64>` and **refuses the payload** when
//! either disagrees with the H4CR record that carried it
//! (`world_fold.rs::fold_sunk_convoy_history`). The generic dispatcher `fold_domain_data`
//! passes `None` for both (`world_fold.rs::fold_domain_data`); only the production dispatcher
//! `apply_domain_data` supplies them (`world_fold.rs::apply_domain_data`):
//!
//! ```text
//! expected_frame_hour     = hour_index_from_raw(record.game_hour_raw)   // `world_fold.rs::apply_domain_data`
//! expected_source_sequence = Some(record.sequence)                      // `world_fold.rs::apply_domain_data`
//! ```
//!
//! `hour_index_from_raw` is `raw - 43_799_999` inside the years-1850..2200
//! window and `None` outside it (`compact_contract/src/calendar.rs:14`-`:27`),
//! so a record whose hour word is out of that window makes `expected_frame_hour`
//! `None`, which a v3 payload — whose `frame_hour` is always `Some` — can never
//! match. That is a refusal, and it is reproduced here rather than smoothed over.
//!
//! **`record_sequence: u64` is a new member of the recipe's step-7 typed-input
//! family** (the event schema already lists `frame_tick`,
//! `FrameMode`, `WarSegment`, `Symbols`, `RowScales`, the fold clock and
//! `ordinal`; it does not list this one). It enters at [`fold_payload`], not at
//! [`record_from_row`]: the binding is a property of the payload against its
//! record, and no field of any row is a function of it. Recording that is the
//! recipe-gap finding this port owes; see the deliverable's open questions.
//!
//! # Three authority/ordering states, not two
//!
//! `SunkConvoyHistoryAuthority` (`naval_history.rs:84`-`:92`) and
//! `SunkConvoyHistoryOrdering` (`:73`-`:82`) together give three reachable
//! states, and the fold treats each differently:
//!
//! 1. **`StaticExact20Receipt` + `FirstSeenAggregateIdentity`** (v3, seven pinned
//!    receipt hashes). The only state that folds. All 750 hours of the reference
//!    capture are this.
//! 2. **`Unavailable` + `FirstSeenAggregateIdentity`** (v3, all-zero receipts).
//!    The codec requires a failure status and zero rows on it (`:347`-`:353`), so
//!    the fold's failure gate fires first (`world_fold.rs::fold_sunk_convoy_history`) and the
//!    hour is an **excluded hour** — the accumulator is set to `None`, not
//!    retained.
//! 3. **`LegacyUnbound`** — v1 (`CanonicalIdentity`, rows sorted by
//!    `(month, killer, owner)`) or v2 (`FirstSeenAggregateIdentity`), neither
//!    carrying a frame binding. On a clean status the fold **errors**
//!    (`world_fold.rs::fold_sunk_convoy_history`): "legacy-unbound and cannot supply production
//!    authority", which is a stream rejection. On a *failure* status the failure
//!    gate fires first and it is an excluded hour instead — the order of the two
//!    checks is load-bearing and is mirrored exactly in [`fold_payload`].
//!
//! # Shape: a keyed set with a carried order ordinal
//!
//! v2 and v3 "preserve the authoritative vector order" — the exact
//! `CGameState+0x5A8` pointer-vector order, which static RE proves is the
//! **first-seen order of aggregate `(month, killer, owner)` buckets**
//! (`naval_history.rs:75`-`:81`). That order is not a function of any carried
//! field, so a bare keyed set would lose it. The rows do have an identity the
//! codec enforces — `(month, killer_country_tag_db_id, owner_tag_db_id)` is
//! rejected as a duplicate (`:459`-`:464`) — so this is the **d11
//! production-lines** reading of the recipe's step-5 ordinal device, not the
//! construction-queue one: the key stays the codec's own identity and the
//! position rides beside it as a required, always-concrete value field.
//!
//! The reference capture settles that the ordinal is load-bearing rather than
//! redundant, which d11's convoy variants could not: across all 750 hours the
//! wire order equals the canonical `(month, killer, owner)` sort in **0** of
//! them. It also shows the vector is not append-only. At hour 60,833,065 the
//! engine drops a whole month (17 rows of month 23,308) and the surviving rows
//! move by **swap-remove**: ordinals 7, 23…28 and 252…261 are vacated and the
//! tail elements 454, 453…445 and 447…438 land in them, in that order. A port
//! that assumed first-seen order means append-only would publish 17 wrong
//! positions on that hour and nothing would fail.
//!
//! # Full replace, and an unavailable hour is not a stale one
//!
//! The fold assigns the whole decoded row set (`world_fold.rs::fold_sunk_convoy_history`);
//! nothing is ever merged. Every non-folding path sets
//! `self.sunk_convoy_history = None` — decode error (`world_fold.rs::fold_sunk_convoy_history`), failure status
//! (`world_fold.rs::fold_sunk_convoy_history`), legacy authority (`world_fold.rs::fold_sunk_convoy_history`), frame mismatch (`world_fold.rs::fold_sunk_convoy_history`), sequence
//! mismatch (`world_fold.rs::fold_sunk_convoy_history`). This is **not** d63's "retain the accumulator": the
//! recorder says the domain-hour is unavailable, so a reader must not carry the
//! previous hour's table forward across it. A keyed-set delta cannot express
//! "the collection is unavailable this hour" — that is gap normalization,
//! which has no implementation in this crate (`lib.rs`) — so [`FoldOutcome`]
//! reports it and the measurement bin re-baselines after it rather than emitting
//! a delta against a state the recorder has discarded. The reference capture has
//! zero such hours, so the path has no coverage; that is a stated caveat, not a
//! silence.
//!
//! **`fold_sunk_convoy_history` is the accumulator's only writer.** Nothing else
//! in `world_fold.rs` assigns `self.sunk_convoy_history`: the field is declared
//! at `:1762`, initialised `None` at `:2415`, read at `:4327` and `:4554`, and
//! written only inside the fold. There is no hour-boundary prune and no
//! cross-domain writer, so the recipe's "other writers" enumeration is empty by
//! inspection rather than by assertion.
//!
//! **No derived field (derived-field contract).** Nothing here changes deterministically with the
//! clock. The mapper still takes `frame_tick` so every domain's boundary has one
//! shape.

use crate::body::{Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 70;
/// The one collection: the global sunk-convoy history table.
pub const HISTORY: u32 = 1;

/// Field ids are stable and append-only; schema-identity contract forbids reuse for a different field.
pub mod field {
    pub const MONTH: u32 = 1;
    pub const KILLER_COUNTRY_TAG_DB_ID: u32 = 2;
    pub const OWNER_TAG_DB_ID: u32 = 3;
    pub const CONVOYS: u32 = 4;
    pub const SOURCE_ORDINAL: u32 = 5;
}

// --- Payload grammar constants, from `compact_contract/src/naval_history.rs`. --

/// `PAYLOAD_MAGIC`, `naval_history.rs:10`.
pub const PAYLOAD_MAGIC: u32 = u32::from_le_bytes(*b"SCH1");
/// `LEGACY_PAYLOAD_VERSION`, `naval_history.rs:16`. Canonical `(month, killer,
/// owner)` order; the engine's first-seen order was not retained.
pub const LEGACY_PAYLOAD_VERSION: u16 = 1;
/// `ORDERED_PAYLOAD_VERSION`, `naval_history.rs:17`.
pub const ORDERED_PAYLOAD_VERSION: u16 = 2;
/// `PAYLOAD_VERSION`, `naval_history.rs:18`. The production form.
pub const PAYLOAD_VERSION: u16 = 3;
/// `LEGACY_HEADER_LEN`, `naval_history.rs:19` — the v1/v2 header, and also the
/// minimum length the decoder needs before it can read any header word (`:267`).
pub const LEGACY_HEADER_LEN: usize = 32;
/// `HEADER_LEN`, `naval_history.rs:20`. 48 bytes of scalars plus 7×32 receipt
/// bytes.
pub const HEADER_LEN: usize = 272;
/// `ROW_LEN`, `naval_history.rs:21`.
pub const ROW_LEN: usize = 24;
/// `MAX_ROWS`, `naval_history.rs:24`. "A payload-allocation bound, not an
/// inferred engine semantic maximum" — its own words.
pub const MAX_ROWS: usize = 65_536;
/// Where the v3 receipt block starts (`naval_history.rs:327`: `48 + index * 32`).
pub const RECEIPT_BLOCK_OFFSET: usize = 48;
/// How many pinned receipts a v3 header carries (`naval_history.rs:317`-`:325`).
pub const RECEIPT_COUNT: usize = 7;

/// `COMPACT_STATUS_FAILURE_MASK`, `compact_contract/src/lib.rs:110`-`:118`. d70
/// has no lane-local status bits (`lib.rs:95`-`:101` registers 49 and 51 only)
/// and no expected-partial tolerance (`:136`-`:158` lists 26, 27, 28 and 54
/// only), so the global mask is the whole failure predicate for this domain.
/// Named here rather than imported because `wire_format`'s library half keeps no
/// dependency on `compact_contract`; the measurement bin, which does depend on
/// it, asserts all three facts so this constant cannot drift unnoticed.
pub const FAILURE_STATUS_MASK: u32 = (1 << 0)   // STATE_UNAVAILABLE
    | (1 << 1)  // PROFILE_MISMATCH
    | (1 << 2)  // NO_CLOCK
    | (1 << 3)  // NO_SCALAR_SOURCE
    | (1 << 6)  // WORKER_PANIC
    | (1 << 7)  // BRIDGE_UNAVAILABLE
    | (1 << 8)  // SOURCE_UNAVAILABLE
    | (1 << 9)  // PARTIAL_DATA
    | (1 << 12); // CONTROL_INVALID

/// The assigned v1 status vocabulary: bits 0..=14
/// (`compact_contract/src/status_flags_base.rs:40`-`:44` — everything above is
/// unassigned and `compact_status_unknown_flags_v1` reports it). The v3 decoder
/// rejects an unassigned bit outright (`naval_history.rs:344`-`:346`), before
/// anything classifies the word, so for this domain the rule is part of the
/// grammar rather than a harness convention. Same drift note as
/// [`FAILURE_STATUS_MASK`].
pub const KNOWN_STATUS_MASK_V1: u32 = (1 << 16) - 1;

/// `SunkConvoyHistoryAuthority`, `naval_history.rs:84`-`:92`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Authority {
    /// Explicit unavailable v3 payload. No row may accompany this state.
    Unavailable,
    /// A readable v1/v2 payload without a frame/build/receipt binding.
    LegacyUnbound,
    /// Exact pinned-a729 owner/serializer/row/mutation receipts.
    StaticExact20Receipt,
}

/// `SunkConvoyHistoryOrdering`, `naval_history.rs:73`-`:82`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ordering {
    /// v1: rows canonically sorted by `(month, killer, owner)`. The engine's
    /// first-seen order was discarded before emission.
    CanonicalIdentity,
    /// v2/v3: the exact `CGameState+0x5A8` pointer-vector order.
    FirstSeenAggregateIdentity,
}

/// The seven pinned receipt hashes a v3 exact payload must carry, in the codec's
/// own order (`naval_history.rs:241`-`:249`, verified at `:317`-`:343`):
/// executable, owner-serializer manifest, owner-serializer artifact,
/// row-mutation manifest, row-serializer artifact, mutation artifact, live
/// fixture receipt.
///
/// This is a **typed input**, in the same family as d29's `Symbols` and d35's
/// `RowScales`, rather than a table of literals copied into this crate. The
/// values are build-pinning receipts owned by `compact_contract`
/// (`naval_history.rs:34`-`:45`); copying them here would put a second copy of a
/// build identity in a crate that has no way to notice when the real one moves.
/// The caller supplies them — `bin/bench_real_d70.rs` from `compact_contract`
/// itself — and this module checks the payload against what it was handed.
///
/// **Why check them at all**, when this crate is a schema layer: an
/// `authority_raw` word of `1` is a claim of exact static authority, and the
/// fold refuses everything else (`world_fold.rs::fold_sunk_convoy_history`). Accepting the claim
/// without the receipts would map a payload from some other build as
/// authoritative — publishing data the recorder rejects, under an authority it
/// does not have.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthorityReceipts {
    pub hashes: [[u8; 32]; RECEIPT_COUNT],
}

impl AuthorityReceipts {
    pub fn new(hashes: [[u8; 32]; RECEIPT_COUNT]) -> Self {
        Self { hashes }
    }
}

pub fn schema() -> Schema {
    use field as f;
    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![Collection {
            collection_id: HISTORY,
            // Full replace every hour (`world_fold.rs::fold_sunk_convoy_history`), but the
            // `(month, killer, owner)` identity persists across hours — a bucket
            // keeps accumulating convoys for as long as the engine keeps it —
            // which is what a keyed set needs, and only a keyed set has a delta
            // grammar (collection-shape contract). The array order the payload also carries is
            // preserved by `SOURCE_ORDINAL` below, so no ordered shape is
            // needed; see the module docs.
            shape: Shape::KeyedSet,
            fields: vec![
                // The codec's own identity triple, in its own order
                // (`naval_history.rs:459`).
                //
                // `Signed`: the codec reads `month` with `read_i32`
                // (`:394`) and range-checks nothing, so a negative word is a
                // payload the fold folds. It is an engine month index — an
                // ordinal on the engine's own calendar, identical under every
                // mod set and resolved by no manifest — so `Unsigned`/`Signed`
                // rather than `ContentId`, the same call d20 makes for
                // `div_type` and d11 for `kind`.
                Field::new(f::MONTH, Kind::Signed).key(0),
                // Country tag db ids. `ContentId` on the deciding question: a
                // content manifest must resolve a country tag for it to keep its
                // meaning under another mod set. d63's `tag_db_id` and d20's
                // `owner` are the same call. Both are **ContentId keys**, the
                // heavier manifest-resolver retrofit class — see the
                // deliverable's open questions.
                Field::new(f::KILLER_COUNTRY_TAG_DB_ID, Kind::ContentId).key(1),
                Field::new(f::OWNER_TAG_DB_ID, Kind::ContentId).key(2),
                // Convoys sunk in this `(month, killer, owner)` bucket.
                // `Monotone`: a bucket's count only grows while its month is
                // live and then freezes — 91 increases and 0 decreases across
                // the reference capture's 749 hour transitions. schema-kind contract makes
                // monotone an encoding hint and never a validity constraint, so
                // a future decrease is legal and costs only bytes.
                //
                // The codec rejects a zero (`naval_history.rs:444`-`:447`), so
                // this field never carries a measured zero — a bucket with no
                // sinkings is a row that does not exist.
                Field::new(f::CONVOYS, Kind::Monotone),
                // The row's dense position in the payload's row vector — the
                // engine's first-seen aggregate order (`naval_history.rs:78`).
                // Required and always concrete: it is the only witness of an
                // order that is a function of no carried field (0 of 750 hours
                // of the reference capture are in canonical key order), and it
                // is what the recipe's step-5 ordinal device carries beside a
                // fold-enforced key.
                //
                // `Unsigned`, not `Monotone`: a position is not a counter, and
                // the capture's swap-remove hour moves 17 of them *downward* at
                // once.
                Field::new(f::SOURCE_ORDINAL, Kind::Unsigned),
            ],
        }],
        event_part: None,
    }
}

// --- The boundary mapping ----------------------------------------------------

/// The SCH1 payload header, as [`split_payload`] reads it
/// (`naval_history.rs:270`-`:365`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub schema_version: u16,
    pub header_len: usize,
    pub row_len: usize,
    pub authority: Authority,
    pub ordering: Ordering,
    pub status_flags: u32,
    pub elapsed_wall_ms: u64,
    pub row_count: u32,
    pub observed_rows: u32,
    /// `Some` on v3 only (`naval_history.rs:357`, `:364`).
    pub frame_hour: Option<u64>,
    /// `Some` on v3 only. Never `Some(0)`: the decoder rejects a zero sequence
    /// (`naval_history.rs:314`-`:316`).
    pub source_sequence: Option<u64>,
}

impl PayloadHeader {
    /// The fold's failure gate (`world_fold.rs:10243`, `handle_compact_failure`
    /// at `world_fold.rs::handle_compact_failure`). See [`FAILURE_STATUS_MASK`] for why the global mask is the
    /// whole predicate for d70.
    pub fn is_failure(self) -> bool {
        self.status_flags & FAILURE_STATUS_MASK != 0
    }
}

/// What one payload did to the accumulator, mirroring
/// `fold_sunk_convoy_history`'s three non-error outcomes. An `Err` from
/// [`fold_payload`] is the fourth: a stream rejection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FoldOutcome {
    /// The payload was exact, bound to its record, and replaced the whole table
    /// (`world_fold.rs::fold_sunk_convoy_history`).
    Folded {
        rows: usize,
        ordering: Ordering,
        frame_hour: u64,
        source_sequence: u64,
    },
    /// A failure status: the accumulator is set to `None` and nothing is folded
    /// (`world_fold.rs::fold_sunk_convoy_history`). **An excluded hour, and an unavailable
    /// one** — the previous hour's table is discarded, not retained, so a reader
    /// must not carry state across it. A keyed set cannot say that (gap
    /// normalisation has no code in this crate), which is why the state is
    /// reported here.
    Unavailable { status_flags: u32 },
}

/// Split an SCH1 payload into its header and its row bytes, applying exactly the
/// checks `compact_contract::naval_history::decode_payload` applies **before it
/// walks the rows** (`naval_history.rs:267`-`:383`), in its own order.
///
/// The split exists because the codec's order is load-bearing in two directions.
/// Everything here rejects a payload whatever its status — the fold calls
/// `decode_payload` first and only then consults the status word
/// (`world_fold.rs::fold_sunk_convoy_history`), so this is the d83 shape: a malformed
/// payload is a refused stream even on a failure-status record. And the checks
/// that *do* depend on the row bytes (the reserved tail, the non-zero fields,
/// the duplicate identity, `observed_rows == row_count`, v1's canonical order)
/// come after, in [`fold_payload`], because that is where the codec puts them.
///
/// `receipts` is the pinned v3 receipt block; see [`AuthorityReceipts`].
///
/// Total: no payload of any length panics.
pub fn split_payload<'a>(
    payload: &'a [u8],
    receipts: &AuthorityReceipts,
) -> core::result::Result<(PayloadHeader, &'a [u8]), String> {
    // `naval_history.rs:267`-`:269`. The v1/v2 header length is the minimum a
    // payload needs before any header word can be read, whatever its version.
    if payload.len() < LEGACY_HEADER_LEN {
        return Err(format!(
            "sunk_convoy_history payload {} shorter than the {LEGACY_HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    let magic = u32_at(payload, 0);
    let schema_version = u16_at(payload, 4);
    let header_len = u16_at(payload, 6) as usize;
    let row_len = u16_at(payload, 8) as usize;
    let authority_raw = u16_at(payload, 10);
    let status_flags = u32_at(payload, 12);
    let elapsed_wall_ms = u64_at(payload, 16);
    let row_count = u32_at(payload, 24);
    let observed_rows = u32_at(payload, 28);

    // `naval_history.rs:279`-`:283`.
    if magic != PAYLOAD_MAGIC {
        return Err(format!(
            "sunk_convoy_history payload magic {magic:#010x} is not SCH1"
        ));
    }
    // `:284`-`:291`.
    if schema_version != LEGACY_PAYLOAD_VERSION
        && schema_version != ORDERED_PAYLOAD_VERSION
        && schema_version != PAYLOAD_VERSION
    {
        return Err(format!(
            "sunk_convoy_history payload version {schema_version} is unsupported"
        ));
    }
    // `:292`-`:301`.
    let expected_header_len = if schema_version == PAYLOAD_VERSION {
        HEADER_LEN
    } else {
        LEGACY_HEADER_LEN
    };
    if header_len != expected_header_len || row_len != ROW_LEN {
        return Err(format!(
            "sunk_convoy_history schema header has header_len={header_len}, row_len={row_len}"
        ));
    }

    // `:302`-`:365`.
    let (authority, frame_hour, source_sequence) = if schema_version == PAYLOAD_VERSION {
        // `:303`-`:311`.
        let authority = match authority_raw {
            0 => Authority::Unavailable,
            1 => Authority::StaticExact20Receipt,
            other => {
                return Err(format!(
                    "sunk_convoy_history authority state {other} is unsupported"
                ))
            }
        };
        // `:312`-`:313`. `read_u64` is fallible in the codec, so a payload
        // between 32 and 47 bytes long is refused here and not read past.
        let frame_hour = checked_u64_at(payload, 32)?;
        let source_sequence = checked_u64_at(payload, 40)?;
        // `:314`-`:316`.
        if source_sequence == 0 {
            return Err("sunk_convoy_history has zero source sequence".to_string());
        }
        // `:317`-`:343`. An unavailable payload carries all-zero receipts; an
        // exact one carries the caller's pinned block, hash for hash.
        for (index, expected) in receipts.hashes.iter().enumerate() {
            let offset = RECEIPT_BLOCK_OFFSET + index * 32;
            let actual = payload
                .get(offset..offset + 32)
                .ok_or_else(|| "sunk_convoy_history authority hash truncated".to_string())?;
            let wanted: &[u8; 32] = if authority == Authority::Unavailable {
                &[0u8; 32]
            } else {
                expected
            };
            if actual != wanted.as_slice() {
                return Err(format!(
                    "sunk_convoy_history authority hash {index} does not match its pinned receipt"
                ));
            }
        }
        // `:344`-`:346`. Validity before classification: an unassigned status bit
        // has no defined meaning, so the payload is refused rather than read as
        // "not a failure".
        let unknown_status = status_flags & !KNOWN_STATUS_MASK_V1;
        if unknown_status != 0 {
            return Err(format!(
                "sunk_convoy_history carries unknown status bits {unknown_status:#x}"
            ));
        }
        // `:347`-`:356`. The two authority states and the status word are
        // coupled in both directions.
        if authority == Authority::Unavailable {
            if row_count != 0 || observed_rows != 0 {
                return Err("unavailable sunk_convoy_history carries rows".to_string());
            }
            if status_flags & FAILURE_STATUS_MASK == 0 {
                return Err("unavailable sunk_convoy_history lacks failure status".to_string());
            }
        } else if status_flags & FAILURE_STATUS_MASK != 0 {
            return Err("exact sunk_convoy_history carries failure status".to_string());
        }
        (authority, Some(frame_hour), Some(source_sequence))
    } else {
        // `:358`-`:364`. v1/v2 have no binding at all, and their `+10` word is a
        // reserved field rather than an authority code.
        if authority_raw != 0 {
            return Err(format!(
                "legacy sunk_convoy_history has nonzero reserved field {authority_raw}"
            ));
        }
        (Authority::LegacyUnbound, None, None)
    };

    // `:400`-`:404`: the ordering is a function of the version alone.
    let ordering = if schema_version == LEGACY_PAYLOAD_VERSION {
        Ordering::CanonicalIdentity
    } else {
        Ordering::FirstSeenAggregateIdentity
    };

    // `:366`-`:370`.
    if row_count as usize > MAX_ROWS {
        return Err(format!(
            "sunk_convoy_history row count {row_count} exceeds {MAX_ROWS}"
        ));
    }
    // `:371`-`:383`. Checked arithmetic: `row_count` is a word from a file on
    // disk and `row_count * ROW_LEN` overflows `usize` for a large enough one.
    let expected_len = (row_count as usize)
        .checked_mul(ROW_LEN)
        .and_then(|bytes| bytes.checked_add(expected_header_len))
        .ok_or_else(|| "sunk_convoy_history payload byte length overflow".to_string())?;
    if payload.len() != expected_len {
        return Err(format!(
            "sunk_convoy_history payload length {} does not exactly cover {row_count} rows \
             ({expected_len})",
            payload.len()
        ));
    }

    Ok((
        PayloadHeader {
            schema_version,
            header_len,
            row_len,
            authority,
            ordering,
            status_flags,
            elapsed_wall_ms,
            row_count,
            observed_rows,
            frame_hour,
            source_sequence,
        },
        &payload[expected_header_len..],
    ))
}

/// The identity of one row: `(month, killer_country_tag_db_id, owner_tag_db_id)`
/// — the triple the codec builds and enforces uniqueness over
/// (`naval_history.rs:459`-`:464`).
///
/// `month` is a **signed** key component (`read_i32`, `:394`), so the key is
/// built directly rather than through `body::key`, which is an unsigned
/// convenience.
pub fn key_from_row(row: &[u8]) -> core::result::Result<Key, String> {
    let parsed = RawRow::parse(row)?;
    Ok(Key(vec![
        parsed.month as i128,
        parsed.killer_country_tag_db_id as i128,
        parsed.owner_tag_db_id as i128,
    ]))
}

/// One 24-byte row of the `sunk_convoy_history` table, as the codec lays it out
/// today, onto its stored Record. `row` is the raw row slice (the caller has
/// already consumed the payload header via [`split_payload`]); SCH1 has exactly
/// one row form in all three versions (`ROW_LEN` is checked against the header
/// word at `naval_history.rs:297`), so length selects nothing and a wrong length
/// is rejected.
///
/// `ordinal` is the row's dense position in the payload's row vector — the
/// recipe's step-5 ordinal device, carried as a value field beside the codec's
/// own key. It is a **parameter, never re-derived**: the mapper, the key
/// extractor and every caller must count positions the same way, which is only
/// true if exactly one of them counts.
///
/// `frame_tick` is derived-field contract's derived-field input (`stored = v + c * (frame_tick >> u)`);
/// d70 declares no derived field, so it is taken and ignored — every domain's
/// mapper has the same shape.
///
/// Fail-closed throughout, mirroring the codec: a row the codec rejects is an
/// `Err` here, never a coerced value. Nothing in this function can panic on any
/// input of any length.
pub fn record_from_row(
    collection: &Collection,
    row: &[u8],
    ordinal: u32,
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    // d70 has no derived field; derived-field contract's transform input is unused. Named rather
    // than `_frame_tick` because the signature is the cross-domain contract.
    let _ = frame_tick;
    // A record built for the wrong collection would otherwise come out shaped by
    // another domain's field ids (`variant_upgrades.rs:633`'s rule).
    if collection.collection_id != HISTORY {
        return Err(format!(
            "sunk_convoy_history has no mapping for collection id {}",
            collection.collection_id
        ));
    }
    let parsed = RawRow::parse(row)?;
    // The ordinal indexes the payload's row vector, which the codec bounds at
    // `MAX_ROWS` (`naval_history.rs:366`), so a position at or beyond that bound
    // cannot come from a payload the fold accepts and is a miscounting caller.
    if ordinal as usize >= MAX_ROWS {
        return Err(format!(
            "sunk_convoy_history source_ordinal {ordinal} is outside the {MAX_ROWS}-row vector"
        ));
    }

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::CONVOYS => Value::Unsigned(parsed.convoys as u64),
            field::SOURCE_ORDINAL => Value::Unsigned(ordinal as u64),
            other => {
                // A collection this mapper does not know how to fill. Filling it
                // with a placeholder would publish invented data under a field
                // id that means something else.
                return Err(format!(
                    "sunk_convoy_history has no mapping for field id {other}"
                ));
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// The decoded rows of one payload, in wire order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RawRow {
    month: i32,
    convoys: u32,
    killer_country_tag_db_id: u32,
    owner_tag_db_id: u32,
}

impl RawRow {
    /// The codec's row read and its per-row validity rules
    /// (`naval_history.rs:385`-`:458`), in the codec's own order: length, then
    /// the reserved tail, then the field reads, then the non-zero rules.
    fn parse(row: &[u8]) -> core::result::Result<Self, String> {
        if row.len() != ROW_LEN {
            return Err(format!(
                "sunk_convoy_history row length {} != {ROW_LEN}",
                row.len()
            ));
        }
        // `:387`-`:392`. A non-zero tail means the row is not this grammar.
        let reserved_tail = u64_at(row, 16);
        if reserved_tail != 0 {
            return Err("sunk_convoy_history row has nonzero reserved tail".to_string());
        }
        let parsed = Self {
            month: i32_at(row, 0),
            convoys: u32_at(row, 4),
            killer_country_tag_db_id: u32_at(row, 8),
            owner_tag_db_id: u32_at(row, 12),
        };
        // `:444`-`:447`. A bucket with no sinkings is a row that does not exist,
        // so a zero here is a malformed row and never a measured zero.
        if parsed.convoys == 0 {
            return Err("sunk_convoy_history row has zero convoys".to_string());
        }
        // `:449`-`:458`.
        for (label, value) in [
            (
                "killer_country_tag_db_id",
                parsed.killer_country_tag_db_id,
            ),
            ("owner_tag_db_id", parsed.owner_tag_db_id),
        ] {
            if value == 0 {
                return Err(format!("sunk_convoy_history row {label} is zero"));
            }
        }
        Ok(parsed)
    }
}

/// The running d70 accumulator: `WorldFold::sunk_convoy_history`
/// (`world_fold.rs:1762`), reduced to what the wire format stores.
///
/// `None` is the recorder's own "this domain-hour is unavailable" — every
/// non-folding path in `fold_sunk_convoy_history` assigns it, and it is
/// **discarding**, not retaining. Keeping the distinction here rather than in a
/// harness is what stops a caller from replaying a stale table across an
/// unavailable hour.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub rows: Option<KeyedState>,
}

impl State {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Fold one d70 DATA payload exactly as `fold_sunk_convoy_history` folds it
/// (`world_fold.rs::fold_sunk_convoy_history`), in the fold's own order.
///
/// This is the recipe's fourth public boundary. It lives here rather than in the
/// harness because the *order* of the five decisions is grammar: the failure
/// gate comes before the authority check, so a legacy payload under a failure
/// status is an excluded hour while the same payload under a clean status
/// refuses the stream. A harness that re-derived that order would disagree with
/// the recorder on exactly the frames that fail.
///
/// `expected_frame_hour` and `expected_record_sequence` are the two typed inputs
/// the payload does not carry; see the module docs. `apply_domain_data` supplies
/// them from the H4CR record (`world_fold.rs::apply_domain_data`) as
/// `hour_index_from_raw(game_hour_raw)` and `Some(record.sequence)`; the generic
/// `fold_domain_data` dispatcher passes `None` for both (`world_fold.rs::fold_domain_data`), and this
/// function reproduces that too — `None` against a v3 payload's `Some` is a
/// mismatch, which is the refusal the fold makes.
///
/// **Error contract: this fold mutates before it errors.** Every `Err` path
/// leaves `state.rows` at `None`, exactly as the fold leaves
/// `self.sunk_convoy_history` (`world_fold.rs::fold_sunk_convoy_history`). A caller
/// that treats an `Err` as "the accumulator is unchanged" disagrees with the
/// recorder.
pub fn fold_payload(
    state: &mut State,
    collection: &Collection,
    payload: &[u8],
    receipts: &AuthorityReceipts,
    expected_frame_hour: Option<u64>,
    expected_record_sequence: Option<u64>,
    frame_tick: u64,
) -> core::result::Result<FoldOutcome, String> {
    // 1. `decode_payload` (`world_fold.rs::fold_sunk_convoy_history`). The whole payload,
    //    rows included, is decoded BEFORE the status word is consulted, so a
    //    malformed payload refuses the stream whatever its status.
    let decoded = match decode(payload, receipts, collection, frame_tick) {
        Ok(decoded) => decoded,
        Err(error) => {
            state.rows = None;
            return Err(format!("sunk_convoy_history SCH1 invalid: {error}"));
        }
    };

    // 2. `handle_compact_failure` (`:10243`-`:10251`). Sets the accumulator to
    //    `None` and folds nothing: an unavailable hour, not a stale one.
    if decoded.header.is_failure() {
        state.rows = None;
        return Ok(FoldOutcome::Unavailable {
            status_flags: decoded.header.status_flags,
        });
    }

    // 3. Authority (`:10252`-`:10258`). v1 and v2 are readable and are still
    //    refused: they cannot supply production authority.
    if decoded.header.authority != Authority::StaticExact20Receipt {
        state.rows = None;
        return Err(format!(
            "sunk_convoy_history SCH1/v{} is legacy-unbound and cannot supply production authority",
            decoded.header.schema_version
        ));
    }

    // 4. Frame binding (`:10259`-`:10266`).
    if decoded.header.frame_hour != expected_frame_hour {
        state.rows = None;
        return Err(format!(
            "sunk_convoy_history frame {:?} does not match record frame {expected_frame_hour:?}",
            decoded.header.frame_hour
        ));
    }

    // 5. Sequence binding (`:10267`-`:10274`).
    if decoded.header.source_sequence != expected_record_sequence {
        state.rows = None;
        return Err(format!(
            "sunk_convoy_history sequence {:?} does not match record sequence \
             {expected_record_sequence:?}",
            decoded.header.source_sequence
        ));
    }

    // 6. Full replace (`:10275`-`:10282`).
    let rows = decoded.rows.len();
    state.rows = Some(decoded.rows);
    Ok(FoldOutcome::Folded {
        rows,
        ordering: decoded.header.ordering,
        // Both are `Some` here: only a v3 payload reaches this point, and step 4
        // and step 5 have already matched them against the record.
        frame_hour: decoded.header.frame_hour.unwrap_or_default(),
        source_sequence: decoded.header.source_sequence.unwrap_or_default(),
    })
}

/// One payload decoded into its header and its keyed rows — the whole of
/// `decode_payload`, header side and row side.
#[derive(Clone, Debug)]
pub struct Decoded {
    pub header: PayloadHeader,
    pub rows: KeyedState,
}

/// [`split_payload`] plus the codec's row walk and its cross-row rules
/// (`naval_history.rs:384`-`:409`, `validate_rows` at `:423`-`:473`): every
/// row's tail, fields and non-zero rules, then `observed_rows`, then the
/// duplicate-identity rule and v1's strict-sort rule.
///
/// **The per-row non-zero rules sit one step earlier here than in the codec**,
/// which reads every row first and only checks them inside `validate_rows`
/// afterwards. They live in [`RawRow::parse`] because [`key_from_row`] and
/// [`record_from_row`] must refuse the same rows the codec does, and those two
/// see one row at a time. The set of predicates is identical, so the accept/
/// reject classification of any payload is identical too and only the message of
/// a doubly-malformed payload differs — which `bin/bench_real_d70.rs` pins by
/// running `compact_contract`'s own `decode_payload` beside this one on every
/// record of the capture and refusing the stream if the two ever disagree.
///
/// Public because the measurement bin needs the decode without the fold's
/// accumulator semantics — and because sharing one decode is what keeps the bin,
/// the tests and [`fold_payload`] from drifting apart.
///
/// Total: no payload of any length panics.
pub fn decode(
    payload: &[u8],
    receipts: &AuthorityReceipts,
    collection: &Collection,
    frame_tick: u64,
) -> core::result::Result<Decoded, String> {
    let (header, body) = split_payload(payload, receipts)?;

    // `:384`-`:399`: read every row, tail check included, before any cross-row
    // rule runs.
    let mut raw: Vec<RawRow> = Vec::with_capacity(header.row_count as usize);
    for (row_index, row) in body.chunks_exact(ROW_LEN).enumerate() {
        raw.push(
            RawRow::parse(row).map_err(|error| format!("row {row_index}: {error}"))?,
        );
    }

    // `validate_rows`, `:434`-`:440`. The codec checks this AFTER the row reads,
    // so a payload that is wrong in both ways reports the row problem first.
    if header.observed_rows as usize != raw.len() {
        return Err(format!(
            "sunk_convoy_history observed_rows {} does not equal emitted row count {}",
            header.observed_rows,
            raw.len()
        ));
    }

    let mut rows: KeyedState = KeyedState::new();
    let mut previous: Option<(i32, u32, u32)> = None;
    for (row_index, parsed) in raw.iter().enumerate() {
        let identity = (
            parsed.month,
            parsed.killer_country_tag_db_id,
            parsed.owner_tag_db_id,
        );
        let ordinal = u32::try_from(row_index)
            .map_err(|_| "sunk_convoy_history row index exceeds u32".to_string())?;
        let key = key_from_row(&body[row_index * ROW_LEN..(row_index + 1) * ROW_LEN])?;
        let record = record_from_row(
            collection,
            &body[row_index * ROW_LEN..(row_index + 1) * ROW_LEN],
            ordinal,
            frame_tick,
        )?;
        // `:459`-`:464`. A repeated identity is a rejected payload, never a
        // last-writer-wins overwrite — which is also what makes the keyed set
        // lossless: the codec guarantees the map cannot collapse two rows.
        if rows.insert(key, record).is_some() {
            return Err(format!(
                "sunk_convoy_history row {row_index} duplicates month/killer/owner identity"
            ));
        }
        // `:465`-`:469`. v1 canonicalised its rows, so a v1 payload that is not
        // strictly sorted is malformed. v2/v3 carry the engine's own order and
        // are not sorted at all.
        if header.ordering == Ordering::CanonicalIdentity
            && previous.is_some_and(|prior| prior >= identity)
        {
            return Err(format!(
                "sunk_convoy_history row {row_index} is not strictly sorted by month/killer/owner"
            ));
        }
        previous = Some(identity);
    }

    Ok(Decoded { header, rows })
}

// --- Byte readers -------------------------------------------------------------

/// The v3 header reaches past the 32 bytes [`split_payload`] has checked, so
/// these two reads are fallible exactly as the codec's `read_u64` is (`:496`).
fn checked_u64_at(bytes: &[u8], offset: usize) -> core::result::Result<u64, String> {
    let raw = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| format!("sunk_convoy_history u64 at {offset} is truncated"))?;
    Ok(u64::from_le_bytes(raw.try_into().expect("eight-byte slice")))
}

/// Callers reach these only after a length check, so the slices are in bounds.
fn u16_at(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn i32_at(b: &[u8], off: usize) -> i32 {
    i32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
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

    /// A stand-in receipt block. The values are irrelevant to every property
    /// under test — what matters is that the payload and the caller agree — so
    /// the tests use a distinguishable pattern rather than a copy of
    /// `compact_contract`'s pinned build identity. `bench_real_d70.rs` is where
    /// the real receipts are used, and where the two are cross-checked against
    /// the codec on every record of the capture.
    fn receipts() -> AuthorityReceipts {
        let mut hashes = [[0u8; 32]; RECEIPT_COUNT];
        for (index, hash) in hashes.iter_mut().enumerate() {
            for (byte_index, byte) in hash.iter_mut().enumerate() {
                *byte = (index as u8) * 32 + byte_index as u8 + 1;
            }
        }
        AuthorityReceipts::new(hashes)
    }

    /// A raw 24-byte row, laid out as `naval_history.rs:256`-`:262` writes one.
    fn raw_row(month: i32, convoys: u32, killer: u32, owner: u32) -> Vec<u8> {
        let mut row = Vec::with_capacity(ROW_LEN);
        row.extend_from_slice(&month.to_le_bytes());
        row.extend_from_slice(&convoys.to_le_bytes());
        row.extend_from_slice(&killer.to_le_bytes());
        row.extend_from_slice(&owner.to_le_bytes());
        row.extend_from_slice(&0u64.to_le_bytes());
        assert_eq!(row.len(), ROW_LEN);
        row
    }

    /// A v3 payload, built exactly as `encode_v3` builds one
    /// (`naval_history.rs:200`-`:264`) so a test can perturb one word at a time.
    fn v3_payload(
        rows: &[Vec<u8>],
        authority_raw: u16,
        status_flags: u32,
        frame_hour: u64,
        source_sequence: u64,
        observed_rows: u32,
    ) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&PAYLOAD_MAGIC.to_le_bytes());
        payload.extend_from_slice(&PAYLOAD_VERSION.to_le_bytes());
        payload.extend_from_slice(&(HEADER_LEN as u16).to_le_bytes());
        payload.extend_from_slice(&(ROW_LEN as u16).to_le_bytes());
        payload.extend_from_slice(&authority_raw.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&7u64.to_le_bytes()); // elapsed_wall_ms
        payload.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        payload.extend_from_slice(&observed_rows.to_le_bytes());
        payload.extend_from_slice(&frame_hour.to_le_bytes());
        payload.extend_from_slice(&source_sequence.to_le_bytes());
        for hash in receipts().hashes {
            if authority_raw == 0 {
                payload.extend_from_slice(&[0u8; 32]);
            } else {
                payload.extend_from_slice(&hash);
            }
        }
        assert_eq!(payload.len(), HEADER_LEN);
        for row in rows {
            payload.extend_from_slice(row);
        }
        payload
    }

    /// A v1/v2 payload (`encode_legacy_ordered_payload`,
    /// `naval_history.rs:110`-`:144`), version selectable.
    fn legacy_payload(rows: &[Vec<u8>], version: u16, status_flags: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&PAYLOAD_MAGIC.to_le_bytes());
        payload.extend_from_slice(&version.to_le_bytes());
        payload.extend_from_slice(&(LEGACY_HEADER_LEN as u16).to_le_bytes());
        payload.extend_from_slice(&(ROW_LEN as u16).to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        payload.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        assert_eq!(payload.len(), LEGACY_HEADER_LEN);
        for row in rows {
            payload.extend_from_slice(row);
        }
        payload
    }

    fn three_rows() -> Vec<Vec<u8>> {
        // Deliberately NOT in canonical `(month, killer, owner)` order: the
        // capture's wire order never is, and the ordinal is what carries it.
        vec![
            raw_row(23_310, 5, 8, 9),
            raw_row(23_308, 2, 4, 9),
            raw_row(23_309, 41, 4, 68),
        ]
    }

    fn good_payload() -> Vec<u8> {
        v3_payload(&three_rows(), 1, 0, 4_242, 99, 3)
    }

    #[test]
    fn the_schema_is_valid() {
        schema()
            .validate(grid().log2())
            .expect("d70 schema is well-formed");
    }

    #[test]
    fn it_is_a_keyed_set_keyed_by_month_killer_and_owner() {
        let c = collection();
        assert_eq!(c.shape, Shape::KeyedSet, "baseline replaces, deltas upsert");
        let keys = c.key_fields();
        assert_eq!(keys.len(), 3, "the codec's identity triple");
        assert_eq!(keys[0].field_id, field::MONTH);
        assert_eq!(keys[1].field_id, field::KILLER_COUNTRY_TAG_DB_ID);
        assert_eq!(keys[2].field_id, field::OWNER_TAG_DB_ID);
        assert_eq!(
            keys[0].kind,
            Kind::Signed,
            "`month` is read with read_i32 and range-checked nowhere"
        );
        for key in &keys[1..] {
            assert_eq!(
                key.kind,
                Kind::ContentId,
                "a country tag is content-defined (content-identity contract), so a manifest must resolve it"
            );
        }
        assert!(keys.iter().all(|k| k.kind.legal_as_key()));
        assert_eq!(c.fields.len(), 5, "3 key components + convoys + ordinal");
        let values: Vec<u32> = c.value_fields().map(|f| f.field_id).collect();
        assert_eq!(values, vec![field::CONVOYS, field::SOURCE_ORDINAL]);
    }

    #[test]
    fn the_content_id_sweep_finds_exactly_the_two_country_tags() {
        // the ContentId retrofit sweep can tell "checked, none" from "not checked" only
        // if every domain states its result. d70's is two — and both are KEY
        // components, the heavier retrofit class, because a manifest remap
        // reorders keys and changes the byte layout of every baseline.
        let c = collection();
        let content: Vec<u32> = c
            .fields
            .iter()
            .filter(|f| matches!(f.kind, Kind::ContentId))
            .map(|f| f.field_id)
            .collect();
        assert_eq!(
            content,
            vec![field::KILLER_COUNTRY_TAG_DB_ID, field::OWNER_TAG_DB_ID]
        );
        assert!(c
            .fields
            .iter()
            .filter(|f| matches!(f.kind, Kind::ContentId))
            .all(|f| f.is_key()));
    }

    #[test]
    fn no_field_declares_any_empty_state() {
        // The recipe's step-8 case 3 asks for one raw-bytes case per declared
        // capability. This collection declares NONE, and that is the whole
        // statement: every field of every row SCH1 admits is concrete. The codec
        // has no flag word, no gate bit and no optional segment — a row is 24
        // bytes of four required integers plus a reserved tail that must be zero
        // (`naval_history.rs:385`-`:399`) — and it rejects rather than folds
        // every empty case it can express: a zero `convoys`, a zero killer and a
        // zero owner (`:444`-`:458`). So there is no `.optional()`,
        // `.nullable()` or `.unknown_capable()` here, and no state test for one.
        // `the_mapper_rejects_every_row_the_codec_rejects` is where those
        // rejections are pinned.
        let c = collection();
        assert_eq!(c.optional_count(), 0);
        assert!(c.value_fields().all(|f| !f.nullable));
        assert!(c.value_fields().all(|f| !f.unknown_capable));
        assert!(c.fields.iter().all(|f| !f.optional));
    }

    #[test]
    fn no_field_is_quantized_because_none_is_continuous() {
        // Step 4's "quantization saves bytes" test does not apply: d70 has no
        // continuous field at all. Every field of the payload is a raw integer
        // the codec reads and the fold stores untouched — there is no scale
        // constant anywhere in `naval_history.rs`, which is why the step-8
        // offset test below checks transposition rather than scale.
        let c = collection();
        assert_eq!(
            c.fields
                .iter()
                .filter(|f| matches!(f.kind, Kind::Continuous))
                .count(),
            0
        );
        assert!(c.fields.iter().all(|f| !f.quant.is_quantized()));
    }

    #[test]
    fn no_field_is_derived() {
        // derived-field contract derived fields are clock-linear integers; nothing in d70 changes
        // deterministically with the clock, so `frame_tick` never enters a value.
        assert!(collection().fields.iter().all(|f| f.derived.is_none()));
        let c = collection();
        let row = raw_row(23_310, 5, 8, 9);
        for tick in [0u64, 1, 4_242, u64::MAX] {
            assert_eq!(
                record_from_row(&c, &row, 0, tick).expect("maps"),
                record_from_row(&c, &row, 0, 0).expect("maps"),
                "frame_tick {tick} must not reach a value"
            );
        }
    }

    #[test]
    fn a_full_row_roundtrips_byte_for_byte() {
        let c = collection();
        let record = record_from_row(&c, &raw_row(23_310, 5, 8, 9), 17, 0).expect("maps");
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
    fn a_negative_month_and_an_extreme_row_roundtrip_too() {
        // `month` is signed and range-checked nowhere, and `convoys` is a full
        // u32: the round-trip has to hold at the edges of both, not only on the
        // shapes this capture happens to carry.
        let c = collection();
        for row in [
            raw_row(i32::MIN, 1, 1, 1),
            raw_row(-1, u32::MAX, u32::MAX, u32::MAX),
            raw_row(i32::MAX, 1, u32::MAX, 1),
        ] {
            let record = record_from_row(&c, &row, (MAX_ROWS - 1) as u32, 0).expect("maps");
            let mut bytes = Vec::new();
            record.encode(&c, &mut bytes).expect("encodes");
            let (back, pos) = Record::decode(&c, &bytes, 0).expect("decodes");
            assert_eq!(pos, bytes.len());
            assert_eq!(back, record);
            // And the key survives the signed component.
            assert!(key_from_row(&row).is_ok());
        }
    }

    #[test]
    fn every_field_maps_from_its_own_offset() {
        // The recipe's step-6 test. d70 has no scale factor, so the failure this
        // catches is transposition: `month`/`convoys` and `killer`/`owner` are
        // adjacent 4-byte words, and swapping either pair would round-trip
        // perfectly while publishing the wrong table. Every word therefore gets
        // a distinct known value, and both the key and the record are checked.
        let c = collection();
        let row = raw_row(0x1111_1111, 0x2222_2222, 0x3333_3333, 0x4444_4444);
        assert_eq!(
            key_from_row(&row).expect("keys"),
            Key(vec![0x1111_1111, 0x3333_3333, 0x4444_4444]),
            "month at +0, killer at +8, owner at +12"
        );
        let r = record_from_row(&c, &row, 0x5555, 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::CONVOYS)],
            Value::Unsigned(0x2222_2222),
            "convoys at +4"
        );
        assert_eq!(
            r.values[index_of(&c, field::SOURCE_ORDINAL)],
            Value::Unsigned(0x5555),
            "the ordinal is the caller's parameter, not a row word"
        );
        // The signed key component is signed, not reinterpreted as u32.
        assert_eq!(
            key_from_row(&raw_row(-2, 1, 1, 1)).expect("keys"),
            Key(vec![-2, 1, 1])
        );
    }

    #[test]
    fn every_header_word_is_read_from_its_own_offset() {
        // The header is covered by the same rule: a transposed header word
        // mis-slices every row behind it and no round-trip assertion can see it
        // (both arms agree on the wrong slice).
        let payload = v3_payload(&three_rows(), 1, 1 << 10, 0xDEAD_BEEF, 0x1234, 3);
        let (header, body) = split_payload(&payload, &receipts()).expect("well-formed");
        assert_eq!(header.schema_version, PAYLOAD_VERSION);
        assert_eq!(header.header_len, HEADER_LEN);
        assert_eq!(header.row_len, ROW_LEN);
        assert_eq!(header.authority, Authority::StaticExact20Receipt);
        assert_eq!(header.ordering, Ordering::FirstSeenAggregateIdentity);
        assert_eq!(header.status_flags, 1 << 10);
        assert_eq!(header.elapsed_wall_ms, 7);
        assert_eq!(header.row_count, 3);
        assert_eq!(header.observed_rows, 3);
        assert_eq!(header.frame_hour, Some(0xDEAD_BEEF));
        assert_eq!(header.source_sequence, Some(0x1234));
        assert_eq!(body.len(), 3 * ROW_LEN);
        assert_eq!(&body[..ROW_LEN], &three_rows()[0][..]);

        // A legacy header reads its own, shorter shape, and carries no binding.
        let legacy = legacy_payload(&three_rows(), ORDERED_PAYLOAD_VERSION, 0);
        let (header, body) = split_payload(&legacy, &receipts()).expect("well-formed v2");
        assert_eq!(header.schema_version, ORDERED_PAYLOAD_VERSION);
        assert_eq!(header.header_len, LEGACY_HEADER_LEN);
        assert_eq!(header.authority, Authority::LegacyUnbound);
        assert_eq!(header.ordering, Ordering::FirstSeenAggregateIdentity);
        assert_eq!(header.frame_hour, None);
        assert_eq!(header.source_sequence, None);
        assert_eq!(body.len(), 3 * ROW_LEN);

        // v1 differs from v2 in exactly one decoded field: the ordering.
        let mut v1 = legacy_payload(&three_rows(), LEGACY_PAYLOAD_VERSION, 0);
        // v1 requires canonical order, so sort the rows for this one.
        let mut sorted = three_rows();
        sorted.sort();
        v1.truncate(LEGACY_HEADER_LEN);
        for row in &sorted {
            v1.extend_from_slice(row);
        }
        let (header, _) = split_payload(&v1, &receipts()).expect("well-formed v1");
        assert_eq!(header.ordering, Ordering::CanonicalIdentity);
        assert_eq!(header.authority, Authority::LegacyUnbound);
    }

    #[test]
    fn the_mapper_rejects_every_row_the_codec_rejects() {
        let c = collection();
        // Wrong length, both directions, including the empty slice.
        for len in [0usize, 1, ROW_LEN - 1, ROW_LEN + 1, 48] {
            let row = vec![0u8; len];
            assert!(
                record_from_row(&c, &row, 0, 0).is_err(),
                "length {len} must be rejected, not read"
            );
            assert!(key_from_row(&row).is_err(), "length {len}");
        }
        // A zero `convoys` (`naval_history.rs:444`).
        assert!(record_from_row(&c, &raw_row(1, 0, 1, 1), 0, 0).is_err());
        // A zero killer or owner (`:449`-`:458`) — each on its own, and both.
        for (killer, owner) in [(0u32, 1u32), (1, 0), (0, 0)] {
            let row = raw_row(1, 1, killer, owner);
            assert!(
                record_from_row(&c, &row, 0, 0).is_err(),
                "killer {killer} owner {owner}"
            );
            assert!(key_from_row(&row).is_err(), "killer {killer} owner {owner}");
        }
        // A non-zero reserved tail (`:387`-`:392`) — every bit of it, not one
        // sample, because a single set bit anywhere in the 8 bytes is the whole
        // rule.
        for bit in 0..64u32 {
            let mut row = raw_row(1, 1, 1, 1);
            row[16..24].copy_from_slice(&(1u64 << bit).to_le_bytes());
            assert!(
                record_from_row(&c, &row, 0, 0).is_err(),
                "reserved tail bit {bit}"
            );
            assert!(key_from_row(&row).is_err(), "reserved tail bit {bit}");
        }
        // A month of zero is a legitimate value, not an empty state: the codec
        // range-checks `month` nowhere.
        assert!(record_from_row(&c, &raw_row(0, 1, 1, 1), 0, 0).is_ok());
        assert!(key_from_row(&raw_row(0, 1, 1, 1)).is_ok());
    }

    #[test]
    fn the_mapper_refuses_an_ordinal_outside_the_row_vector() {
        // The ordinal indexes a vector the codec bounds at MAX_ROWS, so a caller
        // that miscounts is caught rather than publishing a position no payload
        // can carry. The boundary itself is not off by one.
        let c = collection();
        let row = raw_row(1, 1, 1, 1);
        assert!(record_from_row(&c, &row, (MAX_ROWS - 1) as u32, 0).is_ok());
        for ordinal in [MAX_ROWS as u32, MAX_ROWS as u32 + 1, u32::MAX] {
            assert!(
                record_from_row(&c, &row, ordinal, 0).is_err(),
                "ordinal {ordinal}"
            );
        }
    }

    #[test]
    fn the_mapper_refuses_a_collection_it_cannot_fill() {
        // A caller handing over another domain's collection must not get a
        // record full of placeholders under this domain's field ids — and a
        // wrong collection ID must not be silently filled either.
        let mut c = collection();
        c.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_row(&c, &raw_row(1, 1, 1, 1), 0, 0).is_err());

        let mut wrong_id = collection();
        wrong_id.collection_id = HISTORY + 1;
        assert!(record_from_row(&wrong_id, &raw_row(1, 1, 1, 1), 0, 0).is_err());
    }

    #[test]
    fn split_payload_enforces_the_codecs_header_rules() {
        let good = good_payload();
        assert!(split_payload(&good, &receipts()).is_ok());

        // Short of the minimum header, at every length.
        for len in 0..LEGACY_HEADER_LEN {
            assert!(split_payload(&good[..len], &receipts()).is_err(), "len {len}");
        }
        // A v3 payload cut inside its own header: the two u64 reads and the
        // receipt block are all fallible.
        for len in LEGACY_HEADER_LEN..HEADER_LEN {
            assert!(
                split_payload(&good[..len], &receipts()).is_err(),
                "v3 header cut at {len}"
            );
        }
        // Bad magic (`naval_history.rs:279`).
        let mut bad_magic = good.clone();
        bad_magic[0..4].copy_from_slice(&0u32.to_le_bytes());
        assert!(split_payload(&bad_magic, &receipts())
            .unwrap_err()
            .contains("SCH1"));
        // Every unsupported version, not one sample (`:284`).
        for version in [0u16, 4, 5, 1000, u16::MAX] {
            let mut bad = good.clone();
            bad[4..6].copy_from_slice(&version.to_le_bytes());
            assert!(
                split_payload(&bad, &receipts()).is_err(),
                "version {version}"
            );
        }
        // The version selects the header length, so a v3 payload claiming the
        // legacy header length is refused and vice versa (`:292`-`:301`).
        for wrong in [LEGACY_HEADER_LEN as u16, 0, 271, 273] {
            let mut bad = good.clone();
            bad[6..8].copy_from_slice(&wrong.to_le_bytes());
            assert!(split_payload(&bad, &receipts()).is_err(), "header_len {wrong}");
        }
        for wrong in [0u16, 23, 25, 48] {
            let mut bad = good.clone();
            bad[8..10].copy_from_slice(&wrong.to_le_bytes());
            assert!(split_payload(&bad, &receipts()).is_err(), "row_len {wrong}");
        }
        // An unsupported v3 authority code (`:306`).
        for raw in [2u16, 3, u16::MAX] {
            let mut bad = good.clone();
            bad[10..12].copy_from_slice(&raw.to_le_bytes());
            assert!(split_payload(&bad, &receipts()).is_err(), "authority {raw}");
        }
        // A zero source sequence (`:314`).
        let mut zero_sequence = good.clone();
        zero_sequence[40..48].copy_from_slice(&0u64.to_le_bytes());
        assert!(split_payload(&zero_sequence, &receipts())
            .unwrap_err()
            .contains("source sequence"));
        // A length that does not exactly cover the row count (`:378`), in both
        // directions.
        let mut short = good.clone();
        short.pop();
        assert!(split_payload(&short, &receipts()).is_err());
        let mut long = good.clone();
        long.push(0);
        assert!(split_payload(&long, &receipts()).is_err());
        // A row_count whose byte length overflows `usize` must be rejected, not
        // wrapped into a plausible small length. It trips the MAX_ROWS bound
        // first, which is the codec's order too (`:366` before `:371`).
        let mut huge = good.clone();
        huge[24..28].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(split_payload(&huge, &receipts()).is_err());
        // MAX_ROWS itself is not off by one: a header claiming one row too many
        // is refused for exceeding the bound rather than for its length.
        let mut over = good.clone();
        over[24..28].copy_from_slice(&((MAX_ROWS + 1) as u32).to_le_bytes());
        assert!(split_payload(&over, &receipts())
            .unwrap_err()
            .contains("exceeds"));
    }

    #[test]
    fn split_payload_checks_every_pinned_receipt() {
        // A payload claiming exact authority under some other build's receipts
        // would be mapped as authoritative and refused by the recorder. Each of
        // the seven is checked at its own offset, so a transposed pair fails.
        let good = good_payload();
        for index in 0..RECEIPT_COUNT {
            for byte in [0usize, 31] {
                let mut bad = good.clone();
                bad[RECEIPT_BLOCK_OFFSET + index * 32 + byte] ^= 0xFF;
                assert!(
                    split_payload(&bad, &receipts()).is_err(),
                    "receipt {index} byte {byte}"
                );
            }
        }
        // A caller supplying a different receipt block refuses the same payload:
        // the check is against what the caller pinned, not against a constant
        // this crate carries.
        let mut other = receipts();
        other.hashes[3][7] ^= 1;
        assert!(split_payload(&good, &other).is_err());
        // v1/v2 have no receipt block at all, so the caller's receipts are
        // irrelevant there.
        let legacy = legacy_payload(&three_rows(), ORDERED_PAYLOAD_VERSION, 0);
        assert!(split_payload(&legacy, &other).is_ok());
    }

    #[test]
    fn split_payload_couples_authority_to_the_status_word_both_ways() {
        // `naval_history.rs:347`-`:356`. An unavailable payload must carry a
        // failure status and no rows; an exact one must carry no failure status.
        const FAILED: u32 = 1 << 8; // COMPACT_STATUS_SOURCE_UNAVAILABLE

        // The well-formed unavailable payload: zero rows, all-zero receipts.
        let unavailable = v3_payload(&[], 0, FAILED, 4_242, 99, 0);
        let (header, body) = split_payload(&unavailable, &receipts()).expect("well-formed");
        assert_eq!(header.authority, Authority::Unavailable);
        assert!(header.is_failure());
        assert!(body.is_empty());

        // Unavailable without a failure status.
        assert!(split_payload(&v3_payload(&[], 0, 0, 4_242, 99, 0), &receipts()).is_err());
        // Unavailable carrying rows.
        assert!(
            split_payload(&v3_payload(&three_rows(), 0, FAILED, 4_242, 99, 3), &receipts())
                .is_err()
        );
        // Unavailable claiming observed rows without carrying any.
        let mut observed_only = v3_payload(&[], 0, FAILED, 4_242, 99, 0);
        observed_only[28..32].copy_from_slice(&1u32.to_le_bytes());
        assert!(split_payload(&observed_only, &receipts()).is_err());
        // Unavailable carrying a real receipt block instead of zeros.
        let mut receipted = v3_payload(&[], 0, FAILED, 4_242, 99, 0);
        receipted[RECEIPT_BLOCK_OFFSET..RECEIPT_BLOCK_OFFSET + 32]
            .copy_from_slice(&receipts().hashes[0]);
        assert!(split_payload(&receipted, &receipts()).is_err());

        // Exact under every failure bit, one at a time.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            assert!(
                split_payload(&v3_payload(&three_rows(), 1, 1 << bit, 4_242, 99, 3), &receipts())
                    .is_err(),
                "exact payload under failure bit {bit}"
            );
        }
        // ...and every informational bit is still accepted.
        for bit in [4u32, 5, 10, 11, 13] {
            assert!(
                split_payload(&v3_payload(&three_rows(), 1, 1 << bit, 4_242, 99, 3), &receipts())
                    .is_ok(),
                "exact payload under informational bit {bit}"
            );
        }
    }

    #[test]
    fn split_payload_refuses_an_unassigned_status_bit_before_classifying_it() {
        // `naval_history.rs:344`-`:346`. Bits 15..=31 have no defined
        // classification, so a payload carrying one is refused rather than read
        // as "not a failure" — the validity-before-classification rule, here
        // built into the grammar itself rather than added by a harness.
        // Bit 15 is ASSIGNED (`COMPACT_STATUS_BASELINE_CLEAR`, the stream's deliberate
        // resync primitive), so the UNASSIGNED run starts at 16.
        for bit in 16..32u32 {
            for extra in [0u32, 1, 1 << 10] {
                let status = (1u32 << bit) | extra;
                assert!(
                    split_payload(
                        &v3_payload(&three_rows(), 1, status, 4_242, 99, 3),
                        &receipts()
                    )
                    .is_err(),
                    "status bit {bit} (+{extra:#x})"
                );
            }
        }
        // The whole known v1 mask minus the failure bits is still accepted.
        let informational = KNOWN_STATUS_MASK_V1 & !FAILURE_STATUS_MASK;
        assert!(split_payload(
            &v3_payload(&three_rows(), 1, informational, 4_242, 99, 3),
            &receipts()
        )
        .is_ok());
    }

    #[test]
    fn decode_carries_the_wire_order_into_the_ordinal() {
        // The whole reason the ordinal exists: the payload's row order is not
        // the keyed set's order, so the position has to be carried. These three
        // rows are in an order that is neither ascending nor descending by key.
        let c = collection();
        let decoded = decode(&good_payload(), &receipts(), &c, 0).expect("decodes");
        assert_eq!(decoded.rows.len(), 3);
        let ordinal_of = |month: i128, killer: i128, owner: i128| {
            decoded.rows[&Key(vec![month, killer, owner])].values[index_of(&c, field::SOURCE_ORDINAL)]
        };
        assert_eq!(ordinal_of(23_310, 8, 9), Value::Unsigned(0));
        assert_eq!(ordinal_of(23_308, 4, 9), Value::Unsigned(1));
        assert_eq!(ordinal_of(23_309, 4, 68), Value::Unsigned(2));
        // The keyed set's own iteration order is the canonical one, which is a
        // DIFFERENT order — this is exactly what the ordinal recovers.
        let by_key: Vec<u64> = decoded
            .rows
            .values()
            .map(|record| match record.values[index_of(&c, field::SOURCE_ORDINAL)] {
                Value::Unsigned(v) => v,
                ref other => panic!("ordinal is {other:?}"),
            })
            .collect();
        assert_eq!(by_key, vec![1, 2, 0]);
    }

    #[test]
    fn decode_enforces_the_codecs_cross_row_rules() {
        let c = collection();
        // `observed_rows != row_count` (`naval_history.rs:436`), both directions.
        for observed in [0u32, 2, 4] {
            let payload = v3_payload(&three_rows(), 1, 0, 4_242, 99, observed);
            assert!(
                decode(&payload, &receipts(), &c, 0).is_err(),
                "observed_rows {observed} against 3 rows"
            );
        }
        // A duplicated identity (`:459`-`:464`) is a rejected payload, never a
        // collapsed keyed set. Duplicating differs from the near-misses below,
        // which must all be accepted.
        let mut duplicated = three_rows();
        duplicated.push(duplicated[1].clone());
        assert!(decode(
            &v3_payload(&duplicated, 1, 0, 4_242, 99, 4),
            &receipts(),
            &c,
            0
        )
        .is_err());
        // A row differing in exactly one identity component is NOT a duplicate.
        for near in [
            raw_row(23_309, 2, 4, 9),
            raw_row(23_308, 2, 5, 9),
            raw_row(23_308, 2, 4, 10),
        ] {
            let mut rows = three_rows();
            rows.push(near);
            assert!(
                decode(&v3_payload(&rows, 1, 0, 4_242, 99, 4), &receipts(), &c, 0).is_ok(),
                "a distinct identity is not a duplicate"
            );
        }
        // Two rows sharing an identity but differing in `convoys` are still a
        // duplicate: the identity is the key, not the whole row.
        let mut same_identity = three_rows();
        same_identity.push(raw_row(23_308, 99, 4, 9));
        assert!(decode(
            &v3_payload(&same_identity, 1, 0, 4_242, 99, 4),
            &receipts(),
            &c,
            0
        )
        .is_err());
    }

    #[test]
    fn v1_must_be_canonically_sorted_and_v2_must_not_be_required_to_be() {
        // `naval_history.rs:400`-`:409`, `:465`-`:469`. The ordering rule is a
        // function of the version, and applying it to the wrong one would either
        // refuse payloads the fold folds or accept a v1 payload the codec does
        // not.
        let c = collection();
        let unsorted = three_rows();
        let mut sorted = three_rows();
        sorted.sort();

        // v2 carries the engine's order and is not sorted; both are accepted.
        for rows in [&unsorted, &sorted] {
            let payload = legacy_payload(rows, ORDERED_PAYLOAD_VERSION, 0);
            assert!(decode(&payload, &receipts(), &c, 0).is_ok());
        }
        // v1 is canonicalised: the sorted form is accepted, the unsorted refused.
        assert!(decode(
            &legacy_payload(&sorted, LEGACY_PAYLOAD_VERSION, 0),
            &receipts(),
            &c,
            0
        )
        .is_ok());
        assert!(decode(
            &legacy_payload(&unsorted, LEGACY_PAYLOAD_VERSION, 0),
            &receipts(),
            &c,
            0
        )
        .unwrap_err()
        .contains("strictly sorted"));
        // A legacy payload with a non-zero reserved word at +10 (`:359`).
        let mut reserved = legacy_payload(&sorted, ORDERED_PAYLOAD_VERSION, 0);
        reserved[10..12].copy_from_slice(&1u16.to_le_bytes());
        assert!(decode(&reserved, &receipts(), &c, 0).is_err());
    }

    #[test]
    fn the_fold_replaces_the_whole_table_and_binds_it_to_its_record() {
        // `world_fold.rs::fold_sunk_convoy_history`: the two typed inputs the payload does
        // not carry are checked, and then the table is replaced outright.
        let c = collection();
        let mut state = State::new();
        let outcome = fold_payload(
            &mut state,
            &c,
            &good_payload(),
            &receipts(),
            Some(4_242),
            Some(99),
            0,
        )
        .expect("folds");
        assert_eq!(
            outcome,
            FoldOutcome::Folded {
                rows: 3,
                ordering: Ordering::FirstSeenAggregateIdentity,
                frame_hour: 4_242,
                source_sequence: 99,
            }
        );
        assert_eq!(state.rows.as_ref().map(|rows| rows.len()), Some(3));

        // Full replace, not a merge: a second payload with one row leaves one.
        let one = v3_payload(&[raw_row(23_400, 1, 7, 7)], 1, 0, 4_243, 100, 1);
        fold_payload(&mut state, &c, &one, &receipts(), Some(4_243), Some(100), 0)
            .expect("folds");
        assert_eq!(state.rows.as_ref().map(|rows| rows.len()), Some(1));
        assert!(state
            .rows
            .as_ref()
            .expect("folded")
            .contains_key(&Key(vec![23_400, 7, 7])));
    }

    #[test]
    fn the_fold_refuses_a_payload_that_does_not_match_its_record() {
        // The headline: `record_sequence` and the frame hour are inputs the
        // payload does not carry, and a mismatch in either is a refusal
        // (`world_fold.rs::fold_sunk_convoy_history`). Swept over both fields and over the
        // `None` the generic dispatcher passes (`world_fold.rs::fold_domain_data`), rather than one case.
        let c = collection();
        let payload = good_payload();
        let cases: [(Option<u64>, Option<u64>); 6] = [
            (Some(4_243), Some(99)),
            (Some(0), Some(99)),
            (None, Some(99)),
            (Some(4_242), Some(100)),
            (Some(4_242), None),
            (None, None),
        ];
        for (hour, sequence) in cases {
            let mut state = State::new();
            state.rows = Some(KeyedState::new());
            let error = fold_payload(&mut state, &c, &payload, &receipts(), hour, sequence, 0)
                .expect_err("a mismatched binding is a refusal");
            assert!(
                error.contains("does not match"),
                "hour {hour:?} sequence {sequence:?}: {error}"
            );
            // And the accumulator is discarded, not retained: this fold mutates
            // before it errors (`:10262`, `:10269`).
            assert_eq!(state.rows, None, "hour {hour:?} sequence {sequence:?}");
        }
        // The matching case is still folded, so the check is a binding and not a
        // blanket refusal.
        let mut state = State::new();
        assert!(fold_payload(&mut state, &c, &payload, &receipts(), Some(4_242), Some(99), 0).is_ok());
    }

    #[test]
    fn a_failure_status_makes_the_hour_unavailable_rather_than_stale() {
        // `world_fold.rs::fold_sunk_convoy_history`. The accumulator is set to `None`, so a
        // reader must not carry the previous hour's table across it. Every
        // failure bit, one at a time.
        let c = collection();
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let status = 1u32 << bit;
            let mut state = State::new();
            fold_payload(
                &mut state,
                &c,
                &good_payload(),
                &receipts(),
                Some(4_242),
                Some(99),
                0,
            )
            .expect("folds a clean payload first");
            assert!(state.rows.is_some());

            let unavailable = v3_payload(&[], 0, status, 4_242, 99, 0);
            let outcome = fold_payload(
                &mut state,
                &c,
                &unavailable,
                &receipts(),
                Some(4_242),
                Some(99),
                0,
            )
            .expect("a failure status is an excluded hour, not a refusal");
            assert_eq!(
                outcome,
                FoldOutcome::Unavailable {
                    status_flags: status
                },
                "bit {bit}"
            );
            assert_eq!(
                state.rows, None,
                "bit {bit}: the accumulator is discarded, not retained"
            );
        }
    }

    #[test]
    fn the_failure_gate_runs_before_the_authority_check() {
        // The order is load-bearing and is the reason d70 has three authority
        // states rather than two: the SAME legacy payload is an excluded hour
        // under a failure status (`world_fold.rs::fold_sunk_convoy_history`) and a stream rejection
        // under a clean one (`world_fold.rs::fold_sunk_convoy_history`).
        let c = collection();
        let mut sorted = three_rows();
        sorted.sort();
        for version in [LEGACY_PAYLOAD_VERSION, ORDERED_PAYLOAD_VERSION] {
            let rows = if version == LEGACY_PAYLOAD_VERSION {
                sorted.clone()
            } else {
                three_rows()
            };
            // Clean status: refused, and the accumulator is discarded.
            let mut state = State::new();
            state.rows = Some(KeyedState::new());
            let error = fold_payload(
                &mut state,
                &c,
                &legacy_payload(&rows, version, 0),
                &receipts(),
                None,
                None,
                0,
            )
            .expect_err("a legacy payload cannot supply production authority");
            assert!(error.contains("legacy-unbound"), "v{version}: {error}");
            assert_eq!(state.rows, None, "v{version}");

            // Failure status: excluded, never reaching the authority check.
            let mut state = State::new();
            state.rows = Some(KeyedState::new());
            let outcome = fold_payload(
                &mut state,
                &c,
                &legacy_payload(&rows, version, 1 << 8),
                &receipts(),
                None,
                None,
                0,
            )
            .expect("the failure gate fires first");
            assert!(matches!(outcome, FoldOutcome::Unavailable { .. }), "v{version}");
            assert_eq!(state.rows, None, "v{version}");
        }
    }

    #[test]
    fn a_malformed_payload_discards_the_accumulator_and_refuses() {
        // `world_fold.rs::fold_sunk_convoy_history`: `decode_payload` runs before the
        // status word is consulted, so a malformed payload is a refusal whatever
        // its status — and it still discards the accumulator.
        let c = collection();
        const FAILED: u32 = 1 << 8;
        let mut broken = good_payload();
        broken[0] ^= 0xFF;
        let mut broken_failed = v3_payload(&[], 0, FAILED, 4_242, 99, 0);
        broken_failed[0] ^= 0xFF;
        for payload in [broken, broken_failed] {
            let mut state = State::new();
            state.rows = Some(KeyedState::new());
            assert!(fold_payload(
                &mut state,
                &c,
                &payload,
                &receipts(),
                Some(4_242),
                Some(99),
                0
            )
            .is_err());
            assert_eq!(state.rows, None);
        }
    }

    #[test]
    fn the_boundary_never_panics_on_arbitrary_bytes() {
        // Every public entry point is total: these read files from disk.
        let c = collection();
        let r = receipts();
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..6_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let len = (state as usize) % 400;
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }
            if len >= 4 && state & 1 == 0 {
                bytes[..4].copy_from_slice(&PAYLOAD_MAGIC.to_le_bytes());
            }
            let _ = split_payload(&bytes, &r);
            let _ = decode(&bytes, &r, &c, 0);
            let mut acc = State::new();
            let _ = fold_payload(&mut acc, &c, &bytes, &r, Some(1), Some(1), 0);
            let _ = key_from_row(&bytes);
            let _ = record_from_row(&c, &bytes, 0, 0);
        }
        // And corruption of a genuinely well-formed payload, byte by byte.
        let good = good_payload();
        for index in 0..good.len() {
            let mut bad = good.clone();
            bad[index] ^= 0xFF;
            let _ = decode(&bad, &r, &c, 0);
        }
    }
}
