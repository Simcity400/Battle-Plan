//! d92 `country_preferred_tactic_live` — each country's selected combat tactic.
//!
//! Derived from `world_fold.rs:14984`-`:15047`
//! (`fold_country_preferred_tactics`, dispatched at `:4747`), its payload codec
// @world-fold-authority domain=92 functions=fold_country_preferred_tactics lines=14984-15047 sha256=71d2fd757b4fb2313157fe9236ed757d33079ecc19ce02c2d3fbfe631d9acf54 closure=f4ea78c8c4a9ff485827253a1b60ce1dd56b9ad523576be7dcb5d0f74c6dabbf
//! `extractor/native/compact_contract/src/preferred_tactic.rs`, and the collector
//! read that produces every row
//! (`extractor/native/collector/src/attach_log.rs:32490`-`:32557`,
//! `:33037`-`:33170`). `extractor/documents/domains/` carries no d92 dossier, so
//! the fold, the codec and the collector read are the whole basis.
//!
//! **Payload grammar** (`preferred_tactic.rs:74`-`:180`). A 164-byte header:
//!
//! | offset | width | header word |
//! |---|---|---|
//! | `+0`   | u64      | `read_time_us` — capture diagnostic, not world state |
//! | `+8`   | u32      | `row_count` |
//! | `+12`  | u32      | `selected_count` |
//! | `+16`  | u32      | `status_flags` |
//! | `+20`  | 4 bytes  | `MAGIC` = `PTC1` |
//! | `+24`  | u16      | `VERSION` = 1 |
//! | `+26`  | u16      | `HEADER_LEN` = 164 |
//! | `+28`  | u64      | `source_sequence` |
//! | `+36`  | 32 bytes | `EXECUTABLE_SHA256` |
//! | `+68`  | 32 bytes | `CONSUMER_EVIDENCE_SHA256` |
//! | `+100` | 32 bytes | `WRITER_EVIDENCE_SHA256` |
//! | `+132` | 32 bytes | `IDENTITY_EVIDENCE_SHA256` |
//!
//! followed by `row_count` **variable-length** rows — `tag_db_id` u32 at `+0`,
//! `tactic_database_id` i32 at `+4`, `flags` u16 at `+8`, `token_len` u16 at
//! `+10`, then `token_len` UTF-8 bytes. **No field is scaled**: every value is a
//! raw integer or a token, so the 32768-vs-100000 class of error has no surface
//! here. There is no legacy short row and no delta grammar.
//!
//! **Full replace per hour** (`world_fold.rs::fold_country_preferred_tactics`): the fold rebuilds
//! `country_preferred_tactics_this_hour` from the record's rows and discards the
//! prior state. There is no accumulator to carry across hours, and every
//! non-folding path clears it outright.
//!
//! # Four-state semantics — the collector is fail-closed for the WHOLE frame
//!
//! A row can carry exactly two shapes, and the codec makes that a hard invariant
//! (`preferred_tactic.rs:155`-`:163`, `:196`-`:213`): either
//! `(tactic_database_id == 0, tactic_id == None, flags == 0)` or
//! `(tactic_database_id > 0, tactic_id == Some(token), flags == SELECTED|TOKEN_PRESENT)`.
//! Nothing else decodes.
//!
//! **The empty shape is `null`, not `unknown`, and the collector is explicit
//! about why.** `compact_country_preferred_tactic_row` (`attach_log.rs:32490`)
//! reads `CCountry+0x15D0` and emits the empty row on exactly ONE path: the
//! pointee's vtable equals `COMBAT_TACTIC_NONE_VTABLE_RVA`, the engine's
//! authenticated no-tactic sentinel class (`:32512`-`:32519`). Every other
//! outcome — a null pointer, a misaligned pointer, an unreadable vtable, an
//! unexpected vtable, a pointer outside the map-control heap, an unreadable or
//! non-positive database id, an unreadable or non-UTF-8 or malformed token — is
//! an `Err`, and `collect_country_preferred_tactic_rows` propagates it with `?`
//! (`:33075`-`:33081`), which makes the **whole frame** source-unavailable
//! (`:33303`-`:33311`). The codec's own module header says the same in one line:
//! *"A null field is not an authenticated no-selection witness and must remain
//! source-unavailable"* (`preferred_tactic.rs:5`-`:6`).
//!
//! So a row that reaches this mapper at all is a row whose read succeeded, and
//! the empty shape means the country's preferred-tactic slot genuinely points at
//! the no-tactic sentinel. That is record-state contract's **null** — "applicable, the relation
//! genuinely points at nothing" — word for word. It is not `unknown`, because a
//! failed read never produces a row; it is not `absent`, because every country in
//! the D18 roster has the slot. **No d92 field is ever `unknown` or `absent`**,
//! and the schema declares neither capability.
//!
//! This is the rare domain where the recipe's Null/Unknown tie-break does not
//! run: the evidence is direct, not absent.
//!
//! # `tactic_database_id` and `tactic_id` are BOTH carried
//!
//! They name the same tactic two ways, and the redundancy was checked against
//! the fold rather than assumed away. The codec requires the mapping to be
//! **injective within one payload** (`preferred_tactic.rs:203`-`:209`), and
//! nothing anywhere requires it to be the same mapping in the next payload: the
//! fold stores whatever decodes (`world_fold.rs::fold_country_preferred_tactics`), so a frame whose id↔token
//! mapping differs from the previous hour's is a frame the fold accepts. Neither
//! field is therefore a function of the other on the payloads the fold folds, and
//! dropping either would lose data on input the recorder accepts — the recipe's
//! "never gate against the fold", applied to a field rather than a row. The
//! measured global stability of the mapping is reported by the harness, not
//! relied on here.
//!
//! **Both are `ContentId`.** The token is content-defined by construction. The
//! database id is the index the engine assigns a tactic **definition loaded from
//! game content** — `IDENTITY_EVIDENCE_SHA256` pins "exact tactic registry
//! population" (`preferred_tactic.rs:43`-`:45`) — so under another mod set the
//! same integer denotes a different tactic, which is exactly step 3's deciding
//! question ("must a content manifest resolve this id for it to keep its meaning
//! across mod sets?"). The survey called it `Unsigned`; this module disagrees with it
//! deliberately, because classing a content-resolved id as `Unsigned` would hide
//! it from codec-measurement contract's ContentId retrofit sweep — the one failure the sweep exists to
//! prevent. The disagreement is recorded in the deliverable's open questions.
//!
//! # `source_sequence` — carried, in a singleton collection
//!
//! `source_sequence` is frame-level, not per-row, and it is **consumed by d96**:
//! `fold_country_preferred_tactic_weights` refuses to fold without a clean d92
//! snapshot *and* its sequence (`world_fold.rs::fold_country_preferred_tactic_weights`), and
//! `tactic_weight.rs:356` rejects a d96 payload whose sequence differs. Dropping
//! it would make d96 unportable, so it is carried — as a one-row collection under
//! a constant key, the shape d68 established for a payload-level scalar with no
//! natural key (`d68.rs:130`-`:146`, `:248`-`:259`).
//!
//! It is **not** declared a derived-field contract derived field, though it looks like one: the
//! collector computes it as `hour_index_from_raw(game_hour_raw) + 1`
//! (`attach_log.rs:33335`, `calendar.rs:22`-`:26`), which is clock-linear with
//! `c = 1`, `v = 1`. Two reasons it stays a plain value. The fold enforces
//! nothing of the kind — it stores whatever u64 decodes — so declaring the
//! transform would refuse frames the recorder folds, and derived-field contract's `u` must equal the
//! container's `ticks_per_hour_log2` for the term to be the game hour
//! (`lib.rs`'s `the_derived_shift_recovers_the_game_hour`), which is a container
//! decision this crate does not get to freeze into a schema. The identity is
//! measured over the capture and reported as an open question instead.
//!
//! # Header words that are NOT fields
//!
//! `read_time_us` and `status_flags` are capture diagnostics rather than world
//! state, exactly as d63 omits its `elapsed_wall_ms`. `row_count` and
//! `selected_count` are functions of the rows the record already carries — the
//! codec enforces `selected_count == rows.filter(id > 0).count()`
//! (`preferred_tactic.rs:181`-`:183`) — so per the recipe's "a value the fold
//! hard-enforces as a function of carried state is not a field" they are checked
//! in [`split_payload`] and left off the wire.
//!
//! # Modelled dependency: the D18 roster-CONTAINMENT gate
//!
//! The fold refuses the hour unless a **proven D18 record landed this same hour**
//! (`world_fold.rs::fold_country_preferred_tactics`) and the observed tag set **covers** the
//! current D18 roster (`world_fold.rs::fold_country_preferred_tactics`) — containment, not equality. The two
//! sides are different enumerations by construction: d92's producer walks the
//! COMPLETE gamestate country table (`attach_log.rs:33054`) so that an
//! unreadable slot fails the frame closed (`:30886`-`:30891`), while D18
//! publishes the deliberately FILTERED identity cache (`:60364`). d92 is
//! therefore a superset — 439 rows against D18's 356 on `runs/batch3/deepwar_c`,
//! the same 83 extra ids in all 896 hours, none ever missing.
//!
//! A d92 row for a tag D18 did not publish is **kept**, exactly as the fold
//! keeps it (`:10360`-`:10367`): the projection is D18-keyed and never reads it
//! (`:6329`), but the D96 join validates against the whole d92 row set, which is
//! the set the collector itself validated before encoding
//! (`compact_contract/src/tactic_weight.rs::validate_against_preferred_tactic`).
//!
//! That makes D18 a modelled dependency, so [`fold_payload`] takes the roster and
//! the freshness flag as typed inputs (recipe step 7) rather than re-deriving or
//! assuming them.
//!
//! # Error contract, and why there is no header/body split
//!
//! `fold_country_preferred_tactics` calls `preferred_tactic::decode_payload`
//! **first**, before it looks at the status word (`world_fold.rs::fold_country_preferred_tactics`).
//! The whole payload — header, every row, every invariant, trailing bytes — is
//! therefore validated on every record whatever its status, which is d83's shape
//! and not d63's: there is no non-folding exit sitting between a header check and
//! the body, so [`split_payload`] carries the entire decode and no `peek_header`
//! is needed or offered.
//!
//! [`fold_payload`] **mutates before erroring**, mirroring the fold exactly: each
//! of the fold's refusal paths clears `country_preferred_tactics_this_hour` (and,
//! on the status path, the sequence too) before returning
//! (`world_fold.rs::fold_country_preferred_tactics`). A harness that assumed
//! all-or-nothing would disagree with the recorder on exactly the frames that
//! fail.

use std::collections::{BTreeMap, BTreeSet};

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 92;
/// The per-country table.
pub const COUNTRIES: u32 = 1;
/// The payload-level scalar carried under a constant key — see the module docs.
pub const FRAME: u32 = 2;

/// Field ids are stable and append-only; schema-identity contract forbids reuse for a different
/// field. Numbered uniquely across the whole domain, not per collection, so a
/// later reader cannot confuse two collections' ids.
pub mod field {
    pub const TAG_DB_ID: u32 = 1;
    pub const TACTIC_DATABASE_ID: u32 = 2;
    pub const TACTIC_ID: u32 = 3;
    pub const FRAME_SINGLETON: u32 = 4;
    pub const SOURCE_SEQUENCE: u32 = 5;
}

// --- Payload grammar, mirrored from `compact_contract/src/preferred_tactic.rs`.
//
// The constants are mirrored rather than imported: `wire_format` is the
// reference implementation of the spec and takes no dependency on the codec
// crate outside its measurement bins (d62 set that precedent, d83 followed it,
// and it is followed here rather than re-decided). `MAGIC`, `VERSION` and
// `HEADER_LEN` are all checked by `split_payload`, which is what makes the
// mirror safe: a producer that changed the grammar cannot decode past them.

/// `preferred_tactic.rs:11` — `PTC1`.
pub const MAGIC: [u8; 4] = *b"PTC1";
/// `preferred_tactic.rs:12`.
pub const VERSION: u16 = 1;
/// `preferred_tactic.rs:13`.
pub const HEADER_LEN: usize = 164;
/// `preferred_tactic.rs`'s `MAX_ROWS`. Was `1024`, a guess at the
/// vanilla-plus-DLC country count that refused the whole roster payload on any
/// larger mod; a country count is content, so it is the structural row width
/// and [`max_rows_in`] is what bounds the allocation.
pub const MAX_ROWS: usize = i32::MAX as usize;

/// `preferred_tactic.rs`'s `max_rows_in`: the largest number of rows
/// `remaining` bytes of payload can hold, a row being at least
/// [`FIXED_ROW_LEN`] bytes. A FORMAT bound — without it a four-byte stream word
/// decides an allocation.
#[must_use]
pub const fn max_rows_in(remaining: usize) -> usize {
    remaining / FIXED_ROW_LEN
}
/// `preferred_tactic.rs:15`.
pub const MAX_TOKEN_BYTES: usize = 256;
/// The fixed part of a row: `tag_db_id`, `tactic_database_id`, `flags`,
/// `token_len` (`preferred_tactic.rs:100`-`:106`).
pub const FIXED_ROW_LEN: usize = 12;

/// `preferred_tactic.rs:17`.
pub const FLAG_SELECTED: u16 = 1 << 0;
/// `preferred_tactic.rs:18`.
pub const FLAG_TOKEN_PRESENT: u16 = 1 << 1;
/// `preferred_tactic.rs:19`. Any other bit is a hard error (`:148`).
pub const KNOWN_FLAGS: u16 = FLAG_SELECTED | FLAG_TOKEN_PRESENT;

/// `preferred_tactic.rs:21`-`:26`.
pub const EXECUTABLE_SHA256: [u8; 32] = [
    0x87, 0x0e, 0xbf, 0x1f, 0x61, 0x52, 0x62, 0xfa, 0xcc, 0xa4, 0x8a, 0x3b, 0x94, 0x77, 0x75, 0xc4,
    0x1f, 0x0a, 0x53, 0x38, 0xf3, 0x06, 0x8c, 0x40, 0x95, 0x82, 0x09, 0xf5, 0xc4, 0x1c, 0x65, 0xad,
];
/// `preferred_tactic.rs:30`-`:34`.
pub const CONSUMER_EVIDENCE_SHA256: [u8; 32] = [
    0x43, 0x06, 0x65, 0xf0, 0x5f, 0xec, 0x12, 0x94, 0x79, 0x0c, 0xb7, 0xe6, 0x17, 0x34, 0x26, 0x48,
    0xb7, 0x47, 0x57, 0x31, 0xec, 0x5b, 0xd0, 0xc4, 0xa1, 0x9a, 0x40, 0x12, 0x3a, 0xf2, 0x4d, 0xb3,
];
/// `preferred_tactic.rs:38`-`:42`.
pub const WRITER_EVIDENCE_SHA256: [u8; 32] = [
    0xd6, 0x32, 0x7e, 0x0e, 0x86, 0x22, 0x9e, 0x5c, 0x1e, 0xc6, 0xbe, 0x0c, 0x13, 0x1b, 0xd9, 0x5d,
    0xcb, 0x31, 0x89, 0x71, 0x9d, 0x68, 0x26, 0x4e, 0x7b, 0xb5, 0x95, 0x99, 0x54, 0xd4, 0x3a, 0x49,
];
/// `preferred_tactic.rs:46`-`:50`.
pub const IDENTITY_EVIDENCE_SHA256: [u8; 32] = [
    0x75, 0xd2, 0x25, 0xd3, 0x08, 0x98, 0x5d, 0xad, 0x5e, 0xfe, 0x74, 0x33, 0xef, 0xad, 0x10, 0xff,
    0xae, 0xe1, 0xbc, 0x48, 0x43, 0x26, 0x74, 0x7a, 0x7c, 0x96, 0xb9, 0x6e, 0x3b, 0x49, 0xac, 0xa8,
];

/// The constant key component of the [`FRAME`] singleton. Zero, so a reader that
/// ever sees a second key in that collection knows the producer is wrong.
pub const SINGLETON_KEY_COMPONENT: u64 = 0;

pub fn schema() -> Schema {
    use field as f;
    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![
            Collection {
                collection_id: COUNTRIES,
                // Full replace each hour (`world_fold.rs::fold_country_preferred_tactics`), but `tag_db_id`
                // is a stable identity that persists across hours, which is what
                // a keyed set needs — and only a keyed set has a delta grammar
                // (collection-shape contract). The roster is constant at 439 rows on the measured
                // capture, so the delta path is what this domain actually costs.
                shape: Shape::KeyedSet,
                fields: vec![
                    // The country's `tag_db_id` (`world_fold.rs::fold_country_preferred_tactics`), the same
                    // identity D18 keys its roster by and which d92 is gated on
                    // being EQUAL to. `ContentId` because a country tag is
                    // content-defined (content-identity contract); d63 and d83 class theirs the same.
                    // This is a ContentId **key**, the heavier codec-measurement contract retrofit class
                    // — key components sort and delta-encode by raw value.
                    Field::new(f::TAG_DB_ID, Kind::ContentId).key(0),
                    // Null exactly when the country's slot points at the engine's
                    // authenticated no-tactic sentinel; never unknown, never
                    // absent. See the module docs for the collector evidence.
                    Field::new(f::TACTIC_DATABASE_ID, Kind::ContentId).nullable(),
                    Field::new(f::TACTIC_ID, Kind::ContentId).nullable(),
                ],
            },
            Collection {
                collection_id: FRAME,
                // One row, always, under `SINGLETON_KEY_COMPONENT`. A keyed set
                // with a constant key is d68's shape for a payload-level scalar,
                // and it gives the value a delta grammar like any other row.
                shape: Shape::KeyedSet,
                fields: vec![
                    // `Unsigned`, not `ContentId`: the constant names nothing a
                    // content manifest resolves (d68.rs:255`-`:259`).
                    Field::new(f::FRAME_SINGLETON, Kind::Unsigned).key(0),
                    // The producer advances it once per captured hour
                    // (`attach_log.rs:33335`). schema-kind contract makes `Monotone` an encoding
                    // hint and never a validity constraint, so a producer that
                    // reset it would still encode. Always present: the header
                    // always carries the word, so no capability is declared.
                    Field::new(f::SOURCE_SEQUENCE, Kind::Monotone),
                ],
            },
        ],
        event_part: None,
    }
}

// --- content-identity contract symbol resolution ----------------------------------------------------

/// The content-defined `tactic_id` vocabulary, as manifest ids.
///
/// schema-kind contract declares no string kind and `Kind::legal_as_key` admits only `Unsigned`,
/// `Signed` and `ContentId` (`schema.rs:29`-`:31`), but content-identity contract already answers what
/// these payloads carry: "Country tags, equipment names, province and state ids
/// are content-defined. The stream carries manifest ids as varints, never text"
/// A combat-tactic token is content-defined —
/// `IDENTITY_EVIDENCE_SHA256` pins the registry population that mints it — so it
/// is a `ContentId` resolved through a table the **caller supplies**.
///
/// content-identity contract's ids are file-global and in-stream interning is forbidden — "it must be
/// fully read before any seek" — so this table is an **input** to the boundary,
/// built once from the whole content set before any record is mapped, and never
/// grown by the mapper.
///
/// The production assignment now lives in the neutral content-manifest crate's
/// `symbol_manifest` module; this domain-local mapper still awaits the
/// file-global resolver retrofit. [`Symbols::from_vocabulary`] uses a
/// measurement stand-in that is
/// deterministic, order-independent and derived only from the content set — the
/// 0-based index into the sorted, deduplicated list — and it is labelled as a
/// stand-in, not as an answer. This is d29's table, taken deliberately unchanged
/// (`d29.rs:205`); two properties make it safe here:
///
/// - Nothing downstream computes from the id's value. `ContentId` is a plain
///   varint (schema-kind contract), and `tactic_id` is a **value** field, not a key, so no key
///   ordering or delta layout depends on the choice. (This domain's key is a
///   `ContentId` too, but its ids are raw `tag_db_id` casts, not symbol ids —
///   a separate retrofit, flagged separately.)
/// - The measurement is invariant to it over **this domain's own vocabulary**:
///   any assignment over fewer than 128 symbols is a one-byte varint for every
///   row. content-identity contract's ids are file-global, so an assignment over a whole content set can
///   still widen the varint; the claim made is the one measured, not more.
///
/// During this mapper's content-identity contract integration, this type is replaced by the
/// container's manifest resolver and every call site changes with it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Symbols {
    /// Sorted and deduplicated; the id of a symbol is its index.
    vocabulary: Vec<String>,
}

impl Symbols {
    /// A table with no symbols. A row carrying a token is rejected against it —
    /// fail-closed, so an unresolvable symbol is a diagnosed error and never a
    /// record with the token quietly dropped.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build from every token the content set can produce. The input is sorted
    /// and deduplicated here, so callers may pass it in any order and two callers
    /// over the same content set always mint the same ids.
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

    /// The id a symbol resolves to, or `None` if it is outside the content set.
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

    pub fn len(&self) -> usize {
        self.vocabulary.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vocabulary.is_empty()
    }
}

// --- The boundary mapping ----------------------------------------------------

/// The 164-byte payload header (`preferred_tactic.rs:113`-`:131`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub read_time_us: u64,
    pub row_count: u32,
    pub selected_count: u32,
    pub status_flags: u32,
    pub source_sequence: u64,
}

/// One decoded row (`preferred_tactic.rs:52`-`:57`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TacticRow {
    pub tag_db_id: u32,
    pub tactic_database_id: i32,
    pub tactic_id: Option<String>,
}

impl TacticRow {
    /// Whether the country's slot points at a tactic. The codec makes this the
    /// same question as "is `tactic_id` present" (`preferred_tactic.rs:155`).
    pub fn is_selected(&self) -> bool {
        self.tactic_database_id > 0
    }
}

/// The codec's per-row none/selected identity invariant
/// (`preferred_tactic.rs:196`-`:213`), as a predicate over a decoded row.
///
/// A row is one of exactly two shapes: `(0, None)`, the authenticated no-tactic
/// sentinel, or `(id > 0, Some(token))` with a token the codec would accept.
/// Nothing else decodes, so nothing else may be mapped.
///
/// **This is checked at BOTH public boundaries, not only in [`split_payload`].**
/// `TacticRow`'s fields are public, so a caller can hand [`record_from_row`] a
/// row that never came from a payload; without this check the mapper would emit
/// an impossible four-state pairing — `Null` for the database id beside a real
/// `ContentId` for the token — for a row the fold could never produce. A
/// convention enforced in one entry point is not an invariant.
pub fn check_row(row: &TacticRow) -> core::result::Result<(), String> {
    if row.tag_db_id == 0 {
        return Err("country_preferred_tactic row with zero tag_db_id".into());
    }
    match (row.tactic_database_id, row.tactic_id.as_deref()) {
        (0, None) => Ok(()),
        (id, Some(token)) if id > 0 => {
            if token.is_empty()
                || token.len() > MAX_TOKEN_BYTES
                || token.chars().any(char::is_control)
            {
                return Err(format!(
                    "country_preferred_tactic tactic token {token:?} is invalid"
                ));
            }
            Ok(())
        }
        (id, token) => Err(format!(
            "country_preferred_tactic row {} has an invalid tactic id/token pair \
             (id {id}, token {})",
            row.tag_db_id,
            if token.is_some() { "present" } else { "absent" }
        )),
    }
}

/// Split a d92 DATA payload into its header and its rows, applying **every**
/// check `preferred_tactic::decode_payload` applies (`:113`-`:180`) plus
/// `validate_rows` (`:186`-`:222`).
///
/// The fold decodes the whole payload before it consults the status word
/// (`world_fold.rs::fold_country_preferred_tactics`), so there is no cheaper header-only exit to preserve
/// and nothing here is deferred: rows are variable-length, so the walk is
/// required to find a row boundary at all.
///
/// This lives here rather than in each caller so the grammar has exactly one
/// implementation: a measurement harness that re-derives it is a harness that can
/// drift from the recorder without anything failing. Nothing in this function can
/// panic on any byte string.
pub fn split_payload(
    payload: &[u8],
) -> core::result::Result<(PayloadHeader, Vec<TacticRow>), String> {
    // `:114`-`:116`.
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "country_preferred_tactic payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    let header = PayloadHeader {
        read_time_us: u64_at(payload, 0),
        row_count: u32_at(payload, 8),
        selected_count: u32_at(payload, 12),
        status_flags: u32_at(payload, 16),
        source_sequence: u64_at(payload, 28),
    };
    // `:123`-`:129`.
    if payload[20..24] != MAGIC
        || u16_at(payload, 24) != VERSION
        || u16_at(payload, 26) as usize != HEADER_LEN
    {
        return Err("country_preferred_tactic PTC1 magic/version/header mismatch".into());
    }
    // `:131`-`:136`. Four authority receipts; a mismatch means the payload was
    // not produced by the build this grammar belongs to.
    for (offset, expected, what) in [
        (36usize, &EXECUTABLE_SHA256, "executable"),
        (68, &CONSUMER_EVIDENCE_SHA256, "consumer evidence"),
        (100, &WRITER_EVIDENCE_SHA256, "writer evidence"),
        (132, &IDENTITY_EVIDENCE_SHA256, "identity evidence"),
    ] {
        if &payload[offset..offset + 32] != expected.as_slice() {
            return Err(format!(
                "country_preferred_tactic PTC1 {what} authority receipt mismatch"
            ));
        }
    }
    // `:137`-`:139`.
    let row_count = header.row_count as usize;
    let selected_count = header.selected_count as usize;
    if row_count > MAX_ROWS || selected_count > row_count {
        return Err(format!(
            "country_preferred_tactic row count outside bounds \
             (row_count {row_count}, selected_count {selected_count}, max {MAX_ROWS})"
        ));
    }

    // The count is stream-supplied and sizes the allocation, so it is bounded
    // by what the remaining payload can hold.
    if row_count > max_rows_in(payload.len() - HEADER_LEN) {
        return Err(format!(
            "country_preferred_tactic row count {row_count} exceeds the payload's capacity"
        ));
    }

    let mut at = HEADER_LEN;
    let mut rows = Vec::with_capacity(row_count);
    for index in 0..row_count {
        // `:142`-`:146`. Every read is bounds-checked before it happens; a
        // truncated payload is an error, never a panic.
        let fixed_end = at
            .checked_add(FIXED_ROW_LEN)
            .ok_or("country_preferred_tactic row cursor overflow")?;
        if fixed_end > payload.len() {
            return Err(format!(
                "country_preferred_tactic row {index} truncated at byte {at}"
            ));
        }
        let tag_db_id = u32_at(payload, at);
        let tactic_database_id = i32_at(payload, at + 4);
        let flags = u16_at(payload, at + 8);
        let token_len = u16_at(payload, at + 10) as usize;
        at = fixed_end;
        // `:147`-`:149`.
        if flags & !KNOWN_FLAGS != 0 || token_len > MAX_TOKEN_BYTES {
            return Err(format!(
                "country_preferred_tactic row {index} flags {flags:#x} / token length \
                 {token_len} invalid"
            ));
        }
        let end = at
            .checked_add(token_len)
            .ok_or("country_preferred_tactic token cursor overflow")?;
        if end > payload.len() {
            return Err(format!(
                "country_preferred_tactic row {index} token truncated at byte {at}"
            ));
        }
        let token = &payload[at..end];
        at = end;
        // `:153`-`:161`.
        let tactic_id = if token.is_empty() {
            None
        } else {
            Some(
                core::str::from_utf8(token)
                    .map_err(|_| {
                        format!("country_preferred_tactic row {index} token is not UTF-8")
                    })?
                    .to_owned(),
            )
        };
        // `:162`-`:169`: the none/selected identity invariant, in the codec's own
        // form. `tactic_database_id < 0` is rejected outright; a selected row must
        // carry BOTH flags and a token; an unselected row must carry neither.
        let selected = tactic_database_id > 0;
        if tactic_database_id < 0
            || (selected != (flags == KNOWN_FLAGS && tactic_id.is_some()))
            || (!selected && (flags != 0 || tactic_id.is_some()))
        {
            return Err(format!(
                "country_preferred_tactic row {index} none/selected identity invariant failed \
                 (id {tactic_database_id}, flags {flags:#x}, token {})",
                if tactic_id.is_some() { "present" } else { "absent" }
            ));
        }
        rows.push(TacticRow {
            tag_db_id,
            tactic_database_id,
            tactic_id,
        });
    }
    // `:171`-`:173`.
    if at != payload.len() {
        return Err(format!(
            "country_preferred_tactic trailing bytes: consumed {at} of {}",
            payload.len()
        ));
    }
    validate_rows(&rows)?;
    // `:176`-`:178`. `selected_count` is a function of the rows, which is why it
    // is checked here and not carried on the wire.
    let observed_selected = rows.iter().filter(|row| row.is_selected()).count();
    if observed_selected != selected_count {
        return Err(format!(
            "country_preferred_tactic selected count mismatch: header {selected_count}, \
             rows {observed_selected}"
        ));
    }
    Ok((header, rows))
}

/// `preferred_tactic::validate_rows` (`:186`-`:222`), applied to decoded rows.
///
/// Two rules that are easy to lose and both load-bearing: tags are unique,
/// **ascending** and non-zero (so the keyed set's order is the payload's order),
/// and the id↔token mapping is injective **within this payload** — which is
/// precisely the scope of the guarantee, and why both fields are carried.
fn validate_rows(rows: &[TacticRow]) -> core::result::Result<(), String> {
    if rows.len() > MAX_ROWS {
        return Err(format!(
            "country_preferred_tactic {} rows exceeds {MAX_ROWS}",
            rows.len()
        ));
    }
    let mut previous = 0u32;
    let mut by_id: BTreeMap<i32, &str> = BTreeMap::new();
    let mut by_token: BTreeMap<&str, i32> = BTreeMap::new();
    let mut tags: BTreeSet<u32> = BTreeSet::new();
    for row in rows {
        if row.tag_db_id == 0 || row.tag_db_id <= previous || !tags.insert(row.tag_db_id) {
            return Err(format!(
                "country_preferred_tactic rows must have unique ascending nonzero tag ids \
                 (saw {} after {previous})",
                row.tag_db_id
            ));
        }
        previous = row.tag_db_id;
        // The per-row shape rule has exactly one implementation, shared with
        // `record_from_row` — see [`check_row`].
        check_row(row)?;
        // The cross-row rule, which only makes sense over a whole payload: the
        // id↔token mapping is injective WITHIN this payload and nowhere wider.
        if let (id, Some(token)) = (row.tactic_database_id, row.tactic_id.as_deref()) {
            if id > 0
                && (by_id.insert(id, token).is_some_and(|old| old != token)
                    || by_token.insert(token, id).is_some_and(|old| old != id))
            {
                return Err(
                    "country_preferred_tactic tactic id/token mapping is not injective".into(),
                );
            }
        }
    }
    Ok(())
}

/// The key of one country row: its `tag_db_id` (`world_fold.rs::fold_country_preferred_tactics`).
///
/// A zero `tag_db_id` never decodes (`preferred_tactic.rs:194`), and `TacticRow`
/// is publicly constructible, so the check is here rather than assumed.
pub fn key_from_row(row: &TacticRow) -> core::result::Result<Key, String> {
    if row.tag_db_id == 0 {
        return Err("country_preferred_tactic row with zero tag_db_id".into());
    }
    Ok(key(&[row.tag_db_id as u64]))
}

/// The [`FRAME`] singleton's only key.
pub fn key_from_frame() -> Key {
    key(&[SINGLETON_KEY_COMPONENT])
}

/// One decoded row of the `countries` table, as the collector emits it today,
/// onto its stored Record. The caller has already consumed the payload header via
/// [`split_payload`], which is also what decoded and validated this row; d92 has
/// exactly one row form, so nothing here selects between forms.
///
/// `frame_tick` is derived-field contract's derived-field input (stored = v + c * (frame_tick >> u));
/// d92 declares no derived field, so it is taken and ignored — every domain's
/// mapper has the same shape. `symbols` is step 3's content-identity contract resolver, the typed input
/// a content-defined text field requires (d29's `symbols: &Symbols`).
///
/// Fail-closed throughout: an unresolvable token is an `Err`, never a value
/// weakened to `Unknown` — the string is right here, so "not captured" would be
/// false, and a token outside the content set means the caller built the table
/// from the wrong content, which is a diagnosis rather than data.
pub fn record_from_row(
    collection: &Collection,
    row: &TacticRow,
    frame_tick: u64,
    symbols: &Symbols,
) -> core::result::Result<Record, String> {
    // d92 has no derived field; derived-field contract's transform input is unused. Named rather
    // than `_frame_tick` because the signature is the cross-domain contract.
    let _ = frame_tick;
    // A record built for the wrong collection would otherwise come out shaped by
    // whatever fields that collection happens to have.
    if collection.collection_id != COUNTRIES {
        return Err(format!(
            "country_preferred_tactic country row built against collection {}",
            collection.collection_id
        ));
    }
    // The key is not carried in the record body (record-state contract), but its validity is still
    // part of this row's validity.
    let _ = key_from_row(row)?;
    // `TacticRow` is publicly constructible, so `split_payload` having checked
    // the shape is not a precondition this function may assume. Without this, a
    // hand-built `(0, Some(token))` row would publish `Null` for the database id
    // beside a real `ContentId` for the token — a four-state pairing no payload
    // the fold accepts can produce.
    check_row(row)?;

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            // Null on the authenticated no-tactic sentinel — see the module docs.
            // The raw cast is the unresolved manifest-id assignment: today's boundary
            // mappers cast the source id straight into `ContentId`, and this field
            // is listed in the deliverable's retrofit sweep.
            field::TACTIC_DATABASE_ID => {
                if row.is_selected() {
                    Value::ContentId(row.tactic_database_id as u64)
                } else {
                    Value::Null
                }
            }
            field::TACTIC_ID => match row.tactic_id.as_deref() {
                None => Value::Null,
                Some(token) => match symbols.id(token) {
                    Some(id) => Value::ContentId(id),
                    None => {
                        return Err(format!(
                            "country_preferred_tactic token {token:?} is not in the symbol \
                             table ({} symbol(s))",
                            symbols.len()
                        ))
                    }
                },
            },
            other => {
                // Filling an unknown field with a placeholder would publish
                // invented data under a field id that means something else.
                return Err(format!(
                    "country_preferred_tactic has no mapping for field id {other}"
                ));
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// The [`FRAME`] singleton's record: the payload-level `source_sequence`.
///
/// Takes the parsed header rather than the raw payload so the header grammar is
/// read exactly once, by [`split_payload`].
pub fn record_from_frame_header(
    collection: &Collection,
    header: &PayloadHeader,
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    if collection.collection_id != FRAME {
        return Err(format!(
            "country_preferred_tactic frame row built against collection {}",
            collection.collection_id
        ));
    }
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::SOURCE_SEQUENCE => Value::Unsigned(header.source_sequence),
            other => {
                return Err(format!(
                    "country_preferred_tactic frame has no mapping for field id {other}"
                ))
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// Every `tactic_id` token one payload carries, for building a [`Symbols`] table
/// over a whole capture before any record is mapped (content-identity contract: fully read first).
///
/// Shares [`split_payload`] rather than re-walking the payload, so the vocabulary
/// and the records can never disagree about what a row says.
pub fn tokens_in_payload(payload: &[u8]) -> core::result::Result<Vec<String>, String> {
    let (_, rows) = split_payload(payload)?;
    Ok(rows.into_iter().filter_map(|row| row.tactic_id).collect())
}

// --- The frame-level fold ----------------------------------------------------

/// The accumulator `fold_country_preferred_tactics` maintains, as wire state.
///
/// The two collections are held as ONE array in [`schema`] order, so
/// [`State::as_slice`] is the shape `encode_baseline`, `encode_delta` and
/// `decode_delta` take with no copy and no chance of a caller passing them in
/// the wrong order.
#[derive(Clone, Debug, Default)]
pub struct State {
    collections: [KeyedState; 2],
}

/// Index of [`COUNTRIES`] in [`State::as_slice`] and in `schema().collections`.
pub const COUNTRIES_INDEX: usize = 0;
/// Index of [`FRAME`] in [`State::as_slice`] and in `schema().collections`.
pub const FRAME_INDEX: usize = 1;

impl State {
    pub fn new() -> Self {
        Self::default()
    }

    /// Both collections in [`schema`] order.
    pub fn as_slice(&self) -> &[KeyedState] {
        &self.collections
    }

    pub fn countries(&self) -> &KeyedState {
        &self.collections[COUNTRIES_INDEX]
    }

    pub fn frame(&self) -> &KeyedState {
        &self.collections[FRAME_INDEX]
    }

    /// Rows in the country table — the domain's size, which is what the
    /// measurement reports per hour.
    pub fn len(&self) -> usize {
        self.countries().len()
    }

    pub fn is_empty(&self) -> bool {
        self.countries().is_empty()
    }

    /// Drop the hour's state, as the fold's gate refusals do
    /// (`world_fold.rs::fold_country_preferred_tactics`).
    fn clear(&mut self) {
        for collection in &mut self.collections {
            collection.clear();
        }
    }
}

/// Why [`fold_payload`] did not fold a record. The three outcomes are named
/// because the harness has to report them apart (recipe step 9).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FoldRefusal {
    /// The hour carried no proven D18 record (`world_fold.rs::fold_country_preferred_tactics`).
    NoCurrentHourRoster,
    /// The observed tag set does not COVER the current D18 roster: at least one
    /// tag the projection can name has no measured row (`world_fold.rs::fold_country_preferred_tactics`).
    RosterDoesNotCoverD18 {
        missing: usize,
        d18: usize,
        observed: usize,
    },
}

/// Fold one d92 DATA payload into `acc`, exactly as `fold_country_preferred_tactics`
/// folds it (`world_fold.rs::fold_country_preferred_tactics`).
///
/// The caller has already excluded a failure-status record — that exclusion is
/// the fold's own `handle_compact_failure` (`:10311`) and it sits *after* the
/// decode, which is why `split_payload` runs on every record whatever its status.
///
/// `roster` and `identity_known` are the D18 dependency the fold reads and the
/// payload does not carry (recipe step 7's typed-input rule): `roster` is the
/// current `tag_by_db_id` key set and `identity_known` is
/// `country_identity_known_this_hour`, which is true only when a proven D18
/// record landed **this** hour (`world_fold.rs:1934`-`:1944`).
///
/// **Error contract: this MUTATES before erroring**, mirroring the fold. Every
/// refusal path clears the accumulator first, so a caller must not treat an `Err`
/// as leaving the prior hour's state intact — the recorder does not.
pub fn fold_payload(
    schema: &Schema,
    acc: &mut State,
    payload: &[u8],
    roster: &BTreeSet<u32>,
    identity_known: bool,
    frame_tick: u64,
    symbols: &Symbols,
) -> core::result::Result<(), String> {
    // The decode comes first, before any gate: `world_fold.rs::fold_country_preferred_tactics`.
    let (header, rows) = split_payload(payload)?;

    // `:10320`-`:10325`.
    if !identity_known {
        acc.clear();
        return Err(refusal_message(FoldRefusal::NoCurrentHourRoster));
    }
    // `:10345`-`:10359`. CONTAINMENT (D18 ⊆ d92), not equality: d92 reads the
    // complete country table and D18 the filtered cache, so a superset is the
    // normal shape and only a MISSING D18 tag is a loss.
    let observed: BTreeSet<u32> = rows.iter().map(|row| row.tag_db_id).collect();
    let missing = roster.difference(&observed).count();
    if missing != 0 {
        acc.clear();
        return Err(refusal_message(FoldRefusal::RosterDoesNotCoverD18 {
            missing,
            d18: roster.len(),
            observed: observed.len(),
        }));
    }

    let countries = schema.collection(COUNTRIES).map_err(|e| format!("{e:?}"))?;
    let frame = schema.collection(FRAME).map_err(|e| format!("{e:?}"))?;

    // Full replace (`:10360`-`:10366`): the prior state is discarded, not merged.
    let mut next_countries = KeyedState::new();
    for row in &rows {
        let record = record_from_row(countries, row, frame_tick, symbols)?;
        // The fold collects into a map keyed by `tag_db_id` (`:10364`), and
        // `validate_rows` has already proved the tags unique, so an insert can
        // never collapse two rows here.
        if next_countries.insert(key_from_row(row)?, record).is_some() {
            return Err(format!(
                "country_preferred_tactic duplicate tag_db_id {} survived validation",
                row.tag_db_id
            ));
        }
    }
    let mut next_frame = KeyedState::new();
    next_frame.insert(
        key_from_frame(),
        record_from_frame_header(frame, &header, frame_tick)?,
    );

    acc.collections[COUNTRIES_INDEX] = next_countries;
    acc.collections[FRAME_INDEX] = next_frame;
    Ok(())
}

fn refusal_message(refusal: FoldRefusal) -> String {
    match refusal {
        FoldRefusal::NoCurrentHourRoster => {
            "country_preferred_tactic requires a current-hour D18 identity roster".into()
        }
        FoldRefusal::RosterDoesNotCoverD18 {
            missing,
            d18,
            observed,
        } => format!(
            "country_preferred_tactic roster does not cover current D18 roster \
             ({missing} of {d18} D18 tags missing; d92 carries {observed})"
        ),
    }
}

/// Callers reach these only after a bounds check, so the slices are in range.
fn u64_at(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().expect("8 bytes"))
}

fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().expect("4 bytes"))
}

fn i32_at(b: &[u8], off: usize) -> i32 {
    i32::from_le_bytes(b[off..off + 4].try_into().expect("4 bytes"))
}

fn u16_at(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(b[off..off + 2].try_into().expect("2 bytes"))
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

    fn countries() -> Collection {
        schema().collections[0].clone()
    }

    fn frame_collection() -> Collection {
        schema().collections[1].clone()
    }

    fn index_of(c: &Collection, id: u32) -> usize {
        c.value_fields()
            .position(|f| f.field_id == id)
            .expect("field is in the schema")
    }

    fn symbols() -> Symbols {
        Symbols::from_vocabulary(["blitzkrieg", "encirclement", "shock", "trench_warfare"])
    }

    /// One raw row, packed the way `preferred_tactic::encode_payload` packs one
    /// (`preferred_tactic.rs:98`-`:109`).
    fn raw_row(tag_db_id: u32, tactic_database_id: i32, flags: u16, token: &[u8]) -> Vec<u8> {
        let mut row = Vec::with_capacity(FIXED_ROW_LEN + token.len());
        row.extend_from_slice(&tag_db_id.to_le_bytes());
        row.extend_from_slice(&tactic_database_id.to_le_bytes());
        row.extend_from_slice(&flags.to_le_bytes());
        row.extend_from_slice(&(token.len() as u16).to_le_bytes());
        row.extend_from_slice(token);
        row
    }

    /// A selected row, with both flags and a token, as the collector emits it.
    fn selected_row(tag_db_id: u32, id: i32, token: &str) -> Vec<u8> {
        raw_row(tag_db_id, id, KNOWN_FLAGS, token.as_bytes())
    }

    /// The authenticated no-tactic sentinel row (`attach_log.rs:32515`-`:32519`).
    fn none_row(tag_db_id: u32) -> Vec<u8> {
        raw_row(tag_db_id, 0, 0, b"")
    }

    /// How many of these raw rows are selected, read back out of the packed
    /// bytes so the header a test builds always agrees with its body.
    fn selected_in(rows: &[Vec<u8>]) -> u32 {
        rows.iter()
            .filter(|row| i32::from_le_bytes(row[4..8].try_into().expect("4 bytes")) > 0)
            .count() as u32
    }

    /// A whole PTC1 payload whose header agrees with its rows.
    fn payload_with(
        rows: &[Vec<u8>],
        read_time_us: u64,
        status_flags: u32,
        source_sequence: u64,
    ) -> Vec<u8> {
        payload_raw(
            rows,
            read_time_us,
            rows.len() as u32,
            selected_in(rows),
            status_flags,
            source_sequence,
        )
    }

    /// A payload with every header word supplied, so a test can build a header
    /// that disagrees with its body on purpose.
    fn payload_raw(
        rows: &[Vec<u8>],
        read_time_us: u64,
        row_count: u32,
        selected_count: u32,
        status_flags: u32,
        source_sequence: u64,
    ) -> Vec<u8> {
        let mut p = Vec::with_capacity(HEADER_LEN);
        p.extend_from_slice(&read_time_us.to_le_bytes());
        p.extend_from_slice(&row_count.to_le_bytes());
        p.extend_from_slice(&selected_count.to_le_bytes());
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&MAGIC);
        p.extend_from_slice(&VERSION.to_le_bytes());
        p.extend_from_slice(&(HEADER_LEN as u16).to_le_bytes());
        p.extend_from_slice(&source_sequence.to_le_bytes());
        p.extend_from_slice(&EXECUTABLE_SHA256);
        p.extend_from_slice(&CONSUMER_EVIDENCE_SHA256);
        p.extend_from_slice(&WRITER_EVIDENCE_SHA256);
        p.extend_from_slice(&IDENTITY_EVIDENCE_SHA256);
        assert_eq!(p.len(), HEADER_LEN, "header is {HEADER_LEN} bytes");
        for row in rows {
            p.extend_from_slice(row);
        }
        p
    }

    fn roster_of(ids: &[u32]) -> BTreeSet<u32> {
        ids.iter().copied().collect()
    }

    // --- Step 8.1: the schema validates. -------------------------------------

    #[test]
    fn the_schema_is_valid() {
        schema()
            .validate(grid().log2())
            .expect("d92 schema is well-formed");
    }

    // --- Step 8.5: shape and key. --------------------------------------------

    #[test]
    fn countries_is_a_keyed_set_keyed_by_the_country() {
        let c = countries();
        assert_eq!(c.shape, Shape::KeyedSet, "baseline replaces, deltas upsert");
        let keys = c.key_fields();
        assert_eq!(keys.len(), 1, "the fold keys rows by tag_db_id alone");
        assert_eq!(keys[0].field_id, field::TAG_DB_ID);
        assert_eq!(
            keys[0].kind,
            Kind::ContentId,
            "a country tag is content-defined (content-identity contract), so a manifest must resolve it"
        );
        assert!(keys[0].kind.legal_as_key());
        assert_eq!(c.fields.len(), 3, "1 key + database id + token");
    }

    #[test]
    fn the_frame_collection_is_a_singleton_keyed_set() {
        let c = frame_collection();
        assert_eq!(c.shape, Shape::KeyedSet);
        let keys = c.key_fields();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].field_id, field::FRAME_SINGLETON);
        assert_eq!(
            keys[0].kind,
            Kind::Unsigned,
            "the constant names nothing a content manifest resolves"
        );
        assert_eq!(c.fields.len(), 2, "1 constant key + source_sequence");
    }

    // --- Step 8.3: the states the schema declares, and only those. -----------

    #[test]
    fn the_only_declared_capability_is_nullable() {
        // The mapper may emit exactly the empty states the schema declares.
        // `Absent` would claim a country in the roster has no preferred-tactic
        // slot; `Unknown` would claim the value was not captured, which cannot
        // happen — a failed read never produces a row, it makes the whole frame
        // source-unavailable (`attach_log.rs:33075`-`:33081`). See the module
        // docs.
        let c = countries();
        assert_eq!(c.optional_count(), 0, "no field is ever inapplicable");
        assert!(
            c.value_fields().all(|f| !f.unknown_capable),
            "no field can be uncaptured on a row that exists"
        );
        let nullable: Vec<u32> = c
            .value_fields()
            .filter(|f| f.nullable)
            .map(|f| f.field_id)
            .collect();
        assert_eq!(
            nullable,
            vec![field::TACTIC_DATABASE_ID, field::TACTIC_ID],
            "both halves of the tactic reference are null together"
        );

        // The frame singleton declares nothing at all: the header always carries
        // the word.
        let f = frame_collection();
        assert_eq!(f.optional_count(), 0);
        assert!(f.value_fields().all(|x| !x.nullable && !x.unknown_capable));
    }

    #[test]
    fn a_selected_row_maps_to_both_content_ids() {
        let c = countries();
        let (_, rows) =
            split_payload(&payload_with(&[selected_row(7, 42, "encirclement")], 0, 0, 1))
                .expect("payload decodes");
        let r = record_from_row(&c, &rows[0], 0, &symbols()).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::TACTIC_DATABASE_ID)],
            Value::ContentId(42),
            "the engine database index, preserved as a raw integer"
        );
        assert_eq!(
            r.values[index_of(&c, field::TACTIC_ID)],
            Value::ContentId(1),
            "\"encirclement\" is index 1 of the sorted stand-in vocabulary"
        );
    }

    #[test]
    fn the_no_tactic_sentinel_maps_to_null_on_both_fields() {
        // The ONLY path on which the collector emits this row is the
        // authenticated `COMBAT_TACTIC_NONE_VTABLE_RVA` sentinel
        // (`attach_log.rs:32512`-`:32519`), so record-state contract's "the relation genuinely
        // points at nothing" is exactly right and `Unknown` would be a lie.
        let c = countries();
        let (_, rows) = split_payload(&payload_with(&[none_row(7)], 0, 0, 1)).expect("decodes");
        let r = record_from_row(&c, &rows[0], 0, &symbols()).expect("maps");
        assert_eq!(r.values[index_of(&c, field::TACTIC_DATABASE_ID)], Value::Null);
        assert_eq!(r.values[index_of(&c, field::TACTIC_ID)], Value::Null);
    }

    #[test]
    fn a_source_sequence_of_zero_is_a_value_not_an_empty_state() {
        // record-state contract keeps zero out of the three empty states. The country collection
        // has no field that can carry a real zero — a zero `tag_db_id` is
        // rejected (`preferred_tactic.rs:194`) and a zero `tactic_database_id`
        // IS the null sentinel — so the frame singleton is where this case lives.
        let f = frame_collection();
        let (header, _) = split_payload(&payload_with(&[none_row(7)], 0, 0, 0)).expect("decodes");
        let r = record_from_frame_header(&f, &header, 0).expect("maps");
        assert_eq!(
            r.values[index_of(&f, field::SOURCE_SEQUENCE)],
            Value::Unsigned(0)
        );
    }

    #[test]
    fn a_token_outside_the_content_set_is_refused_not_weakened() {
        let c = countries();
        let (_, rows) =
            split_payload(&payload_with(&[selected_row(7, 42, "sturmtruppen")], 0, 0, 1))
                .expect("decodes");
        let err = record_from_row(&c, &rows[0], 0, &symbols()).expect_err("refuses");
        assert!(err.contains("not in the symbol table"), "{err}");
        // Fail-closed against an empty table too, rather than emitting id 0.
        assert!(record_from_row(&c, &rows[0], 0, &Symbols::empty()).is_err());
    }

    #[test]
    fn the_mapper_enforces_the_row_shape_on_a_hand_built_row() {
        // `TacticRow`'s fields are public, so a caller can reach `record_from_row`
        // with a row that never came from a payload. the
        // reviewer's input was `(tag 1, id 0, Some("shock"))`, which without a
        // check publishes `Null` for the database id beside a real `ContentId`
        // for the token — a four-state pairing no payload the fold accepts can
        // produce. Swept over the whole shape space, not just that one input.
        let c = countries();
        let s = symbols();
        let legal: &[(u32, i32, Option<&str>)] = &[(1, 0, None), (1, 5, Some("shock"))];
        let candidates: &[(u32, i32, Option<&str>)] = &[
            (1, 0, None),             // legal none
            (1, 5, Some("shock")),    // legal selected
            (1, 0, Some("shock")),    // the reviewer's input: zero id, token
            (1, 5, None),             // positive id, no token
            (1, -1, None),            // negative id
            (1, -1, Some("shock")),   // negative id with a token
            (1, i32::MIN, None),      // the extreme negative
            (1, 5, Some("")),         // empty token
            (1, 5, Some("sho\u{1}ck")), // control character
            (0, 0, None),             // zero tag_db_id
            (0, 5, Some("shock")),    // zero tag_db_id, otherwise legal
        ];
        for (tag, id, token) in candidates {
            let row = TacticRow {
                tag_db_id: *tag,
                tactic_database_id: *id,
                tactic_id: token.map(str::to_string),
            };
            let is_legal = legal.contains(&(*tag, *id, *token));
            assert_eq!(
                record_from_row(&c, &row, 0, &s).is_ok(),
                is_legal,
                "hand-built row (tag {tag}, id {id}, token {token:?})"
            );
            // `check_row` is the single shared implementation, so it must agree
            // with the mapper on every one of these.
            assert_eq!(check_row(&row).is_ok(), is_legal);
        }
        // An over-long token is refused too — the same bound `split_payload` uses.
        let long = "a".repeat(MAX_TOKEN_BYTES + 1);
        let row = TacticRow {
            tag_db_id: 1,
            tactic_database_id: 5,
            tactic_id: Some(long),
        };
        assert!(check_row(&row).is_err());
        assert!(record_from_row(&c, &row, 0, &s).is_err());
    }

    #[test]
    fn a_record_built_against_the_wrong_collection_is_refused() {
        // The country mapper's field loop would otherwise fill the frame
        // collection's fields from a country row.
        let (header, rows) = split_payload(&payload_with(&[none_row(7)], 0, 0, 1)).expect("ok");
        assert!(record_from_row(&frame_collection(), &rows[0], 0, &symbols()).is_err());
        assert!(record_from_frame_header(&countries(), &header, 0).is_err());
    }

    // --- Step 8.2: byte-identical round trip. --------------------------------

    #[test]
    fn a_selected_row_roundtrips_byte_for_byte() {
        let c = countries();
        let (_, rows) =
            split_payload(&payload_with(&[selected_row(7, 42, "shock")], 0, 0, 1)).expect("ok");
        let record = record_from_row(&c, &rows[0], 0, &symbols()).expect("maps");
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
    fn a_null_row_roundtrips_byte_for_byte() {
        let c = countries();
        let (_, rows) = split_payload(&payload_with(&[none_row(7)], 0, 0, 1)).expect("ok");
        let record = record_from_row(&c, &rows[0], 0, &symbols()).expect("maps");
        let mut bytes = Vec::new();
        record.encode(&c, &mut bytes).expect("encodes");
        let (back, pos) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(pos, bytes.len());
        assert_eq!(back, record);
        let mut reencoded = Vec::new();
        back.encode(&c, &mut reencoded).expect("re-encodes");
        assert_eq!(reencoded, bytes);
    }

    #[test]
    fn a_whole_two_collection_state_roundtrips_through_a_baseline() {
        let schema = schema();
        let mut acc = State::new();
        let payload = payload_with(
            &[
                selected_row(3, 42, "shock"),
                none_row(9),
                selected_row(11, 7, "blitzkrieg"),
            ],
            123,
            0,
            5,
        );
        fold_payload(
            &schema,
            &mut acc,
            &payload,
            &roster_of(&[3, 9, 11]),
            true,
            0,
            &symbols(),
        )
        .expect("folds");
        let states = acc.as_slice();
        let mut bytes = Vec::new();
        encode_baseline(&schema, states, &mut bytes).expect("baseline encodes");
        let (back, consumed) = decode_baseline(&schema, &bytes, 0).expect("baseline decodes");
        assert_eq!(consumed, bytes.len());
        assert_eq!(back.len(), 2, "both collections survive");
        assert_eq!(back[0], states[0], "countries are stored-equal");
        assert_eq!(back[1], states[1], "the frame singleton is stored-equal");
        let mut reencoded = Vec::new();
        encode_baseline(&schema, &back, &mut reencoded).expect("re-encodes");
        assert_eq!(reencoded, bytes, "baseline round trip is byte-identical");
    }

    // --- Step 8.4: quantization. ---------------------------------------------

    #[test]
    fn no_field_is_continuous_so_quantization_does_not_arise() {
        // Step 4's "quantization saves bytes" test does not apply: d92 has no
        // `Continuous` field at all. Every value it carries is an integer
        // identity or a symbol id, and the fold applies no scale factor anywhere
        // (`world_fold.rs::fold_country_preferred_tactics` stores the decoded rows verbatim).
        for c in &schema().collections {
            for f in c.value_fields() {
                assert!(
                    !matches!(f.kind, Kind::Continuous),
                    "field {} is continuous",
                    f.field_id
                );
                // A non-continuous field carries `Quant::None` — quantization is
                // not merely unused here, it is inexpressible.
                assert_eq!(f.quant, Quant::None, "field {} is unquantized", f.field_id);
                assert!(!f.quant.is_quantized());
            }
        }
    }

    #[test]
    fn no_field_is_derived() {
        // `source_sequence` LOOKS clock-linear — the collector computes it as
        // `hour_index_from_raw(game_hour_raw) + 1` (`attach_log.rs:33335`) — but
        // the fold enforces nothing of the kind and derived-field contract's `u` is a container
        // decision. See the module docs and the deliverable's open questions.
        for c in &schema().collections {
            assert!(c.fields.iter().all(|f| f.derived.is_none()));
        }
    }

    // --- The codec-measurement contract ContentId retrofit sweep. -----------------------------------

    #[test]
    fn the_content_id_inventory_is_asserted_for_the_retrofit_sweep() {
        // "This domain has no ContentId" is a sweep result, not a silence — and
        // so is the inventory when it is non-empty. Every one of these changes
        // during the content-identity contract resolver retrofit. `TAG_DB_ID` is the heavier
        // class: a ContentId **key** sorts and delta-encodes by raw value, so a
        // remap reorders keys and changes the byte layout of every baseline.
        let schema = schema();
        let mut keys = Vec::new();
        let mut values = Vec::new();
        for c in &schema.collections {
            for f in &c.fields {
                if f.kind == Kind::ContentId {
                    if f.is_key() {
                        keys.push(f.field_id);
                    } else {
                        values.push(f.field_id);
                    }
                }
            }
        }
        assert_eq!(keys, vec![field::TAG_DB_ID], "one ContentId key");
        assert_eq!(
            values,
            vec![field::TACTIC_DATABASE_ID, field::TACTIC_ID],
            "two ContentId values: the raw-cast database index and the symbol id"
        );
    }

    // --- Step 8.6: every offset, header words included. ----------------------

    #[test]
    fn every_header_word_is_read_from_its_own_offset() {
        // A transposed header word mis-slices every row behind it, and no round
        // trip can see that — both arms agree on the wrong slice. Each word gets
        // a distinguishable value.
        let rows = [selected_row(3, 42, "shock"), none_row(9)];
        let payload = payload_raw(&rows, 0x1122_3344_5566_7788, 2, 1, 0, 0x0102_0304_0506_0708);
        let (header, decoded) = split_payload(&payload).expect("decodes");
        assert_eq!(header.read_time_us, 0x1122_3344_5566_7788, "u64 at +0");
        assert_eq!(header.row_count, 2, "u32 at +8");
        assert_eq!(header.selected_count, 1, "u32 at +12");
        assert_eq!(header.status_flags, 0, "u32 at +16");
        assert_eq!(header.source_sequence, 0x0102_0304_0506_0708, "u64 at +28");
        assert_eq!(decoded.len(), 2);
    }

    #[test]
    fn a_nonzero_status_word_is_read_from_its_own_offset() {
        // Separately from the word above, because the harness's exclusion
        // decision reads exactly this offset.
        let payload = payload_with(&[none_row(3)], 0, 0xDEAD_BEEF, 1);
        assert_eq!(
            split_payload(&payload).expect("decodes").0.status_flags,
            0xDEAD_BEEF
        );
    }

    #[test]
    fn every_row_word_is_read_from_its_own_offset() {
        // The row's four fixed words are adjacent and same-width in pairs, which
        // is the transposition class d62 was bitten by. Distinguishable values,
        // one assertion each.
        let payload = payload_with(&[selected_row(0x0A0B_0C0D, 0x1234_5678, "shock")], 0, 0, 1);
        let (_, rows) = split_payload(&payload).expect("decodes");
        assert_eq!(rows[0].tag_db_id, 0x0A0B_0C0D, "u32 at row +0");
        assert_eq!(rows[0].tactic_database_id, 0x1234_5678, "i32 at row +4");
        assert_eq!(
            rows[0].tactic_id.as_deref(),
            Some("shock"),
            "token at row +12"
        );
        // The key is built from `tag_db_id` and nothing else.
        assert_eq!(key_from_row(&rows[0]).expect("keys"), key(&[0x0A0B_0C0D]));
    }

    #[test]
    fn variable_length_rows_walk_to_the_right_boundaries() {
        // Rows are variable-length, so a wrong `token_len` read would silently
        // re-slice every row after it. Three different token lengths in a row.
        let payload = payload_with(
            &[
                selected_row(1, 5, "shock"),
                none_row(2),
                selected_row(3, 6, "trench_warfare"),
            ],
            0,
            0,
            1,
        );
        let (_, rows) = split_payload(&payload).expect("decodes");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].tactic_id.as_deref(), Some("shock"));
        assert_eq!(rows[1].tactic_id, None);
        assert_eq!(rows[2].tactic_id.as_deref(), Some("trench_warfare"));
        assert_eq!(rows[2].tag_db_id, 3, "the third row starts where it should");
    }

    // --- `split_payload` carries every check the codec carries. --------------

    #[test]
    fn a_short_payload_is_refused() {
        for len in [0usize, 1, HEADER_LEN - 1] {
            assert!(split_payload(&vec![0u8; len]).is_err(), "len {len}");
        }
    }

    #[test]
    fn a_bad_magic_version_or_header_length_is_refused() {
        let good = payload_with(&[none_row(1)], 0, 0, 1);
        for (offset, len) in [(20usize, 4usize), (24, 2), (26, 2)] {
            let mut bad = good.clone();
            for byte in &mut bad[offset..offset + len] {
                *byte ^= 0xFF;
            }
            assert!(
                split_payload(&bad).is_err(),
                "offset {offset} must be checked"
            );
        }
    }

    #[test]
    fn a_broken_authority_receipt_is_refused() {
        let good = payload_with(&[none_row(1)], 0, 0, 1);
        for offset in [36usize, 68, 100, 132] {
            let mut bad = good.clone();
            bad[offset] ^= 0xFF;
            assert!(
                split_payload(&bad).is_err(),
                "receipt at {offset} must be checked"
            );
        }
    }

    #[test]
    fn a_row_count_outside_bounds_is_refused() {
        // `row_count > MAX_ROWS` and `selected_count > row_count`, the codec's
        // two bounds (`preferred_tactic.rs:137`-`:139`). Neither may reach the
        // row walk, and the huge count must not allocate or overflow.
        let big = payload_raw(&[], 0, (MAX_ROWS + 1) as u32, 0, 0, 1);
        assert!(split_payload(&big).is_err());
        let huge = payload_raw(&[], 0, u32::MAX, 0, 0, 1);
        assert!(split_payload(&huge).is_err());
        let over_selected = payload_raw(&[none_row(1)], 0, 1, 2, 0, 1);
        assert!(split_payload(&over_selected).is_err());
    }

    #[test]
    fn a_selected_count_that_disagrees_with_the_rows_is_refused() {
        let honest = payload_raw(&[none_row(1), none_row(2)], 0, 2, 0, 0, 1);
        assert!(split_payload(&honest).is_ok(), "0 selected is the truth here");
        let bad = payload_raw(&[selected_row(1, 5, "shock"), none_row(2)], 0, 2, 0, 0, 1);
        let err = split_payload(&bad).expect_err("refuses");
        assert!(err.contains("selected count mismatch"), "{err}");
    }

    #[test]
    fn trailing_bytes_are_refused() {
        let mut bad = payload_with(&[none_row(1)], 0, 0, 1);
        bad.push(0);
        let err = split_payload(&bad).expect_err("refuses");
        assert!(err.contains("trailing bytes"), "{err}");
    }

    #[test]
    fn a_truncated_row_or_token_is_refused_not_panicked() {
        let good = payload_with(&[selected_row(1, 5, "shock")], 0, 0, 1);
        // Every prefix past the header is a truncation somewhere in the row.
        for cut in HEADER_LEN..good.len() {
            assert!(
                split_payload(&good[..cut]).is_err(),
                "prefix of {cut} bytes must be refused"
            );
        }
    }

    #[test]
    fn unknown_row_flags_and_an_oversized_token_are_refused() {
        let bad = payload_with(&[raw_row(1, 5, 1 << 5, b"shock")], 0, 0, 1);
        assert!(split_payload(&bad).is_err(), "unknown flag bit");
        let token = vec![b'a'; MAX_TOKEN_BYTES + 1];
        let bad = payload_with(&[raw_row(1, 5, KNOWN_FLAGS, &token)], 0, 0, 1);
        assert!(split_payload(&bad).is_err(), "token over {MAX_TOKEN_BYTES}");
    }

    #[test]
    fn the_none_selected_identity_invariant_is_enforced_in_every_direction() {
        // The codec's `:162`-`:169`, swept rather than sampled: every combination
        // of (id sign, flags, token presence) that is not one of the two legal
        // shapes must be refused.
        let legal: &[(i32, u16, &[u8])] = &[(0, 0, b""), (5, KNOWN_FLAGS, b"shock")];
        let candidates: &[(i32, u16, &[u8])] = &[
            (0, 0, b""),                       // legal none
            (5, KNOWN_FLAGS, b"shock"),        // legal selected
            (-1, 0, b""),                      // negative id
            (-1, KNOWN_FLAGS, b"shock"),       // negative id with a token
            (5, 0, b"shock"),                  // selected without flags
            (5, KNOWN_FLAGS, b""),             // selected without a token
            (5, FLAG_SELECTED, b"shock"),      // one flag only
            (5, FLAG_TOKEN_PRESENT, b"shock"), // the other flag only
            (0, KNOWN_FLAGS, b""),             // none carrying flags
            (0, 0, b"shock"),                  // none carrying a token
            (0, KNOWN_FLAGS, b"shock"),
        ];
        for (id, flags, token) in candidates {
            let payload = payload_raw(
                &[raw_row(1, *id, *flags, token)],
                0,
                1,
                u32::from(*id > 0),
                0,
                1,
            );
            let is_legal = legal.contains(&(*id, *flags, *token));
            assert_eq!(
                split_payload(&payload).is_ok(),
                is_legal,
                "row (id {id}, flags {flags:#x}, token {token:?})"
            );
        }
    }

    #[test]
    fn tags_must_be_unique_ascending_and_nonzero() {
        for rows in [
            vec![none_row(0)],              // zero tag
            vec![none_row(2), none_row(1)], // descending
            vec![none_row(2), none_row(2)], // duplicate
        ] {
            let payload = payload_raw(&rows, 0, rows.len() as u32, 0, 0, 1);
            assert!(split_payload(&payload).is_err(), "must be refused");
        }
        let ok = payload_with(&[none_row(1), none_row(2), none_row(300)], 0, 0, 1);
        assert!(split_payload(&ok).is_ok());
    }

    #[test]
    fn a_non_injective_id_token_mapping_is_refused_within_one_payload() {
        // The codec's guarantee is per-payload injectivity, and this is what
        // holds it (`preferred_tactic.rs:203`-`:209`). It is ALSO the reason both
        // fields are carried: nothing extends this to the next payload.
        let two_tokens_one_id = payload_with(
            &[selected_row(1, 5, "shock"), selected_row(2, 5, "blitzkrieg")],
            0,
            0,
            1,
        );
        assert!(split_payload(&two_tokens_one_id).is_err());
        let two_ids_one_token = payload_with(
            &[selected_row(1, 5, "shock"), selected_row(2, 6, "shock")],
            0,
            0,
            1,
        );
        assert!(split_payload(&two_ids_one_token).is_err());
        // The same (id, token) repeated across countries is legal and normal.
        let shared = payload_with(
            &[selected_row(1, 5, "shock"), selected_row(2, 5, "shock")],
            0,
            0,
            1,
        );
        assert!(split_payload(&shared).is_ok());
    }

    #[test]
    fn a_control_character_or_non_utf8_token_is_refused() {
        let bad = payload_with(&[raw_row(1, 5, KNOWN_FLAGS, b"sho\x01ck")], 0, 0, 1);
        assert!(split_payload(&bad).is_err(), "control character");
        let bad = payload_with(&[raw_row(1, 5, KNOWN_FLAGS, &[0xFF, 0xFE])], 0, 0, 1);
        assert!(split_payload(&bad).is_err(), "not UTF-8");
    }

    // --- The fold: full replace and the modelled D18 gate. -------------------

    #[test]
    fn the_fold_full_replaces_both_collections() {
        let schema = schema();
        let mut acc = State::new();
        fold_payload(
            &schema,
            &mut acc,
            &payload_with(&[selected_row(1, 5, "shock"), none_row(2)], 0, 0, 10),
            &roster_of(&[1, 2]),
            true,
            0,
            &symbols(),
        )
        .expect("folds");
        assert_eq!(acc.countries().len(), 2);
        // A different roster entirely: nothing of the prior hour survives.
        fold_payload(
            &schema,
            &mut acc,
            &payload_with(&[none_row(7)], 0, 0, 11),
            &roster_of(&[7]),
            true,
            0,
            &symbols(),
        )
        .expect("folds");
        assert_eq!(
            acc.countries().len(),
            1,
            "prior rows are discarded, not merged"
        );
        assert!(acc.countries().contains_key(&key(&[7])));
        let f = frame_collection();
        assert_eq!(
            acc.frame()[&key_from_frame()].values[index_of(&f, field::SOURCE_SEQUENCE)],
            Value::Unsigned(11),
            "the singleton carries the new hour's sequence"
        );
    }

    #[test]
    fn the_fold_refuses_an_hour_with_no_proven_d18_record() {
        let schema = schema();
        let mut acc = State::new();
        let err = fold_payload(
            &schema,
            &mut acc,
            &payload_with(&[none_row(1)], 0, 0, 1),
            &roster_of(&[1]),
            false,
            0,
            &symbols(),
        )
        .expect_err("refuses");
        assert!(err.contains("current-hour D18 identity roster"), "{err}");
    }

    #[test]
    fn the_roster_gate_is_containment_d18_inside_d92() {
        let schema = schema();
        let mut acc = State::new();
        // A d92 record that MISSES a D18 tag is refused: that tag is one the
        // projection can name (`world_fold.rs::project_country_preferred_tactics`) and would otherwise be
        // published as unavailable while the hour counted as measured.
        let err = fold_payload(
            &schema,
            &mut acc,
            &payload_with(&[none_row(1)], 0, 0, 1),
            &roster_of(&[1, 2]),
            true,
            0,
            &symbols(),
        )
        .expect_err("refuses a roster it does not cover");
        assert!(err.contains("does not cover current D18 roster"), "{err}");
        assert!(err.contains("1 of 2 D18 tags missing"), "{err}");
        assert!(err.contains("d92 carries 1"), "{err}");
        // A strict superset is the NORMAL shape and must fold: d92 reads the
        // complete country table, D18 the filtered cache.
        fold_payload(
            &schema,
            &mut acc,
            &payload_with(&[none_row(1), none_row(2)], 0, 0, 1),
            &roster_of(&[1]),
            true,
            0,
            &symbols(),
        )
        .expect("a superset covers the D18 roster");
        assert_eq!(
            acc.countries().len(),
            2,
            "the extra row is kept, not dropped: the D96 join validates the whole d92 row set"
        );
        // Same size, different membership — still a miss, still refused.
        assert!(fold_payload(
            &schema,
            &mut acc,
            &payload_with(&[none_row(1)], 0, 0, 1),
            &roster_of(&[2]),
            true,
            0,
            &symbols(),
        )
        .is_err());
    }

    /// The production shape, at scale. `runs/batch3/deepwar_c` carries a d92
    /// roster of 439 against a D18 roster of 356 in every one of its 896 hours —
    /// the same 83 extra ids each hour, none ever missing (measured off the
    /// capture; 71 of the extras sit above D18's highest id, 12 interleave, and
    /// exactly one of them carries a selected tactic). The equality gate refused
    /// all 896; containment folds all 896 and keeps every row.
    #[test]
    fn the_deepwar_c_superset_shape_folds_and_keeps_every_row() {
        let schema = schema();
        let mut acc = State::new();
        let d18: Vec<u32> = (1..=356).collect();
        // 356 D18 tags + 83 tags D18 filtered out; one of the extras is selected.
        let mut ids: Vec<u32> = d18.clone();
        ids.extend(357..=439);
        let rows: Vec<Vec<u8>> = ids
            .iter()
            .map(|&id| {
                if id == 400 {
                    selected_row(id, 7, "shock")
                } else {
                    none_row(id)
                }
            })
            .collect();
        fold_payload(
            &schema,
            &mut acc,
            &payload_with(&rows, 0, 0, 1),
            &roster_of(&d18),
            true,
            0,
            &symbols(),
        )
        .expect("439 rows cover a 356-tag D18 roster");
        assert_eq!(acc.countries().len(), 439, "no row is dropped");
        let c = schema.collection(COUNTRIES).expect("countries");
        assert_eq!(
            acc.countries()[&key(&[400u64])].values[index_of(&c, field::TACTIC_DATABASE_ID)],
            Value::ContentId(7),
            "the one selected extra keeps its tactic"
        );
        assert_eq!(
            acc.countries()[&key(&[439u64])].values[index_of(&c, field::TACTIC_DATABASE_ID)],
            Value::Null,
            "the highest extra id is stored as the authenticated none row"
        );
        // Drop a single D18 tag from the middle of the same 439-row shape and the
        // hour must be refused: that is the loss containment exists to catch.
        let short: Vec<Vec<u8>> = ids
            .iter()
            .filter(|&&id| id != 200)
            .map(|&id| none_row(id))
            .collect();
        let err = fold_payload(
            &schema,
            &mut acc,
            &payload_with(&short, 0, 0, 2),
            &roster_of(&d18),
            true,
            0,
            &symbols(),
        )
        .expect_err("a missing D18 tag is a loss");
        assert!(err.contains("1 of 356 D18 tags missing"), "{err}");
        assert!(err.contains("d92 carries 438"), "{err}");
    }

    #[test]
    fn a_refused_fold_clears_the_accumulator_as_the_recorder_does() {
        // The fold's error contract is NOT all-or-nothing: every gate refusal
        // sets `country_preferred_tactics_this_hour = None` before returning
        // (`world_fold.rs::fold_country_preferred_tactics`). A harness that assumed otherwise
        // would keep publishing last hour's tactics.
        let schema = schema();
        let mut acc = State::new();
        fold_payload(
            &schema,
            &mut acc,
            &payload_with(&[none_row(1)], 0, 0, 1),
            &roster_of(&[1]),
            true,
            0,
            &symbols(),
        )
        .expect("folds");
        assert_eq!(acc.countries().len(), 1);
        assert!(fold_payload(
            &schema,
            &mut acc,
            &payload_with(&[none_row(1)], 0, 0, 2),
            &roster_of(&[1, 2]),
            true,
            0,
            &symbols(),
        )
        .is_err());
        assert!(acc.countries().is_empty(), "state is dropped, not retained");
        assert!(acc.frame().is_empty());
    }

    #[test]
    fn a_malformed_payload_does_not_clear_the_accumulator() {
        // The decode runs BEFORE the gates (`world_fold.rs::fold_country_preferred_tactics`), and on that
        // path the fold's `?` returns before either clear. The recorder's own
        // Err arm then clears it (`world_fold.rs::apply_domain_data`), which is the caller's job,
        // not this boundary's — mirroring it here would clear twice and hide
        // which path fired.
        let schema = schema();
        let mut acc = State::new();
        fold_payload(
            &schema,
            &mut acc,
            &payload_with(&[none_row(1)], 0, 0, 1),
            &roster_of(&[1]),
            true,
            0,
            &symbols(),
        )
        .expect("folds");
        let mut bad = payload_with(&[none_row(1)], 0, 0, 2);
        bad.push(0);
        assert!(
            fold_payload(&schema, &mut acc, &bad, &roster_of(&[1]), true, 0, &symbols(),).is_err()
        );
        assert_eq!(
            acc.countries().len(),
            1,
            "the decode path leaves state alone"
        );
    }

    #[test]
    fn an_empty_roster_folds_to_an_empty_table() {
        // The fold accepts a zero-row payload against an empty roster: the sets
        // are equal. Mirroring the fold's permissiveness rather than inventing a
        // minimum.
        let schema = schema();
        let mut acc = State::new();
        fold_payload(
            &schema,
            &mut acc,
            &payload_with(&[], 0, 0, 3),
            &BTreeSet::new(),
            true,
            0,
            &symbols(),
        )
        .expect("folds");
        assert!(acc.countries().is_empty());
        assert_eq!(acc.frame().len(), 1, "the singleton is still emitted");
    }

    #[test]
    fn the_vocabulary_helper_agrees_with_the_rows() {
        let payload = payload_with(
            &[
                selected_row(1, 5, "shock"),
                none_row(2),
                selected_row(3, 6, "blitzkrieg"),
            ],
            0,
            0,
            1,
        );
        assert_eq!(
            tokens_in_payload(&payload).expect("walks"),
            vec!["shock".to_string(), "blitzkrieg".to_string()],
            "only selected rows carry a token"
        );
    }
}
