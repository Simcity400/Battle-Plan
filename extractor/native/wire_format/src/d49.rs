//! d49 `naval_combat_lifecycle_live` — the per-battle scalars of every live
//! naval combat.
//!
//! Derived from `world_fold.rs:20794`-`:20882`
//! (`fold_naval_combat_lifecycle`, dispatched at `:4718`), the lane codec that
// @world-fold-authority domain=49 functions=fold_naval_combat_lifecycle lines=20794-20882 sha256=513cfd661b84ade3f5778895882af340697db1795ab087292bd87d3ddf297eb6
//! owns the payload grammar
//! (`extractor/native/lanes/naval_combat_live/src/domain49_50_naval_combat_live.rs`),
//! the fold's own renderer (`world_fold.rs:8765`-`:8838`), and the collector walk
//! that produces the presence flags
//! (`extractor/native/collector/src/attach_log.rs:39800`-`:39860`,
//! `:40890`-`:40925`). There is no `extractor/documents/domains/d49_*.md`, so
//! those four are the whole basis.
//!
//! **Payload grammar** (`domain49_50_naval_combat_live.rs:1789`-`:1830` for the
//! header, `:1702`-`:1739` for the rows, `:1832`-`:1864` for the string table): a
//! 28-byte header, `row_count` fixed 72-byte rows, and a one-entry string table
//! that must be empty, with **no trailing bytes**. There is no legacy short row —
//! the header's own `row_len` word must equal 72 (`:1807`), so the length never
//! selects a row form.
//!
//! Header:
//!
//! | offset | width | field | note |
//! |---|---|---|---|
//! | `+0`  | u16 | `schema_version` | must be `2` (`:1802`) |
//! | `+2`  | u16 | `row_len`        | must be `72` (`:1807`) |
//! | `+4`  | u64 | `elapsed_wall_ms`| collector timing, not game state |
//! | `+12` | u32 | `status_flags`   | the failure gate and the lane-local bit 11 |
//! | `+16` | u32 | `row_count`      | `<= 100_000` (`:1817`) |
//! | `+20` | u32 | `string_count`   | must be `1` (`:1820`) |
//! | `+24` | u32 | `reserved`       | must be `0` (`:1812`) |
//!
//! **There is no `changed_rows` word at all**, so there is no row-count invariant
//! to enforce and none is invented. Asserting `changed_rows == row_count` on a
//! pure incremental payload was the d10 round-1 finding; this payload does not
//! even carry the word.
//!
//! Row (`:1721`-`:1734`):
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+0`  | u32 | `combat_id`              | raw       | none (key; `1..=0x7fff_ffff`, `:1878`) |
//! | `+4`  | u64 | `generation`             | raw       | none (key; unconstrained) |
//! | `+12` | u64 | `body_ptr`               | —         | **never read** — see below |
//! | `+20` | u32 | `flags`                  | —         | — |
//! | `+24` | u32 | `location_id`            | raw       | `1<<1`; present ⇒ `1..=1_000_000` (`:1884`) |
//! | `+28` | u32 | `region_id`              | raw       | `1<<6`; present ⇒ `1..=100_000` (`:1897`) |
//! | `+32` | u32 | `attacker_member_count`  | raw       | `1<<2` |
//! | `+36` | u32 | `defender_member_count`  | raw       | `1<<3` |
//! | `+40` | i32 | `attacker_positioning`   | `/100_000` | `1<<4`; present ⇒ `0..=100_000` (`:1934`) |
//! | `+44` | i32 | `defender_positioning`   | `/100_000` | `1<<5`; present ⇒ `0..=100_000` (`:1942`) |
//! | `+48` | i64 | `attacker_anti_air`      | `/100_000` | `1<<7`; no range check |
//! | `+56` | i64 | `defender_anti_air`      | `/100_000` | `1<<8`; no range check |
//! | `+64` | u64 | `reserved_tail`          | —         | must be `0` (`:1960`) |
//!
//! Every gated-off word must additionally be packed as zero — the lane checks
//! that field by field (`:1892`, `:1905`, `:1910`, `:1915`, `:1920`, `:1927`,
//! `:1950`, `:1955`), which is what makes an ungated non-zero word a malformed row
//! rather than data.
//!
//! **The two scales are both `/100_000`, and both cells are signed.** The fold's
//! renderer divides `attacker_positioning_raw`/`defender_positioning_raw` — `i32`
//! — by `100_000.0` at `world_fold.rs:8800` and `:8814`, and
//! `attacker_anti_air_raw`/`defender_anti_air_raw` — `i64` — by the same constant
//! at `:8825` and `:8836`. This is the one domain in the survey where a single
//! `x100000` grid appears in **both** an `i32` and an `i64` cell, so
//! `every_scaled_field_reads_at_its_own_offset` checks all four independently:
//! reading either pair at `32768` would be ~3x wrong and every round-trip test
//! would still pass.
//!
//! **`body_ptr` is a witness, not a field.** It is a raw process address the
//! collector uses to detect combat-id reuse — a new body under an old id starts a
//! new `generation` (`attach_log.rs:40896`-`:40903`) — and the lane's own decoder
//! **discards it outright**, handing the fold `body_ptr: 0` on every row
//! (`domain49_50_naval_combat_live.rs:512`). The fold therefore never sees it, no
//! authority constrains it, and nothing here reads or rejects any value of it. The
//! information it carries is already on the wire as `generation`.
//!
//! **Pure cumulative delta: no baseline record, no mode word** (recipe shape 3).
//! The fold says so in its own words at `world_fold.rs:20091`-`:20095`: *"the
//! collector emits every active row on the first frame (empty previous baseline)
//! and only changes thereafter, so no separate baseline record exists to gate
//! on."* There is consequently **no** `reject_delta_before_baseline` call, no
//! `replace_or_sparse_after_first`, and no `FrameMode` — every record is a patch
//! of upserts and low-bit deletes against a retained accumulator. So a
//! domain-hour's state is this module's [`State`] after the hour's record, never
//! the record's rows, and every caller folds through the one
//! [`State::fold_payload`].
//!
//! **Two whole-hour fold refusals sit at DIFFERENT positions around the failure
//! gate, and the order is load-bearing.** Copying one domain's order into another
//! was a measured review finding (d62), so both are cited:
//!
//! 1. **Location/region promotion incomplete — BEFORE `handle_compact_failure`**
//!    (`world_fold.rs:20047`-`:20062`). Fires when the payload's status word
//!    carries the lane-local bit 11 `STATUS_LOCATION_CANDIDATE_UNVALIDATED`
//!    (`domain49_50_naval_combat_live.rs:32`) **or** any non-deleted row omits
//!    `location_id` or `region_id`. Under the recorder's `boundary_validation`
//!    mode this returns `Err`; on the production path it marks the domain-hour
//!    unknown, retains the accumulator, and returns `Ok`.
//! 2. **`handle_compact_failure`** (`:20066`-`:20072`, the helper at `:2863`),
//!    through `compact_status_failure_flags_after_expected_partial_tolerance`.
//!    d49 has no expected-partial tolerance, but it **does** have a registered
//!    lane-local mask (`compact_contract/src/lib.rs:93`-`:101`), so bit 11 is
//!    stripped before the failure test and is never confused with the global
//!    `COMPACT_STATUS_TRANSITION_BASELINE_RESET` that shares its value.
//! 3. **Side scalars incomplete — AFTER `handle_compact_failure`**
//!    (`:20076`-`:20087`). Fires when any non-deleted row omits any of
//!    `attacker_positioning`, `defender_positioning`, `attacker_anti_air` or
//!    `defender_anti_air`.
//!
//! All three fold nothing and retain the accumulator, so all three are **excluded
//! hours** for a measurement harness — not stream rejections. The only stream
//! rejection is a payload `decode_lifecycle_rows` refuses (`:20045`), which the
//! fold turns into `Err`: that says the bytes are not this grammar.
//!
//! **What the two refusals imply about the four-state decisions, and it decides
//! six of the eight gated fields.** Refusals 1 and 3 are payload-level: a payload
//! containing a live row that omits `location_id`, `region_id` or any of the four
//! side scalars folds **nothing at all**. So no row the fold ever folds can have
//! any of those six gates clear, and the mapper can never emit an empty state for
//! them. Per the recipe's "declare exactly what your mapper can emit", none of the
//! six declares `optional`, `nullable` or `unknown_capable`; and because such a
//! row can only come from a payload the fold refuses, [`record_from_row`] rejects
//! it rather than inventing a state — the same shape as its rejection of a deleted
//! row. This is **not** gating against the fold: the fold publishes nothing for
//! that row either. `the_fold_never_folds_a_row_the_mapper_rejects` pins the
//! implication, and `fold_payload` implements the refusals at their own positions
//! so the *reason* an hour is excluded stays recoverable.
//!
//! **The two member counts are the only fields with a reachable empty state, and
//! it is `unknown`.** They are gated by bits 2 and 3 and no whole-hour refusal
//! mentions them, so a folded row may carry them clear. Today's collector always
//! sets both — it writes `Some(combat.attacker_member_count)` unconditionally
//! (`attach_log.rs:40917`-`:40918`) over a counter it incremented itself while
//! walking the members it read (`:39995`-`:40001`) — so the clear gate is not
//! reachable through the shipping producer and this capture never exercises it.
//! It is mapped anyway, because a port may not refuse what the fold accepts merely
//! because the emitter provably never produces it (the d22 ruling). The state is
//! **unknown**: a naval battle always has two sides, so there is no relation
//! pointing at nothing (not `null`) and the concept applies to every row (not
//! `absent`); a clear gate could only mean the count was not reported, which is
//! §3's "applicable, not captured". A side with genuinely no members reads `0`
//! with its flag **set** — 0 is a measured value in this capture — and per §3 that
//! is a real `Zero`. This is a tie-break under the recipe's "choose Unknown" rule,
//! since no producer path exercises the clear gate; it is in the deliverable's
//! open questions.
//!
//! **A deleted row is a removal, not a record** (`world_fold.rs:20097`-`:20100`).
//! Bit 0 is `LIFECYCLE_ROW_DELETED`; the fold removes the key and `continue`s
//! without reading another word. In the measured capture the collector packs
//! tombstones carrying the last-known values with every presence bit still set, so
//! mapping one would publish a live battle that no longer exists.
//! [`State::fold_payload`] branches on [`is_deleted_row`] before ever calling
//! [`record_from_row`], which rejects a deleted row outright.
//!
//! **Duplicate keys inside one payload are legal and are applied in order.**
//! Unlike its d50 sibling, which refuses a repeated member identity
//! (`world_fold.rs:20160`-`:20166`), this fold walks the rows and upserts or
//! removes each in turn (`:20096`-`:20125`) with no uniqueness check. The fold's
//! permissive last-writer-wins collapse is mirrored exactly, so the keyed set
//! reproduces its stored state; `duplicate_keys_in_one_payload_apply_in_order` is
//! the test.
//!
//! **No derived field (§6).** Nothing here changes deterministically with the
//! clock: positioning and anti-air move with the battle, and a member count moves
//! when a ship joins or sinks. The mapper still takes `frame_tick` so every
//! domain's boundary has one shape.

use std::collections::BTreeMap;

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Quant, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 49;
pub const COMBATS: u32 = 1;

/// Field ids are stable and append-only; §11 forbids reuse for a different field.
pub mod field {
    pub const COMBAT_ID: u32 = 1;
    pub const GENERATION: u32 = 2;
    pub const LOCATION_ID: u32 = 3;
    pub const REGION_ID: u32 = 4;
    pub const ATTACKER_MEMBER_COUNT: u32 = 5;
    pub const DEFENDER_MEMBER_COUNT: u32 = 6;
    pub const ATTACKER_POSITIONING: u32 = 7;
    pub const DEFENDER_POSITIONING: u32 = 8;
    pub const ATTACKER_ANTI_AIR: u32 = 9;
    pub const DEFENDER_ANTI_AIR: u32 = 10;
}

// --- Payload grammar, from `domain49_50_naval_combat_live.rs`. ----------------

/// `PAYLOAD_HEADER_LEN`, `domain49_50_naval_combat_live.rs:25`.
pub const HEADER_LEN: usize = 28;
/// `LIFECYCLE_ROW_LEN`, `:26`. Fixed; there is no legacy form.
pub const ROW_LEN: usize = 72;
/// `SCHEMA_VERSION`, `:24`. Version 2 is what promoted the two positioning cells
/// from rounded milli-units to the exact `x100000` values (`:19`-`:23`).
pub const SCHEMA_VERSION: u16 = 2;
/// The lane emits exactly one, empty, string-table entry (`:1820`, `:1850`).
pub const STRING_COUNT: u32 = 1;
/// The trailing string table: one `u16` length word, which must be zero.
pub const STRING_TABLE_LEN: usize = 2;
/// `:1817`.
pub const MAX_ROW_COUNT: u32 = 100_000;

/// `LIFECYCLE_ROW_DELETED`, `:38`, applied at `world_fold.rs:20098`.
pub const ROW_FLAG_DELETED: u32 = 1 << 0;
pub const ROW_FLAG_LOCATION_PRESENT: u32 = 1 << 1;
pub const ROW_FLAG_ATTACKER_COUNT_PRESENT: u32 = 1 << 2;
pub const ROW_FLAG_DEFENDER_COUNT_PRESENT: u32 = 1 << 3;
pub const ROW_FLAG_ATTACKER_POSITIONING_PRESENT: u32 = 1 << 4;
pub const ROW_FLAG_DEFENDER_POSITIONING_PRESENT: u32 = 1 << 5;
pub const ROW_FLAG_REGION_PRESENT: u32 = 1 << 6;
pub const ROW_FLAG_ATTACKER_ANTI_AIR_PRESENT: u32 = 1 << 7;
pub const ROW_FLAG_DEFENDER_ANTI_AIR_PRESENT: u32 = 1 << 8;
/// `LIFECYCLE_KNOWN_FLAGS`, `:62`-`:70`. Any other bit is a hard error (`:1872`).
pub const KNOWN_ROW_FLAGS: u32 = ROW_FLAG_DELETED
    | ROW_FLAG_LOCATION_PRESENT
    | ROW_FLAG_ATTACKER_COUNT_PRESENT
    | ROW_FLAG_DEFENDER_COUNT_PRESENT
    | ROW_FLAG_ATTACKER_POSITIONING_PRESENT
    | ROW_FLAG_DEFENDER_POSITIONING_PRESENT
    | ROW_FLAG_REGION_PRESENT
    | ROW_FLAG_ATTACKER_ANTI_AIR_PRESENT
    | ROW_FLAG_DEFENDER_ANTI_AIR_PRESENT;

/// The six gates a folded row can never have clear, because refusals 1 and 3
/// refuse the whole payload that carries such a row. See the module docs.
pub const REQUIRED_ON_A_FOLDED_LIVE_ROW: u32 = ROW_FLAG_LOCATION_PRESENT
    | ROW_FLAG_REGION_PRESENT
    | ROW_FLAG_ATTACKER_POSITIONING_PRESENT
    | ROW_FLAG_DEFENDER_POSITIONING_PRESENT
    | ROW_FLAG_ATTACKER_ANTI_AIR_PRESENT
    | ROW_FLAG_DEFENDER_ANTI_AIR_PRESENT;

/// The renderer's divisor for all four scaled cells (`world_fold.rs:8800`,
/// `:8814`, `:8825`, `:8836`). It is `10^5`, so `Quant::Fixed(5)` is exactly the
/// source's own grid rather than a precision budget.
pub const FIXED_SCALE: f64 = 100_000.0;
pub const SCALE_DIGITS: u8 = 5;

/// `MAX_COMBAT_ID`, `domain49_50_naval_combat_live.rs:86`, enforced at `:1878`
/// together with a zero rejection.
pub const MAX_COMBAT_ID: u32 = 0x7fff_ffff;
/// `MAX_LOCATION_ID`, `:92`, enforced at `:1884` (zero also rejected).
pub const MAX_LOCATION_ID: u32 = 1_000_000;
/// `MAX_REGION_ID`, `:93`, enforced at `:1897` (zero also rejected).
pub const MAX_REGION_ID: u32 = 100_000;
/// `MAX_POSITIONING_RAW`, `:96`. The lane admits a present positioning cell only
/// in the closed `[0, MAX_POSITIONING_RAW]` interval (`:1934`, `:1942`), which is
/// why `Fixed(5)` is exact for that pair unconditionally.
pub const MAX_POSITIONING_RAW: i32 = 100_000;

/// The magnitude of an anti-air raw up to which `Kind::Continuous` + `Fixed(5)`
/// reproduces the source integer **exactly**. It is a documented property, **not
/// a gate**: the mapper accepts every `i64` the lane accepts, and the lane
/// range-checks the anti-air cells not at all (there is no `MAX_*` predicate for
/// them in `validate_decoded_lifecycle_row`, unlike location, region and
/// positioning).
///
/// An anti-air raw is an `i64` (`:1731`-`:1732`) and `Value::Continuous` is a
/// binary64, so the pair is exact only while the round trip
/// `round_ties_even((raw / 10^5) * 10^5)` returns `raw`. That holds for every
/// `|raw| <= 2^51` on every magnitude `fixed_five_reproduces_every_raw_in_the_band`
/// tries — the edges, one value per decade, the 2,000 magnitudes immediately below
/// the band's top, and 20,000 deterministic pseudorandom draws. That is a SAMPLE,
/// not an enumeration, so the honest statement is "no failure was found at or
/// below `2^51`", and this constant is published on that evidence. The maximal
/// exact band is not claimed in either direction; `d38::FIRST_KNOWN_INEXACT_AMOUNT_RAW`
/// is one existence witness above it, which does not make it the smallest failing
/// magnitude nor establish that the exact magnitudes form a contiguous run.
///
/// Above the band the stored value is still a legal point of the declared `10^-5`
/// grid and encode/decode stays exactly self-consistent (§6's escape rule keeps an
/// unstable integer off the quantized path, `value.rs:195`), but a consumer
/// recovering the raw integer may see it differ by one. Closing that residue needs
/// an integer-domain `Fixed` in the codec — a `WIRE_FORMAT.md` §13 item, not a
/// schema-side decision. It is in the deliverable's open questions.
///
/// For scale: the band is ~2.25e10 in game units, and the largest anti-air raw
/// over the whole `batch3/deepwar_a` capture is 5,612,750 — 56.13 game units.
pub const EXACT_ANTI_AIR_RAW_BAND: i64 = 1 << 51;

/// `COMPACT_STATUS_DELTA_DATA`, `compact_contract`. Named here because d49 is
/// not admitted to the production-codec dependency surface; the measurement
/// bin asserts the local value and shipping contract agree.
///
/// It is **informational for this domain**: d49's fold never reads it, because a
/// pure cumulative delta has no baseline to gate against. Parsed and reported,
/// never enforced.
pub const STATUS_DELTA_DATA: u32 = 1 << 10;

/// `DOMAIN49_STATUS_LOCATION_CANDIDATE_UNVALIDATED` — the lane-local badge at bit
/// 11 (`domain49_50_naval_combat_live.rs:32`,
/// `compact_contract/src/lib.rs:92`-`:93`). Its value overlaps the global
/// `COMPACT_STATUS_TRANSITION_BASELINE_RESET`, which is why
/// `lane_local_status_mask(49)` strips it before any global interpretation.
pub const STATUS_LOCATION_CANDIDATE_UNVALIDATED: u32 = 1 << 11;
/// `lane_local_status_mask(49)`, `compact_contract/src/lib.rs:94`-`:101`.
pub const LANE_LOCAL_STATUS_MASK: u32 = STATUS_LOCATION_CANDIDATE_UNVALIDATED;

/// `COMPACT_STATUS_FAILURE_MASK`. d49 carries no expected-partial tolerance
/// (`compact_contract/src/lib.rs` lists domains 26, 27, 28 and 54 only), so once
/// the lane-local mask is stripped this is the whole failure predicate for this
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
    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![Collection {
            collection_id: COMBATS,
            // `(combat_id, generation)` is the fold's own key
            // (`world_fold.rs:20097`) and it persists across hours: the fold
            // patches and deletes against it rather than rebuilding the table,
            // which is what a keyed set needs — and only a keyed set has a delta
            // grammar (§7.3).
            shape: Shape::KeyedSet,
            fields: vec![
                // `Unsigned`, not `ContentId`: a naval combat id is a runtime
                // instance identity minted while the battle exists, not something
                // a content manifest resolves. Battle 165521 is a different battle
                // next campaign, and no mod set defines it.
                Field::new(f::COMBAT_ID, Kind::Unsigned).key(0),
                // `Unsigned`: the collector's own epoch counter, bumped whenever a
                // combat id is reused on a new body (`attach_log.rs:40896`-`:40903`).
                // Runtime, not content, and unconstrained by any authority.
                Field::new(f::GENERATION, Kind::Unsigned).key(1),
                // `ContentId`: a province id is content-defined (§5) — a manifest
                // must resolve province 2757 for it to keep its meaning under
                // another mod set, which is the deciding question for
                // ContentId-vs-Unsigned. The fold renders this word as
                // `location_province` (`world_fold.rs:8777`), and d20 classes its
                // `location_province` the same way. Never empty on a folded row;
                // see the module docs.
                Field::new(f::LOCATION_ID, Kind::ContentId),
                // `ContentId` for the same reason: a naval strategic region is a
                // content definition, and the collector obtains it by looking the
                // province up in a province→region map
                // (`attach_log.rs:39810`-`:39816`). Never empty on a folded row.
                Field::new(f::REGION_ID, Kind::ContentId),
                // `Unsigned`, not `Monotone`: a side's member count falls when a
                // ship sinks or disengages as readily as it rises. These are the
                // only two fields with a reachable empty state, and it is
                // `unknown` — see the module docs.
                Field::new(f::ATTACKER_MEMBER_COUNT, Kind::Unsigned).unknown_capable(),
                Field::new(f::DEFENDER_MEMBER_COUNT, Kind::Unsigned).unknown_capable(),
                // The four scaled cells. `Continuous` because the fold's renderer
                // divides every one of them by `FIXED_SCALE`.
                //
                // `Quant::Fixed(5)` is not a precision *budget*: the source is a
                // signed CFixedPoint over a `10^5` denominator, so `fixed(5)` is
                // the source's own grid and reproduces the source integer bit for
                // bit. §6's "exact unless justified" bar is met by a stronger fact
                // than a precision argument — nothing is rounded at all. It also
                // matters that the raw integer survives: the fold's renderer emits
                // `attacker_positioning_raw` beside `attacker_positioning`
                // (`world_fold.rs:8790`-`:8795`) and the same for the other three,
                // so a downstream consumer requires the integer, and `fixed(5)` is
                // precisely the quantization under which `round(value * 1e5)`
                // returns it unchanged, where `exact` would spend 8 raw bytes per
                // value to carry the same information.
                //
                // The comparison against `Quant::Exact` holds in BOTH directions.
                // For the positioning pair it is unconditional: the lane admits a
                // present cell only in `[0, 100_000]` (`:1934`), so `fixed(5)` is
                // exact over the entire accepted range. For the anti-air pair the
                // lossy step at large magnitude is `raw as f64` in the mapper,
                // which both quantizations share, so `exact` recovers the source
                // integer over exactly the same band (`EXACT_ANTI_AIR_RAW_BAND`)
                // and spends 8 bytes doing it; above that band neither carries the
                // integer exactly. So `fixed(5)` never loses anything `exact` would
                // have kept.
                //
                // Never empty on a folded row; see the module docs.
                Field::new(f::ATTACKER_POSITIONING, Kind::Continuous)
                    .quant(Quant::Fixed(SCALE_DIGITS)),
                Field::new(f::DEFENDER_POSITIONING, Kind::Continuous)
                    .quant(Quant::Fixed(SCALE_DIGITS)),
                Field::new(f::ATTACKER_ANTI_AIR, Kind::Continuous)
                    .quant(Quant::Fixed(SCALE_DIGITS)),
                Field::new(f::DEFENDER_ANTI_AIR, Kind::Continuous)
                    .quant(Quant::Fixed(SCALE_DIGITS)),
            ],
        }],
        event_part: None,
    }
}

// --- The boundary mapping ----------------------------------------------------

/// The 28-byte payload header (`domain49_50_naval_combat_live.rs:1789`-`:1830`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub schema_version: u16,
    pub row_len: u16,
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub row_count: u32,
    pub string_count: u32,
    pub reserved: u32,
}

impl PayloadHeader {
    /// Informational only — d49's fold never reads this bit. Reported by the
    /// harness, enforced nowhere.
    pub fn is_delta(self) -> bool {
        self.status_flags & STATUS_DELTA_DATA != 0
    }

    /// The lane-local location-candidate badge (`:32`). Half of refusal 1.
    pub fn location_candidate_unvalidated(self) -> bool {
        self.status_flags & STATUS_LOCATION_CANDIDATE_UNVALIDATED != 0
    }

    /// The status word as global interpretation must see it: the lane-local mask
    /// stripped first (`compact_contract::compact_status_for_global_interpretation`,
    /// `compact_contract/src/lib.rs:105`-`:110`). Without this, bit 11 reads as
    /// `COMPACT_STATUS_TRANSITION_BASELINE_RESET`.
    pub fn status_for_global_interpretation(self) -> u32 {
        self.status_flags & !LANE_LOCAL_STATUS_MASK
    }

    /// The fold's failure gate (`world_fold.rs:20066`, helper at `:2863`).
    pub fn is_failure(self) -> bool {
        self.status_for_global_interpretation() & FAILURE_STATUS_MASK != 0
    }
}

/// Parse and check the payload header, applying every check
/// `decode_payload_header` performs (`domain49_50_naval_combat_live.rs:1789`-`:1830`),
/// in its own order.
///
/// **Position.** Every one of these runs inside `decode_lifecycle_rows`, which the
/// fold calls FIRST and whose `Err` it propagates (`world_fold.rs:20044`-`:20046`).
/// So a header failure is a stream rejection for every payload whatever its status
/// — d49's status gates cannot shield a malformed header, unlike the folds whose
/// early exits precede their length check.
pub fn parse_header(payload: &[u8]) -> core::result::Result<PayloadHeader, String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "naval_combat_lifecycle payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    let header = PayloadHeader {
        schema_version: u16_at(payload, 0),
        row_len: u16_at(payload, 2),
        elapsed_wall_ms: u64::from_le_bytes(payload[4..12].try_into().expect("8 bytes")),
        status_flags: u32_at(payload, 12),
        row_count: u32_at(payload, 16),
        string_count: u32_at(payload, 20),
        reserved: u32_at(payload, 24),
    };
    // The lane's own order: version, row length, reserved, row-count limit,
    // string count (`:1802`-`:1824`).
    if header.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "naval_combat_lifecycle unsupported schema version {}",
            header.schema_version
        ));
    }
    if header.row_len as usize != ROW_LEN {
        return Err(format!(
            "naval_combat_lifecycle row length {} != expected {ROW_LEN}",
            header.row_len
        ));
    }
    if header.reserved != 0 {
        return Err(format!(
            "naval_combat_lifecycle payload reserved field is nonzero ({})",
            header.reserved
        ));
    }
    if header.row_count > MAX_ROW_COUNT {
        return Err(format!(
            "naval_combat_lifecycle row count {} exceeds limit {MAX_ROW_COUNT}",
            header.row_count
        ));
    }
    if header.string_count != STRING_COUNT {
        return Err(
            "naval_combat_lifecycle expects exactly one empty string table entry".to_string(),
        );
    }
    Ok(header)
}

/// Split a d49 DATA payload into its header and its rows slice, adding the lane's
/// row-bounds walk (`:1717`) and its string-table check (`:1832`-`:1864`) to
/// [`parse_header`]'s invariants.
///
/// The string table is one `u16` length word that must be zero, and **the payload
/// must end exactly there**: `cursor != payload.len()` is reported as trailing
/// bytes (`:1857`), which makes the total length exactly
/// `HEADER_LEN + row_count * ROW_LEN + STRING_TABLE_LEN`.
///
/// This lives here rather than in each caller so the header grammar has exactly
/// one implementation: a measurement harness that re-derives it is a harness that
/// can drift from the recorder without anything failing.
pub fn split_payload(payload: &[u8]) -> core::result::Result<(PayloadHeader, &[u8]), String> {
    let header = parse_header(payload)?;
    // Checked arithmetic: `row_count` is input from a file on disk, and
    // `row_count * ROW_LEN` overflows `usize` for a large enough word. The lane
    // reaches the same rejection by walking row by row and running out of buffer.
    let rows_len = (header.row_count as usize)
        .checked_mul(ROW_LEN)
        .ok_or_else(|| {
            format!(
                "naval_combat_lifecycle row_count {} overflows",
                header.row_count
            )
        })?;
    let expected = rows_len
        .checked_add(HEADER_LEN)
        .and_then(|len| len.checked_add(STRING_TABLE_LEN))
        .ok_or_else(|| {
            format!(
                "naval_combat_lifecycle row_count {} overflows",
                header.row_count
            )
        })?;
    if payload.len() != expected {
        return Err(format!(
            "naval_combat_lifecycle len {} != {expected}",
            payload.len()
        ));
    }
    let table_at = HEADER_LEN + rows_len;
    let string_len = u16_at(payload, table_at);
    if string_len != 0 {
        return Err(format!(
            "naval_combat_lifecycle string table index 0 must be empty (len {string_len})"
        ));
    }
    Ok((header, &payload[HEADER_LEN..table_at]))
}

/// The key of one compact row: `(combat_id, generation)` (`world_fold.rs:20097`).
///
/// `combat_id` carries the lane's own zero and upper-bound rejection (`:1878`);
/// `generation` carries none, because no authority constrains it.
pub fn key_from_row(row: &[u8]) -> core::result::Result<Key, String> {
    let (combat_id, generation) = key_fields_of_row(row)?;
    Ok(key(&[combat_id as u64, generation]))
}

/// The key's two components, with the lane's `combat_id` predicate (`:1878`).
pub fn key_fields_of_row(row: &[u8]) -> core::result::Result<(u32, u64), String> {
    check_row_len(row)?;
    let combat_id = u32_at(row, 0);
    if combat_id == 0 || combat_id > MAX_COMBAT_ID {
        return Err(format!(
            "naval_combat_lifecycle row invalid combat_id {combat_id}"
        ));
    }
    Ok((combat_id, u64_at(row, 4)))
}

/// The row's flag word, with the lane's unknown-bit rejection (`:1872`).
pub fn row_flags(row: &[u8]) -> core::result::Result<u32, String> {
    check_row_len(row)?;
    let flags = u32_at(row, 20);
    let unknown = flags & !KNOWN_ROW_FLAGS;
    if unknown != 0 {
        return Err(format!(
            "naval_combat_lifecycle row has unknown flags {unknown:#010x}"
        ));
    }
    Ok(flags)
}

/// Whether this row removes its key from the set rather than carrying a state
/// (`world_fold.rs:20098`-`:20101`).
pub fn is_deleted_row(row: &[u8]) -> core::result::Result<bool, String> {
    Ok(row_flags(row)? & ROW_FLAG_DELETED != 0)
}

/// Every predicate `validate_decoded_lifecycle_row` applies
/// (`domain49_50_naval_combat_live.rs:1866`-`:1965`), in its own order, on any row
/// whether live or deleted.
///
/// The lane runs this over every decoded row before the fold sees any of them, so
/// a single bad row anywhere in the payload is a stream rejection, not a skipped
/// row. `record_from_row` and [`is_deleted_row`]'s caller both go through it.
pub fn validate_row(row: &[u8]) -> core::result::Result<(), String> {
    let flags = row_flags(row)?;
    let _ = key_fields_of_row(row)?;

    let location_id = u32_at(row, 24);
    if flags & ROW_FLAG_LOCATION_PRESENT != 0 {
        if location_id == 0 || location_id > MAX_LOCATION_ID {
            return Err(format!(
                "naval_combat_lifecycle row invalid location_id {location_id}"
            ));
        }
    } else if location_id != 0 {
        return Err("naval_combat_lifecycle row has location without flag".to_string());
    }

    let region_id = u32_at(row, 28);
    if flags & ROW_FLAG_REGION_PRESENT != 0 {
        if region_id == 0 || region_id > MAX_REGION_ID {
            return Err(format!(
                "naval_combat_lifecycle row invalid region_id {region_id}"
            ));
        }
    } else if region_id != 0 {
        return Err("naval_combat_lifecycle row has region without flag".to_string());
    }

    // The two counts have NO upper bound in the lane — only the zero-packing rule.
    if flags & ROW_FLAG_ATTACKER_COUNT_PRESENT == 0 && u32_at(row, 32) != 0 {
        return Err("naval_combat_lifecycle row has attacker count without flag".to_string());
    }
    if flags & ROW_FLAG_DEFENDER_COUNT_PRESENT == 0 && u32_at(row, 36) != 0 {
        return Err("naval_combat_lifecycle row has defender count without flag".to_string());
    }

    for (present, offset, side) in [
        (ROW_FLAG_ATTACKER_POSITIONING_PRESENT, 40usize, "attacker"),
        (ROW_FLAG_DEFENDER_POSITIONING_PRESENT, 44, "defender"),
    ] {
        let raw = i32_at(row, offset);
        if flags & present != 0 {
            if !(0..=MAX_POSITIONING_RAW).contains(&raw) {
                return Err(format!(
                    "naval_combat_lifecycle row {side} positioning raw {raw} is outside \
                     0..={MAX_POSITIONING_RAW}"
                ));
            }
        } else if raw != 0 {
            return Err(format!(
                "naval_combat_lifecycle row has {side} positioning without flag"
            ));
        }
    }

    // The anti-air cells are range-checked by NOTHING in the lane — only the
    // zero-packing rule applies. A cutoff here would refuse rows the recorder
    // folds; see `EXACT_ANTI_AIR_RAW_BAND`.
    for (present, offset, side) in [
        (ROW_FLAG_ATTACKER_ANTI_AIR_PRESENT, 48usize, "attacker"),
        (ROW_FLAG_DEFENDER_ANTI_AIR_PRESENT, 56, "defender"),
    ] {
        if flags & present == 0 && i64_at(row, offset) != 0 {
            return Err(format!(
                "naval_combat_lifecycle row has {side} anti-air without flag"
            ));
        }
    }

    if u64_at(row, 64) != 0 {
        return Err("naval_combat_lifecycle row trailing reserved field is nonzero".to_string());
    }
    Ok(())
}

/// One compact row of the `combats` table, as the collector emits it today, onto
/// its stored Record. `row` is the raw 72-byte row slice (the caller has already
/// consumed the table header via [`split_payload`]); d49 has exactly one row form,
/// so length selects nothing and a wrong length is rejected. `frame_tick` is §6's
/// derived-field input (stored = v + c * (frame_tick >> u)); d49 declares no
/// derived field, so it is taken and ignored — every domain's mapper has the same
/// shape.
///
/// Two row shapes are **preconditions of the grammar**, not values, and both are
/// rejected rather than mapped:
///
/// - A **deleted** row removes its key from the set (`world_fold.rs:20098`), so it
///   is not a record. [`State::fold_payload`] branches on [`is_deleted_row`]
///   before ever calling this.
/// - A **live row missing any of the six fields in [`REQUIRED_ON_A_FOLDED_LIVE_ROW`]**
///   can only occur inside a payload that refusal 1 or refusal 3 refuses whole
///   (`world_fold.rs:20047`-`:20062`, `:20076`-`:20087`), so the fold publishes
///   nothing for it either. Mapping it would require inventing an empty state for
///   a field that has none — see the module docs. This is a precondition, not a
///   narrowing: `the_fold_never_folds_a_row_the_mapper_rejects` pins that every
///   row reaching this function through `fold_payload` satisfies it.
///
/// Fail-closed throughout, mirroring `fold_naval_combat_lifecycle` and the lane's
/// own row validator: a row either would reject is an `Err` here, never a coerced
/// value. Nothing in this function can panic on any input.
pub fn record_from_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    // d49 has no derived field; §6's transform input is unused. Named rather than
    // `_frame_tick` because the signature is the cross-domain contract.
    let _ = frame_tick;
    validate_row(row)?;
    let flags = row_flags(row)?;
    if flags & ROW_FLAG_DELETED != 0 {
        return Err(
            "naval_combat_lifecycle deleted row is a removal, not a record".to_string(),
        );
    }
    let missing = REQUIRED_ON_A_FOLDED_LIVE_ROW & !flags;
    if missing != 0 {
        return Err(format!(
            "naval_combat_lifecycle live row omits gate(s) {missing:#010x}; the fold refuses \
             the whole payload that carries it (world_fold.rs:20047, :20076)"
        ));
    }

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::LOCATION_ID => Value::ContentId(u32_at(row, 24) as u64),
            field::REGION_ID => Value::ContentId(u32_at(row, 28) as u64),
            // The only reachable empty state in this schema, and it is `unknown`
            // — see the module docs.
            field::ATTACKER_MEMBER_COUNT => gated_unsigned(
                flags & ROW_FLAG_ATTACKER_COUNT_PRESENT != 0,
                u32_at(row, 32) as u64,
            ),
            field::DEFENDER_MEMBER_COUNT => gated_unsigned(
                flags & ROW_FLAG_DEFENDER_COUNT_PRESENT != 0,
                u32_at(row, 36) as u64,
            ),
            // `world_fold.rs:8800` / `:8814` — `f64::from(i32) / 100_000.0`.
            field::ATTACKER_POSITIONING => {
                Value::Continuous(f64::from(i32_at(row, 40)) / FIXED_SCALE)
            }
            field::DEFENDER_POSITIONING => {
                Value::Continuous(f64::from(i32_at(row, 44)) / FIXED_SCALE)
            }
            // `world_fold.rs:8825` / `:8836` — `i64 as f64 / 100_000.0`.
            field::ATTACKER_ANTI_AIR => {
                Value::Continuous(i64_at(row, 48) as f64 / FIXED_SCALE)
            }
            field::DEFENDER_ANTI_AIR => {
                Value::Continuous(i64_at(row, 56) as f64 / FIXED_SCALE)
            }
            other => {
                // A collection this mapper does not know how to fill. Filling it
                // with a placeholder would publish invented data under a field id
                // that means something else.
                return Err(format!(
                    "naval_combat_lifecycle has no mapping for field id {other}"
                ));
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// A gate that is clear means the count was not reported, not that there is
/// nothing to count — see the module docs. §3 calls that **unknown**.
fn gated_unsigned(present: bool, value: u64) -> Value {
    if present {
        Value::Unsigned(value)
    } else {
        Value::Unknown
    }
}

fn check_row_len(row: &[u8]) -> core::result::Result<(), String> {
    if row.len() != ROW_LEN {
        return Err(format!(
            "naval_combat_lifecycle row length {} != {ROW_LEN}",
            row.len()
        ));
    }
    Ok(())
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
    u64::from_le_bytes(b[off..off + 8].try_into().expect("8 bytes"))
}

fn i64_at(b: &[u8], off: usize) -> i64 {
    i64::from_le_bytes(b[off..off + 8].try_into().expect("8 bytes"))
}

// --- The frame fold ----------------------------------------------------------

/// What one d49 DATA record did to the accumulator.
///
/// Three of the four variants fold nothing and retain the accumulator, so each is
/// an **excluded hour** for a measurement harness rather than a stream rejection —
/// the recipe's "third outcome" rule. A stream rejection is an `Err` from
/// [`State::fold_payload`], which says the payload is not this grammar.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FoldOutcome {
    /// Refusal 1, **before** the failure gate (`world_fold.rs:20047`-`:20062`):
    /// the lane-local bit 11 is set, or a live row omits `location_id` or
    /// `region_id`. Under the recorder's `boundary_validation` mode this is an
    /// `Err`; on the production path the domain-hour is unknown and the
    /// accumulator is retained. Either way nothing folds, so the hour is not d49
    /// data and belongs in no measurement.
    LocationRegionIncomplete {
        /// The status badge fired, independent of the row scan.
        status_badge: bool,
        /// A live row omitted `location_id` or `region_id`.
        row_incomplete: bool,
    },
    /// `handle_compact_failure` (`world_fold.rs:2863`, called at `:20066`): the
    /// domain-hour is unknown and the accumulator is retained.
    FailureRetained,
    /// Refusal 3, **after** the failure gate (`:20076`-`:20087`): a live row
    /// omitted one of the four `+0x138`/`+0x150` side scalars.
    SideScalarsIncomplete,
    /// The record was applied. There is no `reset` here: d49 is a pure cumulative
    /// delta with no baseline record, so nothing ever clears the accumulator.
    Folded { upserts: usize, deletes: usize },
}

impl FoldOutcome {
    /// Whether this outcome contributed a measurable domain-hour. Exactly the
    /// fold's own `naval_combat_lifecycle_known_this_hour` witness, which is set
    /// only on the path that reaches `world_fold.rs:20126`.
    pub fn is_measured(self) -> bool {
        matches!(self, FoldOutcome::Folded { .. })
    }
}

/// The reconstructed `naval_combat_lifecycle` state, folded exactly as
/// `fold_naval_combat_lifecycle` folds it (`world_fold.rs:20040`-`:20129`).
///
/// d49 is a pure cumulative delta, so an hour's state is **not** an hour's rows:
/// it is this accumulator after the hour's record. Every caller — the measurement
/// harness and the tests — folds through this one implementation, because a second
/// one is a second answer to "what did hour N hold".
///
/// **The other writer of the recorder's accumulator, and why it is inert for this
/// collection.** `self.naval_combats` is written by exactly two folds
/// (verified by grep over `recorder_host` and `collector`):
/// `fold_naval_combat_lifecycle`, which inserts, removes and sets every lifecycle
/// scalar, and `fold_naval_combat_members` (d50), which only ever does
/// `combat.members.insert(..)` / `.remove(..)` on an entry it obtained with
/// `get_mut` (`world_fold.rs:20205`-`:20217`). d50 never inserts an entry, never
/// removes one, and never touches a lifecycle scalar — its `members` map is d50's
/// own collection, which this port does not carry. So d50 is **proved inert** for
/// d49's keyed set by construction, not merely on this capture; the measurement
/// bin additionally asserts it per capture by re-checking that the d49 row count
/// is the only thing that moves the key set.
///
/// **`fold_payload`'s error contract is all-or-nothing.** The lane decodes and
/// validates the entire payload before the fold applies a single row
/// (`world_fold.rs:20044`, `domain49_50_naval_combat_live.rs:496`-`:531`), so a
/// payload that fails anywhere leaves the accumulator exactly as it was. This
/// mirrors the recorder rather than assuming it.
#[derive(Clone, Debug, Default)]
pub struct State {
    /// The battle table, keyed by `(combat_id, generation)`.
    pub combats: KeyedState,
}

impl State {
    /// Fold one d49 DATA payload. `schema` must be [`schema()`]'s.
    ///
    /// The order of the checks is the fold's own order, and it is load-bearing:
    /// the lane's full decode and row validation first (`:20044`), then refusal 1
    /// (`:20047`), then the failure gate (`:20066`), then refusal 3 (`:20076`),
    /// and only then the row loop (`:20096`). Refusals 1 and 3 straddle the
    /// failure gate, so a failure-status payload whose live rows also lack a
    /// location is excluded for the *location* reason, while one whose live rows
    /// lack only a side scalar is excluded for the *failure* reason — the fold
    /// never reaches refusal 3 in that case.
    ///
    /// Every rejection path is an `Err` — a payload the recorder would refuse is
    /// refused here, never partially applied. On `Err` the accumulator is left
    /// untouched.
    pub fn fold_payload(
        &mut self,
        schema: &Schema,
        payload: &[u8],
        frame_tick: u64,
    ) -> core::result::Result<FoldOutcome, String> {
        // `decode_lifecycle_rows` (`:20044`): header, row bounds, string table and
        // every row predicate, all before the fold looks at anything. Its `Err`
        // propagates out of the fold, so it is a stream rejection.
        let (header, body) = split_payload(payload)?;
        let rows: Vec<&[u8]> = body.chunks_exact(ROW_LEN).collect();
        if rows.len() != header.row_count as usize {
            return Err(format!(
                "naval_combat_lifecycle decoded {} rows for row_count {}",
                rows.len(),
                header.row_count
            ));
        }
        for row in &rows {
            validate_row(row)?;
        }

        // Refusal 1, BEFORE the failure gate (`:20047`-`:20062`).
        let status_badge = header.location_candidate_unvalidated();
        let mut row_incomplete = false;
        for row in &rows {
            let flags = row_flags(row)?;
            if flags & ROW_FLAG_DELETED != 0 {
                continue;
            }
            if flags & (ROW_FLAG_LOCATION_PRESENT | ROW_FLAG_REGION_PRESENT)
                != (ROW_FLAG_LOCATION_PRESENT | ROW_FLAG_REGION_PRESENT)
            {
                row_incomplete = true;
                break;
            }
        }
        if status_badge || row_incomplete {
            return Ok(FoldOutcome::LocationRegionIncomplete {
                status_badge,
                row_incomplete,
            });
        }

        // `handle_compact_failure` (`:20066`), with the lane-local bit already
        // stripped by `PayloadHeader::is_failure`.
        if header.is_failure() {
            return Ok(FoldOutcome::FailureRetained);
        }

        // Refusal 3, AFTER the failure gate (`:20076`-`:20087`).
        const SIDE_SCALARS: u32 = ROW_FLAG_ATTACKER_POSITIONING_PRESENT
            | ROW_FLAG_DEFENDER_POSITIONING_PRESENT
            | ROW_FLAG_ATTACKER_ANTI_AIR_PRESENT
            | ROW_FLAG_DEFENDER_ANTI_AIR_PRESENT;
        for row in &rows {
            let flags = row_flags(row)?;
            if flags & ROW_FLAG_DELETED == 0 && flags & SIDE_SCALARS != SIDE_SCALARS {
                return Ok(FoldOutcome::SideScalarsIncomplete);
            }
        }

        let collection = schema
            .collections
            .iter()
            .find(|c| c.collection_id == COMBATS)
            .ok_or_else(|| {
                "naval_combat_lifecycle schema has no combats collection".to_string()
            })?;

        // Decode every row before touching the accumulator, so the all-or-nothing
        // error contract holds even for a caller that continues after an `Err`.
        let mut patch: Vec<(Key, Option<Record>)> = Vec::with_capacity(rows.len());
        for row in &rows {
            let k = key_from_row(row)?;
            if is_deleted_row(row)? {
                patch.push((k, None));
            } else {
                patch.push((k, Some(record_from_row(collection, row, frame_tick)?)));
            }
        }

        // The fold's own loop (`:20096`-`:20125`): in payload order, with no
        // uniqueness check, so a repeated key is last-writer-wins.
        let mut upserts = 0usize;
        let mut deletes = 0usize;
        for (k, record) in patch {
            match record {
                Some(record) => {
                    upserts += 1;
                    self.combats.insert(k, record);
                }
                None => {
                    deletes += 1;
                    self.combats.remove(&k);
                }
            }
        }
        Ok(FoldOutcome::Folded { upserts, deletes })
    }
}

/// A fresh, empty accumulator. `BTreeMap::new` behind a name, so callers do not
/// have to know the state's shape.
pub fn empty_state() -> State {
    State {
        combats: BTreeMap::new(),
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

    #[allow(clippy::too_many_arguments)]
    fn raw_row(
        combat_id: u32,
        generation: u64,
        body_ptr: u64,
        flags: u32,
        location_id: u32,
        region_id: u32,
        counts: [u32; 2],
        positioning: [i32; 2],
        anti_air: [i64; 2],
        reserved_tail: u64,
    ) -> Vec<u8> {
        let mut row = Vec::with_capacity(ROW_LEN);
        row.extend_from_slice(&combat_id.to_le_bytes());
        row.extend_from_slice(&generation.to_le_bytes());
        row.extend_from_slice(&body_ptr.to_le_bytes());
        row.extend_from_slice(&flags.to_le_bytes());
        row.extend_from_slice(&location_id.to_le_bytes());
        row.extend_from_slice(&region_id.to_le_bytes());
        for v in counts {
            row.extend_from_slice(&v.to_le_bytes());
        }
        for v in positioning {
            row.extend_from_slice(&v.to_le_bytes());
        }
        for v in anti_air {
            row.extend_from_slice(&v.to_le_bytes());
        }
        row.extend_from_slice(&reserved_tail.to_le_bytes());
        assert_eq!(row.len(), ROW_LEN);
        row
    }

    /// A complete live row: every presence bit set, as the shipping collector
    /// packs one (`attach_log.rs:40908`-`:40923`).
    fn live_row(
        combat_id: u32,
        generation: u64,
        location_id: u32,
        region_id: u32,
        counts: [u32; 2],
        positioning: [i32; 2],
        anti_air: [i64; 2],
    ) -> Vec<u8> {
        raw_row(
            combat_id,
            generation,
            0xdead_beef_0000_0001,
            KNOWN_ROW_FLAGS & !ROW_FLAG_DELETED,
            location_id,
            region_id,
            counts,
            positioning,
            anti_air,
            0,
        )
    }

    fn deleted_row(combat_id: u32, generation: u64) -> Vec<u8> {
        raw_row(
            combat_id,
            generation,
            0,
            ROW_FLAG_DELETED,
            0,
            0,
            [0; 2],
            [0; 2],
            [0; 2],
            0,
        )
    }

    fn full_row() -> Vec<u8> {
        live_row(
            165_521,
            7,
            2_757,
            41,
            [12, 5],
            [100_000, 14_084],
            [3_885_772, 0],
        )
    }

    fn payload(status_flags: u32, rows: &[Vec<u8>]) -> Vec<u8> {
        payload_with(SCHEMA_VERSION, ROW_LEN as u16, status_flags, rows.len() as u32,
                     STRING_COUNT, 0, rows, 0)
    }

    #[allow(clippy::too_many_arguments)]
    fn payload_with(
        schema_version: u16,
        row_len: u16,
        status_flags: u32,
        row_count: u32,
        string_count: u32,
        reserved: u32,
        rows: &[Vec<u8>],
        string_len: u16,
    ) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&schema_version.to_le_bytes());
        p.extend_from_slice(&row_len.to_le_bytes());
        p.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&row_count.to_le_bytes());
        p.extend_from_slice(&string_count.to_le_bytes());
        p.extend_from_slice(&reserved.to_le_bytes());
        assert_eq!(p.len(), HEADER_LEN);
        for r in rows {
            p.extend_from_slice(r);
        }
        p.extend_from_slice(&string_len.to_le_bytes());
        p
    }

    // ---- Schema ------------------------------------------------------------

    #[test]
    fn the_schema_is_valid() {
        schema()
            .validate(grid().log2())
            .expect("d49 schema is well-formed");
    }

    #[test]
    fn it_is_a_keyed_set_keyed_by_the_combat_and_its_generation() {
        let c = collection();
        assert_eq!(c.shape, Shape::KeyedSet, "deltas upsert and tombstone");
        let keys = c.key_fields();
        assert_eq!(keys.len(), 2, "the fold keys rows by (combat_id, generation)");
        assert_eq!(keys[0].field_id, field::COMBAT_ID);
        assert_eq!(keys[1].field_id, field::GENERATION);
        for k in &keys {
            assert_eq!(
                k.kind,
                Kind::Unsigned,
                "field {} is a runtime instance identity, not content",
                k.field_id
            );
            assert!(k.kind.legal_as_key());
            assert!(
                !k.optional && !k.nullable && !k.unknown_capable && k.derived.is_none(),
                "field {} is a key member and must be concrete",
                k.field_id
            );
        }
        assert_eq!(c.fields.len(), 10, "2 key + 8 gated value fields");
    }

    #[test]
    fn exactly_the_two_member_counts_declare_an_empty_state() {
        // The whole capability declaration, asserted as a set. The six fields the
        // two whole-hour refusals protect cannot reach an empty state on any row
        // the fold folds, so none of them declares one — every declared capability
        // costs real bits.
        let c = collection();
        assert_eq!(c.optional_count(), 0, "no field is ever inapplicable");
        assert!(
            c.value_fields().all(|f| !f.nullable),
            "no field is a relation that can point at nothing"
        );
        let unknown_capable: Vec<u32> = c
            .value_fields()
            .filter(|f| f.unknown_capable)
            .map(|f| f.field_id)
            .collect();
        assert_eq!(
            unknown_capable,
            vec![field::ATTACKER_MEMBER_COUNT, field::DEFENDER_MEMBER_COUNT]
        );
    }

    #[test]
    fn the_content_ids_are_exactly_the_province_and_the_region() {
        // §13's manifest-id rule will rewrite every ContentId mapper, so the
        // retrofit sweep needs "checked, these two" rather than silence. Neither
        // is a key component, so this domain is in the lighter retrofit class.
        let c = collection();
        let content: Vec<u32> = c
            .fields
            .iter()
            .filter(|f| matches!(f.kind, Kind::ContentId))
            .map(|f| f.field_id)
            .collect();
        assert_eq!(content, vec![field::LOCATION_ID, field::REGION_ID]);
        assert!(
            c.key_fields().iter().all(|f| f.kind != Kind::ContentId),
            "no ContentId key component: the heavier retrofit class does not apply"
        );
    }

    #[test]
    fn no_field_is_derived() {
        // §6 derived fields are clock-linear integers; nothing in d49 changes
        // deterministically with the clock, so `frame_tick` never enters a value.
        assert!(collection().fields.iter().all(|f| f.derived.is_none()));
    }

    // ---- Round trips -------------------------------------------------------

    #[test]
    fn a_full_row_roundtrips_byte_for_byte() {
        let c = collection();
        let record = record_from_row(&c, &full_row(), 0).expect("maps");
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
    fn the_unknown_member_counts_roundtrip_byte_for_byte() {
        // The only empty state the schema declares, through the codec, one gate at
        // a time and then both.
        let c = collection();
        for cleared in [
            ROW_FLAG_ATTACKER_COUNT_PRESENT,
            ROW_FLAG_DEFENDER_COUNT_PRESENT,
            ROW_FLAG_ATTACKER_COUNT_PRESENT | ROW_FLAG_DEFENDER_COUNT_PRESENT,
        ] {
            let flags = (KNOWN_ROW_FLAGS & !ROW_FLAG_DELETED) & !cleared;
            // A gated-off word must pack a zero, so build the row that way.
            let counts = [
                if flags & ROW_FLAG_ATTACKER_COUNT_PRESENT != 0 { 12 } else { 0 },
                if flags & ROW_FLAG_DEFENDER_COUNT_PRESENT != 0 { 5 } else { 0 },
            ];
            let row = raw_row(9, 1, 0, flags, 2_757, 41, counts, [1, 2], [3, 4], 0);
            let record = record_from_row(&c, &row, 0).expect("maps");
            for (bit, id) in [
                (ROW_FLAG_ATTACKER_COUNT_PRESENT, field::ATTACKER_MEMBER_COUNT),
                (ROW_FLAG_DEFENDER_COUNT_PRESENT, field::DEFENDER_MEMBER_COUNT),
            ] {
                let expected = if cleared & bit != 0 {
                    Value::Unknown
                } else if bit == ROW_FLAG_ATTACKER_COUNT_PRESENT {
                    Value::Unsigned(12)
                } else {
                    Value::Unsigned(5)
                };
                assert_eq!(record.values[index_of(&c, id)], expected, "field {id}");
            }
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
    fn a_real_zero_is_a_value_not_an_empty_state() {
        // Every flag set, every measured word zero: a battle whose sides carry no
        // members, no positioning advantage and no anti-air. §3 keeps zero out of
        // the three empty states, and this capture really does contain
        // `attacker_member_count == 0` rows with the flag set.
        let c = collection();
        let row = live_row(1, 0, 1, 1, [0, 0], [0, 0], [0, 0]);
        let r = record_from_row(&c, &row, 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::ATTACKER_MEMBER_COUNT)],
            Value::Unsigned(0)
        );
        assert_eq!(
            r.values[index_of(&c, field::DEFENDER_MEMBER_COUNT)],
            Value::Unsigned(0)
        );
        for id in [
            field::ATTACKER_POSITIONING,
            field::DEFENDER_POSITIONING,
            field::ATTACKER_ANTI_AIR,
            field::DEFENDER_ANTI_AIR,
        ] {
            assert_eq!(
                r.values[index_of(&c, id)],
                Value::Continuous(0.0),
                "field {id} zero is a measured value"
            );
        }
        // `generation` is a key member and may legitimately be zero.
        assert_eq!(key_from_row(&row).expect("keys"), key(&[1, 0]));
    }

    // ---- Offsets and scales ------------------------------------------------

    #[test]
    fn every_field_reads_at_its_own_offset_through_its_own_scale() {
        // Step 8.6, the test that catches both a transposed pair and a
        // 32768-vs-100000 scale error. Every word is distinct, so a swap of any
        // two fails. The four scaled cells are checked against exact f64 readings.
        let c = collection();
        let row = live_row(
            0x0102_0304,
            0x0a0b_0c0d_0e0f_1011,
            123_456,
            65_432,
            [7, 9],
            [100_000, 50_000],
            [250_000, -125_000],
        );
        assert_eq!(
            key_from_row(&row).expect("keys"),
            key(&[0x0102_0304, 0x0a0b_0c0d_0e0f_1011])
        );
        let r = record_from_row(&c, &row, 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::LOCATION_ID)],
            Value::ContentId(123_456)
        );
        assert_eq!(
            r.values[index_of(&c, field::REGION_ID)],
            Value::ContentId(65_432)
        );
        assert_eq!(
            r.values[index_of(&c, field::ATTACKER_MEMBER_COUNT)],
            Value::Unsigned(7)
        );
        assert_eq!(
            r.values[index_of(&c, field::DEFENDER_MEMBER_COUNT)],
            Value::Unsigned(9)
        );
        // 100_000 / 100_000 == 1.0, 50_000 == 0.5. Read at 32768 these would be
        // 3.05 and 1.53, and every round-trip test would still pass.
        assert_eq!(
            r.values[index_of(&c, field::ATTACKER_POSITIONING)],
            Value::Continuous(1.0)
        );
        assert_eq!(
            r.values[index_of(&c, field::DEFENDER_POSITIONING)],
            Value::Continuous(0.5)
        );
        assert_eq!(
            r.values[index_of(&c, field::ATTACKER_ANTI_AIR)],
            Value::Continuous(2.5)
        );
        assert_eq!(
            r.values[index_of(&c, field::DEFENDER_ANTI_AIR)],
            Value::Continuous(-1.25)
        );

        // The i32 pair and the i64 pair share one grid but not one cell width, so
        // each is exercised at a magnitude the other cannot hold.
        let wide = live_row(1, 1, 1, 1, [0, 0], [1, 2], [i32::MAX as i64 + 1, -1]);
        let r = record_from_row(&c, &wide, 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::ATTACKER_POSITIONING)],
            Value::Continuous(1.0 / FIXED_SCALE)
        );
        assert_eq!(
            r.values[index_of(&c, field::DEFENDER_POSITIONING)],
            Value::Continuous(2.0 / FIXED_SCALE)
        );
        assert_eq!(
            r.values[index_of(&c, field::ATTACKER_ANTI_AIR)],
            Value::Continuous((i32::MAX as i64 + 1) as f64 / FIXED_SCALE)
        );
        assert_eq!(
            r.values[index_of(&c, field::DEFENDER_ANTI_AIR)],
            Value::Continuous(-1.0 / FIXED_SCALE)
        );
    }

    #[test]
    fn the_header_words_read_at_their_own_offsets() {
        // Step 8.6's header half: a transposed header word mis-slices every row
        // behind it and no round-trip or replay assertion can see that.
        //
        // Every word carries a DISTINCT value, so no pair of reads can be swapped
        // and still satisfy this test. Three of the seven are pinned to a constant
        // by the grammar — `schema_version` is 2, `row_len` is 72, `string_count`
        // is 1 — so `row_count` is 3 to keep it clear of all three, and the
        // perturbation sweep below is what covers the constrained words: it is not
        // enough to read them once at a value another word also holds.
        let rows = vec![full_row(), deleted_row(2, 1), deleted_row(3, 1)];
        let p = payload_with(
            SCHEMA_VERSION,
            ROW_LEN as u16,
            0xc00,
            3,
            STRING_COUNT,
            0,
            &rows,
            0,
        );
        let header = parse_header(&p).expect("parses");
        assert_eq!(header.schema_version, SCHEMA_VERSION);
        assert_eq!(header.row_len as usize, ROW_LEN);
        assert_eq!(header.status_flags, 0xc00);
        assert_eq!(header.row_count, 3);
        assert_eq!(header.string_count, STRING_COUNT);
        assert_eq!(header.reserved, 0);
        // Six of the seven values are pairwise distinct as written (2, 72,
        // 0, 0xc00, 3, 1, 0 — `elapsed_wall_ms` and `reserved` share 0, and the
        // sweep below separates them). Assert that, so a later edit cannot quietly
        // make two of them equal and blunt the test.
        let distinct = [
            u64::from(header.schema_version),
            u64::from(header.row_len),
            u64::from(header.status_flags),
            u64::from(header.row_count),
            u64::from(header.string_count),
        ];
        for (i, a) in distinct.iter().enumerate() {
            for b in &distinct[i + 1..] {
                assert_ne!(a, b, "the header test's values must stay pairwise distinct");
            }
        }
        // `elapsed_wall_ms` at +4 is the only word `payload_with` writes as zero
        // that is otherwise unconstrained; give it a distinct value and check
        // nothing else moves.
        let mut p2 = p.clone();
        p2[4..12].copy_from_slice(&0x0807_0605_0403_0201u64.to_le_bytes());
        let h2 = parse_header(&p2).expect("parses");
        assert_eq!(h2.elapsed_wall_ms, 0x0807_0605_0403_0201);
        assert_eq!(h2.status_flags, 0xc00);
        assert_eq!(h2.row_count, 3);
        assert_eq!(h2.schema_version, SCHEMA_VERSION);
        assert_eq!(h2.row_len as usize, ROW_LEN);
        assert_eq!(h2.string_count, STRING_COUNT);
        assert_eq!(h2.reserved, 0);

        // Perturbation sweep: change ONE byte of the header at a time and require
        // that exactly the field owning that byte moves (or that the parse is
        // refused, for the three words the grammar pins to a constant). This is
        // what makes the offset map transposition-proof for the CONSTRAINED words
        // too — a read of `string_count` at `schema_version`'s offset survives an
        // equal-values test but not this one.
        //
        // The expectation is computed INDEPENDENTLY of `parse_header`: the test
        // knows each word's byte span, applies the same XOR to the baseline value
        // itself, and derives both whether the perturbed header must be refused
        // and what all seven fields must read if it is not. Accepting "either an
        // Err or an Ok in which only the owner moved" would let a parser that
        // wrongly admits schema version 88, or wrongly rejects row_count 89, pass
        // — the refusal map has to be byte-exact.
        //
        // Word spans, from `decode_payload_header`
        // (`domain49_50_naval_combat_live.rs:1796`-`:1801`).
        const WORDS: [(usize, usize); 7] = [
            (0, 2),   // schema_version
            (2, 2),   // row_len
            (4, 8),   // elapsed_wall_ms
            (12, 4),  // status_flags
            (16, 4),  // row_count
            (20, 4),  // string_count
            (24, 4),  // reserved
        ];
        let baseline = [
            u64::from(SCHEMA_VERSION),
            ROW_LEN as u64,
            0u64,
            0xc00u64,
            3u64,
            u64::from(STRING_COUNT),
            0u64,
        ];
        for byte in 0..HEADER_LEN {
            let mut perturbed = p.clone();
            perturbed[byte] ^= 0x5a;
            // Which word owns this byte, and the same flip applied to the value.
            let (word, start, _len) = WORDS
                .iter()
                .enumerate()
                .find_map(|(i, &(start, len))| {
                    (byte >= start && byte < start + len).then_some((i, start, len))
                })
                .expect("every header byte belongs to a word");
            let mut expected = baseline;
            expected[word] ^= 0x5au64 << (8 * (byte - start));

            // `parse_header`'s own predicates, restated over the expected values.
            let must_refuse = expected[0] != u64::from(SCHEMA_VERSION)
                || expected[1] != ROW_LEN as u64
                || expected[6] != 0
                || expected[4] > u64::from(MAX_ROW_COUNT)
                || expected[5] != u64::from(STRING_COUNT);

            match parse_header(&perturbed) {
                Err(e) => assert!(
                    must_refuse,
                    "byte {byte} (word {word}) must parse, not be refused: {e}"
                ),
                Ok(h) => {
                    assert!(!must_refuse, "byte {byte} (word {word}) must be refused");
                    // Every field, against the independently computed expectation
                    // — not merely "the owner moved".
                    assert_eq!(u64::from(h.schema_version), expected[0], "byte {byte}");
                    assert_eq!(u64::from(h.row_len), expected[1], "byte {byte}");
                    assert_eq!(h.elapsed_wall_ms, expected[2], "byte {byte}");
                    assert_eq!(u64::from(h.status_flags), expected[3], "byte {byte}");
                    assert_eq!(u64::from(h.row_count), expected[4], "byte {byte}");
                    assert_eq!(u64::from(h.string_count), expected[5], "byte {byte}");
                    assert_eq!(u64::from(h.reserved), expected[6], "byte {byte}");
                }
            }
        }
        // The status word's three meanings, each read from the same offset.
        assert!(header.is_delta(), "bit 10 is DELTA_DATA");
        assert!(header.location_candidate_unvalidated(), "bit 11 is the badge");
        assert!(!header.is_failure(), "bit 11 is masked before the failure test");
        assert_eq!(header.status_for_global_interpretation(), STATUS_DELTA_DATA);
    }

    #[test]
    fn fixed_five_reproduces_every_raw_in_the_band() {
        // The Fixed(5)-vs-Exact defence, as an executable claim.
        let q = Quant::Fixed(SCALE_DIGITS);
        let mut raws: Vec<i64> = vec![0, 1, -1, 5, 99_999, 100_000, 100_001, 5_612_750];
        for decade in 0..16u32 {
            raws.push(10i64.pow(decade));
            raws.push(-10i64.pow(decade));
            raws.push(10i64.pow(decade) + 1);
        }
        for step in 0..2_000i64 {
            raws.push(EXACT_ANTI_AIR_RAW_BAND - step);
            raws.push(-(EXACT_ANTI_AIR_RAW_BAND - step));
        }
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        for _ in 0..20_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let magnitude = (state % (EXACT_ANTI_AIR_RAW_BAND as u64 + 1)) as i64;
            raws.push(if state & 1 == 0 { magnitude } else { -magnitude });
        }
        for raw in raws {
            if raw.unsigned_abs() > EXACT_ANTI_AIR_RAW_BAND as u64 {
                continue;
            }
            let v = raw as f64 / FIXED_SCALE;
            assert_eq!(
                quantize(v, q),
                Some(raw),
                "raw {raw} must survive fixed(5) unchanged"
            );
            assert_eq!(dequantize(raw, q), v, "raw {raw} dequantizes to the mapper's f64");
        }
        // The positioning pair is exact over its ENTIRE accepted range, which is
        // small enough to enumerate rather than sample.
        for raw in 0..=MAX_POSITIONING_RAW {
            let v = f64::from(raw) / FIXED_SCALE;
            assert_eq!(quantize(v, q), Some(raw as i64), "positioning raw {raw}");
        }
    }

    #[test]
    fn quantization_saves_bytes_against_an_all_exact_variant() {
        // Step 8.4. The comparison is against the same states encoded with
        // `Quant::Exact`, which is the alternative this domain actually had.
        let c = collection();
        let mut exact_schema = schema();
        for f in &mut exact_schema.collections[0].fields {
            if matches!(f.kind, Kind::Continuous) {
                f.quant = Quant::Exact;
            }
        }
        let exact_c = exact_schema.collections[0].clone();

        let mut fixed_state: KeyedState = BTreeMap::new();
        let mut exact_state: KeyedState = BTreeMap::new();
        // A table shaped like the capture's busiest hour, scaled up so the
        // comparison is not dominated by the key list.
        for i in 0..64u32 {
            let row = live_row(
                165_000 + i,
                1 + u64::from(i % 7),
                225 + i * 37,
                1 + i % 60,
                [i % 13, (i + 3) % 11],
                [(i as i32 * 1_361) % 100_001, (i as i32 * 907) % 100_001],
                [i as i64 * 88_123, -(i as i64) * 4_007],
            );
            let k = key_from_row(&row).expect("keys");
            fixed_state.insert(k.clone(), record_from_row(&c, &row, 0).expect("maps"));
            exact_state.insert(k, record_from_row(&exact_c, &row, 0).expect("maps"));
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
    fn every_i64_anti_air_survives_encode_and_decode_unchanged() {
        // The property that must hold over the WHOLE `i64` domain, because the
        // lane range-checks the anti-air cells not at all. Above
        // `EXACT_ANTI_AIR_RAW_BAND` the recovered raw integer may drift by one,
        // but the wire never disagrees with itself.
        let c = collection();
        let mut raws: Vec<i64> = vec![
            0,
            1,
            -1,
            5_612_750,
            EXACT_ANTI_AIR_RAW_BAND,
            EXACT_ANTI_AIR_RAW_BAND + 1,
            -(EXACT_ANTI_AIR_RAW_BAND + 1),
            (1 << 52) + 1,
            (1 << 53) + 1,
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
            let row = live_row(1, 1, 1, 1, [0, 0], [0, 0], [raw, raw.wrapping_neg()]);
            let record = record_from_row(&c, &row, 0)
                .unwrap_or_else(|e| panic!("raw {raw} maps: {e}"));
            let mut bytes = Vec::new();
            record
                .encode(&c, &mut bytes)
                .unwrap_or_else(|e| panic!("raw {raw} encodes: {e:?}"));
            let (back, pos) = Record::decode(&c, &bytes, 0)
                .unwrap_or_else(|e| panic!("raw {raw} decodes: {e:?}"));
            assert_eq!(pos, bytes.len(), "raw {raw}");
            assert_eq!(back, record, "raw {raw}");
            let mut reencoded = Vec::new();
            back.encode(&c, &mut reencoded).expect("re-encodes");
            assert_eq!(reencoded, bytes, "raw {raw}");
        }
    }

    // ---- Refusals ----------------------------------------------------------

    #[test]
    fn the_mapper_rejects_every_row_the_lane_rejects() {
        let c = collection();
        // Wrong length, both directions, including the empty slice.
        for len in [0usize, 1, ROW_LEN - 1, ROW_LEN + 1, 88] {
            let row = vec![0u8; len];
            assert!(record_from_row(&c, &row, 0).is_err(), "length {len}");
            assert!(key_from_row(&row).is_err(), "length {len}");
            assert!(row_flags(&row).is_err(), "length {len}");
            assert!(is_deleted_row(&row).is_err(), "length {len}");
            assert!(validate_row(&row).is_err(), "length {len}");
        }
        // An unknown flag bit (`:1872`) — every one of them, not one sample.
        for bit in 9..32u32 {
            let mut row = full_row();
            let flags = (KNOWN_ROW_FLAGS & !ROW_FLAG_DELETED) | (1 << bit);
            row[20..24].copy_from_slice(&flags.to_le_bytes());
            assert!(record_from_row(&c, &row, 0).is_err(), "flag bit {bit}");
            assert!(row_flags(&row).is_err(), "flag bit {bit}");
        }
        // combat_id zero and above the bound; the bound itself is accepted.
        for bad in [0u32, MAX_COMBAT_ID + 1, u32::MAX] {
            let row = live_row(bad, 1, 1, 1, [0, 0], [0, 0], [0, 0]);
            assert!(record_from_row(&c, &row, 0).is_err(), "combat_id {bad}");
            assert!(key_from_row(&row).is_err(), "combat_id {bad}");
        }
        assert!(record_from_row(&c, &live_row(MAX_COMBAT_ID, 1, 1, 1, [0, 0], [0, 0], [0, 0]), 0)
            .is_ok());
        // location_id / region_id: zero and over the bound reject, the bound itself
        // is accepted.
        for (loc, reg) in [(0u32, 1u32), (MAX_LOCATION_ID + 1, 1), (1, 0), (1, MAX_REGION_ID + 1)] {
            assert!(
                record_from_row(&c, &live_row(1, 1, loc, reg, [0, 0], [0, 0], [0, 0]), 0).is_err(),
                "location {loc} region {reg}"
            );
        }
        assert!(record_from_row(
            &c,
            &live_row(1, 1, MAX_LOCATION_ID, MAX_REGION_ID, [0, 0], [0, 0], [0, 0]),
            0
        )
        .is_ok());
        // Positioning outside the closed [0, 100000] interval, both directions.
        for pos in [-1i32, MAX_POSITIONING_RAW + 1, i32::MIN, i32::MAX] {
            assert!(
                record_from_row(&c, &live_row(1, 1, 1, 1, [0, 0], [pos, 0], [0, 0]), 0).is_err(),
                "attacker positioning {pos}"
            );
            assert!(
                record_from_row(&c, &live_row(1, 1, 1, 1, [0, 0], [0, pos], [0, 0]), 0).is_err(),
                "defender positioning {pos}"
            );
        }
        // A non-zero trailing reserved word (`:1960`), single bits included.
        for bit in 0..64u32 {
            let row = raw_row(
                1, 1, 0, KNOWN_ROW_FLAGS & !ROW_FLAG_DELETED, 1, 1, [0, 0], [0, 0], [0, 0],
                1u64 << bit,
            );
            assert!(record_from_row(&c, &row, 0).is_err(), "reserved bit {bit}");
        }
        // A gated-off word carrying a non-zero value: one field at a time, the
        // whole set.
        let cases: [(u32, usize, i64); 8] = [
            (ROW_FLAG_LOCATION_PRESENT, 24, 1),
            (ROW_FLAG_REGION_PRESENT, 28, 1),
            (ROW_FLAG_ATTACKER_COUNT_PRESENT, 32, 1),
            (ROW_FLAG_DEFENDER_COUNT_PRESENT, 36, 1),
            (ROW_FLAG_ATTACKER_POSITIONING_PRESENT, 40, 1),
            (ROW_FLAG_DEFENDER_POSITIONING_PRESENT, 44, 1),
            (ROW_FLAG_ATTACKER_ANTI_AIR_PRESENT, 48, 1),
            (ROW_FLAG_DEFENDER_ANTI_AIR_PRESENT, 56, 1),
        ];
        for (bit, offset, value) in cases {
            let mut row = raw_row(
                1, 1, 0, (KNOWN_ROW_FLAGS & !ROW_FLAG_DELETED) & !bit, 0, 0, [0, 0], [0, 0],
                [0, 0], 0,
            );
            let width = if offset >= 48 { 8 } else { 4 };
            row[offset..offset + width].copy_from_slice(&value.to_le_bytes()[..width]);
            assert!(
                validate_row(&row).is_err(),
                "gate {bit:#x} clear with a non-zero word at +{offset}"
            );
        }
    }

    #[test]
    fn the_mapper_maps_every_anti_air_the_lane_accepts() {
        // The lane range-checks nothing about the anti-air cells, so neither does
        // the mapper. This is the d36 round-1 finding's property, swept over the
        // magnitudes a cutoff would have caught.
        let c = collection();
        for raw in [
            EXACT_ANTI_AIR_RAW_BAND,
            -EXACT_ANTI_AIR_RAW_BAND,
            EXACT_ANTI_AIR_RAW_BAND + 1,
            -(EXACT_ANTI_AIR_RAW_BAND + 1),
            1 << 52,
            1 << 53,
            100_000_000_000_000_000,
            i64::MAX,
            i64::MIN,
        ] {
            assert!(
                record_from_row(&c, &live_row(1, 1, 1, 1, [0, 0], [0, 0], [raw, raw]), 0).is_ok(),
                "anti_air raw {raw} is accepted by the lane and must be mapped"
            );
        }
        // The member counts have no upper bound in the lane either.
        for count in [0u32, 1, 1_000_000, u32::MAX] {
            assert!(
                record_from_row(&c, &live_row(1, 1, 1, 1, [count, count], [0, 0], [0, 0]), 0)
                    .is_ok(),
                "member count {count}"
            );
        }
        // Nor does `generation`, nor `body_ptr`.
        for generation in [0u64, 1, u64::MAX] {
            assert!(record_from_row(
                &c,
                &live_row(1, generation, 1, 1, [0, 0], [0, 0], [0, 0]),
                0
            )
            .is_ok());
        }
        for body_ptr in [0u64, 1, u64::MAX] {
            let row = raw_row(
                1, 1, body_ptr, KNOWN_ROW_FLAGS & !ROW_FLAG_DELETED, 1, 1, [0, 0], [0, 0],
                [0, 0], 0,
            );
            assert!(
                record_from_row(&c, &row, 0).is_ok(),
                "body_ptr {body_ptr} is a witness the lane discards; nothing may reject it"
            );
        }
    }

    #[test]
    fn a_deleted_row_is_a_removal_not_a_record() {
        let c = collection();
        // The capture's own tombstone shape: every presence bit still set, values
        // carried, delete bit on. Mapping it would publish a battle that ended.
        let mut tombstone = full_row();
        tombstone[20..24].copy_from_slice(&KNOWN_ROW_FLAGS.to_le_bytes());
        assert!(is_deleted_row(&tombstone).expect("flags"));
        assert!(validate_row(&tombstone).is_ok(), "the lane accepts this row");
        assert!(record_from_row(&c, &tombstone, 0).is_err());
        // And the bare tombstone, with everything gated off.
        let bare = deleted_row(3, 1);
        assert!(validate_row(&bare).is_ok());
        assert!(is_deleted_row(&bare).expect("flags"));
        assert!(record_from_row(&c, &bare, 0).is_err());
    }

    #[test]
    fn a_live_row_missing_a_whole_hour_gate_is_refused_not_invented() {
        // Each of the six gates the two payload-level refusals protect, cleared one
        // at a time. The lane itself accepts every one of these rows — it is the
        // FOLD that refuses the payload carrying it — so the mapper's refusal must
        // be about the gate, not about row validity.
        let c = collection();
        for bit in [
            ROW_FLAG_LOCATION_PRESENT,
            ROW_FLAG_REGION_PRESENT,
            ROW_FLAG_ATTACKER_POSITIONING_PRESENT,
            ROW_FLAG_DEFENDER_POSITIONING_PRESENT,
            ROW_FLAG_ATTACKER_ANTI_AIR_PRESENT,
            ROW_FLAG_DEFENDER_ANTI_AIR_PRESENT,
        ] {
            let flags = (KNOWN_ROW_FLAGS & !ROW_FLAG_DELETED) & !bit;
            let row = raw_row(
                1,
                1,
                0,
                flags,
                if flags & ROW_FLAG_LOCATION_PRESENT != 0 { 2_757 } else { 0 },
                if flags & ROW_FLAG_REGION_PRESENT != 0 { 41 } else { 0 },
                [1, 2],
                [
                    if flags & ROW_FLAG_ATTACKER_POSITIONING_PRESENT != 0 { 5 } else { 0 },
                    if flags & ROW_FLAG_DEFENDER_POSITIONING_PRESENT != 0 { 6 } else { 0 },
                ],
                [
                    if flags & ROW_FLAG_ATTACKER_ANTI_AIR_PRESENT != 0 { 7 } else { 0 },
                    if flags & ROW_FLAG_DEFENDER_ANTI_AIR_PRESENT != 0 { 8 } else { 0 },
                ],
                0,
            );
            assert!(
                validate_row(&row).is_ok(),
                "the lane accepts a row with gate {bit:#x} clear"
            );
            assert!(
                record_from_row(&c, &row, 0).is_err(),
                "gate {bit:#x} clear must be refused, never given an invented empty state"
            );
        }
    }

    #[test]
    fn the_mapper_refuses_a_collection_it_cannot_fill() {
        // A caller handing over another domain's collection must not get a record
        // full of placeholders under this domain's field ids.
        let mut c = collection();
        c.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_row(&c, &full_row(), 0).is_err());
    }

    // ---- Header and payload split -----------------------------------------

    #[test]
    fn split_payload_enforces_the_lanes_structural_rules() {
        let rows = vec![full_row(), deleted_row(2, 1)];
        let p = payload(0, &rows);
        let (header, body) = split_payload(&p).expect("well-formed payload");
        assert_eq!(header.row_count, 2);
        assert_eq!(body.len(), 2 * ROW_LEN);
        assert_eq!(&body[..ROW_LEN], &rows[0][..]);
        assert_eq!(&body[ROW_LEN..], &rows[1][..]);

        // Short of the header.
        for len in 0..HEADER_LEN {
            assert!(parse_header(&p[..len]).is_err(), "len {len}");
            assert!(split_payload(&p[..len]).is_err(), "len {len}");
        }
        // A wrong schema version (`:1802`).
        for version in [0u16, 1, 3, u16::MAX] {
            let bad = payload_with(version, ROW_LEN as u16, 0, 2, STRING_COUNT, 0, &rows, 0);
            assert!(parse_header(&bad).is_err(), "version {version}");
        }
        // A wrong row_len word (`:1807`) — the header claims a row form that does
        // not exist, which would mis-slice every row behind it.
        for row_len in [0u16, 24, 71, 73, 88, u16::MAX] {
            let bad = payload_with(SCHEMA_VERSION, row_len, 0, 2, STRING_COUNT, 0, &rows, 0);
            assert!(parse_header(&bad).is_err(), "row_len {row_len}");
        }
        // A non-zero reserved word (`:1812`), single bits included.
        for bit in 0..32u32 {
            let bad =
                payload_with(SCHEMA_VERSION, ROW_LEN as u16, 0, 2, STRING_COUNT, 1 << bit, &rows, 0);
            assert!(parse_header(&bad).is_err(), "reserved bit {bit}");
        }
        // A string_count other than one (`:1820`).
        for count in [0u32, 2, u32::MAX] {
            let bad = payload_with(SCHEMA_VERSION, ROW_LEN as u16, 0, 2, count, 0, &rows, 0);
            assert!(parse_header(&bad).is_err(), "string_count {count}");
        }
        // A row_count over the lane's limit (`:1817`), and the limit itself
        // accepted by `parse_header` (the length check lives in `split_payload`).
        let over = payload_with(
            SCHEMA_VERSION, ROW_LEN as u16, 0, MAX_ROW_COUNT + 1, STRING_COUNT, 0, &rows, 0,
        );
        assert!(parse_header(&over).is_err());
        let at = payload_with(
            SCHEMA_VERSION, ROW_LEN as u16, 0, MAX_ROW_COUNT, STRING_COUNT, 0, &rows, 0,
        );
        assert!(parse_header(&at).is_ok());
        assert!(split_payload(&at).is_err(), "the length still has to match");
        // Length that does not match the row count, in both directions.
        let mut short = p.clone();
        short.pop();
        assert!(split_payload(&short).is_err());
        let mut long = p.clone();
        long.push(0);
        assert!(split_payload(&long).is_err(), "trailing bytes (`:1857`)");
        // A non-empty string table entry (`:1850`).
        let non_empty = payload_with(
            SCHEMA_VERSION, ROW_LEN as u16, 0, 2, STRING_COUNT, 0, &rows, 1,
        );
        assert!(split_payload(&non_empty).is_err());
        assert!(split_payload(&[]).is_err());
    }

    #[test]
    fn the_row_count_bound_makes_the_length_arithmetic_unoverflowable() {
        // `split_payload`'s `checked_mul`/`checked_add` are defence in depth, NOT a
        // live path: `parse_header` runs first and refuses any `row_count` above
        // `MAX_ROW_COUNT` (`domain49_50_naval_combat_live.rs:1817`), so the largest
        // product `split_payload` can ever compute is this one. Asserting the
        // reachable bound is honest where asserting "overflow is rejected" would
        // be vacuous — no payload can reach the checked operators' failure arm.
        //
        // The bound is stated against `usize`, so it holds on a 32-bit target too:
        // 100_000 * 72 + 28 + 2 = 7_200_030, four orders of magnitude below
        // `u32::MAX`.
        let widest = (MAX_ROW_COUNT as usize)
            .checked_mul(ROW_LEN)
            .and_then(|len| len.checked_add(HEADER_LEN))
            .and_then(|len| len.checked_add(STRING_TABLE_LEN))
            .expect("the row-count bound keeps the payload length inside usize");
        assert_eq!(widest, 7_200_030);
        assert!(
            widest < u32::MAX as usize,
            "the widest legal payload must fit a 32-bit usize"
        );
        // And the bound itself is enforced, so nothing larger reaches the
        // arithmetic: one above the limit is refused by `parse_header`, and the
        // extreme word is refused there too rather than wrapping.
        for row_count in [MAX_ROW_COUNT + 1, u32::MAX / 2, u32::MAX] {
            let p = payload_with(
                SCHEMA_VERSION,
                ROW_LEN as u16,
                0,
                row_count,
                STRING_COUNT,
                0,
                &[],
                0,
            );
            assert!(parse_header(&p).is_err(), "row_count {row_count}");
            assert!(split_payload(&p).is_err(), "row_count {row_count}");
        }
    }

    #[test]
    fn the_key_is_the_combat_and_its_generation() {
        assert_eq!(key_from_row(&full_row()).expect("keys"), key(&[165_521, 7]));
        assert_eq!(
            key_fields_of_row(&full_row()).expect("keys"),
            (165_521, 7)
        );
        assert!(key_from_row(&[]).is_err());
        // Two rows differing only in `generation` are two different battles: the
        // key components must not be transposed or collapsed.
        assert_ne!(
            key_from_row(&live_row(5, 1, 1, 1, [0, 0], [0, 0], [0, 0])).expect("keys"),
            key_from_row(&live_row(5, 2, 1, 1, [0, 0], [0, 0], [0, 0])).expect("keys")
        );
        assert_ne!(
            key_from_row(&live_row(1, 5, 1, 1, [0, 0], [0, 0], [0, 0])).expect("keys"),
            key_from_row(&live_row(5, 1, 1, 1, [0, 0], [0, 0], [0, 0])).expect("keys")
        );
    }

    // ---- The frame fold ----------------------------------------------------

    fn fold(state: &mut State, payload: &[u8]) -> core::result::Result<FoldOutcome, String> {
        state.fold_payload(&schema(), payload, 0)
    }

    #[test]
    fn every_record_is_a_patch_against_a_retained_accumulator() {
        // d49 is a pure cumulative delta: there is no baseline record, nothing ever
        // clears the accumulator, and the second record's ONE row does not replace
        // the first record's two. A harness that read a record as a snapshot would
        // report one row here.
        let mut s = empty_state();
        let first = payload(
            0,
            &[
                live_row(1, 1, 10, 2, [3, 4], [100, 200], [300, 400]),
                live_row(2, 1, 11, 3, [5, 6], [500, 600], [700, 800]),
            ],
        );
        assert_eq!(
            fold(&mut s, &first).expect("folds"),
            FoldOutcome::Folded { upserts: 2, deletes: 0 }
        );
        assert_eq!(s.combats.len(), 2);
        let second = payload(
            STATUS_DELTA_DATA,
            &[live_row(1, 1, 10, 2, [9, 9], [100, 200], [300, 400])],
        );
        assert_eq!(
            fold(&mut s, &second).expect("folds"),
            FoldOutcome::Folded { upserts: 1, deletes: 0 }
        );
        assert_eq!(s.combats.len(), 2, "the accumulator is retained, not replaced");
        let c = collection();
        assert_eq!(
            s.combats[&key(&[1, 1])].values[index_of(&c, field::ATTACKER_MEMBER_COUNT)],
            Value::Unsigned(9),
            "the patch updated the row it carried"
        );
        // An empty record changes nothing and is still a measured hour: 417 of the
        // capture's 514 hours are exactly this shape.
        assert_eq!(
            fold(&mut s, &payload(STATUS_DELTA_DATA, &[])).expect("folds"),
            FoldOutcome::Folded { upserts: 0, deletes: 0 }
        );
        assert_eq!(s.combats.len(), 2);
    }

    #[test]
    fn a_deleted_row_removes_its_key() {
        let mut s = empty_state();
        fold(&mut s, &payload(0, &[live_row(1, 1, 10, 2, [3, 4], [100, 200], [300, 400])]))
            .expect("folds");
        assert_eq!(s.combats.len(), 1);
        assert_eq!(
            fold(&mut s, &payload(0, &[deleted_row(1, 1)])).expect("folds"),
            FoldOutcome::Folded { upserts: 0, deletes: 1 }
        );
        assert!(s.combats.is_empty());
        // A tombstone for a key that was never inserted is an idempotent no-op —
        // the fold's `remove` on an absent key. 37 of the capture's tombstones are
        // exactly that, because every insert landed in an excluded hour.
        assert_eq!(
            fold(&mut s, &payload(0, &[deleted_row(99, 4)])).expect("folds"),
            FoldOutcome::Folded { upserts: 0, deletes: 1 }
        );
        assert!(s.combats.is_empty());
    }

    #[test]
    fn duplicate_keys_in_one_payload_apply_in_order() {
        // Unlike d50, this fold places no uniqueness check on the rows, so the
        // permissive last-writer-wins collapse is mirrored rather than refused.
        let c = collection();
        let mut s = empty_state();
        fold(
            &mut s,
            &payload(
                0,
                &[
                    live_row(1, 1, 10, 2, [1, 1], [100, 200], [300, 400]),
                    live_row(1, 1, 10, 2, [2, 2], [100, 200], [300, 400]),
                ],
            ),
        )
        .expect("folds");
        assert_eq!(s.combats.len(), 1);
        assert_eq!(
            s.combats[&key(&[1, 1])].values[index_of(&c, field::ATTACKER_MEMBER_COUNT)],
            Value::Unsigned(2),
            "the LAST row wins"
        );
        // Live then deleted in one payload removes the key; deleted then live
        // leaves it present.
        fold(
            &mut s,
            &payload(0, &[live_row(2, 1, 10, 2, [1, 1], [0, 0], [0, 0]), deleted_row(2, 1)]),
        )
        .expect("folds");
        assert!(!s.combats.contains_key(&key(&[2, 1])));
        fold(
            &mut s,
            &payload(0, &[deleted_row(3, 1), live_row(3, 1, 10, 2, [1, 1], [0, 0], [0, 0])]),
        )
        .expect("folds");
        assert!(s.combats.contains_key(&key(&[3, 1])));
    }

    #[test]
    fn the_location_refusal_runs_before_the_failure_gate() {
        // The position that must not be guessed (`world_fold.rs:20047` vs `:20066`).
        // A payload carrying BOTH the location badge and a failure status is
        // excluded for the LOCATION reason, because the fold returns at refusal 1
        // and never reaches `handle_compact_failure`.
        let mut s = empty_state();
        let both = payload(
            STATUS_LOCATION_CANDIDATE_UNVALIDATED | (1 << 9), // + PARTIAL_DATA
            &[live_row(1, 1, 10, 2, [1, 1], [0, 0], [0, 0])],
        );
        assert_eq!(
            fold(&mut s, &both).expect("folds nothing"),
            FoldOutcome::LocationRegionIncomplete {
                status_badge: true,
                row_incomplete: false
            }
        );
        assert!(s.combats.is_empty(), "the accumulator is retained, untouched");
        // The badge alone, on rows that are individually complete: still refused,
        // which is exactly the 72-hour shape of the `batch3/deepwar_a` capture.
        assert_eq!(
            fold(
                &mut s,
                &payload(
                    STATUS_LOCATION_CANDIDATE_UNVALIDATED | STATUS_DELTA_DATA,
                    &[live_row(1, 1, 10, 2, [1, 1], [0, 0], [0, 0])]
                )
            )
            .expect("folds nothing"),
            FoldOutcome::LocationRegionIncomplete {
                status_badge: true,
                row_incomplete: false
            }
        );
        // A live row missing region, with a clean status word: the other half of
        // refusal 1, and the reason the capture's 72 hours fire twice over.
        let missing_region = {
            let flags = (KNOWN_ROW_FLAGS & !ROW_FLAG_DELETED) & !ROW_FLAG_REGION_PRESENT;
            raw_row(1, 1, 0, flags, 10, 0, [1, 1], [0, 0], [0, 0], 0)
        };
        assert_eq!(
            fold(&mut s, &payload(STATUS_DELTA_DATA, &[missing_region])).expect("folds nothing"),
            FoldOutcome::LocationRegionIncomplete {
                status_badge: false,
                row_incomplete: true
            }
        );
        // A DELETED row missing everything does NOT fire refusal 1: the fold's scan
        // is `!row.deleted`.
        assert_eq!(
            fold(&mut s, &payload(STATUS_DELTA_DATA, &[deleted_row(4, 1)])).expect("folds"),
            FoldOutcome::Folded { upserts: 0, deletes: 1 }
        );
    }

    #[test]
    fn the_side_scalar_refusal_runs_after_the_failure_gate() {
        // The mirror of the previous test (`:20076` vs `:20066`). A payload whose
        // live row lacks a side scalar AND carries a failure status is excluded for
        // the FAILURE reason: refusal 3 is never reached.
        let mut s = empty_state();
        let missing_aa = {
            let flags =
                (KNOWN_ROW_FLAGS & !ROW_FLAG_DELETED) & !ROW_FLAG_ATTACKER_ANTI_AIR_PRESENT;
            raw_row(1, 1, 0, flags, 10, 2, [1, 1], [5, 6], [0, 8], 0)
        };
        assert_eq!(
            fold(&mut s, &payload(1 << 9, &[missing_aa.clone()])).expect("folds nothing"),
            FoldOutcome::FailureRetained
        );
        // With a clean status word the same payload reaches refusal 3.
        assert_eq!(
            fold(&mut s, &payload(STATUS_DELTA_DATA, &[missing_aa])).expect("folds nothing"),
            FoldOutcome::SideScalarsIncomplete
        );
        assert!(s.combats.is_empty());
        // Each of the four side scalars fires it on its own.
        for bit in [
            ROW_FLAG_ATTACKER_POSITIONING_PRESENT,
            ROW_FLAG_DEFENDER_POSITIONING_PRESENT,
            ROW_FLAG_ATTACKER_ANTI_AIR_PRESENT,
            ROW_FLAG_DEFENDER_ANTI_AIR_PRESENT,
        ] {
            let flags = (KNOWN_ROW_FLAGS & !ROW_FLAG_DELETED) & !bit;
            let row = raw_row(
                1,
                1,
                0,
                flags,
                10,
                2,
                [1, 1],
                [
                    if flags & ROW_FLAG_ATTACKER_POSITIONING_PRESENT != 0 { 5 } else { 0 },
                    if flags & ROW_FLAG_DEFENDER_POSITIONING_PRESENT != 0 { 6 } else { 0 },
                ],
                [
                    if flags & ROW_FLAG_ATTACKER_ANTI_AIR_PRESENT != 0 { 7 } else { 0 },
                    if flags & ROW_FLAG_DEFENDER_ANTI_AIR_PRESENT != 0 { 8 } else { 0 },
                ],
                0,
            );
            assert_eq!(
                fold(&mut s, &payload(0, &[row])).expect("folds nothing"),
                FoldOutcome::SideScalarsIncomplete,
                "gate {bit:#x}"
            );
        }
        // A DELETED row lacking every side scalar folds normally.
        assert_eq!(
            fold(&mut s, &payload(0, &[deleted_row(9, 1)])).expect("folds"),
            FoldOutcome::Folded { upserts: 0, deletes: 1 }
        );
    }

    #[test]
    fn the_lane_local_bit_is_never_read_as_a_global_failure() {
        // Bit 11 is `COMPACT_STATUS_TRANSITION_BASELINE_RESET` globally and the
        // location badge for d49. `handle_compact_failure` strips it first
        // (`compact_contract/src/lib.rs:105`), so it must never contribute a
        // failure verdict — and since it is not in the failure mask either, the
        // check that matters is that `is_failure` ignores it entirely.
        let h = parse_header(&payload(STATUS_LOCATION_CANDIDATE_UNVALIDATED, &[])).expect("parses");
        assert!(!h.is_failure());
        assert!(h.location_candidate_unvalidated());
        assert_eq!(h.status_for_global_interpretation(), 0);
        // Every global failure bit is still a failure with the badge alongside it.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let h = parse_header(&payload(
                STATUS_LOCATION_CANDIDATE_UNVALIDATED | (1 << bit),
                &[],
            ))
            .expect("parses");
            assert!(h.is_failure(), "failure bit {bit}");
        }
        // And the informational bits are not failures.
        for bit in [4u32, 5, 10, 13] {
            let h = parse_header(&payload(1 << bit, &[])).expect("parses");
            assert!(!h.is_failure(), "informational bit {bit}");
        }
    }

    #[test]
    fn a_failure_status_record_folds_nothing_and_retains_the_accumulator() {
        let mut s = empty_state();
        fold(&mut s, &payload(0, &[live_row(1, 1, 10, 2, [3, 4], [1, 2], [3, 4])]))
            .expect("folds");
        assert_eq!(s.combats.len(), 1);
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            assert_eq!(
                fold(
                    &mut s,
                    &payload(1 << bit, &[live_row(2, 1, 10, 2, [3, 4], [1, 2], [3, 4])])
                )
                .expect("folds nothing"),
                FoldOutcome::FailureRetained,
                "failure bit {bit}"
            );
            assert_eq!(s.combats.len(), 1, "failure bit {bit}: accumulator retained");
        }
    }

    #[test]
    fn the_fold_never_folds_a_row_the_mapper_rejects() {
        // The implication the six no-capability declarations rest on: if
        // `fold_payload` reaches its row loop, every live row it carries satisfies
        // `REQUIRED_ON_A_FOLDED_LIVE_ROW`, so `record_from_row`'s gate refusal is
        // unreachable from the fold path. Swept over every non-empty subset of the
        // six gates, on a payload with a clean status word.
        let schema = schema();
        let gates = [
            ROW_FLAG_LOCATION_PRESENT,
            ROW_FLAG_REGION_PRESENT,
            ROW_FLAG_ATTACKER_POSITIONING_PRESENT,
            ROW_FLAG_DEFENDER_POSITIONING_PRESENT,
            ROW_FLAG_ATTACKER_ANTI_AIR_PRESENT,
            ROW_FLAG_DEFENDER_ANTI_AIR_PRESENT,
        ];
        for subset in 1u32..64 {
            let mut cleared = 0u32;
            for (i, gate) in gates.iter().enumerate() {
                if subset & (1 << i) != 0 {
                    cleared |= gate;
                }
            }
            let flags = (KNOWN_ROW_FLAGS & !ROW_FLAG_DELETED) & !cleared;
            let row = raw_row(
                1,
                1,
                0,
                flags,
                if flags & ROW_FLAG_LOCATION_PRESENT != 0 { 10 } else { 0 },
                if flags & ROW_FLAG_REGION_PRESENT != 0 { 2 } else { 0 },
                [1, 1],
                [
                    if flags & ROW_FLAG_ATTACKER_POSITIONING_PRESENT != 0 { 5 } else { 0 },
                    if flags & ROW_FLAG_DEFENDER_POSITIONING_PRESENT != 0 { 6 } else { 0 },
                ],
                [
                    if flags & ROW_FLAG_ATTACKER_ANTI_AIR_PRESENT != 0 { 7 } else { 0 },
                    if flags & ROW_FLAG_DEFENDER_ANTI_AIR_PRESENT != 0 { 8 } else { 0 },
                ],
                0,
            );
            let mut s = empty_state();
            let outcome = s
                .fold_payload(&schema, &payload(0, &[row]), 0)
                .unwrap_or_else(|e| panic!("subset {subset:#x} is a valid payload: {e}"));
            assert!(
                !outcome.is_measured(),
                "subset {subset:#x} must be an excluded hour, not a folded one"
            );
            assert!(s.combats.is_empty(), "subset {subset:#x}");
        }
    }

    #[test]
    fn a_rejected_payload_leaves_the_accumulator_untouched() {
        // The all-or-nothing error contract: the lane validates every row before
        // the fold applies any, so a bad row LAST in the payload still leaves the
        // first rows unapplied.
        let mut s = empty_state();
        fold(&mut s, &payload(0, &[live_row(1, 1, 10, 2, [3, 4], [1, 2], [3, 4])]))
            .expect("folds");
        let before = s.combats.clone();
        let bad = payload(
            0,
            &[
                live_row(2, 1, 10, 2, [3, 4], [1, 2], [3, 4]),
                live_row(0, 1, 10, 2, [3, 4], [1, 2], [3, 4]), // combat_id 0
            ],
        );
        assert!(fold(&mut s, &bad).is_err());
        assert_eq!(s.combats, before, "nothing from the refused payload landed");
    }

    #[test]
    fn fold_payload_never_panics_on_arbitrary_bytes() {
        // Every public entry point is total over arbitrary input: the harness feeds
        // it bytes from a file on disk.
        let schema = schema();
        let mut state = 0xC0FF_EE12_3456_789Au64;
        for _ in 0..20_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let len = (state as usize) % 200;
            let mut bytes = vec![0u8; len];
            for b in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *b = state as u8;
            }
            let _ = parse_header(&bytes);
            let _ = split_payload(&bytes);
            let _ = key_from_row(&bytes);
            let _ = row_flags(&bytes);
            let _ = is_deleted_row(&bytes);
            let _ = validate_row(&bytes);
            let _ = record_from_row(&schema.collections[0], &bytes, 0);
            let _ = empty_state().fold_payload(&schema, &bytes, 0);
        }
    }
}
