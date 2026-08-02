//! d96 `country_preferred_tactic_weight_live` — the multiplier the land-combat
//! consumer applies to each country's selected combat tactic.
//!
//! Derived from `world_fold.rs:10768`-`:10806`
//! (`fold_country_preferred_tactic_weights`, dispatched at `:4763`) together with
// @world-fold-authority domain=96 functions=fold_country_preferred_tactic_weights lines=10768-10806 sha256=17f387e1fca03b7d7b389c122b989b579a6f8f2b0b3de733f573c9551c36793c
//! the deferred fold it delegates to,
//! `extractor/native/recorder_host/src/tactic_weight_fold.rs:32`-`:80`
//! (`TacticWeightFoldState::fold_payload`); its payload codec
//! `extractor/native/compact_contract/src/tactic_weight.rs`; and the collector
//! read that produces every row
//! (`extractor/native/collector/src/attach_log.rs:32845`-`:33020`,
//! `:33061`-`:33101`). `extractor/documents/domains/` carries no d96 dossier, so
//! the fold, the codec and the collector read are the whole basis.
//!
//! **Payload grammar** (`tactic_weight.rs:162`-`:287`). A 276-byte header:
//!
//! | offset | width | header word |
//! |---|---|---|
//! | `+0`   | u64      | `read_time_us` — capture diagnostic, not world state |
//! | `+8`   | u32      | `observed_country_count` — the D92 roster witness |
//! | `+12`  | u32      | `row_count` |
//! | `+16`  | u32      | `status_flags` |
//! | `+20`  | 4 bytes  | `MAGIC` = `PTW1` |
//! | `+24`  | u16      | `VERSION` = 1 |
//! | `+26`  | u16      | `HEADER_LEN` = 276 |
//! | `+28`  | u64      | `source_sequence` — must equal D92's |
//! | `+36`  | u32      | `FIXED_SCALE` = 100_000, asserted |
//! | `+40`  | u32      | reserved, must be 0 |
//! | `+44`  | i64      | `country_factor_raw` |
//! | `+52`  | 32 bytes | `EXECUTABLE_SHA256` |
//! | `+84`  | 32 bytes | `CONSUMER_EVIDENCE_SHA256` |
//! | `+116` | 32 bytes | `DEFINE_EVIDENCE_SHA256` |
//! | `+148` | 32 bytes | `LOOKUP_EVIDENCE_SHA256` |
//! | `+180` | 32 bytes | `RUNTIME_OBSERVATION_SHA256` |
//! | `+212` | 32 bytes | `TACTIC_KEY_CENSUS_SHA256` |
//! | `+244` | 32 bytes | `NONZERO_OVERRIDE_OBSERVATION_SHA256` |
//!
//! followed by `row_count` **variable-length** rows — `tag_db_id` u32 at `+0`,
//! `tactic_database_id` i32 at `+4`, `modifier_key` u32 at `+8`,
//! `modifier_metadata_flags` u32 at `+12`, `modifier_metadata_gate` i32 at `+16`,
//! `flags` u16 at `+20`, `token_len` u16 at `+22`, `accumulator_raw` i64 at
//! `+24`, `effective_bonus_raw` i64 at `+32`, then `token_len` UTF-8 bytes. There
//! is no legacy short row and no delta grammar.
//!
//! **One scale, and it is carried and asserted rather than assumed.**
//! `FIXED_SCALE = 100_000` (`tactic_weight.rs:26`) is written into the header and
//! the decoder refuses a payload whose word differs (`:222`-`:226`), so the
//! 32768-vs-100000 class of error cannot survive [`split_payload`]. No value is
//! divided at this boundary: every fixed-five quantity travels as the raw `i64`
//! the fold stores, per the recipe's d61 rule ("carry RAW values plus scale
//! metadata, never a chosen projection") and d67's precedent
//! (`d67.rs:461`, a raw fixed-point word as `Kind::Signed`).
//!
//! **Full replace per hour, with no cross-hour accumulator at all.** The state is
//! a single `Option<CountryPreferredTacticWeightSnapshot>` that
//! `begin_hour` clears at every hour boundary (`world_fold.rs::reset_hour_state`,
//! `tactic_weight_fold.rs:22`-`:25`), so one record is one state and a
//! non-folding path leaves the domain-hour unavailable rather than carrying the
//! previous hour forward.
//!
//! # Four-state semantics — the carried presence bool becomes a `null`
//!
//! `accumulator_entry_present` is an explicit presence boolean beside
//! `accumulator_raw` (`tactic_weight.rs:112`-`:113`), and the codec makes the
//! pairing a hard invariant: a row with the bit clear and a non-zero
//! `accumulator_raw` does not decode (`:328`). The survey flagged it as a
//! four-state decision (`runs/batch3/domain_classification.md:1202`-`:1205`), and
//! the decision is that **it is not a field**: it is exactly the empty state of
//! `accumulator_raw`.
//!
//! - **Which empty state.** The collector reads the country's whole modifier
//!   accumulator before it looks anything up — every entry of a bounded,
//!   strictly-ascending array, with the container header re-read and compared
//!   afterwards, and **any** unreadable or invalid entry returning `None`
//!   (`attach_log.rs:32896`-`:32923`) which fails the row
//!   (`AccumulatorUnavailable`, `:33006`-`:33007`). Only after that complete read
//!   does `resolve_sorted_accumulator` binary-search the key and return
//!   `(0, false)` when it is not there (`tactic_weight.rs:156`-`:159`). So a clear
//!   bit means the array was captured in full and genuinely holds no entry for
//!   this tactic's modifier key — §3's **null**, "applicable, the relation
//!   genuinely points at nothing", not `unknown` and not a fabricated zero. The
//!   recipe's Null/Unknown tie-break does not run: the evidence is direct.
//! - **Why the bool itself is left off the wire.** With `accumulator_raw`
//!   declared `.nullable()`, `present == false` is *identically* `Value::Null`
//!   and `present == true` is identically a value — the bit is recoverable from
//!   the four-state with no information anywhere else. Carrying it too would be a
//!   second copy of state the decoder must keep consistent, which is the recipe's
//!   d62/d24 rule ("a value the fold hard-enforces as a function of carried state
//!   is not a field"). The identity is enforced in [`check_row`], not assumed, and
//!   pinned by tests in both directions.
//! - **A real zero is a real zero.** `present == true, accumulator_raw == 0` is a
//!   country whose accumulator carries an explicit zero override, and it maps to
//!   `Value::Signed(0)` — §3 keeps zero out of the three empty states.
//! - **A contradicting row is refused, not mapped.** The mapping above is lossless
//!   only because a clear bit implies a zero word; a clear bit beside a non-zero
//!   word would have its captured value replaced by `Null`. That is d17's shape
//!   (`d17.rs:437`-`:451`), so [`check_row`] refuses the row rather than
//!   publishing something false. The producer cannot emit one — the codec rejects
//!   it at `tactic_weight.rs:328` — but `WeightRow` is publicly constructible, so
//!   the invariant is enforced at both public boundaries.
//!
//! **No d96 field is ever `unknown` or `absent`, and the schema declares neither
//! capability.** The whole d96 read is fail-closed at frame granularity: every
//! per-row failure path — an unreadable pointer, an unexpected vtable, a changed
//! database id, a zero modifier key, an unsupported metadata branch, an
//! unavailable accumulator, an overflowing bonus — returns `Err` from
//! `compact_country_preferred_tactic_weight_row`, and the caller turns any such
//! error into a **whole-collection** `PreferredTacticWeightError`
//! (`attach_log.rs:33081`-`:33099`) whose frame is emitted as a zero-row
//! unavailable PTW1 payload (`:33191`-`:33210`). A row that reaches this mapper is
//! therefore a row whose every read succeeded.
//!
//! **A country with no row is not an empty state either.** The collection is
//! *selected countries only* (`attach_log.rs:33082`, and `:32935`-`:32937` refuses
//! an unselected country outright): a D92 country whose slot holds the no-tactic
//! sentinel has no weight to carry, and D92's own record says so with its `null`
//! pair. That is collection membership, not a field state — and [`check_join`]
//! proves the membership against D92 on every folded frame.
//!
//! # Values the fold hard-enforces, and therefore leaves off the wire
//!
//! Three quantities in the payload are total functions of other carried state,
//! enforced by the same `validate_snapshot` the fold runs on every record
//! (`tactic_weight.rs:289`-`:345`, reached through `decode_payload` at `:285`):
//!
//! - **`effective_bonus_raw` = `country_factor_raw + accumulator_raw`**
//!   (`:326`-`:328`, `checked_add`). Both operands are carried — the factor in the
//!   [`FRAME`] singleton, the accumulator per row with `Null` meaning "no entry",
//!   which the engine's getter reads as `0` (`:156`-`:159`) — so the sum is
//!   recomputable exactly. Re-encoding it would be a second copy the decoder must
//!   keep consistent.
//! - **`multiplier_numerator_raw` = `FIXED_SCALE + effective_bonus_raw`**
//!   (`:118`-`:122`), derived from a derived value. The fold's projection emits it
//!   (`tactic_weight_fold.rs:141`); it is arithmetic, not measurement.
//! - **`modifier_metadata_gate`**, which is **constant 0** on every payload that
//!   decodes: `validate_snapshot` refuses any other value (`:321`-`:325`) and so
//!   does the collector before it ever builds a row (`attach_log.rs:32992`-`:32996`).
//!   The survey called it "a sibling gate … check what it actually means before
//!   mapping" (`domain_classification.md:1206`-`:1207`); read against its
//!   authorities it gates nothing in v1 — it is an *admission* precondition,
//!   because a non-zero gate means the engine's getter takes a callback-dependent
//!   branch that external reading cannot reproduce (`tactic_weight.rs:109`-`:111`).
//!   d62's constant witness word is the precedent: enforce the invariant, test the
//!   identity, leave it off the wire.
//!
//! `modifier_metadata_flags` is **not** in that class and is carried. The codec
//! constrains exactly one bit of it — `MODIFIER_METADATA_CLAMP_BIT` must be clear
//! (`:321`-`:325`) — and leaves the other 31 free, so it is not a function of
//! anything the record carries. That the census observed every live key at flags
//! `3` (`:80`-`:85`) is a property of one build, reported by the harness, never a
//! licence to drop a field the fold accepts variation in.
//!
//! `read_time_us`, `status_flags` and `row_count` are omitted for the reasons d63
//! and d92 omit theirs: the first two are capture diagnostics rather than world
//! state, and `row_count` is a function of the rows the record already carries.
//! All three are checked in [`split_payload`].
//!
//! # `tactic_database_id`, `tactic_id` and `modifier_key`
//!
//! The first two are D92's identity pair, repeated here because the collector
//! re-reads and re-verifies them per row (`attach_log.rs:32938`-`:32966`) and
//! because the fold's join is defined over them (`tactic_weight_fold.rs:62`-`:73`).
//! Both are `ContentId`, on d92's ruling and for its reasons (`d92.rs:86`-`:96`):
//! the token is content-defined by construction and the database id indexes a
//! tactic definition loaded from game content, so a manifest must resolve either
//! for it to keep its meaning under another mod set.
//!
//! `modifier_key` is **`Unsigned`**. It is read verbatim from one engine word,
//! `CCombatTactic+0x174` (`tactic_weight.rs:38`, `attach_log.rs:32968`-`:32975`),
//! hashes nothing on its read path, and the codec labels it "Exact runtime key …
//! **Not portable identity**" (`:105`-`:106`) precisely because the portable
//! identity is the pair above. That is d95's `type_id`/`stable_id` case and d20's
//! `div_type` precedent — stability alone does not make an id content, and being
//! read from an engine struct is not being defined by content. It is carried
//! rather than dropped because nothing in the grammar makes it a function of the
//! tactic: the fold requires only that it is non-zero (`tactic_weight.rs:314`),
//! so a frame whose key differs from the previous hour's for the same tactic is a
//! frame the fold folds. Whether it is in fact constant per tactic on this capture
//! is measured and reported by the harness, exactly as d92 reports its id/token
//! stability.
//!
//! # The frame singleton
//!
//! Three payload-level scalars are carried under a constant key, the shape d68
//! established for a scalar with no natural key (`d68.rs:130`-`:146`) and d92
//! reuses (`d92.rs:98`-`:106`):
//!
//! - **`source_sequence`**, which binds this record to the D92 record of the same
//!   hour: the fold refuses a payload whose sequence differs from D92's
//!   (`tactic_weight_fold.rs:52`-`:56`) and so does the codec's own join
//!   (`tactic_weight.rs:355`-`:359`). Dropping it would make the binding
//!   unverifiable from the wire.
//! - **`observed_country_count`**, the roster witness. It is the D92 roster the
//!   producer walked — *not* D18's filtered cache — and the 2026-07-27 gate fix
//!   turns on exactly that distinction: `fold_payload` compares it against
//!   `preferred_rows.len()` (`tactic_weight_fold.rs:45`-`:56`), where the old code
//!   compared it against a D18 count it could never equal. It is not a function of
//!   anything d96 carries (the rows are the *selected* subset, `rows.len() <=
//!   observed_country_count`, `tactic_weight.rs:238`), so it is carried.
//! - **`country_factor_raw`**, the `NMilitary` define behind every row's bonus
//!   (`tactic_weight.rs:8`-`:11`), read once per frame and re-read and compared at
//!   the close of the walk (`attach_log.rs:33127`-`:33133`). `Signed`, raw.
//!
//! # Modelled dependencies: the D18 roster and a clean D92 snapshot
//!
//! `fold_country_preferred_tactic_weights` runs three gates **before** it decodes
//! anything (`world_fold.rs::fold_country_preferred_tactic_weights`):
//!
//! 1. `country_identity_known_this_hour` — a proven D18 record must have landed
//!    this same hour (`world_fold.rs::fold_country_preferred_tactic_weights`);
//! 2. `country_preferred_tactics_this_hour` — a **clean D92 snapshot** must exist
//!    for this hour (`world_fold.rs::fold_country_preferred_tactic_weights`);
//! 3. `country_preferred_tactics_source_sequence_this_hour` — and its sequence
//!    (`world_fold.rs::fold_country_preferred_tactic_weights`).
//!
//! The last two are one input here, and that is faithful rather than a
//! simplification: the recorder writes both fields together at the end of a
//! successful D92 fold (`world_fold.rs::fold_country_preferred_tactics`) and clears both together at every other
//! site (`:3099`-`:3101`, `:4909`-`:4911`), so `Some`/`None` is always in
//! lockstep. [`fold_payload`] therefore takes [`Preferred`] — D92's row map and
//! its sequence — plus the D18 freshness flag, as typed inputs (recipe step 7),
//! and re-derives none of them.
//!
//! **The D92 rows this port takes are `d92::TacticRow`s**, the same type
//! [`crate::d92`] decodes, so the dependency is the ported grammar rather than a
//! second reading of it. The recorder's own type is
//! `BTreeMap<u32, PreferredTacticRow>` (`world_fold.rs:1842`), and D92's port
//! carries every field of it.
//!
//! **The post-fix roster semantics are reproduced exactly.** D92 keeps the tags
//! D18 filtered out — 83 of them on `runs/batch3/deepwar_c`, including the one
//! *selected* tag 286 — because the D96 join must validate exactly the row set the
//! collector validated before encoding (`world_fold.rs::fold_country_preferred_tactics`,
//! `tactic_weight.rs:350`-`:353`). [`check_join`] iterates D92's whole map, so a
//! port that fed it D18's filtered roster would leave that country's weight row
//! unverified. Its `tag_db_id` is a legal key here and its record is encoded like
//! any other; it is only the recorder's JSON *projection* that cannot name it,
//! because that iterates `tag_by_db_id` (`world_fold.rs::project_country_preferred_tactics`).
//!
//! # Error contract
//!
//! [`fold_payload`] **mutates before erroring**: every refusal path leaves the
//! accumulator cleared. That is the composition the recorder observably has, and
//! it comes from two places — the fold's own `mark_unavailable` on the D18 gate
//! and the failure-status path (`world_fold.rs::fold_country_preferred_tactic_weights`, `tactic_weight_fold.rs:42`),
//! and the recorder's d96 `Err` arm, which calls `mark_unavailable()` on **every**
//! error the fold returns (`world_fold.rs::apply_domain_data`). A harness that assumed
//! all-or-nothing would disagree with the recorder on exactly the frames that
//! fail. (`TacticWeightFoldState::fold_payload` alone leaves `current` untouched
//! on its count/join paths; that difference is unobservable, because the only
//! caller wraps it in the `Err` arm above, and the fold's own hour uniqueness
//! means no second record can read the state in between.)
//!
//! There is **no header-only exit** to preserve. The three gates above sit before
//! the decode, and once the decode runs it is the whole payload — header, rows,
//! trailing bytes and every invariant — because `decode_payload` ends in
//! `validate_snapshot` (`tactic_weight.rs:285`). So [`split_payload`] carries the
//! entire decode, d83's and d92's shape rather than d63's.

use std::collections::{BTreeMap, BTreeSet};

use crate::body::{key, Key, KeyedState};
use crate::d92::TacticRow;
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Schema, Shape};
use crate::value::Value;

/// §5 symbol resolution is **shared with d92**, deliberately.
///
/// `tactic_id` is the same token in both domains — the fold's join requires the
/// two to be equal string-for-string (`tactic_weight_fold.rs:70`) — so two
/// independent stand-in tables would be free to mint different ids for the same
/// token, and a reader joining the two domains would see a mismatch that exists
/// only in the ports. One vocabulary, one assignment. See `d92.rs:302`-`:335` for
/// what the stand-in is and why the measurement is invariant to it.
pub use crate::d92::Symbols;

pub const DOMAIN: u32 = 96;
/// The per-country weight table.
pub const COUNTRIES: u32 = 1;
/// The payload-level scalars carried under a constant key — see the module docs.
pub const FRAME: u32 = 2;

/// Field ids are stable and append-only; §11 forbids reuse for a different
/// field. Numbered uniquely across the whole domain, not per collection, so a
/// later reader cannot confuse two collections' ids.
pub mod field {
    pub const TAG_DB_ID: u32 = 1;
    pub const TACTIC_DATABASE_ID: u32 = 2;
    pub const TACTIC_ID: u32 = 3;
    pub const MODIFIER_KEY: u32 = 4;
    pub const MODIFIER_METADATA_FLAGS: u32 = 5;
    pub const ACCUMULATOR_RAW: u32 = 6;
    pub const FRAME_SINGLETON: u32 = 7;
    pub const SOURCE_SEQUENCE: u32 = 8;
    pub const OBSERVED_COUNTRY_COUNT: u32 = 9;
    pub const COUNTRY_FACTOR_RAW: u32 = 10;
}

// --- Payload grammar, mirrored from `compact_contract/src/tactic_weight.rs`.
//
// The constants are mirrored rather than imported: `wire_format` is the
// reference implementation of the spec and takes no dependency on the codec
// crate outside its measurement bins (d62 set that precedent, d83, d70 and d92
// followed it, and it is followed here rather than re-decided). `MAGIC`,
// `VERSION`, `HEADER_LEN`, `FIXED_SCALE` and all seven receipts are checked by
// `split_payload`, which is what makes the mirror safe: a producer that changed
// the grammar cannot decode past them.

/// `tactic_weight.rs:23` — `PTW1`.
pub const MAGIC: [u8; 4] = *b"PTW1";
/// `tactic_weight.rs:24`.
pub const VERSION: u16 = 1;
/// `tactic_weight.rs:25`.
pub const HEADER_LEN: usize = 276;
/// `tactic_weight.rs:26`. Carried in the header at `+36` and asserted there.
pub const FIXED_SCALE: i64 = 100_000;
/// `tactic_weight.rs:27`.
pub const MAX_COUNTRIES: usize = 1_024;
/// `tactic_weight.rs:28`.
pub const MAX_TOKEN_BYTES: usize = 256;
/// The fixed part of a row (`tactic_weight.rs:192`-`:200`).
pub const FIXED_ROW_LEN: usize = 40;

/// `tactic_weight.rs:44`. A key whose descriptor sets it needs the engine's
/// clamping branch, which external reading cannot reproduce.
pub const MODIFIER_METADATA_CLAMP_BIT: u32 = 1 << 4;
/// `tactic_weight.rs:46`.
pub const FLAG_ACCUMULATOR_ENTRY_PRESENT: u16 = 1 << 0;
/// `tactic_weight.rs:47`. Any other bit is a hard error (`:252`).
pub const KNOWN_ROW_FLAGS: u16 = FLAG_ACCUMULATOR_ENTRY_PRESENT;

/// `COMPACT_STATUS_FAILURE_MASK`, `compact_contract/src/lib.rs:110`-`:118`. d96
/// has no lane-local status bits (`lib.rs:95`-`:101` registers 49 and 51 only)
/// and no expected-partial tolerance (`:136` lists 26, 27, 28 and 54 only), so
/// the global mask is the whole failure predicate for this domain — which is
/// exactly what the fold uses: `compact_status_is_failure`
/// (`tactic_weight_fold.rs:41`), the plain predicate, and then
/// `handle_compact_failure` (`world_fold.rs:10568`) on the same word. Named here
/// rather than imported because d96 is not admitted to the production-codec
/// dependency surface; the measurement bin asserts all three facts so this
/// constant cannot drift unnoticed (d70's pattern,
/// `d70.rs:202`-`:216`).
pub const FAILURE_STATUS_MASK: u32 = (1 << 0)   // STATE_UNAVAILABLE
    | (1 << 1)  // PROFILE_MISMATCH
    | (1 << 2)  // NO_CLOCK
    | (1 << 3)  // NO_SCALAR_SOURCE
    | (1 << 6)  // WORKER_PANIC
    | (1 << 7)  // BRIDGE_UNAVAILABLE
    | (1 << 8)  // SOURCE_UNAVAILABLE
    | (1 << 9)  // PARTIAL_DATA
    | (1 << 12); // CONTROL_INVALID

/// `tactic_weight.rs:49`-`:52`.
pub const EXECUTABLE_SHA256: [u8; 32] = [
    0x87, 0x0e, 0xbf, 0x1f, 0x61, 0x52, 0x62, 0xfa, 0xcc, 0xa4, 0x8a, 0x3b, 0x94, 0x77, 0x75, 0xc4,
    0x1f, 0x0a, 0x53, 0x38, 0xf3, 0x06, 0x8c, 0x40, 0x95, 0x82, 0x09, 0xf5, 0xc4, 0x1c, 0x65, 0xad,
];
/// `tactic_weight.rs:55`-`:58`.
pub const CONSUMER_EVIDENCE_SHA256: [u8; 32] = [
    0xff, 0xa7, 0xe5, 0xb1, 0x4d, 0x1e, 0x26, 0xed, 0x2a, 0x16, 0xfc, 0x66, 0x9a, 0x3a, 0x72, 0x93,
    0xec, 0x4a, 0x01, 0xcd, 0x4a, 0x22, 0x16, 0xf9, 0xaa, 0x90, 0xff, 0x91, 0xf3, 0x74, 0x9d, 0x73,
];
/// `tactic_weight.rs:62`-`:65`.
pub const DEFINE_EVIDENCE_SHA256: [u8; 32] = [
    0x3c, 0x7e, 0x36, 0xd6, 0x39, 0xbe, 0xba, 0x72, 0x5d, 0x94, 0x85, 0x23, 0xe4, 0xc2, 0xa3, 0x25,
    0x6a, 0xe0, 0xc5, 0x96, 0xcc, 0x2e, 0x38, 0xb6, 0x67, 0x55, 0xf7, 0xab, 0x2b, 0x2f, 0x88, 0xcd,
];
/// `tactic_weight.rs:68`-`:71`.
pub const LOOKUP_EVIDENCE_SHA256: [u8; 32] = [
    0x04, 0x22, 0xc6, 0xcd, 0x87, 0x1a, 0xde, 0xdb, 0xba, 0xed, 0xce, 0x53, 0x63, 0xe3, 0x16, 0x3b,
    0xbe, 0x03, 0xdd, 0x2f, 0x8e, 0xd0, 0xd7, 0x98, 0x79, 0xf4, 0xbb, 0x77, 0xe7, 0x1a, 0xce, 0xf8,
];
/// `tactic_weight.rs:75`-`:78`.
pub const RUNTIME_OBSERVATION_SHA256: [u8; 32] = [
    0x91, 0xa5, 0xf2, 0xb7, 0x5e, 0x86, 0xf3, 0xa7, 0xb1, 0xc2, 0x99, 0xa1, 0xfa, 0x07, 0xa8, 0xfe,
    0xf9, 0x52, 0x02, 0xb3, 0x03, 0x70, 0xa3, 0xa1, 0x42, 0x1c, 0xa6, 0x43, 0x64, 0x9f, 0x4a, 0x00,
];
/// `tactic_weight.rs:82`-`:85`.
pub const TACTIC_KEY_CENSUS_SHA256: [u8; 32] = [
    0x92, 0x03, 0xff, 0x38, 0xde, 0x7c, 0x3d, 0x7f, 0x29, 0x04, 0x3d, 0x2d, 0x42, 0x8f, 0xea, 0x44,
    0xd6, 0x07, 0xc7, 0x31, 0x33, 0xe9, 0x95, 0x6f, 0x31, 0x7d, 0xa3, 0xb8, 0xc4, 0xa4, 0x7d, 0xa1,
];
/// `tactic_weight.rs:89`-`:92`.
pub const NONZERO_OVERRIDE_OBSERVATION_SHA256: [u8; 32] = [
    0xf4, 0xd7, 0x1c, 0x3e, 0x5a, 0x4c, 0x73, 0xbb, 0x1a, 0x52, 0x88, 0x76, 0x79, 0xc8, 0xce, 0xcc,
    0xbc, 0xd6, 0x0c, 0x87, 0x27, 0x29, 0xd6, 0x04, 0xbc, 0x10, 0xa0, 0xea, 0x55, 0xd3, 0x07, 0x3c,
];

/// The seven receipts in header order, with their offsets and a name for the
/// error message (`tactic_weight.rs:228`-`:237`).
pub const RECEIPTS: [(usize, &[u8; 32], &str); 7] = [
    (52, &EXECUTABLE_SHA256, "executable"),
    (84, &CONSUMER_EVIDENCE_SHA256, "consumer evidence"),
    (116, &DEFINE_EVIDENCE_SHA256, "define evidence"),
    (148, &LOOKUP_EVIDENCE_SHA256, "lookup evidence"),
    (180, &RUNTIME_OBSERVATION_SHA256, "runtime observation"),
    (212, &TACTIC_KEY_CENSUS_SHA256, "tactic key census"),
    (
        244,
        &NONZERO_OVERRIDE_OBSERVATION_SHA256,
        "nonzero override observation",
    ),
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
                // Full replace each hour (`tactic_weight_fold.rs:74`), but
                // `tag_db_id` is a stable identity that persists across hours,
                // which is what a keyed set needs — and only a keyed set has a
                // delta grammar (§7.3). The selected-country set is constant at
                // 76 rows on the measured capture, so the delta path is what this
                // domain actually costs.
                shape: Shape::KeyedSet,
                fields: vec![
                    // The country's `tag_db_id` (`tactic_weight_fold.rs:60`), the
                    // identity D92 keys its rows by and the join is defined over.
                    // `ContentId` because a country tag is content-defined (§5);
                    // d63, d83 and d92 class theirs the same. This is a ContentId
                    // **key**, the heavier §13 retrofit class — key components
                    // sort and delta-encode by raw value.
                    Field::new(f::TAG_DB_ID, Kind::ContentId).key(0),
                    // D92's identity pair, re-read and re-verified per row
                    // (`attach_log.rs:32938`-`:32966`). Always present: a row for
                    // an unselected country cannot exist (`:32935`-`:32937`) and
                    // `tactic_database_id > 0` / a non-empty token are decode
                    // invariants (`tactic_weight.rs:310`-`:320`).
                    Field::new(f::TACTIC_DATABASE_ID, Kind::ContentId),
                    Field::new(f::TACTIC_ID, Kind::ContentId),
                    // The exact runtime modifier key, `CCombatTactic+0x174`.
                    // `Unsigned`: read verbatim from an engine word, hashing
                    // nothing, and explicitly not portable identity
                    // (`tactic_weight.rs:105`-`:106`) — d95/d20's `div_type`
                    // case. Non-zero is a decode invariant (`:314`).
                    Field::new(f::MODIFIER_KEY, Kind::Unsigned),
                    // The key descriptor's raw flags word. Carried, not dropped:
                    // the codec constrains one bit of it (the clamp bit must be
                    // clear, `:321`-`:325`) and leaves the rest free, so it is
                    // not a function of anything carried.
                    Field::new(f::MODIFIER_METADATA_FLAGS, Kind::Unsigned),
                    // The country's modifier-accumulator entry for this key, raw
                    // fixed-five. `Signed` carries the whole i64 losslessly with
                    // no quantization decision to make (d67's precedent,
                    // `d67.rs:461`). **Null exactly when the accumulator holds no
                    // entry for the key** — the carried presence bool, in the
                    // only place it can be without being a second copy of state.
                    // Never unknown: the array is read in full or the row does
                    // not exist. See the module docs.
                    Field::new(f::ACCUMULATOR_RAW, Kind::Signed).nullable(),
                ],
            },
            Collection {
                collection_id: FRAME,
                // One row, always, under `SINGLETON_KEY_COMPONENT`. A keyed set
                // with a constant key is d68's shape for a payload-level scalar,
                // and it gives the values a delta grammar like any other row.
                shape: Shape::KeyedSet,
                fields: vec![
                    // `Unsigned`, not `ContentId`: the constant names nothing a
                    // content manifest resolves (`d68.rs:255`-`:259`).
                    Field::new(f::FRAME_SINGLETON, Kind::Unsigned).key(0),
                    // The producer advances it once per captured hour
                    // (`attach_log.rs:33175`-`:33178`). §4 makes `Monotone` an
                    // encoding hint and never a validity constraint, so a
                    // producer that reset it would still encode. Always present.
                    Field::new(f::SOURCE_SEQUENCE, Kind::Monotone),
                    // The D92 roster witness the fold's count gate reads
                    // (`tactic_weight_fold.rs:52`). A count, not an identity.
                    Field::new(f::OBSERVED_COUNTRY_COUNT, Kind::Unsigned),
                    // The `NMilitary` define, raw fixed-five, one per frame.
                    Field::new(f::COUNTRY_FACTOR_RAW, Kind::Signed),
                ],
            },
        ],
        event_part: None,
    }
}

// --- The boundary mapping ----------------------------------------------------

/// The 276-byte payload header (`tactic_weight.rs:211`-`:237`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub read_time_us: u64,
    pub observed_country_count: u32,
    pub row_count: u32,
    pub status_flags: u32,
    pub source_sequence: u64,
    pub country_factor_raw: i64,
}

impl PayloadHeader {
    /// The fold's own status classification (`tactic_weight_fold.rs:41`), on the
    /// mask this domain's authorities give it — see [`FAILURE_STATUS_MASK`].
    pub fn is_failure(&self) -> bool {
        self.status_flags & FAILURE_STATUS_MASK != 0
    }
}

/// One decoded row (`tactic_weight.rs:101`-`:115`), field for field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WeightRow {
    pub tag_db_id: u32,
    pub tactic_database_id: i32,
    pub tactic_id: String,
    pub modifier_key: u32,
    pub modifier_metadata_flags: u32,
    pub modifier_metadata_gate: i32,
    pub accumulator_entry_present: bool,
    pub accumulator_raw: i64,
    pub effective_bonus_raw: i64,
}

impl WeightRow {
    /// `tactic_weight.rs:118`-`:122`. Derived twice over — from
    /// `effective_bonus_raw`, which is itself derived — and therefore not a
    /// field. Offered so a consumer computes it the one way the codec does.
    pub fn multiplier_numerator_raw(&self) -> core::result::Result<i64, String> {
        FIXED_SCALE
            .checked_add(self.effective_bonus_raw)
            .ok_or_else(|| "country_preferred_tactic_weight multiplier numerator overflow".into())
    }
}

/// Every per-row invariant `validate_snapshot` states (`tactic_weight.rs:309`-`:332`),
/// as a predicate over one decoded row plus the frame's `country_factor_raw`.
///
/// **This is checked at BOTH public boundaries, not only in [`split_payload`].**
/// `WeightRow`'s fields are public, so a caller can hand [`record_from_row`] a row
/// that never came from a payload. Two of these invariants are load-bearing for
/// the four-state mapping and one is load-bearing for a dropped field:
///
/// - `!accumulator_entry_present` implies `accumulator_raw == 0` (`:328`). The
///   mapper publishes `Null` for a clear presence bit; a clear bit beside a
///   non-zero word would have its captured value replaced by `Null`, which is
///   data loss, so the row is refused instead (d17's rule, `d17.rs:437`-`:451`).
/// - `country_factor_raw + accumulator_raw == effective_bonus_raw` (`:326`-`:328`).
///   `effective_bonus_raw` is left off the wire *because* this holds; a row that
///   breaks it would be re-derived wrongly downstream.
/// - `modifier_metadata_gate == 0` and the clamp bit clear (`:321`-`:325`). The
///   gate is left off the wire because it is constant; a non-zero one would be
///   silently discarded.
///
/// The rest — non-zero tag, positive database id, non-zero modifier key, a
/// non-empty control-free token within `MAX_TOKEN_BYTES` — are the row identity
/// rules at `:310`-`:320`.
pub fn check_row(row: &WeightRow, country_factor_raw: i64) -> core::result::Result<(), String> {
    if row.tag_db_id == 0 {
        return Err("country_preferred_tactic_weight row with zero tag_db_id".into());
    }
    if row.tactic_database_id <= 0 {
        return Err(format!(
            "country_preferred_tactic_weight row {} has non-positive tactic database id {}",
            row.tag_db_id, row.tactic_database_id
        ));
    }
    if row.modifier_key == 0 {
        return Err(format!(
            "country_preferred_tactic_weight row {} has a zero modifier key",
            row.tag_db_id
        ));
    }
    if row.tactic_id.is_empty()
        || row.tactic_id.len() > MAX_TOKEN_BYTES
        || row.tactic_id.chars().any(char::is_control)
    {
        return Err(format!(
            "country_preferred_tactic_weight row {} tactic token {:?} is invalid",
            row.tag_db_id, row.tactic_id
        ));
    }
    if row.modifier_metadata_gate != 0
        || row.modifier_metadata_flags & MODIFIER_METADATA_CLAMP_BIT != 0
    {
        return Err(format!(
            "country_preferred_tactic_weight row {} requires gated or clamped getter semantics \
             (gate {}, flags {:#x})",
            row.tag_db_id, row.modifier_metadata_gate, row.modifier_metadata_flags
        ));
    }
    if !row.accumulator_entry_present && row.accumulator_raw != 0 {
        return Err(format!(
            "country_preferred_tactic_weight row {} carries accumulator value {} with no \
             accumulator entry",
            row.tag_db_id, row.accumulator_raw
        ));
    }
    if country_factor_raw.checked_add(row.accumulator_raw) != Some(row.effective_bonus_raw) {
        return Err(format!(
            "country_preferred_tactic_weight row {} effective bonus {} is not \
             country_factor {country_factor_raw} + accumulator {}",
            row.tag_db_id, row.effective_bonus_raw, row.accumulator_raw
        ));
    }
    row.multiplier_numerator_raw()?;
    Ok(())
}

/// Split a d96 DATA payload into its header and its rows, applying **every**
/// check `tactic_weight::decode_payload` applies (`:206`-`:287`) plus
/// `validate_snapshot` (`:289`-`:345`), in the codec's own order.
///
/// The three cross-domain gates sit before the decode in the fold
/// (`world_fold.rs::fold_country_preferred_tactic_weights`) and the decode is all-or-nothing once it
/// starts, so there is no cheaper header-only exit to preserve and nothing here is
/// deferred: rows are variable-length, so the walk is required to find a row
/// boundary at all.
///
/// This lives here rather than in each caller so the grammar has exactly one
/// implementation: a measurement harness that re-derives it is a harness that can
/// drift from the recorder without anything failing. Nothing in this function can
/// panic on any byte string.
pub fn split_payload(
    payload: &[u8],
) -> core::result::Result<(PayloadHeader, Vec<WeightRow>), String> {
    // `:207`-`:209`.
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "country_preferred_tactic_weight payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    let header = PayloadHeader {
        read_time_us: u64_at(payload, 0),
        observed_country_count: u32_at(payload, 8),
        row_count: u32_at(payload, 12),
        status_flags: u32_at(payload, 16),
        source_sequence: u64_at(payload, 28),
        country_factor_raw: i64_at(payload, 44),
    };
    // `:215`-`:220`.
    if payload[20..24] != MAGIC
        || u16_at(payload, 24) != VERSION
        || u16_at(payload, 26) as usize != HEADER_LEN
    {
        return Err("country_preferred_tactic_weight PTW1 magic/version/header mismatch".into());
    }
    // `:222`-`:226`. The scale is carried, so it is checked rather than assumed:
    // this is what keeps the 32768-vs-100000 class of error out of the mapping.
    if u32_at(payload, 36) as i64 != FIXED_SCALE || u32_at(payload, 40) != 0 {
        return Err(format!(
            "country_preferred_tactic_weight PTW1 scale/reserved mismatch (scale {}, \
             reserved {})",
            u32_at(payload, 36),
            u32_at(payload, 40)
        ));
    }
    // `:228`-`:237`. Seven authority receipts; a mismatch means the payload was
    // not produced by the build this grammar belongs to.
    for (offset, expected, what) in RECEIPTS {
        if &payload[offset..offset + 32] != expected.as_slice() {
            return Err(format!(
                "country_preferred_tactic_weight PTW1 {what} authority receipt mismatch"
            ));
        }
    }
    // `:238`-`:240`.
    let row_count = header.row_count as usize;
    if row_count > MAX_COUNTRIES || row_count > header.observed_country_count as usize {
        return Err(format!(
            "country_preferred_tactic_weight row count outside observed roster \
             (row_count {row_count}, observed {}, max {MAX_COUNTRIES})",
            header.observed_country_count
        ));
    }

    let mut at = HEADER_LEN;
    let mut rows = Vec::with_capacity(row_count);
    for index in 0..row_count {
        // `:243`-`:251`. Every read is bounds-checked before it happens; a
        // truncated payload is an error, never a panic.
        let fixed_end = at
            .checked_add(FIXED_ROW_LEN)
            .ok_or("country_preferred_tactic_weight row cursor overflow")?;
        if fixed_end > payload.len() {
            return Err(format!(
                "country_preferred_tactic_weight row {index} truncated at byte {at}"
            ));
        }
        let tag_db_id = u32_at(payload, at);
        let tactic_database_id = i32_at(payload, at + 4);
        let modifier_key = u32_at(payload, at + 8);
        let modifier_metadata_flags = u32_at(payload, at + 12);
        let modifier_metadata_gate = i32_at(payload, at + 16);
        let flags = u16_at(payload, at + 20);
        let token_len = u16_at(payload, at + 22) as usize;
        let accumulator_raw = i64_at(payload, at + 24);
        let effective_bonus_raw = i64_at(payload, at + 32);
        at = fixed_end;
        // `:252`-`:254`. Note `token_len == 0` is rejected here: unlike d92, every
        // d96 row carries a token, because only a selected country has a row.
        if flags & !KNOWN_ROW_FLAGS != 0 || token_len == 0 || token_len > MAX_TOKEN_BYTES {
            return Err(format!(
                "country_preferred_tactic_weight row {index} flags {flags:#x} / token length \
                 {token_len} invalid"
            ));
        }
        let end = at
            .checked_add(token_len)
            .ok_or("country_preferred_tactic_weight token cursor overflow")?;
        if end > payload.len() {
            return Err(format!(
                "country_preferred_tactic_weight row {index} token truncated at byte {at}"
            ));
        }
        // `:258`-`:260`.
        let tactic_id = core::str::from_utf8(&payload[at..end])
            .map_err(|_| {
                format!("country_preferred_tactic_weight row {index} token is not UTF-8")
            })?
            .to_owned();
        at = end;
        rows.push(WeightRow {
            tag_db_id,
            tactic_database_id,
            tactic_id,
            modifier_key,
            modifier_metadata_flags,
            modifier_metadata_gate,
            accumulator_entry_present: flags & FLAG_ACCUMULATOR_ENTRY_PRESENT != 0,
            accumulator_raw,
            effective_bonus_raw,
        });
    }
    // `:274`-`:276`.
    if at != payload.len() {
        return Err(format!(
            "country_preferred_tactic_weight trailing bytes: consumed {at} of {}",
            payload.len()
        ));
    }
    validate_rows(&header, &rows)?;
    Ok((header, rows))
}

/// `tactic_weight::validate_snapshot` (`:289`-`:345`), applied to a decoded
/// header and rows.
///
/// The failure-status branch is the codec's own and its position matters: a
/// failure snapshot must carry **no rows at all** (`:296`-`:301`) and is then
/// exempt from every per-row rule, because there are no rows to check. The clean
/// branch requires a non-zero roster count and a non-zero source sequence
/// (`:302`-`:304`) before it walks the rows.
fn validate_rows(
    header: &PayloadHeader,
    rows: &[WeightRow],
) -> core::result::Result<(), String> {
    // `:291`-`:295`.
    if header.observed_country_count as usize > MAX_COUNTRIES
        || rows.len() > header.observed_country_count as usize
    {
        return Err(format!(
            "country_preferred_tactic_weight invalid observed roster count \
             (observed {}, rows {})",
            header.observed_country_count,
            rows.len()
        ));
    }
    if header.is_failure() {
        // `:296`-`:301`.
        if !rows.is_empty() {
            return Err(format!(
                "country_preferred_tactic_weight failed snapshot contains {} selected row(s)",
                rows.len()
            ));
        }
        return Ok(());
    }
    // `:302`-`:304`.
    if header.observed_country_count == 0 || header.source_sequence == 0 {
        return Err(
            "country_preferred_tactic_weight clean snapshot lacks roster/source sequence".into(),
        );
    }
    let mut previous = 0u32;
    let mut tags: BTreeSet<u32> = BTreeSet::new();
    let mut by_id: BTreeMap<i32, &str> = BTreeMap::new();
    let mut by_token: BTreeMap<&str, i32> = BTreeMap::new();
    for row in rows {
        // `:310`-`:312`: unique, strictly ascending, non-zero tags — so the keyed
        // set's own order is the payload's order.
        if row.tag_db_id == 0 || row.tag_db_id <= previous || !tags.insert(row.tag_db_id) {
            return Err(format!(
                "country_preferred_tactic_weight rows must have unique ascending nonzero tag ids \
                 (saw {} after {previous})",
                row.tag_db_id
            ));
        }
        previous = row.tag_db_id;
        // The per-row rules have exactly one implementation, shared with
        // `record_from_row` — see [`check_row`].
        check_row(row, header.country_factor_raw)?;
        // `:333`-`:341`: the id↔token mapping is injective WITHIN this payload
        // and nowhere wider, which is why both fields are carried.
        if by_id
            .insert(row.tactic_database_id, row.tactic_id.as_str())
            .is_some_and(|old| old != row.tactic_id.as_str())
            || by_token
                .insert(row.tactic_id.as_str(), row.tactic_database_id)
                .is_some_and(|old| old != row.tactic_database_id)
        {
            return Err(
                "country_preferred_tactic_weight tactic id/token mapping is not injective".into(),
            );
        }
    }
    Ok(())
}

/// The key of one weight row: its `tag_db_id` (`tactic_weight_fold.rs:60`).
///
/// A zero `tag_db_id` never decodes (`tactic_weight.rs:310`), and `WeightRow` is
/// publicly constructible, so the check is here rather than assumed.
pub fn key_from_row(row: &WeightRow) -> core::result::Result<Key, String> {
    if row.tag_db_id == 0 {
        return Err("country_preferred_tactic_weight row with zero tag_db_id".into());
    }
    Ok(key(&[row.tag_db_id as u64]))
}

/// The [`FRAME`] singleton's only key.
pub fn key_from_frame() -> Key {
    key(&[SINGLETON_KEY_COMPONENT])
}

/// One decoded row of the `countries` table, as the collector emits it today,
/// onto its stored Record. The caller has already consumed the payload header via
/// [`split_payload`], which is also what decoded and validated this row; d96 has
/// exactly one row form, so nothing here selects between forms.
///
/// `frame_tick` is §6's derived-field input (stored = v + c * (frame_tick >> u));
/// d96 declares no derived field, so it is taken and ignored — every domain's
/// mapper has the same shape. `symbols` is step 3's §5 resolver, the typed input a
/// content-defined text field requires (d29's `symbols: &Symbols`), shared with
/// d92. `country_factor_raw` is the frame-level value this row's invariants are
/// defined against and the row itself does not carry — the recipe's step-7 typed
/// input, d35's `scales: RowScales` case.
///
/// Fail-closed throughout: an unresolvable token is an `Err`, never a value
/// weakened to `Unknown` — the string is right here, so "not captured" would be
/// false, and a token outside the content set means the caller built the table
/// from the wrong content, which is a diagnosis rather than data.
pub fn record_from_row(
    collection: &Collection,
    row: &WeightRow,
    frame_tick: u64,
    symbols: &Symbols,
    country_factor_raw: i64,
) -> core::result::Result<Record, String> {
    // d96 has no derived field; §6's transform input is unused. Named rather
    // than `_frame_tick` because the signature is the cross-domain contract.
    let _ = frame_tick;
    // A record built for the wrong collection would otherwise come out shaped by
    // whatever fields that collection happens to have.
    if collection.collection_id != COUNTRIES {
        return Err(format!(
            "country_preferred_tactic_weight country row built against collection {}",
            collection.collection_id
        ));
    }
    // The key is not carried in the record body (§3), but its validity is still
    // part of this row's validity.
    let _ = key_from_row(row)?;
    // `WeightRow` is publicly constructible, so `split_payload` having checked the
    // row is not a precondition this function may assume — and two of the checks
    // are what make this mapping lossless. See [`check_row`].
    check_row(row, country_factor_raw)?;

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            // The raw cast is §13's open manifest-id assignment: today's boundary
            // mappers cast the source id straight into `ContentId`, and this field
            // is listed in the deliverable's retrofit sweep.
            field::TACTIC_DATABASE_ID => Value::ContentId(row.tactic_database_id as u64),
            field::TACTIC_ID => match symbols.id(&row.tactic_id) {
                Some(id) => Value::ContentId(id),
                None => {
                    return Err(format!(
                        "country_preferred_tactic_weight token {:?} is not in the symbol table \
                         ({} symbol(s))",
                        row.tactic_id,
                        symbols.len()
                    ))
                }
            },
            field::MODIFIER_KEY => Value::Unsigned(row.modifier_key as u64),
            field::MODIFIER_METADATA_FLAGS => Value::Unsigned(row.modifier_metadata_flags as u64),
            // The carried presence bool, as the four-state it is: Null when the
            // country's accumulator genuinely holds no entry for this key, a real
            // value (zero included) when it does. See the module docs.
            field::ACCUMULATOR_RAW => {
                if row.accumulator_entry_present {
                    Value::Signed(row.accumulator_raw)
                } else {
                    Value::Null
                }
            }
            other => {
                // Filling an unknown field with a placeholder would publish
                // invented data under a field id that means something else.
                return Err(format!(
                    "country_preferred_tactic_weight has no mapping for field id {other}"
                ));
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// The [`FRAME`] singleton's record: the three payload-level scalars.
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
            "country_preferred_tactic_weight frame row built against collection {}",
            collection.collection_id
        ));
    }
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::SOURCE_SEQUENCE => Value::Unsigned(header.source_sequence),
            field::OBSERVED_COUNTRY_COUNT => Value::Unsigned(header.observed_country_count as u64),
            field::COUNTRY_FACTOR_RAW => Value::Signed(header.country_factor_raw),
            other => {
                return Err(format!(
                    "country_preferred_tactic_weight frame has no mapping for field id {other}"
                ))
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// Every `tactic_id` token one payload carries, for building a [`Symbols`] table
/// over a whole capture before any record is mapped (§5: fully read first).
///
/// Shares [`split_payload`] rather than re-walking the payload, so the vocabulary
/// and the records can never disagree about what a row says.
pub fn tokens_in_payload(payload: &[u8]) -> core::result::Result<Vec<String>, String> {
    let (_, rows) = split_payload(payload)?;
    Ok(rows.into_iter().map(|row| row.tactic_id).collect())
}

// --- The D92 dependency ------------------------------------------------------

/// The clean current-hour D92 snapshot d96 folds against
/// (`world_fold.rs:10550`-`:10560`), as a typed input.
///
/// `rows` is `country_preferred_tactics_this_hour` — D92's **whole** row map,
/// including the tags D18 filtered out, which is the set the collector validated
/// before encoding (`tactic_weight.rs:350`-`:353`). `source_sequence` is
/// `country_preferred_tactics_source_sequence_this_hour`. The recorder holds them
/// as two `Option` fields written and cleared together, so one input is faithful —
/// see the module docs.
#[derive(Clone, Copy, Debug)]
pub struct Preferred<'a> {
    pub rows: &'a BTreeMap<u32, TacticRow>,
    pub source_sequence: u64,
}

/// The same-boundary D92 identity join (`tactic_weight_fold.rs:57`-`:73`).
///
/// For **every** country D92 measured: either D92 says "no tactic" and d96 has no
/// row, or D92 names a tactic and d96's row for that country names the same
/// database id and the same token. Anything else refuses the hour.
///
/// **The direction is the fold's, and it is one-way.** The loop iterates D92's
/// map, so a d96 row for a tag D92 does not carry is **not** refused: with `S`
/// selected countries and `N = observed_country_count = preferred.rows.len()`,
/// the only count rule is `rows.len() <= N` (`tactic_weight.rs:238`), which
/// leaves room for `N - S` rows naming tags D92 never mentioned. The fold has
/// exactly this permissiveness (`tactic_weight_fold.rs:62`-`:73`) and the codec's
/// own producer-side join has it too (`tactic_weight.rs:365`-`:376`), so
/// mirroring is the rule (recipe step 7: refuse only where mapping would publish
/// something false — a row keyed by a tag nobody else names is inert, exactly as
/// D92's own unprojectable extras are, and the fold keeps it in state).
///
/// The producer cannot emit one: it builds weight rows only from the countries it
/// walked, inside the same loop that built the D92 rows
/// (`attach_log.rs:33074`-`:33101`). The measurement harness counts any that
/// appear rather than assuming they cannot — 0 on `runs/batch3/deepwar_c`.
pub fn check_join(
    header: &PayloadHeader,
    rows: &[WeightRow],
    preferred: Preferred<'_>,
) -> core::result::Result<(), String> {
    // `tactic_weight_fold.rs:52`-`:56`. The witness is D92's roster size, never
    // D18's: the producer sets `observed_country_count` from the same complete
    // country walk that produced the D92 rows and proves the two agree before it
    // encodes (`tactic_weight.rs:355`). Substituting D18's filtered count here is
    // the defect the 2026-07-27 gate fix removed.
    if header.observed_country_count as usize != preferred.rows.len()
        || header.source_sequence != preferred.source_sequence
    {
        return Err(format!(
            "country_preferred_tactic_weight PTW1/D92 roster count or source-sequence mismatch \
             (PTW1 observed {} seq {}, D92 rows {} seq {})",
            header.observed_country_count,
            header.source_sequence,
            preferred.rows.len(),
            preferred.source_sequence
        ));
    }
    let actual: BTreeMap<u32, &WeightRow> = rows.iter().map(|row| (row.tag_db_id, row)).collect();
    for (&tag_db_id, selected) in preferred.rows {
        match (
            selected.tactic_database_id,
            selected.tactic_id.as_deref(),
            actual.get(&tag_db_id).copied(),
        ) {
            (0, None, None) => {}
            (id, Some(token), Some(weight))
                if id == weight.tactic_database_id && token == weight.tactic_id.as_str() => {}
            _ => {
                return Err(format!(
                    "country_preferred_tactic_weight PTW1/D92 selected tactic identity mismatch \
                     for tag {tag_db_id}"
                ))
            }
        }
    }
    Ok(())
}

// --- The frame-level fold ----------------------------------------------------

/// The accumulator `fold_country_preferred_tactic_weights` maintains, as wire
/// state.
///
/// The two collections are held as ONE array in [`schema`] order, so
/// [`State::as_slice`] is the shape `encode_baseline`, `encode_delta` and
/// `decode_delta` take with no copy and no chance of a caller passing them in the
/// wrong order.
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

    /// Rows in the weight table — the domain's size, which is what the
    /// measurement reports per hour.
    pub fn len(&self) -> usize {
        self.countries().len()
    }

    pub fn is_empty(&self) -> bool {
        self.countries().is_empty()
    }

    /// `TacticWeightFoldState::mark_unavailable` (`tactic_weight_fold.rs:27`-`:30`)
    /// and `begin_hour` (`:22`-`:25`), which clear the snapshot outright. Public
    /// because the recorder calls it from outside the fold too — on the domain's
    /// `Err` arm (`world_fold.rs::apply_domain_data`) and at every hour boundary
    /// (`world_fold.rs::reset_hour_state`).
    pub fn mark_unavailable(&mut self) {
        for collection in &mut self.collections {
            collection.clear();
        }
    }
}

/// What [`fold_payload`] did with a record. The outcomes are named because the
/// harness has to report them apart (recipe step 9).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FoldOutcome {
    /// The payload folded; `acc` holds the hour's state.
    Folded { status_flags: u32 },
    /// A failure status: the fold cleared the snapshot and returned without
    /// folding (`tactic_weight_fold.rs:41`-`:44`, then `world_fold.rs::fold_country_preferred_tactic_weights`).
    FailureStatus { status_flags: u32 },
}

/// Why [`fold_payload`] refused a record. Named for the same reason.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FoldRefusal {
    /// The hour carried no proven D18 record (`world_fold.rs::fold_country_preferred_tactic_weights`).
    NoCurrentHourRoster,
    /// The hour carried no clean D92 snapshot, or none with a source sequence
    /// (`world_fold.rs::fold_country_preferred_tactic_weights`).
    NoCleanPreferredTactic,
}

/// Fold one d96 DATA payload into `acc`, exactly as
/// `fold_country_preferred_tactic_weights` folds it
/// (`world_fold.rs::fold_country_preferred_tactic_weights`) through
/// `TacticWeightFoldState::fold_payload` (`tactic_weight_fold.rs:32`-`:80`).
///
/// The fold's order is reproduced exactly, and it is **not** the harness's order:
/// the two cross-domain gates run BEFORE the payload is decoded, so a malformed
/// payload in an hour with no roster never reaches the decoder here. Production's
/// status- and gate-independent payload authority decodes every record anyway
/// (`domain_data_authority.rs:13246`-`:13262`), which is why the measurement
/// harness calls [`split_payload`] on every record first; that is the authority's
/// order, not the fold's, and the two are kept apart on purpose.
///
/// `identity_known` is `country_identity_known_this_hour` (`world_fold.rs:1934`),
/// true only when a proven D18 record landed **this** hour. `preferred` is the
/// clean D92 snapshot and its sequence. Both are typed inputs (recipe step 7).
///
/// **Error contract: this MUTATES before erroring** — every refusal path clears
/// the accumulator, which is what the recorder observably holds after any d96
/// error (`world_fold.rs::apply_domain_data`). A caller must not treat an `Err` as
/// leaving the prior state intact; the recorder does not.
pub fn fold_payload(
    schema: &Schema,
    acc: &mut State,
    payload: &[u8],
    identity_known: bool,
    preferred: Option<Preferred<'_>>,
    frame_tick: u64,
    symbols: &Symbols,
) -> core::result::Result<FoldOutcome, String> {
    // `:10543`-`:10549`. The D18 gate, before anything is decoded.
    if !identity_known {
        acc.mark_unavailable();
        return Err(refusal_message(FoldRefusal::NoCurrentHourRoster));
    }
    // `:10550`-`:10560`. The clean-D92 gate, still before the decode.
    let Some(preferred) = preferred else {
        acc.mark_unavailable();
        return Err(refusal_message(FoldRefusal::NoCleanPreferredTactic));
    };

    // `tactic_weight_fold.rs:39`-`:40`.
    let (header, rows) = match split_payload(payload) {
        Ok(decoded) => decoded,
        Err(error) => {
            acc.mark_unavailable();
            return Err(error);
        }
    };

    // `:41`-`:44`. Position is the fold's: after the decode, before the join.
    if header.is_failure() {
        acc.mark_unavailable();
        return Ok(FoldOutcome::FailureStatus {
            status_flags: header.status_flags,
        });
    }

    // `:45`-`:73`.
    if let Err(error) = check_join(&header, &rows, preferred) {
        acc.mark_unavailable();
        return Err(error);
    }

    // Everything below can only fail on a caller error (a schema that is not this
    // domain's) or on a row the checks above already accepted; it is built in one
    // place so that EVERY error path leaves the accumulator cleared, with no arm
    // able to drift from that rule.
    match build_state(schema, &header, &rows, frame_tick, symbols) {
        Ok((next_countries, next_frame)) => {
            acc.collections[COUNTRIES_INDEX] = next_countries;
            acc.collections[FRAME_INDEX] = next_frame;
            Ok(FoldOutcome::Folded {
                status_flags: header.status_flags,
            })
        }
        Err(error) => {
            acc.mark_unavailable();
            Err(error)
        }
    }
}

/// Build both collections for one accepted payload. Full replace (`:74`): the
/// prior state is discarded, not merged, so this returns fresh tables rather than
/// patching the accumulator.
fn build_state(
    schema: &Schema,
    header: &PayloadHeader,
    rows: &[WeightRow],
    frame_tick: u64,
    symbols: &Symbols,
) -> core::result::Result<(KeyedState, KeyedState), String> {
    let countries = schema.collection(COUNTRIES).map_err(|e| format!("{e:?}"))?;
    let frame = schema.collection(FRAME).map_err(|e| format!("{e:?}"))?;

    let mut next_countries = KeyedState::new();
    for row in rows {
        let record = record_from_row(
            countries,
            row,
            frame_tick,
            symbols,
            header.country_factor_raw,
        )?;
        // `validate_rows` has already proved the tags unique and ascending, so an
        // insert can never collapse two rows here.
        if next_countries.insert(key_from_row(row)?, record).is_some() {
            return Err(format!(
                "country_preferred_tactic_weight duplicate tag_db_id {} survived validation",
                row.tag_db_id
            ));
        }
    }
    let mut next_frame = KeyedState::new();
    next_frame.insert(
        key_from_frame(),
        record_from_frame_header(frame, header, frame_tick)?,
    );
    Ok((next_countries, next_frame))
}

fn refusal_message(refusal: FoldRefusal) -> String {
    match refusal {
        FoldRefusal::NoCurrentHourRoster => {
            "country_preferred_tactic_weight requires a current-hour D18 identity roster".into()
        }
        FoldRefusal::NoCleanPreferredTactic => {
            "country_preferred_tactic_weight requires a clean current-hour D92 snapshot and its \
             source sequence"
                .into()
        }
    }
}

/// Callers reach these only after a bounds check, so the slices are in range.
fn u64_at(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().expect("8 bytes"))
}

fn i64_at(b: &[u8], off: usize) -> i64 {
    i64::from_le_bytes(b[off..off + 8].try_into().expect("8 bytes"))
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
    use crate::d92;
    use crate::schema::Quant;
    use crate::TickGrid;

    fn grid() -> TickGrid {
        TickGrid::new(3).expect("3 is a legal grid")
    }

    fn countries() -> Collection {
        schema().collections[COUNTRIES_INDEX].clone()
    }

    fn frame_collection() -> Collection {
        schema().collections[FRAME_INDEX].clone()
    }

    fn index_of(c: &Collection, id: u32) -> usize {
        c.value_fields()
            .position(|f| f.field_id == id)
            .expect("field is in the schema")
    }

    fn symbols() -> Symbols {
        Symbols::from_vocabulary(["blitzkrieg", "encirclement", "shock", "trench_warfare"])
    }

    /// One raw row, packed the way `tactic_weight::encode_payload` packs one
    /// (`tactic_weight.rs:185`-`:202`). Every word is a parameter so a test can
    /// build a row that breaks exactly one invariant.
    #[allow(clippy::too_many_arguments)]
    fn raw_row(
        tag_db_id: u32,
        tactic_database_id: i32,
        modifier_key: u32,
        modifier_metadata_flags: u32,
        modifier_metadata_gate: i32,
        flags: u16,
        accumulator_raw: i64,
        effective_bonus_raw: i64,
        token: &[u8],
    ) -> Vec<u8> {
        let mut row = Vec::with_capacity(FIXED_ROW_LEN + token.len());
        row.extend_from_slice(&tag_db_id.to_le_bytes());
        row.extend_from_slice(&tactic_database_id.to_le_bytes());
        row.extend_from_slice(&modifier_key.to_le_bytes());
        row.extend_from_slice(&modifier_metadata_flags.to_le_bytes());
        row.extend_from_slice(&modifier_metadata_gate.to_le_bytes());
        row.extend_from_slice(&flags.to_le_bytes());
        row.extend_from_slice(&(token.len() as u16).to_le_bytes());
        row.extend_from_slice(&accumulator_raw.to_le_bytes());
        row.extend_from_slice(&effective_bonus_raw.to_le_bytes());
        row.extend_from_slice(token);
        assert_eq!(row.len(), FIXED_ROW_LEN + token.len());
        row
    }

    /// A row whose accumulator entry is PRESENT, with the derived bonus made
    /// consistent with `country_factor`.
    fn present_row(
        tag_db_id: u32,
        tactic_database_id: i32,
        token: &str,
        accumulator_raw: i64,
        country_factor_raw: i64,
    ) -> Vec<u8> {
        raw_row(
            tag_db_id,
            tactic_database_id,
            7,
            3,
            0,
            FLAG_ACCUMULATOR_ENTRY_PRESENT,
            accumulator_raw,
            country_factor_raw + accumulator_raw,
            token.as_bytes(),
        )
    }

    /// A row whose accumulator holds NO entry for the key: the presence bit is
    /// clear and the word is zero, so the bonus is the factor alone.
    fn absent_entry_row(
        tag_db_id: u32,
        tactic_database_id: i32,
        token: &str,
        country_factor_raw: i64,
    ) -> Vec<u8> {
        raw_row(
            tag_db_id,
            tactic_database_id,
            7,
            3,
            0,
            0,
            0,
            country_factor_raw,
            token.as_bytes(),
        )
    }

    /// A whole PTW1 payload whose header agrees with its rows.
    fn payload_with(
        rows: &[Vec<u8>],
        observed_country_count: u32,
        status_flags: u32,
        source_sequence: u64,
        country_factor_raw: i64,
    ) -> Vec<u8> {
        payload_raw(
            rows,
            0,
            observed_country_count,
            rows.len() as u32,
            status_flags,
            source_sequence,
            FIXED_SCALE as u32,
            0,
            country_factor_raw,
        )
    }

    /// A payload with every header word supplied, so a test can build a header
    /// that disagrees with its body on purpose.
    #[allow(clippy::too_many_arguments)]
    fn payload_raw(
        rows: &[Vec<u8>],
        read_time_us: u64,
        observed_country_count: u32,
        row_count: u32,
        status_flags: u32,
        source_sequence: u64,
        scale_word: u32,
        reserved: u32,
        country_factor_raw: i64,
    ) -> Vec<u8> {
        let mut p = Vec::with_capacity(HEADER_LEN);
        p.extend_from_slice(&read_time_us.to_le_bytes());
        p.extend_from_slice(&observed_country_count.to_le_bytes());
        p.extend_from_slice(&row_count.to_le_bytes());
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&MAGIC);
        p.extend_from_slice(&VERSION.to_le_bytes());
        p.extend_from_slice(&(HEADER_LEN as u16).to_le_bytes());
        p.extend_from_slice(&source_sequence.to_le_bytes());
        p.extend_from_slice(&scale_word.to_le_bytes());
        p.extend_from_slice(&reserved.to_le_bytes());
        p.extend_from_slice(&country_factor_raw.to_le_bytes());
        for (_, receipt, _) in RECEIPTS {
            p.extend_from_slice(receipt);
        }
        assert_eq!(p.len(), HEADER_LEN, "header is {HEADER_LEN} bytes");
        for row in rows {
            p.extend_from_slice(row);
        }
        p
    }

    /// A D92 row map, as `country_preferred_tactics_this_hour` holds it.
    fn preferred_rows(rows: &[(u32, i32, Option<&str>)]) -> BTreeMap<u32, TacticRow> {
        rows.iter()
            .map(|(tag, id, token)| {
                (
                    *tag,
                    TacticRow {
                        tag_db_id: *tag,
                        tactic_database_id: *id,
                        tactic_id: token.map(str::to_string),
                    },
                )
            })
            .collect()
    }

    // --- Step 8.1: the schema validates. -------------------------------------

    #[test]
    fn the_schema_is_valid() {
        schema()
            .validate(grid().log2())
            .expect("d96 schema is well-formed");
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
            "a country tag is content-defined (§5), so a manifest must resolve it"
        );
        assert!(keys[0].kind.legal_as_key());
        assert_eq!(
            c.fields.len(),
            6,
            "1 key + the D92 identity pair + modifier key + metadata flags + accumulator"
        );
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
        assert_eq!(
            c.fields.len(),
            4,
            "1 constant key + source_sequence + observed_country_count + country_factor_raw"
        );
        assert_eq!(key_from_frame(), key(&[SINGLETON_KEY_COMPONENT]));
    }

    #[test]
    fn the_domain_has_exactly_two_collections() {
        // The survey read d96 as "country rows plus a modifier-accumulator child"
        // keyed `(tag_db_id, key)` (`runs/batch3/domain_classification.md:1176`,
        // `:1264`). PTW1 v1 carries no such child: `encode_payload` writes the
        // header and the rows and nothing else (`tactic_weight.rs:162`-`:204`),
        // and `ModifierAccumulatorEntry` never appears on the wire — it is the
        // collector-side input to `resolve_sorted_accumulator` (`:146`-`:160`),
        // whose ONE resolved value per row is what the row carries. The second
        // collection here is the d68 frame singleton instead. The survey line is
        // corrected in the deliverable.
        let s = schema();
        assert_eq!(s.domain, DOMAIN);
        assert_eq!(s.collections.len(), 2);
        assert_eq!(s.collections[COUNTRIES_INDEX].collection_id, COUNTRIES);
        assert_eq!(s.collections[FRAME_INDEX].collection_id, FRAME);
        // Field ids are unique across the whole domain, so no reader can confuse
        // two collections' ids.
        let mut ids: Vec<u32> = s
            .collections
            .iter()
            .flat_map(|c| c.fields.iter().map(|f| f.field_id))
            .collect();
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), total, "field ids are unique across the domain");
    }

    // --- Step 8.3: the states the schema declares, and only those. -----------

    #[test]
    fn the_only_declared_capability_is_nullable_on_the_accumulator() {
        // The mapper may emit exactly the empty states the schema declares.
        // `Absent` would claim a field is inapplicable to a row that exists;
        // `Unknown` would claim a value was not captured, which cannot happen —
        // every read failure makes the WHOLE weight collection unavailable
        // (`attach_log.rs:33081`-`:33099`) and the frame is emitted with zero
        // rows. See the module docs.
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
            vec![field::ACCUMULATOR_RAW],
            "the accumulator entry is the only relation that can point at nothing"
        );

        // The frame singleton declares nothing at all: the header always carries
        // all three words.
        let f = frame_collection();
        assert_eq!(f.optional_count(), 0);
        assert!(f.value_fields().all(|x| !x.nullable && !x.unknown_capable));
    }

    #[test]
    fn a_present_accumulator_entry_maps_to_a_value_and_a_missing_one_to_null() {
        // The four-state ruling, at the mapper: the carried presence bool IS the
        // null state of `accumulator_raw`, and a present entry holding zero is a
        // real measured zero (§3 keeps zero out of the empty states).
        let c = countries();
        let factor = -25_000i64;
        let payload = payload_with(
            &[
                present_row(1, 5, "shock", 40_000, factor),
                present_row(2, 5, "shock", 0, factor),
                absent_entry_row(3, 6, "blitzkrieg", factor),
            ],
            3,
            0,
            1,
            factor,
        );
        let (header, rows) = split_payload(&payload).expect("payload decodes");
        let at = index_of(&c, field::ACCUMULATOR_RAW);

        let present = record_from_row(&c, &rows[0], 0, &symbols(), header.country_factor_raw)
            .expect("maps");
        assert_eq!(present.values[at], Value::Signed(40_000));

        let zero = record_from_row(&c, &rows[1], 0, &symbols(), header.country_factor_raw)
            .expect("maps");
        assert_eq!(
            zero.values[at],
            Value::Signed(0),
            "a present entry holding zero is a measured zero, never an empty state"
        );

        let missing = record_from_row(&c, &rows[2], 0, &symbols(), header.country_factor_raw)
            .expect("maps");
        assert_eq!(
            missing.values[at],
            Value::Null,
            "no accumulator entry for the key is §3's null, not unknown and not zero"
        );
    }

    #[test]
    fn the_presence_bool_is_recoverable_from_the_four_state_in_both_directions() {
        // Why the bool is not a field: `Null` <-> `!present` is a bijection over
        // every row the codec accepts, so carrying it too would be a second copy
        // of state. Swept over both values of the bit and a range of words,
        // including the ones the codec refuses.
        let c = countries();
        let at = index_of(&c, field::ACCUMULATOR_RAW);
        let factor = 12_345i64;
        for present in [false, true] {
            for accumulator_raw in [i64::MIN + FIXED_SCALE, -1, 0, 1, 100_000, i64::MAX / 4] {
                let row = WeightRow {
                    tag_db_id: 9,
                    tactic_database_id: 5,
                    tactic_id: "shock".into(),
                    modifier_key: 7,
                    modifier_metadata_flags: 3,
                    modifier_metadata_gate: 0,
                    accumulator_entry_present: present,
                    accumulator_raw,
                    effective_bonus_raw: factor.wrapping_add(accumulator_raw),
                };
                let legal = (present || accumulator_raw == 0)
                    && factor.checked_add(accumulator_raw) == Some(row.effective_bonus_raw)
                    && FIXED_SCALE.checked_add(row.effective_bonus_raw).is_some();
                match record_from_row(&c, &row, 0, &symbols(), factor) {
                    Ok(record) => {
                        assert!(legal, "row (present {present}, raw {accumulator_raw}) mapped");
                        assert_eq!(
                            record.values[at] == Value::Null,
                            !present,
                            "the null state recovers the presence bit exactly"
                        );
                    }
                    Err(_) => assert!(
                        !legal,
                        "row (present {present}, raw {accumulator_raw}) was refused"
                    ),
                }
            }
        }
    }

    #[test]
    fn a_clear_presence_bit_beside_a_nonzero_word_is_refused_not_mapped() {
        // d17's rule: the Null mapping is lossless only because a clear bit
        // implies a zero word. Mapping this row would replace a captured value
        // with `Null`, so both public boundaries refuse it — the codec cannot
        // produce one (`tactic_weight.rs:328`) but `WeightRow` is publicly
        // constructible.
        let c = countries();
        let factor = 0i64;
        let contradiction = raw_row(1, 5, 7, 3, 0, 0, 40_000, 40_000, b"shock");
        let payload = payload_with(&[contradiction], 1, 0, 1, factor);
        let err = split_payload(&payload).expect_err("the codec's own invariant");
        assert!(err.contains("with no accumulator entry"), "{err}");

        let row = WeightRow {
            tag_db_id: 1,
            tactic_database_id: 5,
            tactic_id: "shock".into(),
            modifier_key: 7,
            modifier_metadata_flags: 3,
            modifier_metadata_gate: 0,
            accumulator_entry_present: false,
            accumulator_raw: 40_000,
            effective_bonus_raw: 40_000,
        };
        assert!(check_row(&row, factor).is_err());
        assert!(record_from_row(&c, &row, 0, &symbols(), factor).is_err());
    }

    #[test]
    fn the_derived_and_constant_words_are_enforced_rather_than_carried() {
        // Three payload words are left off the wire because they are functions of
        // carried state or constants. Each is enforced at BOTH boundaries, and
        // each is recomputable from what the record does carry.
        let c = countries();
        let factor = -25_000i64;
        let good = present_row(1, 5, "shock", 40_000, factor);
        assert!(split_payload(&payload_with(&[good.clone()], 1, 0, 1, factor)).is_ok());

        // `effective_bonus_raw != country_factor + accumulator`.
        let wrong_bonus = raw_row(
            1,
            5,
            7,
            3,
            0,
            FLAG_ACCUMULATOR_ENTRY_PRESENT,
            40_000,
            40_001,
            b"shock",
        );
        let err = split_payload(&payload_with(&[wrong_bonus], 1, 0, 1, factor))
            .expect_err("the derived identity is enforced");
        assert!(err.contains("effective bonus"), "{err}");

        // A non-zero metadata gate, and the clamp bit.
        for (gate, flags) in [(1i32, 3u32), (0, 3 | MODIFIER_METADATA_CLAMP_BIT)] {
            let row = raw_row(
                1,
                5,
                7,
                flags,
                gate,
                FLAG_ACCUMULATOR_ENTRY_PRESENT,
                40_000,
                factor + 40_000,
                b"shock",
            );
            let err = split_payload(&payload_with(&[row], 1, 0, 1, factor))
                .expect_err("gated or clamped getter semantics are refused");
            assert!(err.contains("gated or clamped"), "{err}");
        }

        // And the recomputation is exact: the record carries the factor and the
        // accumulator, so the two dropped values come back.
        let (header, rows) = split_payload(&payload_with(&[good], 1, 0, 1, factor)).expect("decodes");
        let record = record_from_row(&c, &rows[0], 0, &symbols(), header.country_factor_raw)
            .expect("maps");
        let accumulator = match record.values[index_of(&c, field::ACCUMULATOR_RAW)] {
            Value::Signed(v) => v,
            Value::Null => 0, // no entry: the engine's getter reads zero
            other => panic!("unexpected accumulator value {other:?}"),
        };
        assert_eq!(
            header.country_factor_raw + accumulator,
            rows[0].effective_bonus_raw
        );
        assert_eq!(
            FIXED_SCALE + rows[0].effective_bonus_raw,
            rows[0].multiplier_numerator_raw().expect("no overflow")
        );
        // The gate is constant zero on every payload that decodes.
        assert_eq!(rows[0].modifier_metadata_gate, 0);
    }

    #[test]
    fn the_mapper_enforces_the_row_shape_on_a_hand_built_row() {
        // `WeightRow`'s fields are public, so a caller can reach `record_from_row`
        // with a row that never came from a payload. Swept over the whole shape
        // space rather than one input, and `check_row` — the single shared
        // implementation — must agree with the mapper on every case.
        let c = countries();
        let s = symbols();
        let factor = 1_000i64;
        let base = WeightRow {
            tag_db_id: 1,
            tactic_database_id: 5,
            tactic_id: "shock".into(),
            modifier_key: 7,
            modifier_metadata_flags: 3,
            modifier_metadata_gate: 0,
            accumulator_entry_present: true,
            accumulator_raw: 40_000,
            effective_bonus_raw: 41_000,
        };
        assert!(record_from_row(&c, &base, 0, &s, factor).is_ok());

        let mut cases: Vec<(&str, WeightRow)> = Vec::new();
        let mut push = |name: &'static str, mutate: fn(&mut WeightRow)| {
            let mut row = base.clone();
            mutate(&mut row);
            cases.push((name, row));
        };
        push("zero tag", |r| r.tag_db_id = 0);
        push("zero database id", |r| {
            r.tactic_database_id = 0;
        });
        push("negative database id", |r| r.tactic_database_id = -1);
        push("min database id", |r| r.tactic_database_id = i32::MIN);
        push("zero modifier key", |r| r.modifier_key = 0);
        push("empty token", |r| r.tactic_id = String::new());
        push("control character", |r| r.tactic_id = "sho\u{1}ck".into());
        push("over-long token", |r| {
            r.tactic_id = "a".repeat(MAX_TOKEN_BYTES + 1)
        });
        push("nonzero gate", |r| r.modifier_metadata_gate = 1);
        push("negative gate", |r| r.modifier_metadata_gate = -1);
        push("clamp bit", |r| {
            r.modifier_metadata_flags |= MODIFIER_METADATA_CLAMP_BIT
        });
        push("absent entry with a value", |r| {
            r.accumulator_entry_present = false
        });
        push("broken derived bonus", |r| r.effective_bonus_raw += 1);
        push("multiplier overflow", |r| {
            r.accumulator_raw = i64::MAX - 1;
            r.effective_bonus_raw = i64::MAX;
        });
        for (name, row) in &cases {
            assert!(
                record_from_row(&c, row, 0, &s, factor).is_err(),
                "hand-built row ({name}) must be refused"
            );
            assert!(check_row(row, factor).is_err(), "check_row disagrees ({name})");
        }
        // A row that is legal against one factor and illegal against another: the
        // frame scalar is a real input, not decoration.
        assert!(record_from_row(&c, &base, 0, &s, factor + 1).is_err());
    }

    #[test]
    fn a_token_outside_the_content_set_is_refused_not_weakened() {
        let c = countries();
        let factor = 0i64;
        let payload = payload_with(&[present_row(7, 42, "sturmtruppen", 0, factor)], 1, 0, 1, factor);
        let (header, rows) = split_payload(&payload).expect("decodes");
        let err = record_from_row(&c, &rows[0], 0, &symbols(), header.country_factor_raw)
            .expect_err("refuses");
        assert!(err.contains("not in the symbol table"), "{err}");
        // Fail-closed against an empty table too, rather than emitting id 0.
        assert!(record_from_row(&c, &rows[0], 0, &Symbols::empty(), 0).is_err());
    }

    #[test]
    fn the_symbol_table_is_the_one_d92_uses() {
        // The join requires the two domains' tokens to be equal string-for-string
        // (`tactic_weight_fold.rs:70`), so they must resolve to the same id. This
        // is a type-level fact — `d96::Symbols` IS `d92::Symbols` — and it is
        // asserted so a later split of the two tables fails here.
        let table = Symbols::from_vocabulary(["blitzkrieg", "shock"]);
        let d92_table: d92::Symbols = table.clone();
        assert_eq!(table.id("shock"), d92_table.id("shock"));
        assert_eq!(table.id("shock"), Some(1));
    }

    // --- Step 8.6: every field's offset, and every header word. --------------

    #[test]
    fn every_row_offset_and_header_word_maps_where_it_should() {
        // A transposed row word or header word mis-reads every row behind it, and
        // no round-trip or replay assertion can see that — both arms agree on the
        // wrong slice. So every word gets a distinct known value.
        let c = countries();
        let f = frame_collection();
        let factor = -25_000i64;
        let row = raw_row(
            0x0A0B_0C0D,
            0x1112_1314,
            0x2122_2324,
            0x3132_3334 & !MODIFIER_METADATA_CLAMP_BIT,
            0,
            FLAG_ACCUMULATOR_ENTRY_PRESENT,
            0x4142_4344_4546_4748,
            factor.wrapping_add(0x4142_4344_4546_4748),
            b"shock",
        );
        let payload = payload_raw(
            &[row],
            0x5152_5354_5556_5758,
            1,
            1,
            0,
            0x6162_6364_6566_6768,
            FIXED_SCALE as u32,
            0,
            factor,
        );
        let (header, rows) = split_payload(&payload).expect("decodes");

        // The header, word by word.
        assert_eq!(header.read_time_us, 0x5152_5354_5556_5758);
        assert_eq!(header.observed_country_count, 1);
        assert_eq!(header.row_count, 1);
        assert_eq!(header.status_flags, 0);
        assert_eq!(header.source_sequence, 0x6162_6364_6566_6768);
        assert_eq!(header.country_factor_raw, factor);

        // The row, field by field, through the mapper.
        assert_eq!(rows[0].tag_db_id, 0x0A0B_0C0D);
        assert_eq!(rows[0].tactic_database_id, 0x1112_1314);
        assert_eq!(rows[0].modifier_key, 0x2122_2324);
        assert_eq!(
            rows[0].modifier_metadata_flags,
            0x3132_3334 & !MODIFIER_METADATA_CLAMP_BIT
        );
        assert_eq!(rows[0].modifier_metadata_gate, 0);
        assert!(rows[0].accumulator_entry_present);
        assert_eq!(rows[0].accumulator_raw, 0x4142_4344_4546_4748);
        assert_eq!(rows[0].tactic_id, "shock");

        let record =
            record_from_row(&c, &rows[0], 0, &symbols(), header.country_factor_raw).expect("maps");
        assert_eq!(
            record.values[index_of(&c, field::TACTIC_DATABASE_ID)],
            Value::ContentId(0x1112_1314)
        );
        assert_eq!(
            record.values[index_of(&c, field::TACTIC_ID)],
            Value::ContentId(2),
            "\"shock\" is index 2 of the sorted stand-in vocabulary"
        );
        assert_eq!(
            record.values[index_of(&c, field::MODIFIER_KEY)],
            Value::Unsigned(0x2122_2324)
        );
        assert_eq!(
            record.values[index_of(&c, field::MODIFIER_METADATA_FLAGS)],
            Value::Unsigned((0x3132_3334u32 & !MODIFIER_METADATA_CLAMP_BIT) as u64)
        );
        assert_eq!(
            record.values[index_of(&c, field::ACCUMULATOR_RAW)],
            Value::Signed(0x4142_4344_4546_4748)
        );
        assert_eq!(key_from_row(&rows[0]).expect("key"), key(&[0x0A0B_0C0D]));

        let frame_record = record_from_frame_header(&f, &header, 0).expect("maps");
        assert_eq!(
            frame_record.values[index_of(&f, field::SOURCE_SEQUENCE)],
            Value::Unsigned(0x6162_6364_6566_6768)
        );
        assert_eq!(
            frame_record.values[index_of(&f, field::OBSERVED_COUNTRY_COUNT)],
            Value::Unsigned(1)
        );
        assert_eq!(
            frame_record.values[index_of(&f, field::COUNTRY_FACTOR_RAW)],
            Value::Signed(factor)
        );
    }

    #[test]
    fn the_header_scale_and_reserved_words_are_checked_not_assumed() {
        // The scale is CARRIED. A payload written under a different divisor must
        // be refused rather than mapped at 100_000 — this is the check that keeps
        // the 32768-vs-100000 class of error out of the mapping entirely.
        let factor = 0i64;
        let rows = [present_row(1, 5, "shock", 0, factor)];
        assert!(split_payload(&payload_raw(&rows, 0, 1, 1, 0, 1, FIXED_SCALE as u32, 0, factor)).is_ok());
        for scale in [0u32, 32_768, 1_000, 100_001] {
            let err = split_payload(&payload_raw(&rows, 0, 1, 1, 0, 1, scale, 0, factor))
                .expect_err("a different scale word is refused");
            assert!(err.contains("scale/reserved mismatch"), "{err}");
        }
        let err = split_payload(&payload_raw(&rows, 0, 1, 1, 0, 1, FIXED_SCALE as u32, 1, factor))
            .expect_err("a nonzero reserved word is refused");
        assert!(err.contains("scale/reserved mismatch"), "{err}");
    }

    #[test]
    fn the_magic_version_and_seven_receipts_are_checked() {
        let factor = 0i64;
        let rows = [present_row(1, 5, "shock", 0, factor)];
        let good = payload_with(&rows, 1, 0, 1, factor);
        assert!(split_payload(&good).is_ok());
        for offset in [20usize, 24, 26] {
            let mut bad = good.clone();
            bad[offset] ^= 0xff;
            let err = split_payload(&bad).expect_err("magic/version/header length");
            assert!(err.contains("magic/version/header mismatch"), "{err}");
        }
        for (offset, _, what) in RECEIPTS {
            let mut bad = good.clone();
            bad[offset] ^= 0xff;
            let err = split_payload(&bad).expect_err("receipt");
            assert!(err.contains(what), "{err}");
        }
    }

    #[test]
    fn a_truncated_or_over_long_payload_is_an_error_and_never_a_panic() {
        let factor = 7i64;
        let good = payload_with(
            &[
                present_row(1, 5, "shock", 40_000, factor),
                absent_entry_row(2, 6, "blitzkrieg", factor),
            ],
            2,
            0,
            1,
            factor,
        );
        assert!(split_payload(&good).is_ok());
        for cut in 0..good.len() {
            assert!(
                split_payload(&good[..cut]).is_err(),
                "a {cut}-byte payload must be refused"
            );
        }
        let mut long = good.clone();
        long.push(0);
        let err = split_payload(&long).expect_err("trailing bytes");
        assert!(err.contains("trailing bytes"), "{err}");
        // And a row count the body cannot satisfy.
        let err = split_payload(&payload_raw(&[], 0, 4, 4, 0, 1, FIXED_SCALE as u32, 0, factor))
            .expect_err("row count without rows");
        assert!(err.contains("truncated"), "{err}");
    }

    #[test]
    fn the_row_count_must_sit_inside_the_observed_roster() {
        // `tactic_weight.rs:238`-`:240` and `:291`-`:295`: the selected rows are a
        // subset of the roster the producer walked.
        let factor = 0i64;
        let rows = [
            present_row(1, 5, "shock", 0, factor),
            present_row(2, 6, "blitzkrieg", 0, factor),
        ];
        assert!(split_payload(&payload_with(&rows, 2, 0, 1, factor)).is_ok());
        assert!(split_payload(&payload_with(&rows, 9, 0, 1, factor)).is_ok());
        let err = split_payload(&payload_with(&rows, 1, 0, 1, factor))
            .expect_err("more rows than the roster");
        assert!(err.contains("outside observed roster"), "{err}");
    }

    #[test]
    fn rows_must_be_unique_ascending_and_the_id_token_mapping_injective() {
        let factor = 0i64;
        for rows in [
            vec![
                present_row(2, 5, "shock", 0, factor),
                present_row(1, 6, "blitzkrieg", 0, factor),
            ],
            vec![
                present_row(1, 5, "shock", 0, factor),
                present_row(1, 6, "blitzkrieg", 0, factor),
            ],
        ] {
            let err = split_payload(&payload_with(&rows, 9, 0, 1, factor))
                .expect_err("tags must be unique and ascending");
            assert!(err.contains("unique ascending nonzero tag ids"), "{err}");
        }
        // One database id naming two tokens, and one token naming two ids.
        for rows in [
            vec![
                present_row(1, 5, "shock", 0, factor),
                present_row(2, 5, "blitzkrieg", 0, factor),
            ],
            vec![
                present_row(1, 5, "shock", 0, factor),
                present_row(2, 6, "shock", 0, factor),
            ],
        ] {
            let err = split_payload(&payload_with(&rows, 9, 0, 1, factor))
                .expect_err("the mapping is injective within a payload");
            assert!(err.contains("not injective"), "{err}");
        }
    }

    #[test]
    fn a_failure_status_payload_carries_no_rows_and_skips_the_row_rules() {
        // `tactic_weight.rs:296`-`:304`: a failed snapshot must be rowless, and it
        // is exempt from the clean-snapshot roster/sequence rule — which is what
        // lets the collector emit the all-zero unavailable frame
        // (`attach_log.rs:33191`-`:33210`).
        let empty = payload_raw(&[], 0, 0, 0, FAILURE_STATUS_MASK, 0, FIXED_SCALE as u32, 0, 0);
        let (header, rows) = split_payload(&empty).expect("an unavailable frame decodes");
        assert!(header.is_failure());
        assert!(rows.is_empty());

        let with_rows = payload_raw(
            &[present_row(1, 5, "shock", 0, 0)],
            0,
            1,
            1,
            1 << 8, // SOURCE_UNAVAILABLE
            1,
            FIXED_SCALE as u32,
            0,
            0,
        );
        let err = split_payload(&with_rows).expect_err("a failed snapshot with rows");
        assert!(err.contains("failed snapshot contains"), "{err}");

        // A CLEAN snapshot, by contrast, needs both witnesses.
        for (observed, sequence) in [(0u32, 1u64), (1, 0)] {
            let err = split_payload(&payload_raw(
                &[],
                0,
                observed,
                0,
                0,
                sequence,
                FIXED_SCALE as u32,
                0,
                0,
            ))
            .expect_err("a clean snapshot needs a roster and a sequence");
            assert!(err.contains("lacks roster/source sequence"), "{err}");
        }
    }

    #[test]
    fn the_failure_mask_matches_the_folds_classification() {
        // The library's restated mask, over every single status bit. The bin
        // asserts it against `compact_contract`'s own predicate; here the point is
        // that `is_failure` is the thing `fold_payload` branches on.
        for bit in 0..32u32 {
            let word = 1u32 << bit;
            let header = PayloadHeader {
                read_time_us: 0,
                observed_country_count: 1,
                row_count: 0,
                status_flags: word,
                source_sequence: 1,
                country_factor_raw: 0,
            };
            assert_eq!(header.is_failure(), FAILURE_STATUS_MASK & word != 0);
        }
    }

    // --- The cross-domain join and the fold. ---------------------------------

    #[test]
    fn the_join_validates_every_d92_country_including_the_ones_d18_filtered_out() {
        // The post-fix semantics: D92 keeps the tags D18 filtered out, and the
        // join validates the WHOLE D92 row set — including a selected extra like
        // `deepwar_c`'s tag 286, which is exactly the row dropping the extras
        // would have left unverified (`world_fold.rs::fold_country_preferred_tactics`).
        let factor = 0i64;
        let rows_bytes = [
            present_row(2, 5, "shock", 0, factor),
            present_row(286, 6, "blitzkrieg", 0, factor),
        ];
        let payload = payload_with(&rows_bytes, 3, 0, 1, factor);
        let (header, rows) = split_payload(&payload).expect("decodes");
        let preferred = preferred_rows(&[
            (1, 0, None),                 // in D18, no tactic selected: no d96 row
            (2, 5, Some("shock")),        // in D18, selected
            (286, 6, Some("blitzkrieg")), // NOT in D18, selected
        ]);
        check_join(
            &header,
            &rows,
            Preferred {
                rows: &preferred,
                source_sequence: 1,
            },
        )
        .expect("every D92 country is accounted for");

        // A d96 row missing for a country D92 says is selected.
        let short = payload_with(&[present_row(2, 5, "shock", 0, factor)], 3, 0, 1, factor);
        let (h, r) = split_payload(&short).expect("decodes");
        let err = check_join(
            &h,
            &r,
            Preferred {
                rows: &preferred,
                source_sequence: 1,
            },
        )
        .expect_err("a selected D92 country with no weight row");
        assert!(err.contains("identity mismatch") && err.contains("286"), "{err}");

        // A d96 row for a country D92 says has NO tactic.
        let extra = payload_with(
            &[
                present_row(1, 9, "trench_warfare", 0, factor),
                present_row(2, 5, "shock", 0, factor),
                present_row(286, 6, "blitzkrieg", 0, factor),
            ],
            3,
            0,
            1,
            factor,
        );
        let (h, r) = split_payload(&extra).expect("decodes");
        assert!(check_join(
            &h,
            &r,
            Preferred {
                rows: &preferred,
                source_sequence: 1,
            }
        )
        .is_err());

        // A token or database id that disagrees with D92's — each still
        // injective WITHIN its own payload, so the disagreement is only visible
        // to the join.
        for wrong in [
            present_row(286, 5, "shock", 0, factor),
            present_row(286, 7, "blitzkrieg", 0, factor),
        ] {
            let p = payload_with(&[present_row(2, 5, "shock", 0, factor), wrong], 3, 0, 1, factor);
            let (h, r) = split_payload(&p).expect("decodes");
            assert!(check_join(
                &h,
                &r,
                Preferred {
                    rows: &preferred,
                    source_sequence: 1,
                }
            )
            .is_err());
        }
    }

    #[test]
    fn a_row_for_a_tag_d92_does_not_carry_is_kept_not_refused() {
        // The join is one-way: it iterates D92's map, so a weight row naming a tag
        // D92 never mentioned passes, as long as the count rule still holds
        // (`rows.len() <= observed_country_count`). The fold has exactly this
        // permissiveness (`tactic_weight_fold.rs:62`-`:73`) and so does the codec's
        // producer-side join (`tactic_weight.rs:365`-`:376`), so the port MIRRORS
        // it rather than gating against the fold. The row is inert — nothing else
        // names that tag — and the producer cannot emit one, because it builds
        // weight rows only from the countries it walked
        // (`attach_log.rs:33074`-`:33101`). The harness counts any that appear.
        let factor = 0i64;
        let preferred = preferred_rows(&[(1, 0, None), (2, 5, Some("shock")), (3, 0, None)]);
        let payload = payload_with(
            &[
                present_row(2, 5, "shock", 0, factor),
                present_row(999, 6, "blitzkrieg", 0, factor),
            ],
            3,
            0,
            1,
            factor,
        );
        let (header, rows) = split_payload(&payload).expect("decodes");
        check_join(
            &header,
            &rows,
            Preferred {
                rows: &preferred,
                source_sequence: 1,
            },
        )
        .expect("mirrors the fold's permissiveness");

        // It reaches the state like any other row, keyed by its own tag.
        let schema = schema();
        let mut acc = State::new();
        fold_payload(
            &schema,
            &mut acc,
            &payload,
            true,
            Some(Preferred {
                rows: &preferred,
                source_sequence: 1,
            }),
            0,
            &symbols(),
        )
        .expect("folds");
        assert_eq!(acc.len(), 2, "the unnamed tag's row is kept, not dropped");

        // The count rule is still the bound: one more row than the roster is
        // refused by `split_payload`, before the join is even consulted.
        let too_many = payload_with(
            &[
                present_row(2, 5, "shock", 0, factor),
                present_row(4, 6, "blitzkrieg", 0, factor),
                present_row(5, 9, "trench_warfare", 0, factor),
                present_row(999, 8, "encirclement", 0, factor),
            ],
            3,
            0,
            1,
            factor,
        );
        assert!(split_payload(&too_many).is_err());
    }

    #[test]
    fn every_refusal_path_leaves_the_accumulator_cleared() {
        // The error contract, swept over all five refusal paths rather than the
        // two the gate test covers. The recorder reaches the same state on every
        // one of them, through the fold's own `mark_unavailable` or through the
        // domain's `Err` arm (`world_fold.rs::apply_domain_data`).
        let schema = schema();
        let factor = 0i64;
        let preferred = preferred_rows(&[(1, 0, None), (2, 5, Some("shock"))]);
        let good = payload_with(&[present_row(2, 5, "shock", 40_000, factor)], 2, 0, 1, factor);
        let refusals: Vec<(&str, bool, Option<u64>, Vec<u8>)> = vec![
            ("no D18 roster", false, Some(1), good.clone()),
            ("no clean D92", true, None, good.clone()),
            ("malformed payload", true, Some(1), vec![0u8; 8]),
            (
                "roster witness mismatch",
                true,
                Some(1),
                payload_with(&[present_row(2, 5, "shock", 0, factor)], 9, 0, 1, factor),
            ),
            (
                "source sequence mismatch",
                true,
                Some(7),
                payload_with(&[present_row(2, 5, "shock", 0, factor)], 2, 0, 1, factor),
            ),
            (
                "identity mismatch",
                true,
                Some(1),
                payload_with(&[present_row(2, 6, "blitzkrieg", 0, factor)], 2, 0, 1, factor),
            ),
        ];
        for (name, identity_known, sequence, payload) in refusals {
            let mut acc = State::new();
            fold_payload(
                &schema,
                &mut acc,
                &good,
                true,
                Some(Preferred {
                    rows: &preferred,
                    source_sequence: 1,
                }),
                0,
                &symbols(),
            )
            .expect("folds");
            assert_eq!(acc.len(), 1, "{name}: precondition");

            let preferred_input = sequence.map(|source_sequence| Preferred {
                rows: &preferred,
                source_sequence,
            });
            fold_payload(
                &schema,
                &mut acc,
                &payload,
                identity_known,
                preferred_input,
                0,
                &symbols(),
            )
            .expect_err(name);
            assert!(acc.is_empty(), "{name}: the country table must be cleared");
            assert_eq!(acc.frame().len(), 0, "{name}: and so must the frame");
        }
    }

    #[test]
    fn the_roster_witness_is_d92s_count_and_the_sequence_must_agree() {
        // The 2026-07-27 gate fix, pinned: the witness is `preferred.rows.len()`,
        // never a D18 count. A D92 map of 3 rows and a PTW1 header claiming a
        // roster of 356 — D18's filtered size on `deepwar_c` — must be refused.
        let factor = 0i64;
        let rows_bytes = [present_row(2, 5, "shock", 0, factor)];
        let preferred = preferred_rows(&[(1, 0, None), (2, 5, Some("shock")), (3, 0, None)]);
        let ok = payload_with(&rows_bytes, 3, 0, 11, factor);
        let (h, r) = split_payload(&ok).expect("decodes");
        assert!(check_join(
            &h,
            &r,
            Preferred {
                rows: &preferred,
                source_sequence: 11
            }
        )
        .is_ok());

        for (observed, sequence) in [(2u32, 11u64), (4, 11), (356, 11), (3, 12)] {
            let p = payload_with(&rows_bytes, observed, 0, sequence, factor);
            let (h, r) = split_payload(&p).expect("decodes");
            let err = check_join(
                &h,
                &r,
                Preferred {
                    rows: &preferred,
                    source_sequence: 11,
                },
            )
            .expect_err("roster count or source sequence disagreement");
            assert!(err.contains("roster count or source-sequence mismatch"), "{err}");
        }
    }

    #[test]
    fn the_fold_runs_its_gates_before_the_decode_and_clears_on_every_refusal() {
        // The fold's order (`world_fold.rs::fold_country_preferred_tactic_weights`): D18, then a clean
        // D92 snapshot, and only then the payload. A malformed payload under a
        // failed gate never reaches the decoder — the gate's message is what
        // comes back.
        let schema = schema();
        let factor = 0i64;
        let preferred = preferred_rows(&[(1, 0, None), (2, 5, Some("shock"))]);
        let good = payload_with(&[present_row(2, 5, "shock", 0, factor)], 2, 0, 1, factor);
        let garbage = vec![0u8; 8];

        let mut acc = State::new();
        // Fold once so a later refusal has something to clear.
        assert_eq!(
            fold_payload(
                &schema,
                &mut acc,
                &good,
                true,
                Some(Preferred {
                    rows: &preferred,
                    source_sequence: 1
                }),
                0,
                &symbols(),
            )
            .expect("folds"),
            FoldOutcome::Folded { status_flags: 0 }
        );
        assert_eq!(acc.len(), 1);
        assert_eq!(acc.frame().len(), 1);

        let err = fold_payload(
            &schema,
            &mut acc,
            &garbage,
            false,
            Some(Preferred {
                rows: &preferred,
                source_sequence: 1,
            }),
            0,
            &symbols(),
        )
        .expect_err("no D18 roster this hour");
        assert!(err.contains("current-hour D18 identity roster"), "{err}");
        assert!(acc.is_empty(), "the accumulator is cleared before the error");
        assert_eq!(acc.frame().len(), 0);

        // The clean-D92 gate, also ahead of the decode.
        let err = fold_payload(&schema, &mut acc, &garbage, true, None, 0, &symbols())
            .expect_err("no clean D92 snapshot this hour");
        assert!(err.contains("clean current-hour D92 snapshot"), "{err}");

        // With both gates satisfied, the same garbage is a decode error.
        let err = fold_payload(
            &schema,
            &mut acc,
            &garbage,
            true,
            Some(Preferred {
                rows: &preferred,
                source_sequence: 1,
            }),
            0,
            &symbols(),
        )
        .expect_err("the payload is decoded only after the gates pass");
        assert!(err.contains("shorter than the"), "{err}");
        assert!(acc.is_empty());
    }

    #[test]
    fn a_failure_status_record_clears_the_hour_without_folding() {
        let schema = schema();
        let preferred = preferred_rows(&[(1, 0, None), (2, 5, Some("shock"))]);
        let mut acc = State::new();
        let good = payload_with(&[present_row(2, 5, "shock", 0, 0)], 2, 0, 1, 0);
        fold_payload(
            &schema,
            &mut acc,
            &good,
            true,
            Some(Preferred {
                rows: &preferred,
                source_sequence: 1,
            }),
            0,
            &symbols(),
        )
        .expect("folds");
        assert_eq!(acc.len(), 1);

        let failed = payload_raw(&[], 0, 0, 0, FAILURE_STATUS_MASK, 0, FIXED_SCALE as u32, 0, 0);
        assert_eq!(
            fold_payload(
                &schema,
                &mut acc,
                &failed,
                true,
                Some(Preferred {
                    rows: &preferred,
                    source_sequence: 1
                }),
                0,
                &symbols(),
            )
            .expect("a failure status is not an error"),
            FoldOutcome::FailureStatus {
                status_flags: FAILURE_STATUS_MASK
            }
        );
        assert!(
            acc.is_empty(),
            "the domain-hour is unavailable, not carried forward"
        );
    }

    #[test]
    fn the_fold_replaces_the_whole_table_every_hour() {
        // `tactic_weight_fold.rs:74`: the prior snapshot is discarded, not merged.
        // A country that stops being selected has no row, which the keyed set
        // expresses as a tombstone in the delta.
        let schema = schema();
        let mut acc = State::new();
        let first = preferred_rows(&[(1, 5, Some("shock")), (2, 6, Some("blitzkrieg"))]);
        fold_payload(
            &schema,
            &mut acc,
            &payload_with(
                &[
                    present_row(1, 5, "shock", 0, 0),
                    present_row(2, 6, "blitzkrieg", 0, 0),
                ],
                2,
                0,
                1,
                0,
            ),
            true,
            Some(Preferred {
                rows: &first,
                source_sequence: 1,
            }),
            0,
            &symbols(),
        )
        .expect("folds");
        assert_eq!(acc.len(), 2);

        let second = preferred_rows(&[(1, 5, Some("shock")), (2, 0, None)]);
        fold_payload(
            &schema,
            &mut acc,
            &payload_with(&[present_row(1, 5, "shock", 0, 0)], 2, 0, 2, 0),
            true,
            Some(Preferred {
                rows: &second,
                source_sequence: 2,
            }),
            0,
            &symbols(),
        )
        .expect("folds");
        assert_eq!(acc.len(), 1, "the table is replaced, not merged");
        assert_eq!(acc.frame().len(), 1);
    }

    #[test]
    fn tokens_in_payload_sees_every_row_token() {
        let factor = 0i64;
        let payload = payload_with(
            &[
                present_row(1, 5, "shock", 0, factor),
                present_row(2, 6, "blitzkrieg", 0, factor),
                present_row(3, 5, "shock", 0, factor),
            ],
            3,
            0,
            1,
            factor,
        );
        let tokens = tokens_in_payload(&payload).expect("walks");
        assert_eq!(tokens, vec!["shock", "blitzkrieg", "shock"]);
        let table = Symbols::from_vocabulary(tokens);
        assert_eq!(table.len(), 2, "deduplicated");
        let c = countries();
        let (header, rows) = split_payload(&payload).expect("decodes");
        for row in &rows {
            record_from_row(&c, row, 0, &table, header.country_factor_raw)
                .expect("every token resolves against the pass-1 vocabulary");
        }
    }

    // --- Step 8.2: a full record round-trips byte-identically. ---------------

    #[test]
    fn a_full_state_round_trips_byte_identically() {
        let schema = schema();
        let factor = -25_000i64;
        let preferred = preferred_rows(&[
            (1, 0, None),
            (2, 5, Some("shock")),
            (3, 6, Some("blitzkrieg")),
            (286, 9, Some("trench_warfare")),
        ]);
        let payload = payload_with(
            &[
                present_row(2, 5, "shock", 40_000, factor),
                absent_entry_row(3, 6, "blitzkrieg", factor),
                present_row(286, 9, "trench_warfare", -1_000, factor),
            ],
            4,
            0,
            17,
            factor,
        );
        let mut acc = State::new();
        fold_payload(
            &schema,
            &mut acc,
            &payload,
            true,
            Some(Preferred {
                rows: &preferred,
                source_sequence: 17,
            }),
            0,
            &symbols(),
        )
        .expect("folds");

        let mut first = Vec::new();
        encode_baseline(&schema, acc.as_slice(), &mut first).expect("encodes");
        let (decoded, consumed) = decode_baseline(&schema, &first, 0).expect("decodes");
        assert_eq!(consumed, first.len(), "the baseline consumed exactly");
        let mut again = Vec::new();
        encode_baseline(&schema, &decoded, &mut again).expect("re-encodes");
        assert_eq!(first, again, "encode -> decode -> encode is byte-identical");
        assert_eq!(decoded[COUNTRIES_INDEX].len(), 3);
        assert_eq!(decoded[FRAME_INDEX].len(), 1);
    }

    // --- Step 8.4: quantization does not apply, and why. ---------------------

    #[test]
    fn no_field_is_quantized_or_derived() {
        // Step 4's "quantization saves bytes" test does not apply: d96 has **no
        // continuous field at all**. Its three fixed-five quantities —
        // `accumulator_raw`, `country_factor_raw` and the two values derived from
        // them — travel as raw `i64` words exactly as the fold stores them, per
        // the recipe's d61 raw-carry rule and d67's precedent (`d67.rs:461`), so
        // there is no quantizer to choose and no precision band to state.
        //
        // Nor is any field §6-derived. `source_sequence` is the candidate — the
        // collector computes it as `hour_index_from_raw(game_hour_raw) + 1`
        // (`attach_log.rs:33175`-`:33178`) — and d92 declines the declaration for
        // reasons that apply verbatim here (`d92.rs:108`-`:117`): the fold
        // enforces nothing of the kind, and §6's `u` is a container decision.
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
        // §13's manifest-id assignment rule will change every mapper that emits a
        // ContentId, and a ContentId **key** is the heavier class. "This domain
        // has no ContentId" is a sweep result and not a silence (d35's rule), so
        // the list is asserted rather than described.
        let keys: Vec<u32> = schema()
            .collections
            .iter()
            .flat_map(|c| c.fields.iter())
            .filter(|f| f.kind == Kind::ContentId && f.is_key())
            .map(|f| f.field_id)
            .collect();
        let values: Vec<u32> = schema()
            .collections
            .iter()
            .flat_map(|c| c.fields.iter())
            .filter(|f| f.kind == Kind::ContentId && !f.is_key())
            .map(|f| f.field_id)
            .collect();
        assert_eq!(keys, vec![field::TAG_DB_ID], "the heavier retrofit class");
        assert_eq!(
            values,
            vec![field::TACTIC_DATABASE_ID, field::TACTIC_ID],
            "a raw i32 cast and a §5 symbol id"
        );
    }
}
