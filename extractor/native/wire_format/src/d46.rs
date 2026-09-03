//! d46 `air_variant_archetypes` — the per-variant air archetype/classification
//! table, plus the `ACE1` air-ace-roster trailer that ships inside the same
//! record.
//!
//! Derived from `world_fold.rs:25949`-`:26171`
//! (`fold_air_variant_archetypes`, dispatched at `:4723`), the row grammar and
// @world-fold-authority domain=46 functions=fold_air_variant_archetypes lines=25949-26171 sha256=909606bb88fb11fd6dcb23ab321bc0929e9f5da384ae20a39d44b2e4c183797f closure=fe61f7b276b5e1bbbfdafe2f0efcb1c1c4572efd26567c6b77d674c4e07ac3f1
//! bounds in
//! `extractor/native/lanes/air_wings_variants_lifecycle_live/src/domain46_air_variant_archetypes_live.rs`
//! (authority #2 — production's status-independent payload check), the trailer
//! codec in the same lane's `domain46_air_ace_roster_live.rs`, and the collector
//! read that produces the flags,
//! `extractor/native/collector/src/attach_log.rs:53106`-`:53200`,
//! `:53757`-`:53784`, `:57497`-`:57564`. There is no
//! `documents/domains/d46_*.md` dossier.
//!
//! **Outcome: SPLIT.** The archetype table is a keyed set and is ported and
//! measured. The `ACE1` trailer is a **stop condition** and is reported, not
//! forced into a keyed set — see "The stop" below. Collection id 2 is reserved
//! for it.
//!
//! # Payload grammar
//!
//! A 48-byte header (`world_fold.rs:1011`,
//! `domain46_air_variant_archetypes_live.rs:5`):
//!
//! | offset | width | word | read by the fold? |
//! |---|---|---|---|
//! | `+0`  | u64 | `elapsed_wall_ms`        | no |
//! | `+8`  | u32 | `row_count`             | yes (`:16325`) |
//! | `+12` | u32 | `changed_rows`          | **no** — parsed and reported, never enforced |
//! | `+16` | u32 | `status_flags`          | yes (`:16326`) |
//! | `+20` | u32 | `observed_variant_rows` | **no** |
//! | `+24` | u32 | `classified_rows`       | yes (`:16327`) |
//! | `+28` | u32 | `unresolved_rows`       | yes (`:16328`) |
//! | `+32` | u32 | `static_catalog_hash`   | yes (`:16329`), stamped into every stored row (`:16531`) |
//! | `+36` | u32 | `fixed_scale_raw`       | yes (`:16330`), stamped into every stored row (`:16532`) |
//! | `+40` | u32 | `reserved0`             | yes, must be 0 (`:16339`) |
//! | `+44` | u32 | `reserved1`             | yes, must be 0 (`:16339`) |
//!
//! then `row_count` fixed 32-byte rows:
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+0`  | u32 | `variant_id`                  | raw | none (key; zero rejected `:16356`) |
//! | `+4`  | u32 | `flags`                       | —   | — |
//! | `+8`  | u32 | `equipment_token_raw`         | raw | none |
//! | `+12` | u32 | `equipment_type_token_raw`    | raw | none |
//! | `+16` | u32 | `interface_type_raw`          | raw | none |
//! | `+20` | **i32** | `air_superiority_weight_raw` | header `fixed_scale_raw` | `1<<4` (`:16519`) |
//! | `+24` | u32 | `source_mask`                 | —   | — |
//! | `+28` | u32 | `reserved`                    | —   | must be 0 (`:16364`) |
//!
//! then a **variable-length region**: three length-prefixed (u16) UTF-8 strings
//! per row, in row order — `archetype_token`, `base_airframe_token`, `role_token`
//! (`:16382`-`:16402`) — and then, if any bytes remain, the `ACE1` trailer
//! (`:16493`-`:16500`). The base payload is self-delimiting, which is what makes
//! the trailer's presence unambiguous.
//!
//! # Shape: recipe shape 4, implicit-baseline variant
//!
//! `implicit_delta_baseline_or_sparse` (`world_fold.rs::implicit_delta_baseline_or_sparse`, called at
//! `:16503`) returns `false` the first time the domain is folded and `true` ever
//! after: the FIRST folded frame replaces the whole table, every later frame is a
//! sparse upsert/delete patch. That is the recipe's shape 4
//! (replace-or-sparse-after-first, d31/d34/d36) and **not a sixth shape** — with
//! one difference worth naming, because it makes d46 the purest instance of the
//! shape rather than a different one: d31/d34/d36 call
//! `replace_or_sparse_after_first` (`world_fold.rs::replace_or_sparse_after_first`), which consults
//! `COMPACT_STATUS_DELTA_DATA` and the expected-partial tolerance table, so a
//! first frame that sets `DELTA_DATA` is already sparse there. d46's helper
//! consults **neither**, so "nothing in the payload says so" is literally true:
//! the lane always sets `DELTA_DATA` (`extractor.rs:100`), and the mode word is
//! never read by this fold at all.
//!
//! Two consequences the harness depends on:
//!
//! * There is **no delta-before-baseline exit**. The only non-folding exit is the
//!   failure status (`:16342`-`:16348`), so this domain has exactly two outcomes
//!   beside a stream rejection: a folded hour and an excluded hour.
//! * A failure-status record does **not** consume the implicit baseline, because
//!   the fold returns before `implicit_delta_baseline_or_sparse` is called. The
//!   first *folded* frame is the baseline, whichever hour that is.
//!
//! Deletes are real: `ROW_FLAG_DELETED_LOW_BIT` removes the key (`:16508`-`:16510`)
//! and the collector emits a tombstone for every variant that disappears
//! (`attach_log.rs:53142`-`:53157`). A deleted row must carry **zero**
//! identity/classification data — every numeric field, the source mask and all
//! three strings (`:16403`-`:16418`) — which this module mirrors as a refusal
//! rather than assuming.
//!
//! # Four-state semantics
//!
//! The fold answers only null-vs-value; the collector read decides. Every gate
//! here is clear because a *read* did not resolve, never because the concept does
//! not apply to an air variant, so **every gated field is `unknown`** and nothing
//! is `Absent` or `Null`:
//!
//! * `archetype_token` (`ROW_FLAG_ARCHETYPE`): the collector resolves the
//!   equipment token's name out of a heap-scanned token dictionary and sets the
//!   flag only when the lookup succeeded
//!   (`extractor.rs:249`-`:255`, `attach_log.rs:53125`-`:53131`). A clear gate is
//!   "the dictionary had no entry", i.e. applicable and not captured — record-state contract's
//!   **unknown**. The lane's own comment is the same fail-closed rule:
//!   "Missing dictionary entries therefore remain unresolved" (`extractor.rs:270`-`:275`).
//! * `base_airframe_token`, `role_token` (`1<<2`, `1<<3`): the static-catalog
//!   classification is never emitted by today's producer
//!   (`close_or_badge_equipment_token` hard-codes both to `None`,
//!   `extractor.rs:276`-`:282`) because "Static catalog enrichments may be added
//!   only by a producer that also supplies its admitted catalog hash". Every
//!   airframe has a base airframe and a role, so the concept applies to every row;
//!   it was simply not captured — **unknown**.
//! * `air_superiority_weight_raw` (`1<<4`): the collector hard-codes `None`
//!   (`attach_log.rs:53768`); the field is producer-reserved. Applicable, not
//!   captured — **unknown**.
//!
//! There is no `.optional()` and no `.nullable()` field in this schema, so the
//! presence and null bitmaps are zero bytes wide; only the unknown bitmap is paid
//! for. The states test records that explicitly.
//!
//! # `fixed_scale_raw` is carried, never applied — and never hard-coded
//!
//! The batch-3 survey flags that the fold does **not** assert the header's
//! `fixed_scale_raw` against `FIXED_SCALE`; it only rejects zero (`:16336`) and
//! then stamps the word into every row it stores (`:16532`), where the fold's own
//! projection divides by it per row (`:24437`-`:24439`). Production requires more
//! than the fold does: the status-independent payload authority rejects any
//! `fixed_scale_raw != 100_000` on **every** DATA record
//! (`domain46_air_variant_archetypes_live.rs:172`-`:176`), which is d49's
//! authority-is-stricter-than-the-fold shape.
//!
//! This module mirrors both, in the two places they belong. [`peek_header`] and
//! [`split_payload`] enforce exactly the fold's `!= 0`; [`check_authority_payload`]
//! enforces the authority's `== 100_000`. And the divisor is **carried, not
//! applied**: `air_superiority_weight_raw` goes on the wire as the raw `i64` the
//! fold stores, beside `fixed_scale_raw` as its own field — d61's rule ("carry RAW
//! values plus scale metadata, never a chosen projection"), which here is also
//! forced by stored-equality, since the fold stores both words per row. So this
//! schema has **no continuous field at all** and quantization does not arise.
//!
//! # `source_mask` is decomposed; four of its seven bits are not fields
//!
//! The fold stores the whole `source_mask` word, but four of its seven defined
//! bits are hard-enforced by the accepted grammar and are therefore not data
//! (the d62/d24 rule):
//!
//! * `SOURCE_LIVE_STATS` is required on every non-deleted row (`:16420`-`:16424`)
//!   — constant `1` on every row that becomes a record.
//! * `SOURCE_STATIC_HEURISTIC` and `SOURCE_DEV_SAVE_JOIN_REJECTED` are refused
//!   (`:16425`-`:16434`) — constant `0`.
//! * `SOURCE_STATIC_SOURCE_ENUM` is exactly
//!   `base_airframe_token.is_some() || role_token.is_some()`. The fold's two
//!   checks at `:16462`-`:16487` are an if-and-only-if: carrying static semantics
//!   requires the bit, and the bit requires carrying static semantics. A pure
//!   function of two carried fields, so it is enforced and left off the wire.
//!
//! The remaining three — `SOURCE_LIVE_DEFINITION`, `SOURCE_STATIC_CATALOG`,
//! `SOURCE_LIVE_TOKEN_DICTIONARY` — are constrained but not determined, so they
//! are `Boolean` fields.
//!
//! **One recorded divergence.** Decomposing means bits `7..=31` have nowhere to
//! go, and the FOLD does not reject them (it checks only the three rules above),
//! so [`record_from_row`] **refuses** a row whose `source_mask` carries an
//! undefined bit rather than dropping it. The three things the recipe requires of
//! a divergence: the shape limit is that the schema has one field per defined bit
//! and no field for an undefined one; the producer evidence is that production's
//! status-independent payload authority refuses exactly that payload on every DATA
//! record (`domain46_air_variant_archetypes_live.rs:257`-`:261`), so no stream the
//! recorder certifies can contain one; and
//! `an_undefined_source_mask_bit_is_refused_rather_than_dropped` pins it.
//!
//! # Kinds: the three raw words are `Unsigned`, the three tokens are `ContentId`
//!
//! `equipment_token_raw`, `equipment_type_token_raw` and `interface_type_raw` are
//! the SAME three words d35 and d38 carry, and this module makes the same call for
//! the same reason (`d35.rs:145`-`:159`, `d38.rs:348`-`:356`): to turn
//! `equipment_token_raw` into a name at all the collector **scans the live process
//! heap** for token records (`refresh_compact_air_variant_token_dictionary`,
//! `attach_log.rs:53822`-`:53852`, with an `ambiguous_tokens` set for the ones it
//! cannot pin down). A number resolvable only by walking the heap of the process
//! that produced it is a process-local handle, not a reference a manifest resolves.
//! `Unsigned`. `variant_id` is the same runtime instance identity d35, d38, d39 and
//! d40 already class `Unsigned` (`variant_upgrades.rs:262`-`:268`).
//!
//! The **strings** are the other half of that same story and go the other way:
//! `archetype_token` is the content-defined name the process-local handle points
//! at (`small_plane_cas_airframe_2`), and `base_airframe_token`/`role_token` are
//! catalog classification tokens. content-identity contract settles it — "the stream carries manifest ids
//! as varints, never text" — so all three are `ContentId` resolved through a
//! caller-supplied [`Symbols`] table, d29's device. d46 therefore has ContentId
//! **values** and no ContentId **key**, which is the lighter codec-measurement contract retrofit class.
//!
//! # The stop: the `ACE1` ace roster
//!
//! The trailer carries one row per living-or-dead flying ace, and its `name`,
//! `surname` and `callsign` are **runtime-generated per-ace identities**
//! (`domain46_air_ace_roster_live.rs:127`-`:141`; the codec's own sample is
//! `Tobias` / `Ochsenhirt` / `Fighter of Libau`, `:848`-`:851`). No content
//! manifest resolves *that ace's* name: the strings are drawn and composed at
//! runtime per ace instance, so the recipe's Scope applies exactly as it does to
//! d11's player-typed division name — text no manifest can ever resolve stops the
//! collection carrying it. This is not the step-3 case: the content-identity contract symbol rule is for a
//! vocabulary that comes from mod content, and an ace's identity is not in any
//! catalog. (The archetype table's own three tokens pass that test and are
//! ported — the split is between the two collections, not between two readings of
//! one field.)
//!
//! **A second, independent blocker**, the d17 shape, which would stop the
//! collection even with a symbol id. The roster is FULL-REPLACE with
//! retain-on-incomplete: `apply_air_ace_roster` (`world_fold.rs::apply_air_ace_roster`)
//! replaces the whole map on a clean frame, but on `SOURCE_UNAVAILABLE` or
//! `PARTIAL_DATA` it **retains prior state and marks the domain problem**, because
//! "a read failure must never be written into the replay as ground-truth absence"
//! (`:16546`-`:16554`). A keyed set cannot say "this collection's membership was
//! not measured this hour" — that requires a capture gap at a
//! `(domain, part)` scope, and gap contract has no implementation at all
//! (`lib.rs` module docs). Encoding a retained hour as an unchanged keyed set would
//! assert measured continuity across an interval nobody measured.
//!
//! **The trailer is still fully validated here, at the position production
//! validates it — and that position is two different positions.**
//!
//! * The **fold** parses it at `:16493`-`:16500`, which is *after* the failure
//!   gate at `:16342` and *after* every row and authority check. So on a
//!   failure-status hour the fold never looks at the trailer.
//! * The **payload authority** — `air_wings_variants_lifecycle_live_lane::decode_payload`,
//!   reached from `decode_compact_air_variant_archetypes_data_payload`
//!   (`recorder_host/src/domain_data_authority.rs:7906`-`:7930`, dispatched at
//!   `:16966`) — walks the header, every row, every string **and the whole
//!   trailer** (`domain46_air_variant_archetypes_live.rs:418`-`:421`) on every
//!   DOMAIN_DATA record whatever its status, and any parse error it reports makes
//!   the record `malformed_payload_records`, which the strict certification
//!   verdict refuses.
//!
//! So a malformed trailer refuses the stream **even on a failure-status hour** —
//! d52's lesson — while the fold's own copy is unreachable there.
//! [`decode_payload`] is the fold's position and [`check_authority_payload`] the
//! authority's; the measurement bin runs the second on every record and the first
//! only on the records the fold folds. Everything the stop hands forward — offsets,
//! versions, bounds, kinds, the retain semantics — is in [`ace`] beside the
//! validator, so implementing gap contract plus a symbol/free-text answer does not have to
//! rediscover it.
//!
//! # No derived field (derived-field contract)
//!
//! Nothing here changes deterministically with the clock; the mapper takes
//! `frame_tick` and ignores it so every domain's boundary has one shape.
//!
//! # Transcription, and its limit
//!
//! The row grammar and both authorities live in a capture-lane crate, which
//! `wire_format` must not depend on (the recipe's dependency ruling gives it
//! `compact_contract` only), so every constant and rule below is transcribed by
//! hand and cited to its original line. d56 is in the same position with
//! `domain56_compact.rs` and does the same. Unlike d67/d70 — whose originals live
//! in `compact_contract`, which the bins may call — there is **no dual-decode
//! cross-check available for this domain**; the transcription is held only by
//! citation and by the tests below. Recorded as an open question, not papered over.

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 46;

/// The ported collection: one row per air equipment variant.
pub const ARCHETYPES: u32 = 1;
/// Reserved for the stop-reported `ACE1` ace roster. No collection carries this
/// id today; it exists so the id is not reused for something else before the
/// blockers in the module docs are answered.
pub const ACE_ROSTER: u32 = 2;

/// Archetype field ids. Stable and append-only; schema-identity contract forbids reuse.
pub mod field {
    pub const VARIANT_ID: u32 = 1;
    pub const EQUIPMENT_TOKEN_RAW: u32 = 2;
    pub const EQUIPMENT_TYPE_TOKEN_RAW: u32 = 3;
    pub const INTERFACE_TYPE_RAW: u32 = 4;
    pub const AIR_SUPERIORITY_WEIGHT_RAW: u32 = 5;
    pub const ARCHETYPE_TOKEN: u32 = 6;
    pub const BASE_AIRFRAME_TOKEN: u32 = 7;
    pub const ROLE_TOKEN: u32 = 8;
    pub const LIVE_DEFINITION: u32 = 9;
    pub const STATIC_CATALOG: u32 = 10;
    pub const LIVE_TOKEN_DICTIONARY: u32 = 11;
    pub const STATIC_CATALOG_HASH: u32 = 12;
    pub const FIXED_SCALE_RAW: u32 = 13;
}

// --- Payload grammar constants ----------------------------------------------

/// `AIR_VARIANT_ARCHETYPE_HEADER_LEN`, `world_fold.rs::AIR_VARIANT_ARCHETYPE_HEADER_LEN`;
/// `HEADER_LEN`, `domain46_air_variant_archetypes_live.rs:5`.
pub const HEADER_LEN: usize = 48;
/// `AIR_VARIANT_ARCHETYPE_ROW_LEN`, `world_fold.rs::AIR_VARIANT_ARCHETYPE_ROW_LEN`; the lane's `ROW_LEN`
/// (`:6`). There is no legacy short row.
pub const ROW_LEN: usize = 32;

/// `ROW_FLAG_DELETED_LOW_BIT`, `world_fold.rs::ROW_FLAG_DELETED_LOW_BIT`.
pub const ROW_FLAG_DELETED: u32 = 1;
/// `AIR_VARIANT_ARCHETYPE_ROW_FLAG_ARCHETYPE`, `world_fold.rs::AIR_VARIANT_ARCHETYPE_ROW_FLAG_ARCHETYPE`.
pub const ROW_FLAG_ARCHETYPE: u32 = 1 << 1;
/// `world_fold.rs:1070`.
pub const ROW_FLAG_BASE_AIRFRAME: u32 = 1 << 2;
/// `world_fold.rs:1071`.
pub const ROW_FLAG_ROLE: u32 = 1 << 3;
/// `world_fold.rs:1072`.
pub const ROW_FLAG_AIR_SUPERIORITY_WEIGHT: u32 = 1 << 4;
/// `AIR_VARIANT_ARCHETYPE_KNOWN_FLAGS`, `world_fold.rs::AIR_VARIANT_ARCHETYPE_KNOWN_FLAGS`.
pub const KNOWN_ROW_FLAGS: u32 = ROW_FLAG_DELETED
    | ROW_FLAG_ARCHETYPE
    | ROW_FLAG_BASE_AIRFRAME
    | ROW_FLAG_ROLE
    | ROW_FLAG_AIR_SUPERIORITY_WEIGHT;

/// `SOURCE_LIVE_STATS`, `domain46_air_variant_archetypes_live.rs:17`. Required on
/// every non-deleted row (`world_fold.rs::fold_air_variant_archetypes`), so it is a constant of the
/// accepted grammar and not a field.
pub const SOURCE_LIVE_STATS: u32 = 1;
/// `:18`.
pub const SOURCE_LIVE_DEFINITION: u32 = 1 << 1;
/// `:19`.
pub const SOURCE_STATIC_CATALOG: u32 = 1 << 2;
/// `:20`. Refused by the fold (`world_fold.rs::fold_air_variant_archetypes`), so constant `0`.
pub const SOURCE_DEV_SAVE_JOIN_REJECTED: u32 = 1 << 3;
/// `:21`. Refused by the fold, so constant `0`.
pub const SOURCE_STATIC_HEURISTIC: u32 = 1 << 4;
/// `:22`.
pub const SOURCE_LIVE_TOKEN_DICTIONARY: u32 = 1 << 5;
/// `:23`. Equal to "carries static semantics" on every payload the fold accepts —
/// see the module docs — so it is enforced, not carried.
pub const SOURCE_STATIC_SOURCE_ENUM: u32 = 1 << 6;
/// `KNOWN_SOURCE_MASK`, `:37`-`:44`.
pub const KNOWN_SOURCE_MASK: u32 = SOURCE_LIVE_STATS
    | SOURCE_LIVE_DEFINITION
    | SOURCE_STATIC_CATALOG
    | SOURCE_DEV_SAVE_JOIN_REJECTED
    | SOURCE_STATIC_HEURISTIC
    | SOURCE_LIVE_TOKEN_DICTIONARY
    | SOURCE_STATIC_SOURCE_ENUM;

/// The string prefix the fold refuses as a fabricated identity
/// (`world_fold.rs::fold_air_variant_archetypes`).
pub const FABRICATED_TOKEN_PREFIX: &str = "equipment_token_raw_";

/// `COMPACT_STATUS_DELTA_DATA`. Named here because d46 is not admitted to the
/// production-codec dependency surface; the measurement bin asserts the local
/// value and shipping contract agree (the d24/d32/d56 pattern). **This fold
/// never reads it** — see the shape section of the module docs; it is defined
/// only so the bin can report the word and pin that irrelevance with a test.
pub const STATUS_DELTA_DATA: u32 = 1 << 10;

// --- Bounds the payload AUTHORITY enforces (not the fold) --------------------

/// `FIXED_SCALE_RAW`, `domain46_air_variant_archetypes_live.rs:7`. The authority
/// requires the header word to equal this on every DATA record (`:172`-`:176`);
/// the FOLD requires only that it is non-zero (`world_fold.rs::fold_air_variant_archetypes`). Both are
/// mirrored, in [`check_authority_payload`] and [`peek_header`] respectively.
pub const AUTHORITY_FIXED_SCALE_RAW: u32 = 100_000;
/// `MAX_VARIANT_ID`, `:25`.
pub const MAX_VARIANT_ID: u32 = i32::MAX as u32;
/// `MAX_EQUIPMENT_TOKEN_RAW`, `:26`.
pub const MAX_EQUIPMENT_TOKEN_RAW: u32 = 1_000_000;
/// `MAX_EQUIPMENT_TYPE_TOKEN_RAW`, `:27`.
pub const MAX_EQUIPMENT_TYPE_TOKEN_RAW: u32 = 1_000_000;
/// `MAX_INTERFACE_TYPE_RAW`, `:28`.
pub const MAX_INTERFACE_TYPE_RAW: u32 = 1_000_000;
/// `MAX_STRING_LEN`, `:29`.
pub const MAX_TOKEN_LEN: usize = 160;
/// `MAX_AIR_SUPERIORITY_WEIGHT_RAW`, `:30` — `10 * FIXED_SCALE_RAW`.
pub const MAX_AIR_SUPERIORITY_WEIGHT_RAW: i32 = 10 * AUTHORITY_FIXED_SCALE_RAW as i32;

// --- The stop's hand-forward: the `ACE1` trailer ------------------------------

/// Everything the stop-reported ace roster hands forward, kept beside the
/// validator that enforces it: constants, byte offsets, bounds and the kinds a
/// future port should use. Nothing here is mapped into a record — the collection
/// stops (module docs) — but all of it is *checked*, because production checks it
/// on every DATA record whatever its status.
///
/// Transcribed from
/// `extractor/native/lanes/air_wings_variants_lifecycle_live/src/domain46_air_ace_roster_live.rs`.
pub mod ace {
    /// `ACE_TRAILER_MAGIC`, `:19`.
    pub const TRAILER_MAGIC: [u8; 4] = *b"ACE1";
    /// `ACE_TRAILER_VERSION`, `:20` — the current wire version.
    pub const TRAILER_VERSION_V2: u16 = 2;
    /// `ACE_TRAILER_VERSION_V1`, `:21` — the historical alive-only shape.
    pub const TRAILER_VERSION_V1: u16 = 1;
    /// `ACE_TRAILER_HEADER_LEN`, `:28`.
    pub const TRAILER_HEADER_LEN: usize = 40;
    /// `ACE_TRAILER_ROW_LEN`, `:29`.
    pub const TRAILER_ROW_LEN_V2: usize = 40;
    /// `ACE_TRAILER_ROW_LEN_V1`, `:30`.
    pub const TRAILER_ROW_LEN_V1: usize = 32;

    /// `ACE_STATUS_SOURCE_UNAVAILABLE`, `:36`. The fold RETAINS prior ace state
    /// on this (`world_fold.rs::apply_air_ace_roster`).
    pub const STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
    /// `ACE_STATUS_PARTIAL_DATA`, `:37`. The fold RETAINS on this too
    /// (`world_fold.rs::apply_air_ace_roster`).
    pub const STATUS_PARTIAL_DATA: u32 = 1 << 9;
    /// `ACE_STATUS_WING_ASSIGNMENT_BLOCKED`, `:38`. Assignment-only anomaly; the
    /// frame still REPLACES (`world_fold.rs:16555`-`:16557`).
    pub const STATUS_WING_ASSIGNMENT_BLOCKED: u32 = 1 << 13;
    /// `ACE_KNOWN_STATUS_MASK`, `:39`-`:40`.
    pub const KNOWN_STATUS_MASK: u32 =
        STATUS_SOURCE_UNAVAILABLE | STATUS_PARTIAL_DATA | STATUS_WING_ASSIGNMENT_BLOCKED;

    /// Row flags, `:42`-`:64`.
    pub const ROW_FLAG_NAME: u32 = 1 << 1;
    pub const ROW_FLAG_SURNAME: u32 = 1 << 2;
    pub const ROW_FLAG_CALLSIGN: u32 = 1 << 3;
    pub const ROW_FLAG_WING_ID: u32 = 1 << 4;
    pub const ROW_FLAG_ALIVE_KNOWN: u32 = 1 << 5;
    pub const ROW_FLAG_DEAD: u32 = 1 << 6;
    pub const ROW_FLAG_KILLER_ACE: u32 = 1 << 7;
    pub const ROW_FLAG_KILLER_COUNTRY: u32 = 1 << 8;
    pub const ROW_FLAG_HANDLED_KNOWN: u32 = 1 << 9;
    pub const ROW_FLAG_HANDLED: u32 = 1 << 10;
    pub const ROW_FLAG_KILL_TYPE_KNOWN: u32 = 1 << 11;
    /// `KNOWN_ROW_FLAGS`, `:83`-`:93`.
    pub const KNOWN_ROW_FLAGS_V2: u32 = ROW_FLAG_NAME
        | ROW_FLAG_SURNAME
        | ROW_FLAG_CALLSIGN
        | ROW_FLAG_WING_ID
        | ROW_FLAG_ALIVE_KNOWN
        | ROW_FLAG_DEAD
        | ROW_FLAG_KILLER_ACE
        | ROW_FLAG_KILLER_COUNTRY
        | ROW_FLAG_HANDLED_KNOWN
        | ROW_FLAG_HANDLED
        | ROW_FLAG_KILL_TYPE_KNOWN;
    /// `KNOWN_ROW_FLAGS_V1`, `:94`-`:101`.
    pub const KNOWN_ROW_FLAGS_V1: u32 = ROW_FLAG_NAME
        | ROW_FLAG_SURNAME
        | ROW_FLAG_CALLSIGN
        | ROW_FLAG_WING_ID
        | ROW_FLAG_ALIVE_KNOWN
        | ROW_FLAG_DEAD
        | ROW_FLAG_KILLER_ACE
        | ROW_FLAG_KILLER_COUNTRY;

    /// Source mask, `:66`-`:72`.
    pub const SOURCE_LIVE_COUNTRY_ACE_VECTOR: u32 = 1;
    pub const SOURCE_LIVE_ACE_OBJECT_LAYOUT: u32 = 1 << 1;
    pub const SOURCE_DEV_SAVE_LAYOUT_VALIDATION: u32 = 1 << 2;
    pub const SOURCE_WING_ASSIGNMENT_SCAN_BLOCKED: u32 = 1 << 3;
    pub const SOURCE_LIVE_WING_ACE_ID_ASSIGNMENT: u32 = 1 << 4;
    pub const SOURCE_LIVE_ACE_DEATH_RECORD: u32 = 1 << 5;
    pub const SOURCE_LIVE_ACE_LIFECYCLE_RECORD: u32 = 1 << 6;
    /// `KNOWN_SOURCE_MASK`, `:102`-`:108`.
    pub const KNOWN_SOURCE_MASK_V2: u32 = SOURCE_LIVE_COUNTRY_ACE_VECTOR
        | SOURCE_LIVE_ACE_OBJECT_LAYOUT
        | SOURCE_DEV_SAVE_LAYOUT_VALIDATION
        | SOURCE_WING_ASSIGNMENT_SCAN_BLOCKED
        | SOURCE_LIVE_WING_ACE_ID_ASSIGNMENT
        | SOURCE_LIVE_ACE_DEATH_RECORD
        | SOURCE_LIVE_ACE_LIFECYCLE_RECORD;
    /// `KNOWN_SOURCE_MASK_V1`, `:109`-`:114` — no lifecycle-record bit.
    pub const KNOWN_SOURCE_MASK_V1: u32 = SOURCE_LIVE_COUNTRY_ACE_VECTOR
        | SOURCE_LIVE_ACE_OBJECT_LAYOUT
        | SOURCE_DEV_SAVE_LAYOUT_VALIDATION
        | SOURCE_WING_ASSIGNMENT_SCAN_BLOCKED
        | SOURCE_LIVE_WING_ACE_ID_ASSIGNMENT
        | SOURCE_LIVE_ACE_DEATH_RECORD;

    /// Bounds, `:74`-`:81`.
    /// Was `4096`, a guess at the vanilla-plus-DLC country count.
    /// ENGINE-STRUCTURAL now: `id < *(int32*)(db + 0x0C)` with a signed-`int`
    /// size gives `i32::MAX - 1` — the same derivation `MAX_ACE_ID` and
    /// `MAX_WING_ID` beside it already use for their own engine id spaces.
    /// Pinned by `tests/wire_format_suite/producer_consumer_admission_agreement.rs`.
    pub const MAX_TAG_DB_ID: u32 = (i32::MAX as u32) - 1;
    pub const MAX_ACE_ID: u32 = i32::MAX as u32;
    pub const MAX_COUNTRY_ACE_INDEX: u32 = 4096;
    pub const MAX_WING_ID: u32 = i32::MAX as u32;
    /// cp1252 capped at 96 bytes at the read layer, times the 3-byte worst case
    /// of cp1252 -> UTF-8 (`:78`-`:80`).
    pub const MAX_STRING_LEN: usize = 96 * 3;
    pub const MAX_ACE_ROWS: usize = 65_536;

    /// Trailer header word offsets, relative to the trailer's first byte
    /// (`:287`-`:309`).
    pub mod header_offset {
        pub const MAGIC: usize = 0;
        pub const VERSION: usize = 4;
        pub const HEADER_LEN: usize = 6;
        pub const ROW_LEN: usize = 8;
        pub const RESERVED: usize = 10;
        pub const ROW_COUNT: usize = 12;
        pub const STATUS_FLAGS: usize = 16;
        pub const OBSERVED_COUNTRY_ROWS: usize = 20;
        pub const OBSERVED_ACE_ROWS: usize = 24;
        pub const WING_ASSIGNMENT_ROWS: usize = 28;
        pub const DEAD_ROWS: usize = 32;
        pub const SKIPPED_ROWS: usize = 36;
    }

    /// Trailer row word offsets (`:359`-`:376`). `KILL_TYPE_RAW` and `RESERVED`
    /// exist on v2 rows only; a v1 row is 32 bytes and stops after
    /// `KILLER_COUNTRY`.
    pub mod row_offset {
        pub const TAG_DB_ID: usize = 0;
        pub const ACE_ID: usize = 4;
        pub const FLAGS: usize = 8;
        pub const SOURCE_MASK: usize = 12;
        pub const COUNTRY_ACE_INDEX: usize = 16;
        pub const WING_ID: usize = 20;
        pub const KILLER_ACE_ID: usize = 24;
        pub const KILLER_COUNTRY: usize = 28;
        pub const KILL_TYPE_RAW: usize = 32;
        pub const RESERVED: usize = 36;
    }

    /// The kinds a future port should use, recorded so the stop hands them
    /// forward rather than losing them. **No scale and no continuous field**:
    /// every ace word is an identity, a count or a raw discriminator, so nothing
    /// here needs a quantizer.
    ///
    /// * `tag_db_id`, `killer_country_tag_db_id` — `ContentId` (a country tag is
    ///   content-defined, content-identity contract; d56's `owner_tag_db_id` and d63's `tag_db_id`).
    /// * `ace_id`, `killer_ace_id`, `wing_id`, `country_ace_index` — `Unsigned`
    ///   runtime instance identities (d20's `div_id` rule).
    /// * `alive`, `handled` — `Boolean`, each `unknown_capable`: the flags say
    ///   the byte was read as a *validated* boolean, and a torn read leaves the
    ///   state genuinely uncaptured (`:46`-`:48`, `:57`-`:59`).
    /// * `kill_type_raw` — `Unsigned` and NOT an `Enum`: the codec's own comment
    ///   says "no enum label is assigned because the retained corpus only proves
    ///   raw value 1" (`:62`-`:64`), and an invented `enum_card` would reject
    ///   codes the decoder accepts.
    /// * `name`, `surname`, `callsign` — **the stop**. Free text, no kind.
    ///
    /// The key would be `(tag_db_id, ace_id)`, the fold's own map key
    /// (`world_fold.rs::apply_air_ace_roster`), and the decoder already enforces its uniqueness
    /// (`:420`-`:424`). The blocker is not the key; it is the free text and the
    /// retain-on-incomplete membership gap (module docs).
    pub const KEY_FIELDS: [&str; 2] = ["tag_db_id", "ace_id"];
}

// --- Schema ------------------------------------------------------------------

pub fn schema() -> Schema {
    use field as f;

    let archetypes = Collection {
        collection_id: ARCHETYPES,
        // `variant_id` is the fold's own map key (`world_fold.rs::fold_air_variant_archetypes`), a stable
        // identity that persists across hours — what a keyed set needs, and only a
        // keyed set has a delta grammar (collection-shape contract), which this domain needs because
        // every frame after the first is a sparse patch.
        shape: Shape::KeyedSet,
        fields: vec![
            // A runtime instance identity: variants are designed during play and
            // variant 3421 is a different variant next campaign. `Unsigned`, the
            // same call d35, d38, d39 and d40 make for the same word.
            Field::new(f::VARIANT_ID, Kind::Unsigned).key(0),
            // The three catalog words, read raw and stored per row
            // (`world_fold.rs::fold_air_variant_archetypes`). Process-local
            // token handles and an engine discriminator — see the module docs and
            // `d35.rs:145`-`:159`. A zero `interface_type_raw` is a real measured
            // zero; the authority permits it (`:294`-`:298`).
            Field::new(f::EQUIPMENT_TOKEN_RAW, Kind::Unsigned),
            Field::new(f::EQUIPMENT_TYPE_TOKEN_RAW, Kind::Unsigned),
            Field::new(f::INTERFACE_TYPE_RAW, Kind::Unsigned),
            // The raw fixed-point word, carried RAW beside its divisor rather than
            // projected (d61). `Signed` because the fold reads an `i32`
            // (`world_fold.rs::fold_air_variant_archetypes`) and bounds nothing; the authority's
            // `0..=1_000_000` is not the fold's grammar. Unknown on a clear gate:
            // the collector never reads the value (`attach_log.rs:53768`).
            Field::new(f::AIR_SUPERIORITY_WEIGHT_RAW, Kind::Signed).unknown_capable(),
            // Content-defined text -> `ContentId` through the caller's symbol
            // table (content-identity contract, d29's device). Unknown on a clear gate: the token
            // dictionary did not resolve, the static catalog was not supplied.
            Field::new(f::ARCHETYPE_TOKEN, Kind::ContentId).unknown_capable(),
            Field::new(f::BASE_AIRFRAME_TOKEN, Kind::ContentId).unknown_capable(),
            Field::new(f::ROLE_TOKEN, Kind::ContentId).unknown_capable(),
            // The three `source_mask` bits the accepted grammar leaves free. The
            // other four are constants or a pure function of the two static
            // tokens — enforced in the mapper, left off the wire (module docs).
            Field::new(f::LIVE_DEFINITION, Kind::Boolean),
            Field::new(f::STATIC_CATALOG, Kind::Boolean),
            Field::new(f::LIVE_TOKEN_DICTIONARY, Kind::Boolean),
            // Two header words the fold stamps into every row it stores
            // (`world_fold.rs::fold_air_variant_archetypes`), so they are per-row state: a
            // sparse frame rewrites them only on the rows it carries. The hash is
            // an integrity witness over the static catalog, not a reference into
            // content, so `Unsigned` — no manifest resolves it.
            Field::new(f::STATIC_CATALOG_HASH, Kind::Unsigned),
            Field::new(f::FIXED_SCALE_RAW, Kind::Unsigned),
        ],
    };

    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        // One collection. The `ACE1` ace roster is stop-reported — see the module
        // docs — and collection id 2 is reserved for it.
        collections: vec![archetypes],
        event_part: None,
    }
}

// --- content-identity contract symbols --------------------------------------------------------------

/// The content-set symbol table the three token fields resolve through.
///
/// content-identity contract: symbol ids are **file-global**, assigned from the content set, and
/// "in-stream interning is forbidden — it must be fully read before any seek". So
/// the table is an **input** to this boundary, built once before any record is
/// mapped; it is never grown by the mapper.
///
/// The production assignment now lives in
/// `portable_content_manifest::symbol_manifest`; this domain-local mapper still
/// awaits the file-global resolver retrofit. [`Symbols::from_vocabulary`] uses a
/// measurement stand-in that is deterministic,
/// order-independent and derived only from the content set — the 0-based index
/// into the sorted, deduplicated list — and it is labelled as a stand-in, not as
/// an answer. Two properties make that safe here, the same two d29 states:
/// nothing downstream computes from the id's value (`ContentId` is a plain varint
/// and this domain's key is not a `ContentId`, so no ordering or delta layout
/// depends on the choice), and the measurement is invariant under any assignment
/// over **this domain's own vocabulary** while it stays under 128 symbols, which
/// is a one-byte varint either way. content-identity contract's ids are file-global, so an assignment
/// over the whole content set can still widen the varint; the measurement claims
/// only the invariance it measured.
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

    /// Build from every token the content set can produce. The input is sorted and
    /// deduplicated here, so callers may pass it in any order and two callers over
    /// the same content set always mint the same ids.
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

// --- The payload boundary ----------------------------------------------------

/// The 48-byte payload header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub row_count: u32,
    /// Parsed and reported, never enforced: **this fold never reads the word**
    /// (contrast d63, where it is `== row_count`). The payload authority bounds it
    /// by `row_count` (`domain46_air_variant_archetypes_live.rs:158`-`:162`), which
    /// is where [`check_authority_payload`] checks it.
    pub changed_rows: u32,
    pub status_flags: u32,
    /// Parsed and reported, never enforced by the fold; the authority bounds it
    /// on non-delta frames only (`:163`-`:167`).
    pub observed_variant_rows: u32,
    pub classified_rows: u32,
    pub unresolved_rows: u32,
    pub static_catalog_hash: u32,
    pub fixed_scale_raw: u32,
    pub reserved0: u32,
    pub reserved1: u32,
}

impl PayloadHeader {
    /// The two header words the fold stamps into every row it stores
    /// (`world_fold.rs::fold_air_variant_archetypes`).
    pub fn stamp(&self) -> HeaderStamp {
        HeaderStamp {
            static_catalog_hash: self.static_catalog_hash,
            fixed_scale_raw: self.fixed_scale_raw,
        }
    }

    /// The lane's status word. **The fold never reads this bit** — the baseline is
    /// implicit (`world_fold.rs::fold_air_variant_archetypes`) — so nothing in this module
    /// branches on it; it exists for reporting.
    pub fn declares_delta(&self) -> bool {
        self.status_flags & STATUS_DELTA_DATA != 0
    }
}

/// The two header words the fold copies into every stored row. d35's `RowScales`
/// shape: a header-carried **value** input to the mapper, not merely an invariant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeaderStamp {
    pub static_catalog_hash: u32,
    pub fixed_scale_raw: u32,
}

/// Read the header and apply exactly the checks the fold runs **before** its
/// status gate (`world_fold.rs::fold_air_variant_archetypes`): the 48-byte minimum, the
/// `classified + unresolved == row_count` identity, `fixed_scale_raw != 0`, and
/// both reserved words zero.
///
/// This is the classify boundary: a caller must be able to read the status word
/// without paying for the body, because the fold's failure exit sits between these
/// checks and the row length check. Enforcing the row length here would refuse
/// records the recorder accepts and folds nothing from.
///
/// Total: no payload of any length panics.
pub fn peek_header(payload: &[u8]) -> core::result::Result<PayloadHeader, String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "air_variant_archetypes payload is {} byte(s), expected at least {HEADER_LEN}",
            payload.len()
        ));
    }
    let header = PayloadHeader {
        elapsed_wall_ms: u64::from_le_bytes(payload[0..8].try_into().expect("8 bytes")),
        row_count: u32_at(payload, 8),
        changed_rows: u32_at(payload, 12),
        status_flags: u32_at(payload, 16),
        observed_variant_rows: u32_at(payload, 20),
        classified_rows: u32_at(payload, 24),
        unresolved_rows: u32_at(payload, 28),
        static_catalog_hash: u32_at(payload, 32),
        fixed_scale_raw: u32_at(payload, 36),
        reserved0: u32_at(payload, 40),
        reserved1: u32_at(payload, 44),
    };
    // `world_fold.rs::fold_air_variant_archetypes`. The fold adds two `usize`s with `saturating_add`;
    // over `u32` inputs on a 64-bit host that never saturates, so `u64` arithmetic
    // is the same function without depending on the host's pointer width.
    if u64::from(header.classified_rows) + u64::from(header.unresolved_rows)
        != u64::from(header.row_count)
    {
        return Err(format!(
            "air_variant_archetypes classified {} + unresolved {} != row_count {}",
            header.classified_rows, header.unresolved_rows, header.row_count
        ));
    }
    // `:16336`. The fold rejects ZERO ONLY — it does not compare against
    // `FIXED_SCALE`. The authority's stricter `== 100_000` lives in
    // [`check_authority_payload`]; mirroring it here would refuse payloads this
    // fold folds.
    if header.fixed_scale_raw == 0 {
        return Err("air_variant_archetypes fixed scale is zero".to_string());
    }
    // `:16339`.
    if header.reserved0 != 0 || header.reserved1 != 0 {
        return Err(format!(
            "air_variant_archetypes header reserved fields are nonzero ({:#x}, {:#x})",
            header.reserved0, header.reserved1
        ));
    }
    Ok(header)
}

/// A split payload: the header, the fixed-row region, and the variable-length
/// region behind it (strings, then any `ACE1` trailer).
#[derive(Clone, Copy, Debug)]
pub struct SplitPayload<'a> {
    pub header: PayloadHeader,
    pub rows: &'a [u8],
    pub variable: &'a [u8],
}

/// [`peek_header`] plus the fold's row-length check (`world_fold.rs::fold_air_variant_archetypes`),
/// which the fold runs **after** its status gate — so this is the boundary a caller
/// crosses only for a record it is going to fold.
///
/// Total: no payload of any length panics.
pub fn split_payload(payload: &[u8]) -> core::result::Result<SplitPayload<'_>, String> {
    let header = peek_header(payload)?;
    // Checked arithmetic: `row_count` is a word off a stream on disk. The fold
    // uses `saturating_mul`/`saturating_add`, which for a `u32` count and a
    // 32-byte row cannot saturate on a 64-bit host; `checked_*` is the same
    // function and says so.
    let rows_end = (header.row_count as usize)
        .checked_mul(ROW_LEN)
        .and_then(|bytes| bytes.checked_add(HEADER_LEN))
        .ok_or_else(|| {
            format!(
                "air_variant_archetypes row_count {} overflows the payload length",
                header.row_count
            )
        })?;
    if payload.len() < rows_end {
        return Err(format!(
            "air_variant_archetypes payload is {} byte(s), expected at least {rows_end}",
            payload.len()
        ));
    }
    Ok(SplitPayload {
        header,
        rows: &payload[HEADER_LEN..rows_end],
        variable: &payload[rows_end..],
    })
}

/// One decoded archetype row: the fixed 32-byte words plus the three strings the
/// variable-length region carries for it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedRow {
    pub variant_id: u32,
    pub flags: u32,
    pub equipment_token_raw: u32,
    pub equipment_type_token_raw: u32,
    pub interface_type_raw: u32,
    pub air_superiority_weight_raw: i32,
    pub source_mask: u32,
    pub archetype_token: Option<String>,
    pub base_airframe_token: Option<String>,
    pub role_token: Option<String>,
}

impl ParsedRow {
    /// A tombstone: the key is removed and the row carries no data
    /// (`world_fold.rs::fold_air_variant_archetypes`).
    pub fn is_deleted(&self) -> bool {
        self.flags & ROW_FLAG_DELETED != 0
    }

    /// `world_fold.rs::fold_air_variant_archetypes`. Equal to `SOURCE_STATIC_SOURCE_ENUM` on
    /// every payload the fold accepts, which is why that bit is not a field.
    pub fn carries_static_semantics(&self) -> bool {
        self.base_airframe_token.is_some() || self.role_token.is_some()
    }
}

/// The `ACE1` trailer's validated header. The rows are validated and counted, not
/// retained: the collection is stop-reported.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AceTrailer {
    pub version: u16,
    pub row_len: usize,
    pub row_count: u32,
    pub status_flags: u32,
    pub observed_country_rows: u32,
    pub observed_ace_rows: u32,
    pub wing_assignment_rows: u32,
    pub dead_rows: u32,
    pub skipped_rows: u32,
    /// The trailer's byte span — what the shipping-arm byte split charges to the
    /// stopped segment, and what the cut arm removes.
    pub bytes: usize,
}

impl AceTrailer {
    /// `world_fold.rs::apply_air_ace_roster`: both statuses RETAIN the prior roster
    /// rather than full-replacing it.
    pub fn retains_prior_state(&self) -> bool {
        self.status_flags & (ace::STATUS_SOURCE_UNAVAILABLE | ace::STATUS_PARTIAL_DATA) != 0
    }
}

/// A whole d46 DATA payload as the FOLD reads it: every structural check, every
/// row, every string, the fold's six source/token authority rules, and the whole
/// `ACE1` trailer walk.
#[derive(Clone, Debug)]
pub struct DecodedPayload {
    pub header: PayloadHeader,
    pub rows: Vec<ParsedRow>,
    /// The end of the self-delimiting base payload — header + rows + strings. Any
    /// bytes past it are the trailer.
    pub base_len: usize,
    pub ace: Option<AceTrailer>,
}

/// Decode and validate a payload exactly as `fold_air_variant_archetypes` reads it
/// **after** its status gate (`world_fold.rs::fold_air_variant_archetypes`).
///
/// Deliberately NOT status-independent: the fold returns at `world_fold.rs::fold_air_variant_archetypes` on a failure
/// status and never reaches any of this. Production's *other* authority does run
/// on those records, and that is [`check_authority_payload`], a different rule set
/// — see the module docs.
///
/// Total: no payload of any length panics.
pub fn decode_payload(payload: &[u8]) -> core::result::Result<DecodedPayload, String> {
    let split = split_payload(payload)?;
    let rows_end = HEADER_LEN + split.rows.len();

    // Loop 1, `world_fold.rs::fold_air_variant_archetypes`: the fixed rows, with the three
    // checks the fold makes before it reads a single string.
    let mut rows: Vec<ParsedRow> = Vec::with_capacity(split.rows.len() / ROW_LEN);
    for (index, row) in split.rows.chunks_exact(ROW_LEN).enumerate() {
        let variant_id = u32_at(row, 0);
        if variant_id == 0 {
            return Err(format!(
                "air_variant_archetypes row {index} has zero variant_id"
            ));
        }
        let flags = u32_at(row, 4);
        if flags & !KNOWN_ROW_FLAGS != 0 {
            return Err(format!(
                "air_variant_archetypes row {index} has unknown flags {flags:#x}"
            ));
        }
        let reserved = u32_at(row, 28);
        if reserved != 0 {
            return Err(format!(
                "air_variant_archetypes row {index} reserved field is nonzero ({reserved:#x})"
            ));
        }
        rows.push(ParsedRow {
            variant_id,
            flags,
            equipment_token_raw: u32_at(row, 8),
            equipment_type_token_raw: u32_at(row, 12),
            interface_type_raw: u32_at(row, 16),
            air_superiority_weight_raw: i32_at(row, 20),
            source_mask: u32_at(row, 24),
            archetype_token: None,
            base_airframe_token: None,
            role_token: None,
        });
    }

    // Loop 2, `:16381`-`:16488`: three strings per row, then the flag/string
    // cross-checks and the fold's source/token authority rules.
    let mut cursor = rows_end;
    for (index, row) in rows.iter_mut().enumerate() {
        let archetype = read_string(payload, &mut cursor, "air archetype token")?;
        let base_airframe = read_string(payload, &mut cursor, "air base airframe token")?;
        let role = read_string(payload, &mut cursor, "air role token")?;
        row.archetype_token = flagged_string(
            archetype,
            row.flags,
            ROW_FLAG_ARCHETYPE,
            "air_variant_archetypes archetype",
        )?;
        row.base_airframe_token = flagged_string(
            base_airframe,
            row.flags,
            ROW_FLAG_BASE_AIRFRAME,
            "air_variant_archetypes base_airframe",
        )?;
        row.role_token = flagged_string(
            role,
            row.flags,
            ROW_FLAG_ROLE,
            "air_variant_archetypes role",
        )?;
        check_row(row, split.header.static_catalog_hash)
            .map_err(|e| format!("air_variant_archetypes row {index}: {e}"))?;
    }
    let base_len = cursor;

    // `:16489`-`:16500`: bytes after the self-delimiting base payload are the
    // `ACE1` trailer, and anything else fails the record.
    let ace = if base_len != payload.len() {
        Some(
            decode_ace_trailer(payload, base_len)
                .map_err(|e| format!("air_variant_archetypes ace trailer: {e}"))?,
        )
    } else {
        None
    };

    Ok(DecodedPayload {
        header: split.header,
        rows,
        base_len,
        ace,
    })
}

/// Every token string a payload carries, for the harness's pass-1 vocabulary
/// (content-identity contract forbids in-stream interning, so the table must be complete before pass 2
/// maps anything). The walk is [`decode_payload`]'s, so a payload the fold would
/// refuse contributes no symbols and refuses here too.
pub fn tokens_in_payload(payload: &[u8]) -> core::result::Result<Vec<String>, String> {
    let decoded = decode_payload(payload)?;
    let mut out = Vec::new();
    for row in decoded.rows {
        out.extend(row.archetype_token);
        out.extend(row.base_airframe_token);
        out.extend(row.role_token);
    }
    Ok(out)
}

/// The fold's per-row rules that run after its string walk
/// (`world_fold.rs::fold_air_variant_archetypes`), plus the one refusal this schema's
/// decomposition of `source_mask` requires (module docs).
///
/// Shared by [`decode_payload`] and [`record_from_parsed_row`] so the invariants
/// the mapping rests on have exactly one derivation.
pub fn check_row(row: &ParsedRow, static_catalog_hash: u32) -> core::result::Result<(), String> {
    // `:16403`-`:16418`: a tombstone carries nothing at all.
    if row.is_deleted() {
        if row.equipment_token_raw != 0
            || row.equipment_type_token_raw != 0
            || row.interface_type_raw != 0
            || row.air_superiority_weight_raw != 0
            || row.source_mask != 0
            || row.archetype_token.is_some()
            || row.base_airframe_token.is_some()
            || row.role_token.is_some()
        {
            return Err("deleted row carries identity/classification data".to_string());
        }
        return Ok(());
    }
    // The schema has one Boolean per DEFINED source bit and no field for an
    // undefined one, so an undefined bit is refused rather than dropped. The fold
    // does not check this; production's status-independent payload authority does
    // (`domain46_air_variant_archetypes_live.rs:257`-`:261`), so no certified
    // stream can carry one. See the module docs' divergence note.
    if row.source_mask & !KNOWN_SOURCE_MASK != 0 {
        return Err(format!(
            "source_mask {:#x} carries undefined bits {:#x}",
            row.source_mask,
            row.source_mask & !KNOWN_SOURCE_MASK
        ));
    }
    // `:16420`-`:16424`.
    if row.source_mask & SOURCE_LIVE_STATS == 0 {
        return Err("live row lacks exact live-stats authority".to_string());
    }
    // `:16425`-`:16434`.
    if row.source_mask & (SOURCE_STATIC_HEURISTIC | SOURCE_DEV_SAVE_JOIN_REJECTED) != 0 {
        return Err("heuristic/rejected-save classification is not replay authority".to_string());
    }
    // `:16435`-`:16443`.
    if row
        .archetype_token
        .as_deref()
        .is_some_and(|token| token.starts_with(FABRICATED_TOKEN_PREFIX))
    {
        return Err("fabricated raw-token string identity rejected".to_string());
    }
    let live_token = row.source_mask & SOURCE_LIVE_TOKEN_DICTIONARY != 0;
    let static_catalog = row.source_mask & SOURCE_STATIC_CATALOG != 0;
    let static_enum = row.source_mask & SOURCE_STATIC_SOURCE_ENUM != 0;
    // `:16450`-`:16459`.
    if (live_token && row.archetype_token.is_none())
        || (row.archetype_token.is_some() && !live_token && !static_catalog)
    {
        return Err("token identity disagrees with live dictionary authority".to_string());
    }
    let carries_static_semantics = row.carries_static_semantics();
    // `:16462`-`:16471`.
    if carries_static_semantics && (!static_catalog || !static_enum || static_catalog_hash == 0) {
        return Err("static classification lacks admitted catalog identity/hash".to_string());
    }
    // `:16472`-`:16480`.
    if static_catalog
        && (static_catalog_hash == 0 || (row.archetype_token.is_none() && !carries_static_semantics))
    {
        return Err("static authority/hash is inconsistent with carried semantics".to_string());
    }
    // `:16481`-`:16487`.
    if static_enum && (!static_catalog || !carries_static_semantics) {
        return Err("static authority/hash is inconsistent with carried semantics".to_string());
    }
    Ok(())
}

// --- The boundary mapping ----------------------------------------------------

/// The key of one archetype row: its `variant_id`, with the fold's zero check
/// (`world_fold.rs::fold_air_variant_archetypes`, and the fold's own map key at `world_fold.rs::fold_air_variant_archetypes`).
///
/// Total: no `row` of any length panics.
pub fn key_from_row(row: &[u8]) -> core::result::Result<Key, String> {
    Ok(key(&[u64::from(variant_id_of_row(row)?)]))
}

/// A row's `variant_id`, with the fold's zero check.
pub fn variant_id_of_row(row: &[u8]) -> core::result::Result<u32, String> {
    if row.len() < ROW_LEN {
        return Err(format!(
            "air_variant_archetypes row is {} byte(s), expected {ROW_LEN}",
            row.len()
        ));
    }
    let variant_id = u32_at(row, 0);
    if variant_id == 0 {
        return Err("air_variant_archetypes row has zero variant_id".to_string());
    }
    Ok(variant_id)
}

/// The three token strings a row carries, exactly as the variable-length region
/// holds them: **empty means absent**, which is the flag/string agreement the fold
/// enforces (`world_fold.rs::validate_optional_flagged_string`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RowTokens<'a> {
    pub archetype: &'a str,
    pub base_airframe: &'a str,
    pub role: &'a str,
}

/// One compact row of the `air_variant_archetypes` table, as the collector emits
/// it today, onto its stored Record.
///
/// `row` is the raw 32-byte row slice (the caller has already consumed the table
/// header); there is no legacy form. `frame_tick` is derived-field contract's derived-field input —
/// this domain has no derived field, so it is taken and ignored, which keeps every
/// domain's mapper the same shape.
///
/// Three typed inputs beyond the fixed three, one for each thing the fold reads
/// that the row's own bytes do not carry:
///
/// * `tokens` — the row's three strings, which live in the payload's
///   variable-length region rather than in the row (`world_fold.rs::fold_air_variant_archetypes`);
/// * `stamp` — the two header words the fold copies into every stored row
///   (`world_fold.rs::fold_air_variant_archetypes`), d35's header-carried-value shape;
/// * `symbols` — content-identity contract's resolver for the content-defined tokens (step 3, d29).
///
/// Fail-closed throughout: a row the fold would reject is an `Err`, never a
/// coerced value, and nothing here can panic on any byte string.
pub fn record_from_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
    tokens: RowTokens<'_>,
    stamp: HeaderStamp,
    symbols: &Symbols,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    if row.len() < ROW_LEN {
        return Err(format!(
            "air_variant_archetypes row is {} byte(s), expected {ROW_LEN}",
            row.len()
        ));
    }
    let flags = u32_at(row, 4);
    let reserved = u32_at(row, 28);
    if flags & !KNOWN_ROW_FLAGS != 0 {
        return Err(format!(
            "air_variant_archetypes row has unknown flags {flags:#x}"
        ));
    }
    if reserved != 0 {
        return Err(format!(
            "air_variant_archetypes row reserved field is nonzero ({reserved:#x})"
        ));
    }
    let parsed = ParsedRow {
        variant_id: variant_id_of_row(row)?,
        flags,
        equipment_token_raw: u32_at(row, 8),
        equipment_type_token_raw: u32_at(row, 12),
        interface_type_raw: u32_at(row, 16),
        air_superiority_weight_raw: i32_at(row, 20),
        source_mask: u32_at(row, 24),
        archetype_token: flagged_string(
            tokens.archetype.to_string(),
            flags,
            ROW_FLAG_ARCHETYPE,
            "air_variant_archetypes archetype",
        )?,
        base_airframe_token: flagged_string(
            tokens.base_airframe.to_string(),
            flags,
            ROW_FLAG_BASE_AIRFRAME,
            "air_variant_archetypes base_airframe",
        )?,
        role_token: flagged_string(
            tokens.role.to_string(),
            flags,
            ROW_FLAG_ROLE,
            "air_variant_archetypes role",
        )?,
    };
    record_from_parsed_row(collection, &parsed, stamp, symbols)
}

/// [`record_from_row`] over an already-parsed row, so a caller that has to walk the
/// payload once — the strings are not in the row — does not parse it twice.
pub fn record_from_parsed_row(
    collection: &Collection,
    parsed: &ParsedRow,
    stamp: HeaderStamp,
    symbols: &Symbols,
) -> core::result::Result<Record, String> {
    // The record is per collection, and a record built for the wrong one would
    // otherwise come out silently wrong (d39/d40's rule).
    if collection.collection_id != ARCHETYPES {
        return Err(format!(
            "air_variant_archetypes record built for collection {}, expected {ARCHETYPES}",
            collection.collection_id
        ));
    }
    // A tombstone is not a record: it removes its key and carries no fields
    // (`world_fold.rs::fold_air_variant_archetypes`). Callers reach it through
    // [`ParsedRow::is_deleted`] and [`State::fold_payload`], the d29/d38 shape.
    if parsed.is_deleted() {
        return Err(format!(
            "air_variant_archetypes variant_id {} is a delete row, not a record",
            parsed.variant_id
        ));
    }
    // Every invariant the four-state mapping and the dropped `source_mask` bits
    // rest on, enforced rather than assumed (d17's rule).
    check_row(parsed, stamp.static_catalog_hash)?;

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::EQUIPMENT_TOKEN_RAW => Value::Unsigned(u64::from(parsed.equipment_token_raw)),
            field::EQUIPMENT_TYPE_TOKEN_RAW => {
                Value::Unsigned(u64::from(parsed.equipment_type_token_raw))
            }
            field::INTERFACE_TYPE_RAW => Value::Unsigned(u64::from(parsed.interface_type_raw)),
            // `world_fold.rs::fold_air_variant_archetypes`: the flag decides whether the word is
            // stored at all. Clear gate = the collector never read it
            // (`attach_log.rs:53768`) — record-state contract's unknown. Carried RAW; the divisor is
            // its own field (d61).
            field::AIR_SUPERIORITY_WEIGHT_RAW => {
                if parsed.flags & ROW_FLAG_AIR_SUPERIORITY_WEIGHT != 0 {
                    Value::Signed(i64::from(parsed.air_superiority_weight_raw))
                } else {
                    Value::Unknown
                }
            }
            field::ARCHETYPE_TOKEN => token_value(parsed.archetype_token.as_deref(), symbols)?,
            field::BASE_AIRFRAME_TOKEN => {
                token_value(parsed.base_airframe_token.as_deref(), symbols)?
            }
            field::ROLE_TOKEN => token_value(parsed.role_token.as_deref(), symbols)?,
            field::LIVE_DEFINITION => {
                Value::Bool(parsed.source_mask & SOURCE_LIVE_DEFINITION != 0)
            }
            field::STATIC_CATALOG => Value::Bool(parsed.source_mask & SOURCE_STATIC_CATALOG != 0),
            field::LIVE_TOKEN_DICTIONARY => {
                Value::Bool(parsed.source_mask & SOURCE_LIVE_TOKEN_DICTIONARY != 0)
            }
            field::STATIC_CATALOG_HASH => Value::Unsigned(u64::from(stamp.static_catalog_hash)),
            field::FIXED_SCALE_RAW => Value::Unsigned(u64::from(stamp.fixed_scale_raw)),
            other => return Err(format!("air_variant_archetypes unknown field id {other}")),
        };
        values.push(value);
    }
    Ok(Record { values })
}

/// A content-defined token: `ContentId` through the caller's table, or Unknown
/// when the gate is clear.
fn token_value(token: Option<&str>, symbols: &Symbols) -> core::result::Result<Value, String> {
    match token {
        None => Ok(Value::Unknown),
        Some(text) => match symbols.id(text) {
            Some(id) => Ok(Value::ContentId(id)),
            // Refused, not weakened to `Unknown`: the string is right here, so
            // "not captured" would be false. A symbol outside the content set
            // means the caller built the table from the wrong content, and that is
            // a diagnosis, not data (d29's rule).
            None => Err(format!(
                "air_variant_archetypes token {text:?} is not in the symbol table \
                 ({} symbol(s))",
                symbols.len()
            )),
        },
    }
}

// --- Production's second authority -------------------------------------------

/// The checks production makes on **every** d46 DOMAIN_DATA record, whatever its
/// status: `air_wings_variants_lifecycle_live_lane::decode_payload`, reached from
/// `decode_compact_air_variant_archetypes_data_payload`
/// (`recorder_host/src/domain_data_authority.rs:7906`-`:7930`, dispatched at
/// `:16966`). Any error it reports makes the record `malformed_payload_records`,
/// which the strict certification verdict refuses.
///
/// **This is a different rule set from the fold's, in both directions**, which is
/// why it is a separate function and not folded into [`decode_payload`]:
///
/// * stricter — `fixed_scale_raw == 100_000`, `changed_rows <= row_count`,
///   `row_count <= observed_variant_rows` on a non-delta frame, per-field bounds,
///   a `MAX_TOKEN_LEN` cap, **duplicate `variant_id` rejection** (the fold
///   collapses duplicates last-writer-wins), and the decoded/declared
///   classified/unresolved counts;
/// * weaker — it never validates UTF-8, and it does not make the fold's six
///   source/token authority checks.
///
/// The original accumulates its complaints into a summary and the certification
/// verdict refuses on any of them, so returning the first is verdict-equivalent;
/// the ordering of the messages is not part of the contract.
///
/// Total: no payload of any length panics.
pub fn check_authority_payload(payload: &[u8]) -> core::result::Result<(), String> {
    // `domain46_air_variant_archetypes_live.rs:138`-`:144`.
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "air_variant_archetypes payload is {} byte(s), expected at least {HEADER_LEN}",
            payload.len()
        ));
    }
    let row_count = u32_at(payload, 8) as usize;
    let changed_rows = u32_at(payload, 12) as usize;
    let status_flags = u32_at(payload, 16);
    let observed_variant_rows = u32_at(payload, 20) as usize;
    let classified_rows = u32_at(payload, 24) as usize;
    let unresolved_rows = u32_at(payload, 28) as usize;
    let static_catalog_hash = u32_at(payload, 32);
    let fixed_scale_raw = u32_at(payload, 36);
    let reserved0 = u32_at(payload, 40);
    let reserved1 = u32_at(payload, 44);

    // `:158`-`:162`.
    if changed_rows > row_count {
        return Err(format!(
            "air_variant_archetypes changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    // `:163`-`:167`.
    if row_count > observed_variant_rows && status_flags & STATUS_DELTA_DATA == 0 {
        return Err(format!(
            "air_variant_archetypes row_count {row_count} > observed_variant_rows \
             {observed_variant_rows}"
        ));
    }
    // `:168`-`:171`.
    if classified_rows.saturating_add(unresolved_rows) != row_count {
        return Err(format!(
            "air_variant_archetypes classified+unresolved {} != row_count {row_count}",
            classified_rows.saturating_add(unresolved_rows)
        ));
    }
    // `:172`-`:176` — the check the FOLD does not make.
    if fixed_scale_raw != AUTHORITY_FIXED_SCALE_RAW {
        return Err(format!(
            "air_variant_archetypes fixed scale {fixed_scale_raw} != {AUTHORITY_FIXED_SCALE_RAW}"
        ));
    }
    // `:177`-`:181`.
    if reserved0 != 0 || reserved1 != 0 {
        return Err("air_variant_archetypes header reserved fields are nonzero".to_string());
    }
    // `:184`-`:191`.
    let rows_end = row_count
        .checked_mul(ROW_LEN)
        .and_then(|bytes| bytes.checked_add(HEADER_LEN))
        .ok_or_else(|| {
            format!("air_variant_archetypes row_count {row_count} overflows the payload length")
        })?;
    if payload.len() < rows_end {
        return Err(format!(
            "air_variant_archetypes payload is {} byte(s), expected at least {rows_end}",
            payload.len()
        ));
    }

    // `:220`-`:320`: the per-row rules.
    let mut seen: Vec<u32> = Vec::with_capacity(row_count);
    let mut row_flags: Vec<u32> = Vec::with_capacity(row_count);
    for (index, row) in payload[HEADER_LEN..rows_end]
        .chunks_exact(ROW_LEN)
        .enumerate()
    {
        let variant_id = u32_at(row, 0);
        let flags = u32_at(row, 4);
        let equipment_token_raw = u32_at(row, 8);
        let equipment_type_token_raw = u32_at(row, 12);
        let interface_type_raw = u32_at(row, 16);
        let air_superiority_weight_raw = i32_at(row, 20);
        let source_mask = u32_at(row, 24);
        let row_reserved = u32_at(row, 28);
        let deleted = flags & ROW_FLAG_DELETED != 0;

        if variant_id == 0 || variant_id > MAX_VARIANT_ID {
            return Err(format!(
                "air_variant_archetypes row {index} has invalid variant_id {variant_id}"
            ));
        }
        if seen.contains(&variant_id) {
            return Err(format!(
                "air_variant_archetypes row {index} duplicates variant_id {variant_id}"
            ));
        }
        seen.push(variant_id);
        if flags & !KNOWN_ROW_FLAGS != 0 {
            return Err(format!(
                "air_variant_archetypes row {index} has unknown flags {flags}"
            ));
        }
        if source_mask == 0 && !deleted {
            return Err(format!(
                "air_variant_archetypes row {index} has empty source_mask"
            ));
        }
        if source_mask & !KNOWN_SOURCE_MASK != 0 {
            return Err(format!(
                "air_variant_archetypes row {index} has unknown source_mask {source_mask}"
            ));
        }
        if row_reserved != 0 {
            return Err(format!(
                "air_variant_archetypes row {index} reserved field is nonzero"
            ));
        }
        if deleted {
            if equipment_token_raw != 0
                || equipment_type_token_raw != 0
                || interface_type_raw != 0
                || air_superiority_weight_raw != 0
            {
                return Err(format!(
                    "air_variant_archetypes deleted row {index} carries live fields"
                ));
            }
        } else {
            if equipment_token_raw == 0 || equipment_token_raw > MAX_EQUIPMENT_TOKEN_RAW {
                return Err(format!(
                    "air_variant_archetypes row {index} has invalid equipment_token_raw \
                     {equipment_token_raw}"
                ));
            }
            if equipment_type_token_raw == 0
                || equipment_type_token_raw > MAX_EQUIPMENT_TYPE_TOKEN_RAW
            {
                return Err(format!(
                    "air_variant_archetypes row {index} has invalid equipment_type_token_raw \
                     {equipment_type_token_raw}"
                ));
            }
            if interface_type_raw > MAX_INTERFACE_TYPE_RAW {
                return Err(format!(
                    "air_variant_archetypes row {index} has invalid interface_type_raw \
                     {interface_type_raw}"
                ));
            }
        }
        if flags & ROW_FLAG_AIR_SUPERIORITY_WEIGHT != 0 {
            if !(0..=MAX_AIR_SUPERIORITY_WEIGHT_RAW).contains(&air_superiority_weight_raw) {
                return Err(format!(
                    "air_variant_archetypes row {index} has invalid air_superiority_weight_raw \
                     {air_superiority_weight_raw}"
                ));
            }
        } else if air_superiority_weight_raw != 0 {
            return Err(format!(
                "air_variant_archetypes row {index} has air_superiority_weight without flag"
            ));
        }
        row_flags.push(flags);
    }

    // `:322`-`:400`: the string region, measured but not UTF-8-validated by this
    // authority, and the declared/decoded classification counters.
    let mut cursor = rows_end;
    let mut decoded_classified = 0usize;
    let mut decoded_unresolved = 0usize;
    for (index, flags) in row_flags.iter().enumerate() {
        let archetype_len = skip_string(payload, &mut cursor, "air variant archetype token")?;
        let base_len = skip_string(payload, &mut cursor, "air variant base airframe token")?;
        let role_len = skip_string(payload, &mut cursor, "air variant role token")?;
        for (label, len, flag) in [
            ("archetype", archetype_len, ROW_FLAG_ARCHETYPE),
            ("base_airframe", base_len, ROW_FLAG_BASE_AIRFRAME),
            ("role", role_len, ROW_FLAG_ROLE),
        ] {
            if len > MAX_TOKEN_LEN {
                return Err(format!(
                    "air_variant_archetypes row {index} {label} string is too long ({len})"
                ));
            }
            if len == 0 && flags & flag != 0 {
                return Err(format!(
                    "air_variant_archetypes row {index} has {label} flag with empty string"
                ));
            }
            if len > 0 && flags & flag == 0 {
                return Err(format!(
                    "air_variant_archetypes row {index} has {label} string without flag"
                ));
            }
        }
        if flags & ROW_FLAG_DELETED != 0 && (archetype_len != 0 || base_len != 0 || role_len != 0) {
            return Err(format!(
                "air_variant_archetypes deleted row {index} carries strings"
            ));
        }
        if archetype_len > 0
            || base_len > 0
            || role_len > 0
            || flags & ROW_FLAG_AIR_SUPERIORITY_WEIGHT != 0
        {
            decoded_classified += 1;
        } else {
            decoded_unresolved += 1;
        }
    }

    // `:417`-`:421`: trailing bytes are the `ACE1` trailer, validated here too —
    // which is why a malformed trailer refuses the stream even on a
    // failure-status record (the fold's own copy never runs there).
    if cursor != payload.len() {
        decode_ace_trailer(payload, cursor)
            .map_err(|e| format!("air_variant_archetypes ace trailer: {e}"))?;
    }
    // `:445`-`:456`.
    if decoded_classified != classified_rows {
        return Err(format!(
            "air_variant_archetypes classified_rows {classified_rows} != decoded \
             {decoded_classified}"
        ));
    }
    if decoded_unresolved != unresolved_rows {
        return Err(format!(
            "air_variant_archetypes unresolved_rows {unresolved_rows} != decoded \
             {decoded_unresolved}"
        ));
    }
    let _ = static_catalog_hash;
    Ok(())
}

// --- The stop-reported `ACE1` trailer: validated, never mapped ----------------

/// Strictly decode the `ACE1` trailer at `trailer_offset`, transcribed from
/// `domain46_air_ace_roster_live::decode_trailer_rows` (`:274`-`:632`) — the
/// single validation source both the fold and the payload authority consume.
///
/// Every violation fails the whole trailer, never a partial decode, and the
/// trailer must consume the payload to its exact end. The rows are validated and
/// counted; **none is mapped**, because the collection is stop-reported.
///
/// Total: no payload of any length and no offset panics.
pub fn decode_ace_trailer(
    payload: &[u8],
    trailer_offset: usize,
) -> core::result::Result<AceTrailer, String> {
    use ace::header_offset as h;
    use ace::row_offset as r;

    let trailer = payload
        .get(trailer_offset..)
        .ok_or_else(|| format!("ace trailer offset {trailer_offset} is beyond payload end"))?;
    if trailer.len() < ace::TRAILER_HEADER_LEN {
        return Err(format!(
            "ace trailer is {} byte(s), expected at least {}",
            trailer.len(),
            ace::TRAILER_HEADER_LEN
        ));
    }
    if trailer[h::MAGIC..h::MAGIC + 4] != ace::TRAILER_MAGIC {
        return Err("ace trailer magic is invalid".to_string());
    }
    let version = u16_at(trailer, h::VERSION);
    let header_len = u16_at(trailer, h::HEADER_LEN) as usize;
    let row_len = u16_at(trailer, h::ROW_LEN) as usize;
    let reserved = u16_at(trailer, h::RESERVED);
    let v1 = version == ace::TRAILER_VERSION_V1 && row_len == ace::TRAILER_ROW_LEN_V1;
    let v2 = version == ace::TRAILER_VERSION_V2 && row_len == ace::TRAILER_ROW_LEN_V2;
    if (!v1 && !v2) || header_len != ace::TRAILER_HEADER_LEN || reserved != 0 {
        return Err(format!(
            "ace trailer header shape is invalid (version {version}, header_len {header_len}, \
             row_len {row_len}, reserved {reserved})"
        ));
    }
    let row_count = u32_at(trailer, h::ROW_COUNT) as usize;
    let status_flags = u32_at(trailer, h::STATUS_FLAGS);
    let observed_country_rows = u32_at(trailer, h::OBSERVED_COUNTRY_ROWS);
    let observed_ace_rows = u32_at(trailer, h::OBSERVED_ACE_ROWS);
    let wing_assignment_rows = u32_at(trailer, h::WING_ASSIGNMENT_ROWS);
    let dead_rows = u32_at(trailer, h::DEAD_ROWS);
    let skipped_rows = u32_at(trailer, h::SKIPPED_ROWS);

    if status_flags & !ace::KNOWN_STATUS_MASK != 0 {
        return Err(format!(
            "ace trailer has unknown status flags {status_flags:#x}"
        ));
    }
    if row_count > ace::MAX_ACE_ROWS {
        return Err(format!(
            "ace trailer row_count {row_count} exceeds {}",
            ace::MAX_ACE_ROWS
        ));
    }
    if row_count != observed_ace_rows as usize {
        return Err(format!(
            "ace trailer row_count {row_count} != observed_ace_rows {observed_ace_rows} \
             (rows are full-replace)"
        ));
    }
    if wing_assignment_rows as usize > row_count || dead_rows as usize > row_count {
        return Err(format!(
            "ace trailer assignment/dead counters exceed row_count {row_count}"
        ));
    }
    // `:339`-`:343`: a whole-frame absence must be absent, never zero-filled.
    if status_flags & ace::STATUS_SOURCE_UNAVAILABLE != 0 && row_count != 0 {
        return Err(format!(
            "ace trailer SOURCE_UNAVAILABLE carries {row_count} rows (must be empty)"
        ));
    }

    let rows_end = row_count
        .checked_mul(row_len)
        .and_then(|bytes| bytes.checked_add(header_len))
        .ok_or_else(|| format!("ace trailer row_count {row_count} overflows the trailer length"))?;
    if trailer.len() < rows_end {
        return Err(format!(
            "ace trailer is {} byte(s), expected at least {rows_end}",
            trailer.len()
        ));
    }

    let mut seen: Vec<(u32, u32)> = Vec::with_capacity(row_count);
    let mut decoded_assignment_rows = 0usize;
    let mut decoded_dead_rows = 0usize;
    for (row_index, row) in trailer[header_len..rows_end]
        .chunks_exact(row_len)
        .enumerate()
    {
        let tag_db_id = u32_at(row, r::TAG_DB_ID);
        let ace_id = u32_at(row, r::ACE_ID);
        let flags = u32_at(row, r::FLAGS);
        let source_mask = u32_at(row, r::SOURCE_MASK);
        let country_ace_index = u32_at(row, r::COUNTRY_ACE_INDEX);
        let wing_id = u32_at(row, r::WING_ID);
        let killer_ace_id = u32_at(row, r::KILLER_ACE_ID);
        let killer_country = u32_at(row, r::KILLER_COUNTRY);
        let kill_type_raw = if v1 { 0 } else { u32_at(row, r::KILL_TYPE_RAW) };
        let row_reserved = if v1 { 0 } else { u32_at(row, r::RESERVED) };

        if row_reserved != 0 {
            return Err(format!(
                "ace row {row_index} has nonzero reserved value {row_reserved:#x}"
            ));
        }
        let known_row_flags = if v1 {
            ace::KNOWN_ROW_FLAGS_V1
        } else {
            ace::KNOWN_ROW_FLAGS_V2
        };
        if flags & !known_row_flags != 0 {
            return Err(format!("ace row {row_index} has unknown flags {flags:#x}"));
        }
        if source_mask == 0 {
            return Err(format!("ace row {row_index} has empty source_mask"));
        }
        let known_source_mask = if v1 {
            ace::KNOWN_SOURCE_MASK_V1
        } else {
            ace::KNOWN_SOURCE_MASK_V2
        };
        if source_mask & !known_source_mask != 0 {
            return Err(format!(
                "ace row {row_index} has unknown source_mask {source_mask:#x}"
            ));
        }
        let required_identity =
            ace::SOURCE_LIVE_COUNTRY_ACE_VECTOR | ace::SOURCE_LIVE_ACE_OBJECT_LAYOUT;
        if source_mask & required_identity != required_identity {
            return Err(format!(
                "ace row {row_index} lacks exact country-vector/object-layout identity sources"
            ));
        }
        if tag_db_id == 0 || tag_db_id > ace::MAX_TAG_DB_ID {
            return Err(format!(
                "ace row {row_index} has invalid tag_db_id {tag_db_id}"
            ));
        }
        if ace_id == 0 || ace_id > ace::MAX_ACE_ID {
            return Err(format!("ace row {row_index} has invalid ace_id {ace_id}"));
        }
        if seen.contains(&(tag_db_id, ace_id)) {
            return Err(format!(
                "ace row {row_index} duplicates tag_db_id {tag_db_id} ace_id {ace_id}"
            ));
        }
        seen.push((tag_db_id, ace_id));
        if country_ace_index > ace::MAX_COUNTRY_ACE_INDEX {
            return Err(format!(
                "ace row {row_index} has invalid country_ace_index {country_ace_index}"
            ));
        }
        let has_wing = flags & ace::ROW_FLAG_WING_ID != 0;
        if has_wing {
            if wing_id == 0 || wing_id > ace::MAX_WING_ID {
                return Err(format!("ace row {row_index} has invalid wing_id {wing_id}"));
            }
            decoded_assignment_rows += 1;
        } else if wing_id != 0 {
            return Err(format!("ace row {row_index} has wing_id without flag"));
        }
        if has_wing != (source_mask & ace::SOURCE_LIVE_WING_ACE_ID_ASSIGNMENT != 0) {
            return Err(format!(
                "ace row {row_index} wing assignment disagrees with source authority"
            ));
        }
        let frame_assignment_blocked = status_flags & ace::STATUS_WING_ASSIGNMENT_BLOCKED != 0;
        if frame_assignment_blocked != (source_mask & ace::SOURCE_WING_ASSIGNMENT_SCAN_BLOCKED != 0)
        {
            return Err(format!(
                "ace row {row_index} assignment-blocked source disagrees with frame status"
            ));
        }

        let alive_known = flags & ace::ROW_FLAG_ALIVE_KNOWN != 0;
        let dead = flags & ace::ROW_FLAG_DEAD != 0;
        if dead && !alive_known {
            return Err(format!("ace row {row_index} has DEAD without ALIVE_KNOWN"));
        }
        if dead {
            decoded_dead_rows += 1;
        }
        if flags & ace::ROW_FLAG_KILLER_ACE != 0 {
            if !dead {
                return Err(format!(
                    "ace row {row_index} has KILLER_ACE on a non-dead row"
                ));
            }
            if killer_ace_id == 0 || killer_ace_id > ace::MAX_ACE_ID {
                return Err(format!(
                    "ace row {row_index} has invalid killer_ace_id {killer_ace_id}"
                ));
            }
        } else if killer_ace_id != 0 {
            return Err(format!("ace row {row_index} has killer_ace_id without flag"));
        }
        if flags & ace::ROW_FLAG_KILLER_COUNTRY != 0 {
            if !dead {
                return Err(format!(
                    "ace row {row_index} has KILLER_COUNTRY on a non-dead row"
                ));
            }
            if killer_country == 0 || killer_country > ace::MAX_TAG_DB_ID {
                return Err(format!(
                    "ace row {row_index} has invalid killer_country {killer_country}"
                ));
            }
        } else if killer_country != 0 {
            return Err(format!("ace row {row_index} has killer_country without flag"));
        }
        if source_mask & ace::SOURCE_LIVE_ACE_DEATH_RECORD != 0 && !alive_known {
            return Err(format!(
                "ace row {row_index} claims death-record source without ALIVE_KNOWN"
            ));
        }
        if alive_known && source_mask & ace::SOURCE_LIVE_ACE_DEATH_RECORD == 0 {
            return Err(format!(
                "ace row {row_index} has ALIVE_KNOWN without death-record source"
            ));
        }
        let handled_known = flags & ace::ROW_FLAG_HANDLED_KNOWN != 0;
        let handled = flags & ace::ROW_FLAG_HANDLED != 0;
        if handled && !handled_known {
            return Err(format!(
                "ace row {row_index} has HANDLED without HANDLED_KNOWN"
            ));
        }
        if alive_known && !dead && handled {
            return Err(format!(
                "ace row {row_index} has contradictory alive=true handled=true tuple"
            ));
        }
        let kill_type_known = flags & ace::ROW_FLAG_KILL_TYPE_KNOWN != 0;
        if !kill_type_known && kill_type_raw != 0 {
            return Err(format!("ace row {row_index} has kill_type_raw without flag"));
        }
        if alive_known && !dead && kill_type_known && kill_type_raw != 0 {
            return Err(format!(
                "ace row {row_index} has nonzero death-only kill_type_raw on a living ace"
            ));
        }
        if !v1 && (alive_known != handled_known || alive_known != kill_type_known) {
            return Err(format!(
                "ace row {row_index} has partial v2 lifecycle tuple presence"
            ));
        }
        if source_mask & ace::SOURCE_LIVE_ACE_LIFECYCLE_RECORD != 0
            && (!alive_known || !handled_known || !kill_type_known)
        {
            return Err(format!(
                "ace row {row_index} claims lifecycle-record source without alive, handled, and \
                 kill_type presence"
            ));
        }
        if !v1 && alive_known && source_mask & ace::SOURCE_LIVE_ACE_LIFECYCLE_RECORD == 0 {
            return Err(format!(
                "ace row {row_index} has a v2 lifecycle tuple without lifecycle-record source"
            ));
        }
    }

    // `:582`-`:613`: three free-text strings per row — the stop — read for their
    // length and flag agreement, then the trailer must end exactly here.
    let mut cursor = rows_end;
    for row_index in 0..row_count {
        let row_flags = u32_at(
            trailer,
            header_len + row_index * row_len + ace::row_offset::FLAGS,
        );
        for (label, flag) in [
            ("name", ace::ROW_FLAG_NAME),
            ("surname", ace::ROW_FLAG_SURNAME),
            ("callsign", ace::ROW_FLAG_CALLSIGN),
        ] {
            let len = read_ace_string_len(trailer, &mut cursor, label, row_index)?;
            if len > ace::MAX_STRING_LEN {
                return Err(format!(
                    "ace row {row_index} {label} string is too long ({len})"
                ));
            }
            if len > 0 && row_flags & flag == 0 {
                return Err(format!(
                    "ace row {row_index} has {label} string without flag"
                ));
            }
            if len == 0 && row_flags & flag != 0 {
                return Err(format!(
                    "ace row {row_index} has {label} flag with empty string"
                ));
            }
        }
    }
    if trailer_offset.saturating_add(cursor) != payload.len() {
        return Err(format!(
            "ace trailer has {} trailing byte(s)",
            payload.len().saturating_sub(trailer_offset + cursor)
        ));
    }
    if decoded_assignment_rows != wing_assignment_rows as usize {
        return Err(format!(
            "ace trailer wing_assignment_rows {wing_assignment_rows} != decoded \
             {decoded_assignment_rows}"
        ));
    }
    if decoded_dead_rows != dead_rows as usize {
        return Err(format!(
            "ace trailer dead_rows {dead_rows} != decoded {decoded_dead_rows}"
        ));
    }

    Ok(AceTrailer {
        version,
        row_len,
        row_count: row_count.min(u32::MAX as usize) as u32,
        status_flags,
        observed_country_rows,
        observed_ace_rows,
        wing_assignment_rows,
        dead_rows,
        skipped_rows,
        bytes: cursor,
    })
}

// --- The accumulator ---------------------------------------------------------

/// The accumulator `fold_air_variant_archetypes` maintains: the archetype map
/// (`world_fold.rs:1883`) and the `delta_baselines_seen` membership its implicit
/// baseline reads (`:2972`-`:2978`, `:16503`), plus a count-only mirror of the
/// stop-reported ace roster (`:1887`).
///
/// **No other writer of the archetype map.** `self.air_variant_archetypes` is
/// written only by this fold (`world_fold.rs::fold_air_variant_archetypes`) and cleared with the
/// rest of the world state; every other mention is a read (`world_fold.rs::build_designs`).
/// Nothing to model, prove inert or assert inert.
#[derive(Clone, Debug, Default)]
pub struct State {
    pub archetypes: KeyedState,
    /// `delta_baselines_seen.contains(46)`. False until the first FOLDED frame,
    /// which is therefore the implicit baseline; a failure-status record does not
    /// set it, because the fold returns before `implicit_delta_baseline_or_sparse`.
    pub baseline_seen: bool,
    /// The stop-reported roster's size after this frame — the analogue of d56's
    /// `supremacy_rows`. The fold's own accumulator is a map keyed
    /// `(tag_db_id, ace_id)` (`world_fold.rs::apply_air_ace_roster`); this schema carries no
    /// collection for it, so only its size is mirrored, which is enough to
    /// reproduce the fold's replace-or-retain decision and to report the segment's
    /// bytes.
    pub ace_rows: u32,
    /// Trailers present, full-replaced, and retained-instead — the three counts
    /// the stop report needs.
    pub ace_trailers_seen: u64,
    pub ace_trailers_replaced: u64,
    pub ace_trailers_retained: u64,
}

/// What one payload did to the accumulator. A third outcome exists and is not in
/// this enum — an `Err` from [`State::fold_payload`], which is the payload not
/// being this grammar and therefore a **stream rejection**, never an hour to skip.
///
/// There is no `DeltaBeforeBaseline`: this fold's baseline is implicit, so it has
/// no such exit (module docs).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FoldOutcome {
    Folded,
    /// `world_fold.rs:16342`-`:16348` — `handle_compact_failure` marks the domain
    /// unknown and retains the accumulator. The caller classifies the status word
    /// with `compact_contract`'s own predicate and passes the answer in, so this
    /// module never re-derives a failure mask.
    FailureStatus,
}

impl State {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one d46 DATA payload exactly as `fold_air_variant_archetypes` does,
    /// **in its order**: the header checks, then the status gate, then the body.
    ///
    /// `status_is_failure` is the caller's classification of
    /// `header.status_flags` — the fold's own
    /// `compact_failure_flags_after_expected_partial_tolerance(46, ...) != 0`
    /// (`world_fold.rs::handle_compact_failure`). It is an input rather than something
    /// re-derived here so the library carries no copied status mask.
    ///
    /// **Error contract: all-or-nothing.** Every structural check, every row, every
    /// string, the trailer walk and every record are done before a single
    /// accumulator write, which is what the fold does — it decodes and validates
    /// the whole payload and only then reaches `world_fold.rs::fold_air_variant_archetypes`. A harness that assumed the
    /// partial-mutation contract instead would disagree with the recorder on
    /// exactly the frames that fail.
    pub fn fold_payload(
        &mut self,
        schema: &Schema,
        payload: &[u8],
        status_is_failure: bool,
        frame_tick: u64,
        symbols: &Symbols,
    ) -> core::result::Result<FoldOutcome, String> {
        if schema.collections.len() != 1 {
            return Err(format!(
                "air_variant_archetypes schema has {} collections, expected 1",
                schema.collections.len()
            ));
        }
        let collection = &schema.collections[0];
        // derived-field contract's derived-field input. This domain has no derived field, so the tick
        // is taken and ignored — every domain's boundary has one shape.
        let _ = frame_tick;

        // 1. The header checks the fold runs BEFORE its status gate.
        peek_header(payload)?;
        // 2. Failure status: the accumulator is retained, the body is never read,
        //    and the implicit baseline is NOT consumed.
        if status_is_failure {
            return Ok(FoldOutcome::FailureStatus);
        }
        // 3. The body, decoded and validated whole.
        let decoded = decode_payload(payload)?;
        let stamp = decoded.header.stamp();

        // 4. Every record built before any mutation, in payload order.
        let mut ops: Vec<(Key, Option<Record>)> = Vec::with_capacity(decoded.rows.len());
        for (index, row) in decoded.rows.iter().enumerate() {
            let row_key = key(&[u64::from(row.variant_id)]);
            if row.is_deleted() {
                ops.push((row_key, None));
                continue;
            }
            let record = record_from_parsed_row(collection, row, stamp, symbols)
                .map_err(|e| format!("air_variant_archetypes row {index}: {e}"))?;
            ops.push((row_key, Some(record)));
        }

        // 5. `world_fold.rs::fold_air_variant_archetypes`: the FIRST folded frame replaces, every
        //    later one is a sparse patch. The status word is not consulted.
        let sparse = self.baseline_seen;
        self.baseline_seen = true;
        if !sparse {
            self.archetypes.clear();
        }
        // 6. `:16507`-`:16535`, **in payload order**, which is what makes a
        //    repeated `variant_id` collapse last-writer-wins exactly as the fold's
        //    `BTreeMap` does — mirrored rather than refused (d22's ruling; the
        //    payload authority refuses the duplicate anyway).
        for (row_key, record) in ops {
            match record {
                None => {
                    self.archetypes.remove(&row_key);
                }
                Some(record) => {
                    self.archetypes.insert(row_key, record);
                }
            }
        }

        // 7. `:16536`-`:16538` then `:16562`-`:16631`: the stop-reported roster is
        //    full-replaced on a clean frame and RETAINED on SOURCE_UNAVAILABLE or
        //    PARTIAL_DATA. Mirrored as counts.
        if let Some(trailer) = decoded.ace {
            self.ace_trailers_seen += 1;
            if trailer.retains_prior_state() {
                self.ace_trailers_retained += 1;
            } else {
                self.ace_trailers_replaced += 1;
                self.ace_rows = trailer.row_count;
            }
        }
        Ok(FoldOutcome::Folded)
    }

    /// The collection states in schema order, which is the arity
    /// `encode_baseline`/`encode_delta` require. One entry: the ace roster is
    /// stop-reported and contributes no collection.
    pub fn states(&self) -> [KeyedState; 1] {
        [self.archetypes.clone()]
    }
}

// --- Shared helpers ----------------------------------------------------------

fn u16_at(bytes: &[u8], off: usize) -> u16 {
    let mut word = [0u8; 2];
    for (i, slot) in word.iter_mut().enumerate() {
        *slot = bytes.get(off + i).copied().unwrap_or(0);
    }
    u16::from_le_bytes(word)
}

fn u32_at(bytes: &[u8], off: usize) -> u32 {
    let mut word = [0u8; 4];
    for (i, slot) in word.iter_mut().enumerate() {
        *slot = bytes.get(off + i).copied().unwrap_or(0);
    }
    u32::from_le_bytes(word)
}

fn i32_at(bytes: &[u8], off: usize) -> i32 {
    u32_at(bytes, off) as i32
}

/// The fold's `read_string` (`world_fold.rs::read_string`): a length-prefixed
/// (u16) UTF-8 string, advancing the cursor. Bounds-checked before every read.
fn read_string(
    bytes: &[u8],
    cursor: &mut usize,
    label: &str,
) -> core::result::Result<String, String> {
    if bytes.len() < cursor.saturating_add(2) {
        return Err(format!(
            "{label}: need {} bytes, have {}",
            cursor.saturating_add(2),
            bytes.len()
        ));
    }
    let len = u16_at(bytes, *cursor) as usize;
    *cursor += 2;
    let end = cursor.saturating_add(len);
    if bytes.len() < end {
        return Err(format!("{label}: need {end} bytes, have {}", bytes.len()));
    }
    let start = *cursor;
    *cursor = end;
    core::str::from_utf8(&bytes[start..end])
        .map(str::to_string)
        .map_err(|e| format!("{label}: invalid utf-8: {e}"))
}

/// The authority's `skip_compact_string_len`
/// (`domain46_air_variant_archetypes_live.rs:539`-`:552`): the same walk without
/// the UTF-8 check, returning the length.
fn skip_string(
    bytes: &[u8],
    cursor: &mut usize,
    label: &str,
) -> core::result::Result<usize, String> {
    if bytes.len().saturating_sub(*cursor) < 2 {
        return Err(format!("{label} string is truncated before length"));
    }
    let len = u16_at(bytes, *cursor) as usize;
    *cursor += 2;
    if bytes.len().saturating_sub(*cursor) < len {
        return Err(format!("{label} string length {len} exceeds remaining payload"));
    }
    *cursor += len;
    Ok(len)
}

/// The ace codec's `read_compact_string` (`domain46_air_ace_roster_live.rs:783`-`:806`),
/// reduced to the length: the strings are the stop, so nothing keeps them.
/// The UTF-8 check the original makes is still performed, because an invalid one
/// refuses the trailer there.
fn read_ace_string_len(
    bytes: &[u8],
    cursor: &mut usize,
    label: &str,
    row_index: usize,
) -> core::result::Result<usize, String> {
    if bytes.len().saturating_sub(*cursor) < 2 {
        return Err(format!(
            "ace row {row_index} {label} string is truncated before length"
        ));
    }
    let len = u16_at(bytes, *cursor) as usize;
    *cursor += 2;
    if bytes.len().saturating_sub(*cursor) < len {
        return Err(format!(
            "ace row {row_index} {label} string length {len} exceeds remaining payload"
        ));
    }
    let start = *cursor;
    *cursor += len;
    if len > 0 && core::str::from_utf8(&bytes[start..start + len]).is_err() {
        return Err(format!(
            "ace row {row_index} {label} string is not valid UTF-8"
        ));
    }
    Ok(len)
}

/// The fold's `validate_optional_flagged_string` (`world_fold.rs::validate_optional_flagged_string`):
/// a flag with an empty string and a string without its flag are both refused.
fn flagged_string(
    value: String,
    flags: u32,
    flag: u32,
    label: &str,
) -> core::result::Result<Option<String>, String> {
    if value.is_empty() && flags & flag != 0 {
        return Err(format!("{label} flag with empty string"));
    }
    if !value.is_empty() && flags & flag == 0 {
        return Err(format!("{label} string without flag"));
    }
    Ok((flags & flag != 0).then_some(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{decode_baseline, encode_baseline};
    use crate::schema::Quant;

    const GRID_LOG2: u8 = 4;

    fn collection() -> Collection {
        schema().collections[0].clone()
    }

    fn index_of(c: &Collection, field_id: u32) -> usize {
        c.value_fields()
            .position(|f| f.field_id == field_id)
            .expect("field is in the collection")
    }

    fn symbols() -> Symbols {
        Symbols::from_vocabulary([
            "cv_small_plane_airframe_1",
            "fighter",
            "small_plane_airframe_2",
            "small_plane_cas_airframe_2",
        ])
    }

    /// One 32-byte row, by word.
    #[allow(clippy::too_many_arguments)]
    fn raw_row(
        variant_id: u32,
        flags: u32,
        equipment_token_raw: u32,
        equipment_type_token_raw: u32,
        interface_type_raw: u32,
        air_superiority_weight_raw: i32,
        source_mask: u32,
        reserved: u32,
    ) -> Vec<u8> {
        let mut row = Vec::with_capacity(ROW_LEN);
        for word in [
            variant_id,
            flags,
            equipment_token_raw,
            equipment_type_token_raw,
            interface_type_raw,
            air_superiority_weight_raw as u32,
            source_mask,
            reserved,
        ] {
            row.extend_from_slice(&word.to_le_bytes());
        }
        row
    }

    fn compact_string(text: &str) -> Vec<u8> {
        let mut out = (text.len() as u16).to_le_bytes().to_vec();
        out.extend_from_slice(text.as_bytes());
        out
    }

    /// A whole payload: header, rows, then the three strings per row.
    fn payload_of(
        status_flags: u32,
        static_catalog_hash: u32,
        fixed_scale_raw: u32,
        rows: &[(Vec<u8>, [&str; 3])],
    ) -> Vec<u8> {
        let row_count = rows.len() as u32;
        let classified = rows
            .iter()
            .filter(|(row, tokens)| {
                tokens.iter().any(|t| !t.is_empty())
                    || u32_at(row, 4) & ROW_FLAG_AIR_SUPERIORITY_WEIGHT != 0
            })
            .count() as u32;
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&row_count.to_le_bytes());
        p.extend_from_slice(&row_count.to_le_bytes()); // changed_rows
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&row_count.to_le_bytes()); // observed_variant_rows
        p.extend_from_slice(&classified.to_le_bytes());
        p.extend_from_slice(&(row_count - classified).to_le_bytes());
        p.extend_from_slice(&static_catalog_hash.to_le_bytes());
        p.extend_from_slice(&fixed_scale_raw.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes()); // reserved0
        p.extend_from_slice(&0u32.to_le_bytes()); // reserved1
        for (row, _) in rows {
            p.extend_from_slice(row);
        }
        for (_, tokens) in rows {
            for token in tokens {
                p.extend_from_slice(&compact_string(token));
            }
        }
        p
    }

    /// A live row the fold accepts: live stats + a resolved dictionary token.
    fn live_row(variant_id: u32, token: &str) -> (Vec<u8>, [&str; 3]) {
        (
            raw_row(
                variant_id,
                ROW_FLAG_ARCHETYPE,
                20_000,
                1,
                7,
                0,
                SOURCE_LIVE_STATS | SOURCE_LIVE_TOKEN_DICTIONARY,
                0,
            ),
            [token, "", ""],
        )
    }

    // --- Step 8.1 ------------------------------------------------------------

    #[test]
    fn the_schema_validates() {
        schema().validate(GRID_LOG2).expect("schema is valid");
    }

    // --- Step 8.2 ------------------------------------------------------------

    #[test]
    fn a_full_row_round_trips_byte_identically() {
        let c = collection();
        let s = symbols();
        let row = raw_row(
            4711,
            ROW_FLAG_ARCHETYPE
                | ROW_FLAG_BASE_AIRFRAME
                | ROW_FLAG_ROLE
                | ROW_FLAG_AIR_SUPERIORITY_WEIGHT,
            20_000,
            368,
            27,
            250_000,
            SOURCE_LIVE_STATS
                | SOURCE_LIVE_DEFINITION
                | SOURCE_STATIC_CATALOG
                | SOURCE_STATIC_SOURCE_ENUM
                | SOURCE_LIVE_TOKEN_DICTIONARY,
            0,
        );
        let record = record_from_row(
            &c,
            &row,
            0,
            RowTokens {
                archetype: "small_plane_cas_airframe_2",
                base_airframe: "small_plane_airframe_2",
                role: "fighter",
            },
            HeaderStamp {
                static_catalog_hash: 0xdead_beef,
                fixed_scale_raw: AUTHORITY_FIXED_SCALE_RAW,
            },
            &s,
        )
        .expect("maps");

        let mut state = KeyedState::new();
        state.insert(key_from_row(&row).expect("keys"), record.clone());
        let schema = schema();
        let mut first = Vec::new();
        encode_baseline(&schema, &[state], &mut first).expect("encodes");
        let (decoded, consumed) = decode_baseline(&schema, &first, 0).expect("decodes");
        assert_eq!(consumed, first.len(), "the baseline consumed exactly");
        let mut second = Vec::new();
        encode_baseline(&schema, &decoded, &mut second).expect("re-encodes");
        assert_eq!(first, second, "encode -> decode -> encode is byte-identical");
        assert_eq!(decoded[0].values().next().expect("one row"), &record);
    }

    // --- Step 8.3: every state the schema declares, through the MAPPER --------

    #[test]
    fn the_schema_declares_unknown_and_nothing_else() {
        // The capability set is the whole four-state claim, so it is asserted
        // rather than described: no field is `.optional()` (nothing here is
        // Absent — no gate makes a concept inapplicable to an air variant) and no
        // field is `.nullable()` (no gate is a relation known to point at
        // nothing). Four fields are `.unknown_capable()`. The presence and null
        // bitmaps are therefore zero bytes wide on every record.
        let c = collection();
        assert_eq!(c.optional_count(), 0, "no field is optional");
        assert!(
            c.value_fields().all(|f| !f.nullable),
            "no field is nullable"
        );
        let unknown: Vec<u32> = c
            .value_fields()
            .filter(|f| f.unknown_capable)
            .map(|f| f.field_id)
            .collect();
        assert_eq!(
            unknown,
            vec![
                field::AIR_SUPERIORITY_WEIGHT_RAW,
                field::ARCHETYPE_TOKEN,
                field::BASE_AIRFRAME_TOKEN,
                field::ROLE_TOKEN,
            ]
        );
    }

    #[test]
    fn a_clear_gate_maps_to_unknown_through_the_mapper() {
        let c = collection();
        let s = symbols();
        // Every gate clear: the collector resolved no token and never reads the
        // weight (`attach_log.rs:53768`).
        let row = raw_row(90, 0, 20_000, 1, 0, 0, SOURCE_LIVE_STATS, 0);
        let r = record_from_row(
            &c,
            &row,
            0,
            RowTokens::default(),
            HeaderStamp {
                static_catalog_hash: 0,
                fixed_scale_raw: AUTHORITY_FIXED_SCALE_RAW,
            },
            &s,
        )
        .expect("maps");
        for id in [
            field::AIR_SUPERIORITY_WEIGHT_RAW,
            field::ARCHETYPE_TOKEN,
            field::BASE_AIRFRAME_TOKEN,
            field::ROLE_TOKEN,
        ] {
            assert_eq!(r.values[index_of(&c, id)], Value::Unknown, "field {id}");
        }
    }

    #[test]
    fn a_real_zero_is_a_value_not_an_empty_state() {
        // record-state contract keeps zero out of the three empty states. `interface_type_raw == 0`
        // is a real engine code the authority admits (`:294`-`:298`), and a
        // present `air_superiority_weight_raw` of 0 is a measured zero, not the
        // clear-gate Unknown.
        let c = collection();
        let s = symbols();
        let row = raw_row(
            91,
            ROW_FLAG_AIR_SUPERIORITY_WEIGHT,
            20_000,
            1,
            0,
            0,
            SOURCE_LIVE_STATS,
            0,
        );
        let r = record_from_row(
            &c,
            &row,
            0,
            RowTokens::default(),
            HeaderStamp {
                static_catalog_hash: 0,
                fixed_scale_raw: AUTHORITY_FIXED_SCALE_RAW,
            },
            &s,
        )
        .expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::INTERFACE_TYPE_RAW)],
            Value::Unsigned(0)
        );
        assert_eq!(
            r.values[index_of(&c, field::AIR_SUPERIORITY_WEIGHT_RAW)],
            Value::Signed(0)
        );
    }

    #[test]
    fn a_negative_weight_maps_because_the_fold_accepts_one() {
        // The FOLD bounds nothing (`world_fold.rs::fold_air_variant_archetypes` reads a plain `i32`); the
        // payload authority's `0..=1_000_000` is a separate boundary. Never gate
        // against the fold — d36's rule.
        let c = collection();
        let s = symbols();
        let row = raw_row(
            92,
            ROW_FLAG_AIR_SUPERIORITY_WEIGHT,
            20_000,
            1,
            0,
            -3,
            SOURCE_LIVE_STATS,
            0,
        );
        let r = record_from_row(
            &c,
            &row,
            0,
            RowTokens::default(),
            HeaderStamp {
                static_catalog_hash: 0,
                fixed_scale_raw: AUTHORITY_FIXED_SCALE_RAW,
            },
            &s,
        )
        .expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::AIR_SUPERIORITY_WEIGHT_RAW)],
            Value::Signed(-3)
        );
        assert!(
            check_authority_payload(&payload_of(
                0,
                0,
                AUTHORITY_FIXED_SCALE_RAW,
                &[(row, ["", "", ""])],
            ))
            .is_err(),
            "the authority, a separate boundary, refuses it"
        );
    }

    // --- Step 8.4: quantization ----------------------------------------------

    #[test]
    fn no_field_is_quantized_and_that_is_recorded() {
        // There is no continuous field to quantize. The one measured quantity,
        // `air_superiority_weight_raw`, is a fixed-point word over a divisor the
        // HEADER carries and the fold does not pin (`world_fold.rs::fold_air_variant_archetypes` rejects
        // zero only), so it is carried RAW beside `fixed_scale_raw` per d61's rule
        // rather than projected into a float — which also makes it exactly the
        // integer the fold stores. Nothing is drawn-rather-than-interpolated here.
        let c = collection();
        assert!(
            c.fields.iter().all(|f| f.kind != Kind::Continuous),
            "no continuous field"
        );
        assert!(
            c.fields.iter().all(|f| f.quant == Quant::None),
            "and therefore no quantizer"
        );
    }

    // --- Step 8.5: shape and key ---------------------------------------------

    #[test]
    fn the_shape_and_key_are_the_folds() {
        let c = collection();
        assert_eq!(c.shape, Shape::KeyedSet);
        let keys = c.key_fields();
        assert_eq!(keys.len(), 1, "one key component");
        assert_eq!(keys[0].field_id, field::VARIANT_ID);
        assert_eq!(keys[0].kind, Kind::Unsigned);
        assert!(keys[0].kind.legal_as_key());
        assert!(
            !keys[0].optional && !keys[0].nullable && !keys[0].unknown_capable,
            "a key member is concrete"
        );
        assert_eq!(schema().collections.len(), 1, "the ace roster is stopped");
        assert_eq!(ACE_ROSTER, 2, "and its collection id is reserved");
    }

    #[test]
    fn no_field_is_a_content_id_key() {
        // "Checked, none" rather than silence: a ContentId KEY is the heavier codec-measurement contract
        // retrofit class (a manifest remap reorders keys), and this domain has
        // none. It does carry three ContentId VALUES, listed in the open questions.
        let c = collection();
        assert!(
            c.key_fields().iter().all(|f| f.kind != Kind::ContentId),
            "no ContentId key"
        );
        let content: Vec<u32> = c
            .value_fields()
            .filter(|f| f.kind == Kind::ContentId)
            .map(|f| f.field_id)
            .collect();
        assert_eq!(
            content,
            vec![
                field::ARCHETYPE_TOKEN,
                field::BASE_AIRFRAME_TOKEN,
                field::ROLE_TOKEN
            ]
        );
    }

    // --- Step 8.6: every offset, and every header word -----------------------

    #[test]
    fn every_row_offset_maps_to_its_own_field() {
        // Distinct known bytes at every offset, so a transposition fails here even
        // though no round-trip or replay assertion could see it.
        let c = collection();
        let s = Symbols::from_vocabulary(["A", "B", "C"]);
        let row = raw_row(
            0x1111_1111,
            ROW_FLAG_ARCHETYPE
                | ROW_FLAG_BASE_AIRFRAME
                | ROW_FLAG_ROLE
                | ROW_FLAG_AIR_SUPERIORITY_WEIGHT,
            0x3333_3333,
            0x4444_4444,
            0x5555_5555,
            0x6666_6666,
            SOURCE_LIVE_STATS
                | SOURCE_LIVE_DEFINITION
                | SOURCE_STATIC_CATALOG
                | SOURCE_STATIC_SOURCE_ENUM
                | SOURCE_LIVE_TOKEN_DICTIONARY,
            0,
        );
        assert_eq!(
            key_from_row(&row).expect("keys"),
            key(&[0x1111_1111]),
            "the key is the first word"
        );
        let r = record_from_row(
            &c,
            &row,
            0,
            RowTokens {
                archetype: "A",
                base_airframe: "B",
                role: "C",
            },
            HeaderStamp {
                static_catalog_hash: 0x7777_7777,
                fixed_scale_raw: 0x8888_8888,
            },
            &s,
        )
        .expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::EQUIPMENT_TOKEN_RAW)],
            Value::Unsigned(0x3333_3333)
        );
        assert_eq!(
            r.values[index_of(&c, field::EQUIPMENT_TYPE_TOKEN_RAW)],
            Value::Unsigned(0x4444_4444)
        );
        assert_eq!(
            r.values[index_of(&c, field::INTERFACE_TYPE_RAW)],
            Value::Unsigned(0x5555_5555)
        );
        assert_eq!(
            r.values[index_of(&c, field::AIR_SUPERIORITY_WEIGHT_RAW)],
            Value::Signed(0x6666_6666)
        );
        assert_eq!(
            r.values[index_of(&c, field::ARCHETYPE_TOKEN)],
            Value::ContentId(0)
        );
        assert_eq!(
            r.values[index_of(&c, field::BASE_AIRFRAME_TOKEN)],
            Value::ContentId(1)
        );
        assert_eq!(r.values[index_of(&c, field::ROLE_TOKEN)], Value::ContentId(2));
        assert_eq!(
            r.values[index_of(&c, field::STATIC_CATALOG_HASH)],
            Value::Unsigned(0x7777_7777)
        );
        assert_eq!(
            r.values[index_of(&c, field::FIXED_SCALE_RAW)],
            Value::Unsigned(0x8888_8888)
        );
        for (id, bit) in [
            (field::LIVE_DEFINITION, SOURCE_LIVE_DEFINITION),
            (field::STATIC_CATALOG, SOURCE_STATIC_CATALOG),
            (field::LIVE_TOKEN_DICTIONARY, SOURCE_LIVE_TOKEN_DICTIONARY),
        ] {
            assert_eq!(r.values[index_of(&c, id)], Value::Bool(true), "bit {bit:#x}");
        }
    }

    #[test]
    fn every_header_word_is_read_from_its_own_offset() {
        // A transposed header word mis-slices every row behind it, and both arms
        // of a round-trip agree on the wrong slice, so this is the only test that
        // can see it (d31's rule).
        let mut p = vec![0u8; HEADER_LEN];
        p[0..8].copy_from_slice(&0x0102_0304_0506_0708u64.to_le_bytes());
        p[8..12].copy_from_slice(&0u32.to_le_bytes()); // row_count
        p[12..16].copy_from_slice(&0u32.to_le_bytes()); // changed_rows
        p[16..20].copy_from_slice(&0x0000_0400u32.to_le_bytes()); // status_flags
        p[20..24].copy_from_slice(&0x2222_2222u32.to_le_bytes()); // observed
        p[24..28].copy_from_slice(&0u32.to_le_bytes()); // classified
        p[28..32].copy_from_slice(&0u32.to_le_bytes()); // unresolved
        p[32..36].copy_from_slice(&0x3333_3333u32.to_le_bytes()); // catalog hash
        p[36..40].copy_from_slice(&0x4444_4444u32.to_le_bytes()); // fixed scale
        let header = peek_header(&p).expect("header parses");
        assert_eq!(header.elapsed_wall_ms, 0x0102_0304_0506_0708);
        assert_eq!(header.row_count, 0);
        assert_eq!(header.changed_rows, 0);
        assert_eq!(header.status_flags, STATUS_DELTA_DATA);
        assert!(header.declares_delta());
        assert_eq!(header.observed_variant_rows, 0x2222_2222);
        assert_eq!(header.classified_rows, 0);
        assert_eq!(header.unresolved_rows, 0);
        assert_eq!(header.static_catalog_hash, 0x3333_3333);
        assert_eq!(header.fixed_scale_raw, 0x4444_4444);
        assert_eq!(
            header.stamp(),
            HeaderStamp {
                static_catalog_hash: 0x3333_3333,
                fixed_scale_raw: 0x4444_4444
            }
        );
        assert_eq!(header.reserved0, 0);
        assert_eq!(header.reserved1, 0);
    }

    // --- The fold's own checks -----------------------------------------------

    #[test]
    fn the_folds_header_checks_are_exactly_the_folds() {
        // Zero fixed scale is refused (`:16336`) but a scale the AUTHORITY would
        // refuse is folded, because the fold does not compare against 100_000.
        let mut p = payload_of(0, 0, 7, &[live_row(1, "fighter")]);
        assert!(peek_header(&p).is_ok(), "the fold accepts fixed_scale 7");
        assert!(
            check_authority_payload(&p).is_err(),
            "the authority does not"
        );
        p[36..40].copy_from_slice(&0u32.to_le_bytes());
        assert!(peek_header(&p).is_err(), "zero is refused by both");

        // classified + unresolved must equal row_count (`:16333`).
        let mut p = payload_of(0, 0, AUTHORITY_FIXED_SCALE_RAW, &[live_row(1, "fighter")]);
        p[24..28].copy_from_slice(&5u32.to_le_bytes());
        assert!(peek_header(&p).is_err());

        // Both reserved words must be zero (`:16339`).
        for offset in [40usize, 44] {
            let mut p = payload_of(0, 0, AUTHORITY_FIXED_SCALE_RAW, &[live_row(1, "fighter")]);
            p[offset..offset + 4].copy_from_slice(&1u32.to_le_bytes());
            assert!(peek_header(&p).is_err(), "reserved at {offset}");
        }
    }

    #[test]
    fn changed_rows_is_parsed_and_reported_but_never_enforced() {
        // This fold never reads the word (contrast d63's `== row_count`), so
        // `peek_header`/`split_payload` must not enforce it. The AUTHORITY bounds
        // it, and that is where the bound lives.
        let mut p = payload_of(
            0,
            0,
            AUTHORITY_FIXED_SCALE_RAW,
            &[live_row(1, "fighter"), live_row(2, "fighter")],
        );
        p[12..16].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(peek_header(&p).expect("folds").changed_rows, 0);
        assert!(decode_payload(&p).is_ok());
        assert!(check_authority_payload(&p).is_ok(), "0 <= row_count");
        p[12..16].copy_from_slice(&99u32.to_le_bytes());
        assert!(decode_payload(&p).is_ok(), "still not the fold's business");
        assert!(check_authority_payload(&p).is_err(), "but the authority's");
    }

    #[test]
    fn a_deleted_row_is_a_tombstone_and_must_carry_nothing() {
        let c = collection();
        let s = symbols();
        let empty = raw_row(7, ROW_FLAG_DELETED, 0, 0, 0, 0, 0, 0);
        let parsed = decode_payload(&payload_of(
            0,
            0,
            AUTHORITY_FIXED_SCALE_RAW,
            &[(empty.clone(), ["", "", ""])],
        ))
        .expect("decodes");
        assert!(parsed.rows[0].is_deleted());
        assert!(
            record_from_parsed_row(
                &c,
                &parsed.rows[0],
                HeaderStamp {
                    static_catalog_hash: 0,
                    fixed_scale_raw: AUTHORITY_FIXED_SCALE_RAW
                },
                &s
            )
            .is_err(),
            "a tombstone is not a record"
        );
        // Every word the fold checks (`:16404`-`:16411`), one at a time.
        for word in 2..=6usize {
            let mut row = empty.clone();
            row[word * 4..word * 4 + 4].copy_from_slice(&1u32.to_le_bytes());
            let p = payload_of(0, 0, AUTHORITY_FIXED_SCALE_RAW, &[(row, ["", "", ""])]);
            assert!(decode_payload(&p).is_err(), "deleted row word {word}");
        }
        // ... and the three strings, which need their flags to be legal at all.
        let row = raw_row(7, ROW_FLAG_DELETED | ROW_FLAG_ARCHETYPE, 0, 0, 0, 0, 0, 0);
        let p = payload_of(0, 0, AUTHORITY_FIXED_SCALE_RAW, &[(row, ["x", "", ""])]);
        assert!(decode_payload(&p).is_err(), "deleted row carrying a string");
    }

    #[test]
    fn the_source_authority_rules_are_the_folds() {
        let hash = 0xfeed_face;
        // Live stats are required (`:16420`).
        let row = raw_row(1, 0, 20_000, 1, 0, 0, SOURCE_LIVE_DEFINITION, 0);
        assert!(check_row(&parse_one(&row, ["", "", ""]), hash).is_err());
        // Heuristic and rejected-save are refused (`:16425`).
        for bit in [SOURCE_STATIC_HEURISTIC, SOURCE_DEV_SAVE_JOIN_REJECTED] {
            let row = raw_row(1, 0, 20_000, 1, 0, 0, SOURCE_LIVE_STATS | bit, 0);
            assert!(check_row(&parse_one(&row, ["", "", ""]), hash).is_err());
        }
        // A fabricated raw-token identity is refused (`:16435`).
        let row = raw_row(
            1,
            ROW_FLAG_ARCHETYPE,
            20_000,
            1,
            0,
            0,
            SOURCE_LIVE_STATS | SOURCE_LIVE_TOKEN_DICTIONARY,
            0,
        );
        assert!(
            check_row(&parse_one(&row, ["equipment_token_raw_20000", "", ""]), hash).is_err()
        );
        // The dictionary bit requires the token, and a token requires an authority
        // (`:16450`).
        let row = raw_row(
            1,
            0,
            20_000,
            1,
            0,
            0,
            SOURCE_LIVE_STATS | SOURCE_LIVE_TOKEN_DICTIONARY,
            0,
        );
        assert!(check_row(&parse_one(&row, ["", "", ""]), hash).is_err());
        let row = raw_row(1, ROW_FLAG_ARCHETYPE, 20_000, 1, 0, 0, SOURCE_LIVE_STATS, 0);
        assert!(check_row(&parse_one(&row, ["fighter", "", ""]), hash).is_err());
    }

    #[test]
    fn static_source_enum_is_a_function_of_the_two_static_tokens() {
        // The identity the module docs claim, swept over every combination of the
        // two static tokens and the STATIC_SOURCE_ENUM bit: exactly the agreeing
        // combinations are accepted, which is what makes the bit not a field.
        let hash = 0x1234_5678;
        for base in [false, true] {
            for role in [false, true] {
                for enum_bit in [false, true] {
                    let mut flags = ROW_FLAG_ARCHETYPE;
                    if base {
                        flags |= ROW_FLAG_BASE_AIRFRAME;
                    }
                    if role {
                        flags |= ROW_FLAG_ROLE;
                    }
                    let mut mask = SOURCE_LIVE_STATS | SOURCE_STATIC_CATALOG;
                    if enum_bit {
                        mask |= SOURCE_STATIC_SOURCE_ENUM;
                    }
                    let row = raw_row(1, flags, 20_000, 1, 0, 0, mask, 0);
                    let parsed = parse_one(
                        &row,
                        [
                            "fighter",
                            if base { "airframe" } else { "" },
                            if role { "role" } else { "" },
                        ],
                    );
                    let accepted = check_row(&parsed, hash).is_ok();
                    assert_eq!(
                        accepted,
                        enum_bit == (base || role),
                        "base={base} role={role} enum={enum_bit}"
                    );
                    if accepted {
                        assert_eq!(parsed.carries_static_semantics(), enum_bit);
                    }
                }
            }
        }
    }

    #[test]
    fn an_undefined_source_mask_bit_is_refused_rather_than_dropped() {
        // The recorded divergence: the schema decomposes `source_mask` into one
        // Boolean per DEFINED bit, so bits 7..=31 have nowhere to go and are
        // refused rather than silently dropped. Production's status-independent
        // payload authority refuses the same payload, so no certified stream
        // carries one. Swept over every undefined bit.
        let c = collection();
        let s = symbols();
        for bit in 7..32u32 {
            let mask = SOURCE_LIVE_STATS | (1 << bit);
            let row = raw_row(1, 0, 20_000, 1, 0, 0, mask, 0);
            assert!(
                record_from_row(
                    &c,
                    &row,
                    0,
                    RowTokens::default(),
                    HeaderStamp {
                        static_catalog_hash: 0,
                        fixed_scale_raw: AUTHORITY_FIXED_SCALE_RAW
                    },
                    &s
                )
                .is_err(),
                "bit {bit} is refused by the mapper"
            );
            let p = payload_of(0, 0, AUTHORITY_FIXED_SCALE_RAW, &[(row, ["", "", ""])]);
            assert!(
                check_authority_payload(&p).is_err(),
                "and by production's payload authority"
            );
        }
    }

    #[test]
    fn a_symbol_outside_the_table_is_a_diagnosis_not_an_unknown() {
        let c = collection();
        let row = raw_row(
            1,
            ROW_FLAG_ARCHETYPE,
            20_000,
            1,
            0,
            0,
            SOURCE_LIVE_STATS | SOURCE_LIVE_TOKEN_DICTIONARY,
            0,
        );
        let err = record_from_row(
            &c,
            &row,
            0,
            RowTokens {
                archetype: "not_in_the_content_set",
                ..RowTokens::default()
            },
            HeaderStamp {
                static_catalog_hash: 0,
                fixed_scale_raw: AUTHORITY_FIXED_SCALE_RAW,
            },
            &Symbols::empty(),
        )
        .expect_err("refused");
        assert!(err.contains("symbol table"), "{err}");
    }

    #[test]
    fn a_record_built_for_the_wrong_collection_is_refused() {
        let mut c = collection();
        c.collection_id = ACE_ROSTER;
        let row = raw_row(1, 0, 20_000, 1, 0, 0, SOURCE_LIVE_STATS, 0);
        assert!(record_from_row(
            &c,
            &row,
            0,
            RowTokens::default(),
            HeaderStamp {
                static_catalog_hash: 0,
                fixed_scale_raw: AUTHORITY_FIXED_SCALE_RAW
            },
            &symbols()
        )
        .is_err());
    }

    // --- The accumulator -----------------------------------------------------

    #[test]
    fn the_first_folded_frame_is_the_baseline_and_later_frames_are_sparse() {
        let schema = schema();
        let s = symbols();
        let mut state = State::new();
        // Frame 1: two rows, DELTA_DATA set — and still the implicit baseline,
        // because the fold never reads the bit.
        let p = payload_of(
            STATUS_DELTA_DATA,
            0,
            AUTHORITY_FIXED_SCALE_RAW,
            &[live_row(1, "fighter"), live_row(2, "fighter")],
        );
        assert_eq!(
            state
                .fold_payload(&schema, &p, false, 0, &s)
                .expect("folds"),
            FoldOutcome::Folded
        );
        assert_eq!(state.archetypes.len(), 2);
        // Frame 2: one row. A sparse patch, so row 2 survives.
        let p = payload_of(
            STATUS_DELTA_DATA,
            0,
            AUTHORITY_FIXED_SCALE_RAW,
            &[live_row(3, "fighter")],
        );
        state
            .fold_payload(&schema, &p, false, 0, &s)
            .expect("folds");
        assert_eq!(state.archetypes.len(), 3, "sparse, not a replacement");
        // Frame 3: a tombstone removes a key.
        let p = payload_of(
            STATUS_DELTA_DATA,
            0,
            AUTHORITY_FIXED_SCALE_RAW,
            &[(raw_row(2, ROW_FLAG_DELETED, 0, 0, 0, 0, 0, 0), ["", "", ""])],
        );
        state
            .fold_payload(&schema, &p, false, 0, &s)
            .expect("folds");
        assert_eq!(state.archetypes.len(), 2);
        assert!(!state.archetypes.contains_key(&key(&[2])));
    }

    #[test]
    fn a_failure_status_record_folds_nothing_and_does_not_consume_the_baseline() {
        let schema = schema();
        let s = symbols();
        let mut state = State::new();
        let p = payload_of(
            STATUS_DELTA_DATA,
            0,
            AUTHORITY_FIXED_SCALE_RAW,
            &[live_row(1, "fighter")],
        );
        assert_eq!(
            state.fold_payload(&schema, &p, true, 0, &s).expect("folds"),
            FoldOutcome::FailureStatus
        );
        assert!(state.archetypes.is_empty());
        assert!(
            !state.baseline_seen,
            "the fold returns before implicit_delta_baseline_or_sparse"
        );
        // The next successful frame is therefore still the implicit baseline.
        let p = payload_of(
            STATUS_DELTA_DATA,
            0,
            AUTHORITY_FIXED_SCALE_RAW,
            &[live_row(9, "fighter")],
        );
        state
            .fold_payload(&schema, &p, false, 0, &s)
            .expect("folds");
        assert!(state.baseline_seen);
        assert_eq!(state.archetypes.len(), 1);
    }

    #[test]
    fn a_failure_status_record_is_not_read_past_its_header() {
        // The fold's status gate sits between the header checks and the row
        // length check (`:16342`-`:16351`), so a truncated body on a
        // failure-status record is an excluded hour, not a stream rejection —
        // for the FOLD. (Production's payload authority still refuses it; that
        // is a different boundary, and the harness runs it separately.)
        let schema = schema();
        let s = symbols();
        let mut p = payload_of(0, 0, AUTHORITY_FIXED_SCALE_RAW, &[live_row(1, "fighter")]);
        p.truncate(HEADER_LEN + 4);
        let mut state = State::new();
        assert_eq!(
            state.fold_payload(&schema, &p, true, 0, &s).expect("folds"),
            FoldOutcome::FailureStatus
        );
        assert!(
            state.fold_payload(&schema, &p, false, 0, &s).is_err(),
            "and a stream rejection when it is not excluded"
        );
        assert!(check_authority_payload(&p).is_err());
    }

    #[test]
    fn a_duplicate_variant_id_is_mirrored_by_the_fold_and_refused_by_the_authority() {
        // The FOLD collapses last-writer-wins into a `BTreeMap` (`:16512`), so
        // this module mirrors it rather than refusing (d22's ruling). Production's
        // payload authority refuses the frame status-independently, which is why
        // the harness runs that check on every record.
        let schema = schema();
        let s = symbols();
        let mut state = State::new();
        let first = live_row(5, "fighter");
        let mut second = live_row(5, "small_plane_airframe_2");
        second.0[8..12].copy_from_slice(&30_000u32.to_le_bytes());
        let p = payload_of(0, 0, AUTHORITY_FIXED_SCALE_RAW, &[first, second]);
        state
            .fold_payload(&schema, &p, false, 0, &s)
            .expect("the fold folds it");
        assert_eq!(state.archetypes.len(), 1, "last writer wins");
        let record = state.archetypes.get(&key(&[5])).expect("row 5");
        assert_eq!(
            record.values[index_of(&collection(), field::EQUIPMENT_TOKEN_RAW)],
            Value::Unsigned(30_000)
        );
        assert!(check_authority_payload(&p).is_err(), "the authority refuses");
    }

    #[test]
    fn the_header_stamp_is_per_row_state() {
        // A sparse frame rewrites the two stamped words only on the rows it
        // carries, which is why they are row fields and not one frame constant.
        let schema = schema();
        let s = symbols();
        let mut state = State::new();
        let p = payload_of(
            0,
            0,
            AUTHORITY_FIXED_SCALE_RAW,
            &[live_row(1, "fighter"), live_row(2, "fighter")],
        );
        state
            .fold_payload(&schema, &p, false, 0, &s)
            .expect("folds");
        // The fold accepts a different non-zero scale on a later frame.
        let p = payload_of(0, 0, 1_000, &[live_row(2, "fighter")]);
        state
            .fold_payload(&schema, &p, false, 0, &s)
            .expect("folds");
        let c = collection();
        let scale = index_of(&c, field::FIXED_SCALE_RAW);
        assert_eq!(
            state.archetypes.get(&key(&[1])).expect("row 1").values[scale],
            Value::Unsigned(u64::from(AUTHORITY_FIXED_SCALE_RAW)),
            "the untouched row keeps the old stamp"
        );
        assert_eq!(
            state.archetypes.get(&key(&[2])).expect("row 2").values[scale],
            Value::Unsigned(1_000)
        );
    }

    #[test]
    fn tokens_in_payload_sees_every_string_a_fold_would_store() {
        let p = payload_of(
            0,
            0xabcd,
            AUTHORITY_FIXED_SCALE_RAW,
            &[
                live_row(1, "fighter"),
                (
                    raw_row(
                        2,
                        ROW_FLAG_ARCHETYPE | ROW_FLAG_BASE_AIRFRAME | ROW_FLAG_ROLE,
                        20_000,
                        1,
                        0,
                        0,
                        SOURCE_LIVE_STATS
                            | SOURCE_LIVE_TOKEN_DICTIONARY
                            | SOURCE_STATIC_CATALOG
                            | SOURCE_STATIC_SOURCE_ENUM,
                        0,
                    ),
                    ["a", "b", "c"],
                ),
            ],
        );
        let mut tokens = tokens_in_payload(&p).expect("walks");
        tokens.sort();
        assert_eq!(tokens, vec!["a", "b", "c", "fighter"]);
    }

    // --- The stop-reported `ACE1` trailer ------------------------------------

    fn ace_row(tag_db_id: u32, ace_id: u32, flags: u32, source_mask: u32) -> Vec<u8> {
        let mut row = Vec::with_capacity(ace::TRAILER_ROW_LEN_V2);
        for word in [tag_db_id, ace_id, flags, source_mask, 1, 0, 0, 0, 0, 0] {
            row.extend_from_slice(&(word as u32).to_le_bytes());
        }
        row
    }

    /// A clean v2 trailer with one living, unassigned ace.
    fn ace_trailer(status_flags: u32, rows: &[(Vec<u8>, [&str; 3])]) -> Vec<u8> {
        let mut t = Vec::new();
        t.extend_from_slice(&ace::TRAILER_MAGIC);
        t.extend_from_slice(&ace::TRAILER_VERSION_V2.to_le_bytes());
        t.extend_from_slice(&(ace::TRAILER_HEADER_LEN as u16).to_le_bytes());
        t.extend_from_slice(&(ace::TRAILER_ROW_LEN_V2 as u16).to_le_bytes());
        t.extend_from_slice(&0u16.to_le_bytes());
        t.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        t.extend_from_slice(&status_flags.to_le_bytes());
        t.extend_from_slice(&1u32.to_le_bytes()); // observed_country_rows
        t.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // observed_ace_rows
        t.extend_from_slice(&0u32.to_le_bytes()); // wing_assignment_rows
        t.extend_from_slice(&0u32.to_le_bytes()); // dead_rows
        t.extend_from_slice(&0u32.to_le_bytes()); // skipped_rows
        for (row, _) in rows {
            t.extend_from_slice(row);
        }
        for (_, strings) in rows {
            for s in strings {
                t.extend_from_slice(&compact_string(s));
            }
        }
        t
    }

    fn living_ace(tag_db_id: u32, ace_id: u32) -> (Vec<u8>, [&'static str; 3]) {
        (
            ace_row(
                tag_db_id,
                ace_id,
                ace::ROW_FLAG_NAME
                    | ace::ROW_FLAG_SURNAME
                    | ace::ROW_FLAG_CALLSIGN
                    | ace::ROW_FLAG_ALIVE_KNOWN
                    | ace::ROW_FLAG_HANDLED_KNOWN
                    | ace::ROW_FLAG_KILL_TYPE_KNOWN,
                ace::SOURCE_LIVE_COUNTRY_ACE_VECTOR
                    | ace::SOURCE_LIVE_ACE_OBJECT_LAYOUT
                    | ace::SOURCE_LIVE_ACE_DEATH_RECORD
                    | ace::SOURCE_LIVE_ACE_LIFECYCLE_RECORD,
            ),
            ["Tobias", "Ochsenhirt", "Fighter of Libau"],
        )
    }

    #[test]
    fn a_well_formed_ace_trailer_is_validated_and_counted_never_mapped() {
        let mut p = payload_of(0, 0, AUTHORITY_FIXED_SCALE_RAW, &[live_row(1, "fighter")]);
        let base_len = p.len();
        let trailer = ace_trailer(0, &[living_ace(1, 5001), living_ace(1, 5002)]);
        p.extend_from_slice(&trailer);
        let decoded = decode_payload(&p).expect("decodes");
        assert_eq!(decoded.base_len, base_len);
        let ace_frame = decoded.ace.expect("a trailer is present");
        assert_eq!(ace_frame.row_count, 2);
        assert_eq!(ace_frame.bytes, trailer.len());
        assert!(!ace_frame.retains_prior_state());
        assert!(check_authority_payload(&p).is_ok());
        // No collection carries it.
        assert_eq!(schema().collections.len(), 1);
    }

    #[test]
    fn a_malformed_ace_trailer_refuses_the_record_at_both_authorities() {
        let good = {
            let mut p = payload_of(0, 0, AUTHORITY_FIXED_SCALE_RAW, &[live_row(1, "fighter")]);
            p.extend_from_slice(&ace_trailer(0, &[living_ace(1, 5001)]));
            p
        };
        assert!(decode_payload(&good).is_ok());
        assert!(check_authority_payload(&good).is_ok());
        // Every single-byte corruption of the trailer either refuses or decodes
        // identically; none may be accepted with different counts.
        let base_len = decode_payload(&good).expect("decodes").base_len;
        let mut refused = 0usize;
        for index in base_len..good.len() {
            for bit in 0..8 {
                let mut bad = good.clone();
                bad[index] ^= 1 << bit;
                match decode_payload(&bad) {
                    Err(_) => {
                        refused += 1;
                        assert!(
                            check_authority_payload(&bad).is_err(),
                            "the authority refuses byte {index} bit {bit} too"
                        );
                    }
                    Ok(decoded) => {
                        // The only bits with no meaning are inside the free-text
                        // bytes, which the stop does not carry.
                        assert_eq!(decoded.ace.expect("trailer").row_count, 1);
                    }
                }
            }
        }
        assert!(refused > 0, "the sweep exercised the refusal path");
    }

    #[test]
    fn trailing_bytes_that_are_not_a_trailer_refuse_the_record() {
        let mut p = payload_of(0, 0, AUTHORITY_FIXED_SCALE_RAW, &[live_row(1, "fighter")]);
        p.extend_from_slice(b"NOPE");
        assert!(decode_payload(&p).is_err());
        assert!(check_authority_payload(&p).is_err());
    }

    #[test]
    fn an_incomplete_ace_roster_retains_prior_state() {
        // `world_fold.rs::apply_air_ace_roster`. This is the second, independent blocker
        // in the stop report: a keyed set cannot express "membership was not
        // measured this hour".
        let schema = schema();
        let s = symbols();
        let mut state = State::new();
        let mut p = payload_of(0, 0, AUTHORITY_FIXED_SCALE_RAW, &[live_row(1, "fighter")]);
        p.extend_from_slice(&ace_trailer(0, &[living_ace(1, 5001), living_ace(1, 5002)]));
        state
            .fold_payload(&schema, &p, false, 0, &s)
            .expect("folds");
        assert_eq!(state.ace_rows, 2);
        assert_eq!(state.ace_trailers_replaced, 1);

        for status in [ace::STATUS_PARTIAL_DATA, ace::STATUS_SOURCE_UNAVAILABLE] {
            let mut p = payload_of(0, 0, AUTHORITY_FIXED_SCALE_RAW, &[live_row(1, "fighter")]);
            // SOURCE_UNAVAILABLE must carry zero rows; PARTIAL keeps its
            // survivors. Either way the fold retains.
            let rows: Vec<(Vec<u8>, [&str; 3])> = if status == ace::STATUS_SOURCE_UNAVAILABLE {
                vec![]
            } else {
                vec![living_ace(1, 5001)]
            };
            p.extend_from_slice(&ace_trailer(status, &rows));
            state
                .fold_payload(&schema, &p, false, 0, &s)
                .expect("folds");
            assert_eq!(state.ace_rows, 2, "prior roster retained for {status:#x}");
        }
        assert_eq!(state.ace_trailers_retained, 2);
        assert_eq!(state.ace_trailers_seen, 3);
    }

    #[test]
    fn source_unavailable_must_carry_no_rows() {
        let mut p = payload_of(0, 0, AUTHORITY_FIXED_SCALE_RAW, &[live_row(1, "fighter")]);
        p.extend_from_slice(&ace_trailer(
            ace::STATUS_SOURCE_UNAVAILABLE,
            &[living_ace(1, 5001)],
        ));
        assert!(decode_payload(&p).is_err());
    }

    #[test]
    fn the_ace_stop_hands_forward_its_offsets_and_bounds() {
        // The facts the stop report promises, pinned so they cannot drift out of
        // the module docs: the two row lengths, the header length, the version
        // pair, and the row word offsets a gap contract implementer will slice by.
        assert_eq!(ace::TRAILER_HEADER_LEN, 40);
        assert_eq!(ace::TRAILER_ROW_LEN_V1, 32);
        assert_eq!(ace::TRAILER_ROW_LEN_V2, 40);
        assert_eq!(ace::TRAILER_MAGIC, *b"ACE1");
        assert_eq!(
            [
                ace::row_offset::TAG_DB_ID,
                ace::row_offset::ACE_ID,
                ace::row_offset::FLAGS,
                ace::row_offset::SOURCE_MASK,
                ace::row_offset::COUNTRY_ACE_INDEX,
                ace::row_offset::WING_ID,
                ace::row_offset::KILLER_ACE_ID,
                ace::row_offset::KILLER_COUNTRY,
                ace::row_offset::KILL_TYPE_RAW,
                ace::row_offset::RESERVED,
            ],
            [0, 4, 8, 12, 16, 20, 24, 28, 32, 36]
        );
        assert_eq!(ace::KEY_FIELDS, ["tag_db_id", "ace_id"]);
        assert_eq!(ace::MAX_ACE_ROWS, 65_536);
        assert_eq!(ace::MAX_STRING_LEN, 288);
    }

    // --- Totality ------------------------------------------------------------

    #[test]
    fn no_byte_string_panics_through_any_public_entry() {
        let good = {
            let mut p = payload_of(
                STATUS_DELTA_DATA,
                0,
                AUTHORITY_FIXED_SCALE_RAW,
                &[live_row(1, "fighter")],
            );
            p.extend_from_slice(&ace_trailer(0, &[living_ace(1, 5001)]));
            p
        };
        let schema = schema();
        let s = symbols();
        // Every truncation.
        for len in 0..=good.len() {
            let slice = &good[..len];
            let _ = peek_header(slice);
            let _ = split_payload(slice);
            let _ = decode_payload(slice);
            let _ = tokens_in_payload(slice);
            let _ = check_authority_payload(slice);
            let _ = decode_ace_trailer(slice, len / 2);
            let _ = State::new().fold_payload(&schema, slice, false, 0, &s);
            let _ = State::new().fold_payload(&schema, slice, true, 0, &s);
        }
        // Every single-byte corruption of the whole payload.
        for index in 0..good.len() {
            let mut bad = good.clone();
            bad[index] ^= 0xff;
            let _ = decode_payload(&bad);
            let _ = check_authority_payload(&bad);
            let _ = State::new().fold_payload(&schema, &bad, false, 0, &s);
        }
        // Row-level entries over every short slice.
        let row = raw_row(1, 0, 20_000, 1, 0, 0, SOURCE_LIVE_STATS, 0);
        for len in 0..=row.len() {
            let _ = key_from_row(&row[..len]);
            let _ = variant_id_of_row(&row[..len]);
            let _ = record_from_row(
                &schema.collections[0],
                &row[..len],
                0,
                RowTokens::default(),
                HeaderStamp {
                    static_catalog_hash: 0,
                    fixed_scale_raw: AUTHORITY_FIXED_SCALE_RAW,
                },
                &s,
            );
        }
    }

    /// Decode a one-row payload and hand back its parsed row.
    fn parse_one(row: &[u8], tokens: [&str; 3]) -> ParsedRow {
        let mut parsed = ParsedRow {
            variant_id: u32_at(row, 0),
            flags: u32_at(row, 4),
            equipment_token_raw: u32_at(row, 8),
            equipment_type_token_raw: u32_at(row, 12),
            interface_type_raw: u32_at(row, 16),
            air_superiority_weight_raw: i32_at(row, 20),
            source_mask: u32_at(row, 24),
            archetype_token: None,
            base_airframe_token: None,
            role_token: None,
        };
        parsed.archetype_token = (!tokens[0].is_empty()).then(|| tokens[0].to_string());
        parsed.base_airframe_token = (!tokens[1].is_empty()).then(|| tokens[1].to_string());
        parsed.role_token = (!tokens[2].is_empty()).then(|| tokens[2].to_string());
        parsed
    }
}
