//! d13 `tech_live` — per-country research census and per-slot active research.
//!
//! Derived from `world_fold.rs:16622`-`:16931`
//! (`fold_tech`), its scale constants at `:154`-`:155`, the collector packer
// @world-fold-authority domain=13 functions=fold_tech lines=16622-16931 sha256=48187c31e972b03899c0cdeee26bc7532afa2eb75236b9480ccfcfe10ffcee62 closure=75e96340b5b7386ab9b66b242c5df262cfee012fc833b459efa354218d1e3d1d
//! (`extractor/native/collector/src/attach_log.rs:69183`-`:69328`) and the
//! collector read that produces every presence flag (`:80860`-`:81047`,
//! `:81311`-`:81356`). `extractor/documents/domains/d13_tech_live.md` carries the
//! A29 producer knowledge and is quoted where it decides something.
//!
//! `fold_tech` is the only fold that takes `game_hour_raw` (`:13321`), and its
//! sole consumer is the A29 same-hour authority gate at `:13525`. That hour is
//! the engine's raw date word and **not** the container's tick coordinate
//! (`compact_contract/src/calendar.rs:9`: `hour_index = raw_date - 43_799_999`),
//! so it is a second clock input beside the recipe's `frame_tick` rather than the
//! same one; see [`record_from_active_row`].
//!
//! This domain **accumulates**: one DATA record is not one state. See
//! [`TechState`].
//!
//! # Payload grammar
//!
//! A 32-byte header (`world_fold.rs::fold_tech`, packer `attach_log.rs:69205`-`:69211`):
//!
//! | offset | width | header field | read by the fold? |
//! |---|---|---|---|
//! | `+0`  | u64 | `elapsed_wall_ms`    | no |
//! | `+8`  | u32 | `country_count`      | yes (`:13323`) |
//! | `+12` | u32 | `changed_rows`       | no (packer writes `country_count`, `:69207`) |
//! | `+16` | u32 | `status_flags`       | no — see "status" below |
//! | `+20` | u32 | `progress_scale_raw` | no (packer writes `1000`, `:69209`) |
//! | `+24` | u32 | `live_scale_raw`     | no (packer writes `100000`, `:69210`) |
//! | `+28` | u32 | `active_count`       | yes (`:13324`) |
//!
//! then `country_count` fixed 16-byte country rows, then `active_count`
//! **variable-length** active-research rows. The fold's structural bound is
//! `32 + country_count*16 + active_count*40` as a *minimum* (`:13325`-`:13330`)
//! and exact consumption at the end (`:13591`) — the row tails make the total
//! unpredictable from the header alone.
//!
//! ## Country row, 16 bytes (`world_fold.rs::fold_tech`, packer `:69224`-`:69237`)
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+0`  | u32 | `tag_db_id`            | raw | none (key; **zero is legal**, the fold checks nothing) |
//! | `+4`  | u32 | `flags`                | —   | **the fold never reads this word** |
//! | `+8`  | u32 | `completed_tech_count` | raw | `1<<0` (collector), sentinel `u32::MAX` |
//! | `+12` | u32 | `research_slot_count`  | raw | `1<<1` (collector), sentinel `u32::MAX` |
//!
//! **The four unread bytes at `+4` are the country row's whole four-state
//! answer, and dropping them is fidelity loss.** `fold_tech` reads `+8` and `+12`
//! unconditionally and publishes whatever is there, so a country whose count the
//! collector could not read folds as the literal number `4294967295`
//! (`attach_log.rs:69229`, `:69235`: `unwrap_or(u32::MAX)`). The recipe sends the
//! reader to the collector when the fold is uninformative, and the collector is
//! explicit: `completed_tech_count` is `None` only when the memory is unreadable,
//! the researched-list capacity contradicts its size, or the value trips
//! `MAX_COMPLETED_TECH_COUNT` (`attach_log.rs:80881`-`:80910`), and
//! `research_slot_count` is `None` only on an unreadable word or a value above
//! `MAX_RESEARCH_SLOT_COUNT` (`:80911`-`:80912`). That is `WIRE_FORMAT.md` §3's
//! **unknown** — "applicable, not captured" — word for word, never null and never
//! absent. Those two bounds (2000 and 32, `attach_log/offsets.rs:274`-`:275`)
//! also make `u32::MAX` unreachable as a real reading, so the sentinel is
//! unambiguous.
//!
//! Bit `1<<2` says the country's **active-research list itself** was captured
//! (`attach_log.rs:69221`). Without it, a country with no rows in the
//! `active_research` collection would silently assert "researching nothing" about
//! a list that was never read — the same fidelity loss d23 keeps
//! `resources_captured` for. It is carried as a `Boolean` and is a real measured
//! value in every row: the flag word is always readable, so it is never one of
//! the three empty states.
//!
//! ## Active-research row (`world_fold.rs::fold_tech`, packer `:69272`-`:69281`)
//!
//! 40 fixed bytes, then up to three optional tails in this order:
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+0`  | u32 | `tag_db_id`                | raw       | none (key part 0) |
//! | `+4`  | u32 | `slot_index`               | raw       | none (key part 1) |
//! | `+8`  | u32 | `tech_handle_raw`          | raw       | none |
//! | `+12` | u32 | `tech_def_index_raw`       | raw       | none |
//! | `+16` | u32 | `progress_raw`             | **/1000** (rendered) | none |
//! | `+20` | u32 | `flags`                    | —         | `& !0b1_1111` rejected (`world_fold.rs::fold_tech`) |
//! | `+24` | i64 | `live_progress_raw`        | **/100000** (rendered) | `1<<0` |
//! | `+32` | i64 | `base_required_points_raw` | **/100000** (rendered) | `1<<1` |
//! | tail  | u16-prefixed UTF-8 | `tech_name` | —        | `1<<2` (`world_fold.rs::fold_tech`) |
//! | tail  | 4 × i64 | ahead-of-time quartet   | **/100000** (rendered) | `1<<3` (`world_fold.rs::fold_tech`) |
//! | tail  | 176 B | A29 producer provenance   | —         | `1<<4` (`world_fold.rs::fold_tech`) |
//!
//! **Two scales in one row, and this port carries neither.** `progress_raw`
//! divides by `TECH_PROGRESS_SCALE = 1000` (`:154`, applied `world_fold.rs::fold_tech`) while
//! `live_progress_raw`, `base_required_points_raw` and all four quartet fields
//! divide by `TECH_LIVE_SCALE = 100_000` (`:155`, applied `:13372`, `:13384`,
//! `:13433`-`:13445`) — a 100× error class inside one row. `fold_tech` is the
//! only fold in the file that emits **both** forms of every one of these fields:
//! `progress_raw` *and* `progress_units`, `live_progress_raw` *and*
//! `live_progress_units`, and so on, plus the literal
//! `"ahead_of_time_fixed_scale": 100_000` (`:13420`). Compare `fold_map_detail`
//! (`:30033`) and `fold_country_core_scalars` (`world_fold.rs::fold_country_core_scalars`), which emit the divided
//! value only. So the fold's own answer for d13 is that the raw integer is
//! first-class state and `*_units` is its rendering at a scale the fold also
//! publishes — which is the recipe's derived-value rule, and it is why this
//! schema carries the **raw integers** and leaves the `_units` companions off the
//! wire. Two consequences, both in the deliverable's open questions:
//!
//! * It is exactly lossless over the fold's whole legal domain. The six
//!   `/100000` fields are `i64`, and `fold_tech` accepts every `i64`; any
//!   `Continuous` port would round `raw as f64 / 100000.0` and could not recover
//!   `live_progress_raw` past `2^53`, which the fold hands its consumers exactly.
//! * The scale constants still live here ([`TECH_PROGRESS_SCALE`],
//!   [`TECH_LIVE_SCALE`]) and the step-8 scale test reproduces the fold's own
//!   `_units` expressions from the stored values, so attaching the wrong divisor
//!   downstream is caught by a test rather than by a 100×-wrong replay.
//!
//! ## Status
//!
//! `fold_tech` **never reads `status_flags`** — it has no `handle_compact_failure`
//! call, unlike `fold_country_core_scalars` (`world_fold.rs::fold_country_core_scalars`). Like d10, that forces an
//! explicit exclusion decision on the measurement side rather than a silent one;
//! see `bin/bench_real_d13.rs`, which refuses a row-bearing failure-status record
//! outright and counts-and-skips an empty one. Nothing about it belongs in this
//! module, which never sees the status word except to expose it in
//! [`PayloadHeader`].
//!
//! # Field-level stop-reports
//!
//! Two things this domain captures have **no representable target** under
//! `WIRE_FORMAT.md` §4, whose seven kinds are `Unsigned`, `Signed`, `Monotone`,
//! `Enum`, `Boolean`, `ContentId`, `Continuous` — no string, no bytes. Both are
//! reported rather than dropped quietly, and neither occurs in the measured
//! capture (0 of 364,487 active rows carry either tail).
//!
//! 1. **`tech_name`** (`world_fold.rs::fold_tech`). The string bytes have no
//!    kind. What survives is everything except the bytes: the numeric identity is
//!    carried by `tech_handle_raw`/`tech_def_index_raw`, and the *capture fact* by
//!    the `tech_name_captured` `Boolean`. The name is not independent state — the
//!    collector resolves it as `tech_def_filter[handle]` (`attach_log.rs:81024`),
//!    a content-table rendering of the handle, exactly the argument d23 makes for
//!    its `type_id` names. A §13 manifest that resolves `tech_handle_raw` restores
//!    it; the Boolean records the one thing the handle does not, namely whether
//!    that table resolved this handle at capture time.
//!
//! 2. **The 176-byte A29 provenance block** (`:13452`-`:13544`), except its
//!    `localization_branch`. Three independent reasons, each sufficient:
//!    * Three of its fields are `[u8; 32]` SHA-256 digests (`:13460`, `:13470`,
//!      `:13473`) and §4 has no bytes kind.
//!    * It is an indivisible receipt. Every one of its remaining integers is an
//!      argument to `a29_expected_source_record_sha256`
//!      (`recorder_host/src/domain_data_authority.rs:72`-`:124`) or to
//!      `SelectedPresentReceipt::sealed` (`world_fold.rs::fold_tech`); they
//!      exist to be hashed. Publishing them without the digests publishes a
//!      receipt no consumer can check, which is worse than publishing nothing.
//!    * This crate structurally cannot enforce the two digest clauses (`:13537`,
//!      `:13540`): the expected-digest function and its six pinned constants are
//!      `pub(crate)` in `recorder_host`, which `wire_format` must not depend on,
//!      and copying them is the drift the shared-contract rule forbids. The other
//!      **eighteen** clauses of the `world_fold.rs::fold_tech` predicate need no digest and
//!      are all enforced here, so a row this mapper accepts fails at most those
//!      two.
//!
//!    **A stop-report on storage is not a licence to skip the fold's acceptance
//!    checks.** Because those two clauses cannot be evaluated here, a caller that
//!    used this module to decide whether the recorder would accept a stream could
//!    accept one the recorder refuses — flip a bit in
//!    `selected_present_receipt_sha256` and the eighteen clauses still pass. The
//!    rule is therefore fail-closed and lives with the caller that has an
//!    acceptance policy: [`payload_carries_a29_provenance`] is public for exactly
//!    that, and `bin/bench_real_d13.rs` refuses to measure any stream whose d13
//!    payloads carry a provenance block. The reference capture carries none.
//!
//!    `localization_branch` is ported because it is the one field in the block
//!    with meaning outside the receipt: the dossier's branch table makes it the
//!    *displayed form* (`0` no display, `1` unreduced, `2` reduced), the fold
//!    constrains the quartet's shape by it (`:13513`-`:13522`), and it is not
//!    recoverable from the quartet — the quartet `(0,0,0,0)` satisfies branch 0
//!    and branch 1 alike. The dossier says so directly: "Branch identity comes
//!    from the observed ordered name sequence, never inferred from the values."
//!    Its three codes are mutually exclusive, which is §4's `Enum`.
//!
//! # Four states
//!
//! Every gated field here is **unknown** when its gate is clear, never null and
//! never absent, because every gate is `Option::is_some()` over a read that
//! returns `None` only on a failed capture:
//!
//! * `completed_tech_count`, `research_slot_count` — see above.
//! * `live_progress_raw` — `None` on unreadable memory or a value outside
//!   `0..=MAX_TECH_LIVE_PROGRESS_RAW` (`attach_log.rs:80999`-`:81000`).
//! * `base_required_points_raw` — `None` on an unreadable template, a
//!   non-positive cost or scale, or an overflowing product
//!   (`attach_log.rs:81311`-`:81356`).
//! * the quartet and `ahead_of_time_branch` — `None` when the game hour is
//!   unreadable or no authenticated producer observation exists for this
//!   technology at this hour (`attach_log.rs:81009`-`:81018`). The dossier
//!   records exactly this outcome for a late attach: "The late-attach fold
//!   correctly contained the active research census but no quartet; authenticating
//!   it requires the early-launch selected-Present path." The value existed in the
//!   engine and was not captured. It is emphatically **not** null: branch `0` —
//!   "no display: years 0, penalty 0, reduced penalty 0" — is how *absence of an
//!   ahead-of-time bonus* is expressed, and that is a value, so a clear gate can
//!   never mean "there is nothing".
//!
//! Nothing here is `optional` and nothing is `nullable`: no field is inapplicable
//! to a row this schema carries, and no field is a relation that can point at
//! nothing.
//!
//! # No derived field (§6)
//!
//! §6 admits clock-linear integers only. `progress_raw` rises while a slot
//! researches but resets when the slot takes a new technology and moves at a rate
//! the clock does not fix, so it is not `v + c * (frame_tick >> u)`. The tick
//! parameter is still used — by the A29 same-hour gate, not by a transform.

use core::ops::Range;

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 13;

pub const COUNTRIES: u32 = 1;
pub const ACTIVE_RESEARCH: u32 = 2;

/// Index of each collection in [`schema`]`().collections` and in the slices the
/// body encoders take. The order is the schema's and never changes.
pub const COUNTRIES_INDEX: usize = 0;
pub const ACTIVE_RESEARCH_INDEX: usize = 1;
pub const COLLECTION_COUNT: usize = 2;

/// Field ids are stable and append-only; §11 forbids reusing an id for a
/// *different* field. Ids are unique per collection, and `TAG_DB_ID` is
/// deliberately the same id in both because it names the same concept.
pub mod field {
    pub const TAG_DB_ID: u32 = 1;
    pub const COMPLETED_TECH_COUNT: u32 = 2;
    pub const RESEARCH_SLOTS: u32 = 3;
    /// Whether the country's active-research list was read at all.
    pub const ACTIVE_RESEARCH_CAPTURED: u32 = 4;
    pub const SLOT_INDEX: u32 = 5;
    pub const TECH_HANDLE_RAW: u32 = 6;
    pub const TECH_DEF_INDEX_RAW: u32 = 7;
    pub const PROGRESS_RAW: u32 = 8;
    pub const LIVE_PROGRESS_RAW: u32 = 9;
    pub const BASE_REQUIRED_POINTS_RAW: u32 = 10;
    /// Whether the content table resolved a name for `tech_handle_raw`. The name
    /// itself is a field-level stop-report; see the module docs.
    pub const TECH_NAME_CAPTURED: u32 = 11;
    pub const AHEAD_OF_TIME_YEARS_RAW: u32 = 12;
    pub const AHEAD_OF_TIME_PENALTY_RAW: u32 = 13;
    pub const AHEAD_OF_TIME_REDUCTION_YEARS_RAW: u32 = 14;
    pub const AHEAD_OF_TIME_REDUCED_PENALTY_RAW: u32 = 15;
    pub const AHEAD_OF_TIME_BRANCH: u32 = 16;
}

// --- Payload grammar, from `world_fold.rs::fold_tech`. -------------------

/// `world_fold.rs::fold_tech` — `need(payload, 32, "tech header")`.
pub const HEADER_LEN: usize = 32;
/// `world_fold.rs::fold_tech` — `cursor += 16`.
pub const COUNTRY_ROW_LEN: usize = 16;
/// `world_fold.rs::fold_tech` — `cursor += 40`, before any tail.
pub const ACTIVE_ROW_FIXED_LEN: usize = 40;
/// `world_fold.rs::fold_tech` — four `i64` ahead-of-time values.
pub const AHEAD_QUARTET_LEN: usize = 32;
/// `world_fold.rs::fold_tech` — the A29 producer provenance block.
pub const A29_PROVENANCE_LEN: usize = 176;

/// Country-row flag bits. **The fold never reads this word**; the collector packs
/// it at `attach_log.rs:69214`-`:69225` and it is the only authority on why a
/// value is missing. See the module docs.
pub const COUNTRY_FLAG_COMPLETED_TECH_COUNT: u32 = 1;
pub const COUNTRY_FLAG_RESEARCH_SLOT_COUNT: u32 = 1 << 1;
pub const COUNTRY_FLAG_ACTIVE_RESEARCH: u32 = 1 << 2;

/// The word the collector packs for a gated-off country count
/// (`attach_log.rs:69229`, `:69235`: `unwrap_or(u32::MAX)`). Unreachable as a
/// real reading: the collector's own bounds are 2000 and 32
/// (`attach_log/offsets.rs:274`-`:275`).
pub const COUNTRY_COUNT_ABSENT_SENTINEL: u32 = u32::MAX;

/// Active-row flag bits (`world_fold.rs:13368`, `:13377`, `:13389`, `:13396`,
/// `:13448`).
pub const ACTIVE_FLAG_LIVE_PROGRESS: u32 = 1;
pub const ACTIVE_FLAG_BASE_REQUIRED_POINTS: u32 = 1 << 1;
pub const ACTIVE_FLAG_TECH_NAME: u32 = 1 << 2;
pub const ACTIVE_FLAG_AHEAD_QUARTET: u32 = 1 << 3;
pub const ACTIVE_FLAG_A29_PROVENANCE: u32 = 1 << 4;
/// `world_fold.rs::fold_tech` — `flags & !0b1_1111` is a hard error.
pub const ACTIVE_KNOWN_FLAGS: u32 = 0b1_1111;

/// `TECH_PROGRESS_SCALE`, `world_fold.rs::TECH_PROGRESS_SCALE`. Applies to `progress_raw` alone
/// (`:13366`). The header word at `+20` carries it too (`attach_log.rs:69209`).
pub const TECH_PROGRESS_SCALE: f64 = 1000.0;
pub const TECH_PROGRESS_SCALE_RAW: u32 = 1000;
/// `TECH_LIVE_SCALE`, `world_fold.rs::TECH_LIVE_SCALE`. **100× the other one**, and it applies
/// to six fields in the same row: `live_progress_raw` (`:13372`),
/// `base_required_points_raw` (`:13384`) and all four quartet values
/// (`:13433`-`:13445`). The header word at `+24` carries it too
/// (`attach_log.rs:69210`), and the fold restates it per row as
/// `"ahead_of_time_fixed_scale": 100_000` (`:13420`).
pub const TECH_LIVE_SCALE: f64 = 100_000.0;
pub const TECH_LIVE_SCALE_RAW: u32 = 100_000;

/// `world_fold.rs::fold_tech` — every quartet value must lie in `0..=10^10`.
pub const AHEAD_RAW_MAX: i64 = 10_000_000_000;

/// `world_fold.rs::fold_tech` — the A29 block's `authority` word.
pub const A29_AUTHORITY: u32 = 3;
/// `world_fold.rs::fold_tech` compares the block's `ui_action_gate_return_rva` against
/// `A29_UI_YEAR_HELPER_RETURN_RVA`, which is `pub(crate)` in
/// `recorder_host/src/domain_data_authority.rs:59`. The same value is published
/// knowledge in `documents/domains/d13_tech_live.md:42` ("The helper detour's
/// first in-image return must be RVA `0x01BCF384`"), which is what makes it
/// citable here rather than a private constant smuggled across a crate boundary.
/// It is nevertheless a copied constant and is listed in the open questions.
pub const A29_UI_YEAR_HELPER_RETURN_RVA: u32 = 0x01BC_F384;
/// The three `localization_branch` codes (`world_fold.rs::fold_tech`, dossier table).
pub const AHEAD_BRANCH_CARDINALITY: u32 = 3;

pub fn schema() -> Schema {
    use field as f;

    // `countries`: the per-country research census.
    //
    // The key is `tag_db_id` (`world_fold.rs::fold_tech`), the identity d18's roster
    // defines and d63 keys by. `ContentId` because a country tag is
    // content-defined (§5) — a manifest must resolve it for it to keep its
    // meaning under another mod set. It is a ContentId *key*, which §13 flags as
    // the heavier manifest-retrofit class; see the open questions.
    //
    // `completed_tech_count` is `Monotone`: a researched technology is never
    // un-researched, so the count rises. §4 makes monotone an encoding hint and
    // never a validity constraint, so nothing breaks if the engine ever trims it.
    // `research_slots` is plain `Unsigned` — slots are granted and can be lost
    // with the focus or law that granted them, so the hint would be wrong about
    // the data rather than merely unhelpful.
    let countries = Collection {
        collection_id: COUNTRIES,
        shape: Shape::KeyedSet,
        fields: vec![
            Field::new(f::TAG_DB_ID, Kind::ContentId).key(0),
            Field::new(f::COMPLETED_TECH_COUNT, Kind::Monotone).unknown_capable(),
            Field::new(f::RESEARCH_SLOTS, Kind::Unsigned).unknown_capable(),
            Field::new(f::ACTIVE_RESEARCH_CAPTURED, Kind::Boolean),
        ],
    };

    // `active_research`: one row per occupied research slot.
    //
    // The key is `(tag_db_id, slot_index)` (`world_fold.rs::fold_tech`). A
    // research slot is a stable identity that persists across hours — the same
    // slot keeps working, then takes a new technology — which is what makes the
    // delta grammar worth anything here; keying by the technology instead would
    // turn every completed research into a tombstone plus an insert.
    //
    // **This is a keyed set and not an ordered shape**, which is worth stating
    // because the fold accumulates these rows into a JSON *array* (`world_fold.rs::fold_tech`).
    // Two facts make the array's order carry nothing: the collector sorts by
    // `slot_index` before packing (`attach_log.rs:81045`), and it refuses a
    // country whose slot indices repeat (`:80977`-`:80979`). So payload order is
    // `slot_index` order, which is exactly the keyed set's own canonical order,
    // and no ordering information is lost. A payload that *did* repeat a
    // `(tag, slot_index)` is refused by [`state_from_payload`] rather than
    // silently collapsed, because the fold pushes both rows onto its array and a
    // keyed set cannot hold both — see there.
    //
    // `tech_handle_raw` and `tech_def_index_raw` are both `ContentId`: they index
    // the technology database, which mod content defines, so a manifest must
    // resolve them for them to keep their meaning. The collector requires the two
    // to be equal (`attach_log.rs:80995`-`:80997`) and they are equal in all
    // 364,487 measured rows — but `fold_tech` does not require it, so dropping
    // either one would lose data on input the recorder accepts.
    //
    // `progress_raw` is `Unsigned`, not `Monotone`: it rises while a slot works
    // and drops to zero when the technology completes and the slot takes the
    // next one, which is the common case rather than an exception.
    let active_research = Collection {
        collection_id: ACTIVE_RESEARCH,
        shape: Shape::KeyedSet,
        fields: vec![
            Field::new(f::TAG_DB_ID, Kind::ContentId).key(0),
            Field::new(f::SLOT_INDEX, Kind::Unsigned).key(1),
            Field::new(f::TECH_HANDLE_RAW, Kind::ContentId),
            Field::new(f::TECH_DEF_INDEX_RAW, Kind::ContentId),
            Field::new(f::PROGRESS_RAW, Kind::Unsigned),
            Field::new(f::LIVE_PROGRESS_RAW, Kind::Signed).unknown_capable(),
            Field::new(f::BASE_REQUIRED_POINTS_RAW, Kind::Signed).unknown_capable(),
            Field::new(f::TECH_NAME_CAPTURED, Kind::Boolean),
            Field::new(f::AHEAD_OF_TIME_YEARS_RAW, Kind::Signed).unknown_capable(),
            Field::new(f::AHEAD_OF_TIME_PENALTY_RAW, Kind::Signed).unknown_capable(),
            Field::new(f::AHEAD_OF_TIME_REDUCTION_YEARS_RAW, Kind::Signed).unknown_capable(),
            Field::new(f::AHEAD_OF_TIME_REDUCED_PENALTY_RAW, Kind::Signed).unknown_capable(),
            Field::new(f::AHEAD_OF_TIME_BRANCH, Kind::Enum)
                .enum_card(AHEAD_BRANCH_CARDINALITY)
                .unknown_capable(),
        ],
    };

    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![countries, active_research],
        event_part: None,
    }
}

// --- The payload header and its two tables -----------------------------------

/// The 32-byte payload header (`world_fold.rs::fold_tech`, packer
/// `attach_log.rs:69205`-`:69211`).
///
/// `changed_rows`, `status_flags` and the two scale words are carried here
/// because they are captured bytes, not because anything below acts on them:
/// `fold_tech` reads none of the four, so checking them would reject payloads the
/// recorder folds. `bin/bench_real_d13.rs` reports them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub country_count: u32,
    pub changed_rows: u32,
    pub status_flags: u32,
    pub progress_scale_raw: u32,
    pub live_scale_raw: u32,
    pub active_count: u32,
}

/// The two tables of one payload. `active` is the whole remainder: the rows are
/// variable-length, so only [`active_row_spans`] can find their boundaries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Tables<'a> {
    pub countries: &'a [u8],
    pub active: &'a [u8],
}

/// Split a d13 DATA payload into its header and its two tables, applying the
/// fold's own structural checks (`world_fold.rs::fold_tech`).
///
/// This lives here rather than in each caller so the header grammar has exactly
/// one implementation: a measurement harness that re-derives it is a harness that
/// can drift from the recorder without anything failing.
///
/// The fold's bound is a **minimum**, not an equality — the active rows carry
/// optional tails — so the exact-consumption rule (`world_fold.rs::fold_tech`) belongs to
/// [`active_row_spans`], which is the only function that can evaluate it.
pub fn split_payload(payload: &[u8]) -> core::result::Result<(PayloadHeader, Tables<'_>), String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "tech payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    let header = PayloadHeader {
        elapsed_wall_ms: u64::from_le_bytes(payload[0..8].try_into().expect("8 bytes")),
        country_count: u32_at(payload, 8),
        changed_rows: u32_at(payload, 12),
        status_flags: u32_at(payload, 16),
        progress_scale_raw: u32_at(payload, 20),
        live_scale_raw: u32_at(payload, 24),
        active_count: u32_at(payload, 28),
    };
    // Checked arithmetic: the counts are input from a file on disk, and the
    // fold's `saturating_*` (`:13325`-`:13327`) turns an overflowing product into
    // a length shortfall rather than a wrapped, plausible one. Checked reports the
    // overflow as itself; both refuse the payload.
    let countries_len = (header.country_count as usize)
        .checked_mul(COUNTRY_ROW_LEN)
        .ok_or_else(|| format!("tech country_count {} overflows", header.country_count))?;
    let active_min = (header.active_count as usize)
        .checked_mul(ACTIVE_ROW_FIXED_LEN)
        .ok_or_else(|| format!("tech active_count {} overflows", header.active_count))?;
    let minimum = HEADER_LEN
        .checked_add(countries_len)
        .and_then(|n| n.checked_add(active_min))
        .ok_or_else(|| "tech payload length overflows".to_string())?;
    if payload.len() < minimum {
        return Err(format!("tech len {} < minimum {minimum}", payload.len()));
    }
    let countries_end = HEADER_LEN + countries_len;
    Ok((
        header,
        Tables {
            countries: &payload[HEADER_LEN..countries_end],
            active: &payload[countries_end..],
        },
    ))
}

/// The byte span of each active-research row inside [`Tables::active`], walking
/// the variable tails exactly as `fold_tech` walks them
/// (`world_fold.rs::fold_tech`) and enforcing its trailing-bytes rule
/// (`world_fold.rs::fold_tech`).
///
/// Only structure is decided here — the fixed-length bound (`world_fold.rs::fold_tech`), the flag
/// mask (`world_fold.rs::fold_tech`, the fold's first act on a row), the string's own length
/// (`:13390` via `read_string`, **`u16`-prefixed**, `world_fold.rs::read_string`), and
/// the two fixed tails. Everything semantic stays in [`record_from_active_row`],
/// which is where the recipe puts per-row checks.
pub fn active_row_spans(
    active: &[u8],
    active_count: u32,
) -> core::result::Result<Vec<Range<usize>>, String> {
    let mut spans = Vec::with_capacity(active_count as usize);
    let mut cursor = 0usize;
    for index in 0..active_count as usize {
        // Every step below leaves `cursor <= active.len()`: `active_row_len`
        // returns a length its own slice can carry.
        let len = active_row_len(&active[cursor..])
            .map_err(|e| format!("tech active row {index}: {e}"))?;
        let end = cursor
            .checked_add(len)
            .ok_or_else(|| format!("tech active row {index} length overflows"))?;
        spans.push(cursor..end);
        cursor = end;
    }
    // `world_fold.rs::fold_tech`.
    if cursor != active.len() {
        return Err(format!(
            "tech trailing bytes: consumed {cursor} of {} active-table bytes",
            active.len()
        ));
    }
    Ok(spans)
}

/// The total length of the active-research row that starts at `rest[0]`, or an
/// error if `rest` cannot carry one.
pub fn active_row_len(rest: &[u8]) -> core::result::Result<usize, String> {
    if rest.len() < ACTIVE_ROW_FIXED_LEN {
        return Err(format!(
            "row needs {ACTIVE_ROW_FIXED_LEN} fixed bytes, {} present",
            rest.len()
        ));
    }
    let flags = u32_at(rest, 20);
    // `world_fold.rs::fold_tech` — an unrecognised flag word means the row is not this
    // grammar, so nothing in it (including its length) can be trusted.
    if flags & !ACTIVE_KNOWN_FLAGS != 0 {
        return Err(format!("row has unknown flags {flags:#x}"));
    }
    let mut len = ACTIVE_ROW_FIXED_LEN;
    if flags & ACTIVE_FLAG_TECH_NAME != 0 {
        // `read_string`, `world_fold.rs::read_string`: a **u16** length prefix, then that
        // many UTF-8 bytes. The packer writes the same (`attach_log.rs:71944`).
        if rest.len() < len + 2 {
            return Err("row is too short for the tech_name length prefix".into());
        }
        let name_len = u16::from_le_bytes([rest[len], rest[len + 1]]) as usize;
        len += 2 + name_len;
        if rest.len() < len {
            return Err("row is too short for its tech_name bytes".into());
        }
    }
    if flags & ACTIVE_FLAG_AHEAD_QUARTET != 0 {
        len += AHEAD_QUARTET_LEN;
        if rest.len() < len {
            return Err("row is too short for the ahead-of-time quartet".into());
        }
    }
    if flags & ACTIVE_FLAG_A29_PROVENANCE != 0 {
        // `world_fold.rs::fold_tech` — the provenance block requires the quartet, and
        // the check runs before the 176-byte `need`.
        if flags & ACTIVE_FLAG_AHEAD_QUARTET == 0 {
            return Err("A29 provenance without ahead quartet".into());
        }
        len += A29_PROVENANCE_LEN;
        if rest.len() < len {
            return Err("row is too short for the A29 provenance block".into());
        }
    } else if flags & ACTIVE_FLAG_AHEAD_QUARTET != 0 {
        // `world_fold.rs::fold_tech` — the converse. The fold reaches it only after
        // consuming the quartet, which is why it is checked after the length is
        // known rather than beside the flag mask.
        return Err("ahead quartet lacks observed A29 producer provenance".into());
    }
    Ok(len)
}

// --- Keys --------------------------------------------------------------------

/// The key of one country row: its `tag_db_id` (`world_fold.rs::fold_tech`).
///
/// The fold validates nothing about it — not even zero, unlike d63 (`world_fold.rs::fold_country_core_scalars`) —
/// so neither does this.
pub fn key_from_country_row(row: &[u8]) -> core::result::Result<Key, String> {
    check_country_row_len(row)?;
    Ok(key(&[u32_at(row, 0) as u64]))
}

/// The key of one active-research row: `(tag_db_id, slot_index)`
/// (`world_fold.rs::fold_tech`).
pub fn key_from_active_row(row: &[u8]) -> core::result::Result<Key, String> {
    if row.len() < ACTIVE_ROW_FIXED_LEN {
        return Err(format!(
            "tech active row length {} < {ACTIVE_ROW_FIXED_LEN}",
            row.len()
        ));
    }
    Ok(key(&[u32_at(row, 0) as u64, u32_at(row, 4) as u64]))
}

// --- The boundary mapping ----------------------------------------------------

/// One compact row of the `countries` table, as the collector emits it today,
/// onto its stored Record. `row` is the raw 16-byte row slice (the caller has
/// already consumed the payload header via [`split_payload`]); d13 has exactly
/// one country row form, so length selects nothing and a wrong length is
/// rejected. `frame_tick` is §6's derived-field input
/// (stored = v + c * (frame_tick >> u)); no country field is derived, so it is
/// taken and ignored — every domain's mapper has the same shape.
///
/// The four-state decisions come from the flag word at `+4`, which `fold_tech`
/// never reads; see the module docs for why that is the recipe's answer and not
/// a departure from it.
pub fn record_from_country_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    check_country_row_len(row)?;
    let flags = u32_at(row, 4);
    // Bits above `1<<2` are not rejected: `fold_tech` never looks at this word at
    // all, so refusing an unfamiliar bit would refuse a payload the recorder
    // folds. A future bit would be missed rather than mis-read, which is true of
    // every schema. Recorded in the open questions.
    let completed = gated_country_count(
        flags & COUNTRY_FLAG_COMPLETED_TECH_COUNT != 0,
        u32_at(row, 8),
        "completed_tech_count",
    )?;
    let slots = gated_country_count(
        flags & COUNTRY_FLAG_RESEARCH_SLOT_COUNT != 0,
        u32_at(row, 12),
        "research_slot_count",
    )?;

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        values.push(match f.field_id {
            field::COMPLETED_TECH_COUNT => completed,
            field::RESEARCH_SLOTS => slots,
            field::ACTIVE_RESEARCH_CAPTURED => {
                Value::Bool(flags & COUNTRY_FLAG_ACTIVE_RESEARCH != 0)
            }
            other => return Err(no_mapping("countries", other)),
        });
    }
    Ok(Record::new(values))
}

/// A gated country count. A clear gate is §3's **unknown**: the collector could
/// not read the value (module docs), and it packs `u32::MAX` in its place.
///
/// A clear gate over any *other* word is a contradiction between this domain's
/// two authorities — the collector cannot produce it, and the fold would publish
/// the word as a real count — so it is refused rather than resolved silently in
/// either direction.
fn gated_country_count(
    present: bool,
    word: u32,
    label: &str,
) -> core::result::Result<Value, String> {
    if present {
        Ok(Value::Unsigned(word as u64))
    } else if word == COUNTRY_COUNT_ABSENT_SENTINEL {
        Ok(Value::Unknown)
    } else {
        Err(format!(
            "tech {label} {word} without its capture flag and without the \
             {COUNTRY_COUNT_ABSENT_SENTINEL} absent sentinel"
        ))
    }
}

/// One compact row of the `active_research` table onto its stored Record.
///
/// `row` is the **exact** row slice, as [`active_row_spans`] delimits it; a slice
/// this function does not consume exactly is rejected, so a caller that mis-split
/// the table gets an error rather than a record built from another row's bytes.
///
/// `frame_tick` is the recipe's fixed §6 input, and d13 declares no derived
/// field, so it is taken and ignored — every domain's mapper has the same shape.
///
/// **A tick index is not the fold's game hour.** `fold_tech` is the only fold in
/// `world_fold.rs` that takes `game_hour_raw` (`world_fold.rs::fold_tech`), and its sole consumer is
/// the A29 same-hour authority gate at `world_fold.rs::fold_tech`. But `game_hour_raw` is the
/// engine's stored absolute hour counter, and the container's own coordinate is
/// `hour_index = raw_date - DATE_EPOCH_CONST` with `DATE_EPOCH_CONST = 43_799_999`
/// (`compact_contract/src/calendar.rs:9`, `:22`) — the reference capture's
/// `60_832_407` is hour index `17_032_408`, not `60_832_407`. The two coordinates
/// share a *rate* (`cadence_log2 = 0` makes both hourly) and not an origin, so a
/// §6 tick cannot be fed to the gate. This entry point therefore **refuses** any
/// row carrying a provenance block rather than evaluating the gate against the
/// wrong clock or skipping it in silence; [`record_from_active_row_at_hour`] is
/// the entry that takes the fold's own `game_hour_raw` and can decide it. That is
/// the recipe's own pattern for a header-relative invariant — d26's `FrameMode`,
/// d24's `check_province_id(id, observed_province_rows)` — an explicit extra
/// parameter beside the fixed signature, never a coordinate pun.
pub fn record_from_active_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    if active_row_carries_a29_provenance(row)? {
        return Err(
            "tech active row carries A29 provenance, whose same-hour authority gate needs the \
             frame's game_hour_raw; call record_from_active_row_at_hour"
                .into(),
        );
    }
    record_from_active_row_at_hour(collection, row, None)
}

/// Whether an active-research row carries the A29 provenance tail
/// (`world_fold.rs::fold_tech`). Public so that a caller implementing the fail-closed
/// policy the module docs describe does not re-derive the row's flag layout.
pub fn active_row_carries_a29_provenance(row: &[u8]) -> core::result::Result<bool, String> {
    if row.len() < ACTIVE_ROW_FIXED_LEN {
        return Err(format!(
            "tech active row length {} < {ACTIVE_ROW_FIXED_LEN}",
            row.len()
        ));
    }
    Ok(u32_at(row, 20) & ACTIVE_FLAG_A29_PROVENANCE != 0)
}

/// Whether any active-research row in a whole DATA payload carries the A29
/// provenance tail. Structural only: the payload is split and its rows are
/// delimited, but nothing semantic is decided.
pub fn payload_carries_a29_provenance(payload: &[u8]) -> core::result::Result<bool, String> {
    let (header, tables) = split_payload(payload)?;
    for span in active_row_spans(tables.active, header.active_count)? {
        if active_row_carries_a29_provenance(&tables.active[span])? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// [`record_from_active_row`] in the fold's own shape: `frame_game_hour` is
/// `fold_tech`'s `game_hour_raw` parameter, and `None` skips the same-hour clause
/// exactly as `game_hour_raw.is_some_and(..)` does at `world_fold.rs::fold_tech`.
///
/// This evaluates **eighteen of the twenty** clauses of the fold's A29 acceptance
/// predicate. The two digest equalities (`world_fold.rs::fold_tech`) need
/// `recorder_host` internals this crate cannot reach; the module docs say what a
/// caller must do about them, and `bin/bench_real_d13.rs` does it.
pub fn record_from_active_row_at_hour(
    collection: &Collection,
    row: &[u8],
    frame_game_hour: Option<i32>,
) -> core::result::Result<Record, String> {
    let parsed = parse_active_row(row, frame_game_hour)?;
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        values.push(match f.field_id {
            field::TECH_HANDLE_RAW => Value::ContentId(parsed.tech_handle_raw as u64),
            field::TECH_DEF_INDEX_RAW => Value::ContentId(parsed.tech_def_index_raw as u64),
            field::PROGRESS_RAW => Value::Unsigned(parsed.progress_raw as u64),
            field::LIVE_PROGRESS_RAW => gated_signed(parsed.live_progress_raw),
            field::BASE_REQUIRED_POINTS_RAW => gated_signed(parsed.base_required_points_raw),
            field::TECH_NAME_CAPTURED => Value::Bool(parsed.tech_name_captured),
            field::AHEAD_OF_TIME_YEARS_RAW => gated_signed(parsed.quartet.map(|q| q.0)),
            field::AHEAD_OF_TIME_PENALTY_RAW => gated_signed(parsed.quartet.map(|q| q.1)),
            field::AHEAD_OF_TIME_REDUCTION_YEARS_RAW => gated_signed(parsed.quartet.map(|q| q.2)),
            field::AHEAD_OF_TIME_REDUCED_PENALTY_RAW => gated_signed(parsed.quartet.map(|q| q.3)),
            field::AHEAD_OF_TIME_BRANCH => match parsed.branch {
                Some(branch) => Value::Enum(branch),
                None => Value::Unknown,
            },
            other => return Err(no_mapping("active_research", other)),
        });
    }
    Ok(Record::new(values))
}

/// A gate that is clear means the collector could not read the value, not that
/// the value is nothing — see the module docs. §3 calls that **unknown**.
fn gated_signed(value: Option<i64>) -> Value {
    match value {
        Some(v) => Value::Signed(v),
        None => Value::Unknown,
    }
}

/// One active-research row, decoded and checked. Everything `fold_tech` refuses
/// is an `Err` here, at the same point in the row that the fold refuses it.
struct ActiveRow {
    tech_handle_raw: u32,
    tech_def_index_raw: u32,
    progress_raw: u32,
    live_progress_raw: Option<i64>,
    base_required_points_raw: Option<i64>,
    tech_name_captured: bool,
    quartet: Option<(i64, i64, i64, i64)>,
    branch: Option<u32>,
}

fn parse_active_row(
    row: &[u8],
    frame_game_hour: Option<i32>,
) -> core::result::Result<ActiveRow, String> {
    let len = active_row_len(row)?;
    if len != row.len() {
        return Err(format!(
            "tech active row is {} bytes but its grammar consumes {len}",
            row.len()
        ));
    }
    let flags = u32_at(row, 20);
    let live_word = i64_at(row, 24);
    let base_word = i64_at(row, 32);

    // `world_fold.rs:13374` and `:13386`: a gated-off value must pack a zero.
    let live_progress_raw = if flags & ACTIVE_FLAG_LIVE_PROGRESS != 0 {
        Some(live_word)
    } else if live_word != 0 {
        return Err("tech live_progress_raw without presence flag".into());
    } else {
        None
    };
    let base_required_points_raw = if flags & ACTIVE_FLAG_BASE_REQUIRED_POINTS != 0 {
        Some(base_word)
    } else if base_word != 0 {
        return Err("tech base_required_points_raw without presence flag".into());
    } else {
        None
    };

    let mut cursor = ACTIVE_ROW_FIXED_LEN;
    let tech_name_captured = flags & ACTIVE_FLAG_TECH_NAME != 0;
    if tech_name_captured {
        let name_len = u16::from_le_bytes([row[cursor], row[cursor + 1]]) as usize;
        cursor += 2;
        // `world_fold.rs:13391` — an empty name is a rejected row, and
        // `read_string` (`world_fold.rs::read_string`) rejects invalid UTF-8 before that. The bytes
        // themselves are a field-level stop-report (module docs); they are still
        // validated, because a row the fold refuses must not be measured.
        if name_len == 0 {
            return Err("tech active row has empty tech_name".into());
        }
        core::str::from_utf8(&row[cursor..cursor + name_len])
            .map_err(|e| format!("tech_name: invalid utf-8: {e}"))?;
        cursor += name_len;
    }

    let quartet = if flags & ACTIVE_FLAG_AHEAD_QUARTET != 0 {
        let q = (
            i64_at(row, cursor),
            i64_at(row, cursor + 8),
            i64_at(row, cursor + 16),
            i64_at(row, cursor + 24),
        );
        // `world_fold.rs::fold_tech`.
        if [q.0, q.1, q.2, q.3]
            .iter()
            .any(|value| !(0..=AHEAD_RAW_MAX).contains(value))
        {
            return Err("tech active row has invalid ahead fixed-5 quartet".into());
        }
        cursor += AHEAD_QUARTET_LEN;
        Some(q)
    } else {
        None
    };

    let branch = if flags & ACTIVE_FLAG_A29_PROVENANCE != 0 {
        let quartet = quartet.ok_or("tech A29 provenance without ahead quartet")?;
        let branch = check_a29_provenance(
            &row[cursor..cursor + A29_PROVENANCE_LEN],
            quartet,
            frame_game_hour,
        )?;
        cursor += A29_PROVENANCE_LEN;
        Some(branch)
    } else {
        None
    };

    debug_assert_eq!(cursor, row.len());
    Ok(ActiveRow {
        tech_handle_raw: u32_at(row, 8),
        tech_def_index_raw: u32_at(row, 12),
        progress_raw: u32_at(row, 16),
        live_progress_raw,
        base_required_points_raw,
        tech_name_captured,
        quartet,
        branch,
    })
}

/// Every clause of the A29 exact same-hour authority predicate
/// (`world_fold.rs::fold_tech`) that can be evaluated without recomputing a
/// SHA-256, and the block's `localization_branch` on success.
///
/// Eighteen of the twenty clauses are here. The two that are not
/// (`computed_selected_present_sha256 != ...` at `world_fold.rs::fold_tech` and
/// `expected_source_record_sha256 != Some(..)` at `world_fold.rs::fold_tech`) need
/// `recorder_host::domain_data_authority::a29_expected_source_record_sha256` and
/// `portable_render_stream::SelectedPresentReceipt`, both unreachable from this
/// crate; that is one of the three reasons the block is a field-level
/// stop-report. `slot_index` and `tech_def_index_raw` reach the predicate only as
/// hash inputs (`domain_data_authority.rs:104`-`:105`), so their absence from the
/// checkable set costs nothing beyond those two clauses.
fn check_a29_provenance(
    block: &[u8],
    quartet: (i64, i64, i64, i64),
    frame_game_hour: Option<i32>,
) -> core::result::Result<u32, String> {
    let authority = u32_at(block, 0);
    let branch = u32_at(block, 4);
    let callback_sequence = u64_at(block, 8);
    let callback_thread_id = u32_at(block, 16);
    let producer_game_hour_raw = i32_at(block, 20);
    let callback_qpc_ticks = u64_at(block, 24);
    let qpc_frequency_hz = u64_at(block, 32);
    let source_record_sha256 = &block[40..72];
    let ui_action_gate_return_rva = u32_at(block, 72);
    let candidate_count = u32_at(block, 76);
    let selected_ordinal = u32_at(block, 80);
    let selected_game_hour_raw = i32_at(block, 84);
    let selected_qpc_ticks = u64_at(block, 96);
    let selection_cutoff_qpc_ticks = u64_at(block, 104);
    let candidate_set_sha256 = &block[112..144];
    let selected_present_receipt_sha256 = &block[144..176];

    // `world_fold.rs::fold_tech`, in the fold's own order.
    if authority != A29_AUTHORITY {
        return a29_fail("authority");
    }
    if !matches!(branch, 0 | 1 | 2) {
        return a29_fail("branch");
    }
    let (years, penalty, reduction, reduced_penalty) = quartet;
    let branch_shape_bad = match branch {
        0 => years != 0 || penalty != 0 || reduction < 0 || reduced_penalty != 0,
        1 => reduction != 0 || reduced_penalty != penalty,
        _ => reduction <= 0,
    };
    if branch_shape_bad {
        return a29_fail("branch shape");
    }
    if callback_sequence == 0 {
        return a29_fail("callback_sequence");
    }
    if callback_thread_id == 0 {
        return a29_fail("callback_thread_id");
    }
    // `world_fold.rs::fold_tech` — the same-hour gate, and the whole reason this
    // domain's mapper takes the frame clock.
    if frame_game_hour.is_some_and(|frame_hour| producer_game_hour_raw != frame_hour) {
        return a29_fail("producer_game_hour_raw");
    }
    if callback_qpc_ticks == 0 {
        return a29_fail("callback_qpc_ticks");
    }
    if qpc_frequency_hz == 0 {
        return a29_fail("qpc_frequency_hz");
    }
    if ui_action_gate_return_rva != A29_UI_YEAR_HELPER_RETURN_RVA {
        return a29_fail("ui_action_gate_return_rva");
    }
    if candidate_count == 0 {
        return a29_fail("candidate_count");
    }
    if selected_ordinal >= candidate_count {
        return a29_fail("selected_ordinal");
    }
    if selected_game_hour_raw != producer_game_hour_raw {
        return a29_fail("selected_game_hour_raw");
    }
    if callback_qpc_ticks > selected_qpc_ticks {
        return a29_fail("callback_qpc_ticks ordering");
    }
    if selected_qpc_ticks == 0 {
        return a29_fail("selected_qpc_ticks");
    }
    if selection_cutoff_qpc_ticks < selected_qpc_ticks {
        return a29_fail("selection_cutoff_qpc_ticks");
    }
    if candidate_set_sha256.iter().all(|byte| *byte == 0) {
        return a29_fail("candidate_set_sha256");
    }
    if selected_present_receipt_sha256.iter().all(|byte| *byte == 0) {
        return a29_fail("selected_present_receipt_sha256");
    }
    if source_record_sha256.iter().all(|byte| *byte == 0) {
        return a29_fail("source_record_sha256");
    }
    Ok(branch)
}

/// The fold's single message for every clause of the predicate
/// (`world_fold.rs::fold_tech`), plus the clause that failed. The clause name is
/// diagnosis only and changes nothing about which rows are accepted.
fn a29_fail(clause: &str) -> core::result::Result<u32, String> {
    Err(format!(
        "tech A29 provenance failed exact same-hour authority ({clause})"
    ))
}

/// A collection this mapper does not know how to fill. Filling it with a
/// placeholder would publish invented data under a field id that means something
/// else.
fn no_mapping(collection: &str, field_id: u32) -> String {
    format!("tech {collection} has no mapping for field id {field_id}")
}

// --- The domain fold ---------------------------------------------------------

/// The accumulated state of both collections, in [`schema`] order.
///
/// **d13 is upsert-by-country, not full replace, and one record is not one
/// state.** `fold_tech` builds a fresh `per_tag` map from the payload and then
/// publishes it with `for (tag, obj) in per_tag { self.country_mut(tag).insert("research", obj) }`
/// (`world_fold.rs::fold_tech`). It touches only the tags the payload
/// carries; a country the payload omits keeps the `research` object it already
/// had. The collector *does* omit countries — `compact_tech_data_payload` packs
/// only `rows.iter().filter(|row| row.research.is_some())`
/// (`attach_log.rs:69190`), and `read_country_research` returns `None` whenever
/// the country's tech-status object is unreadable or fails its vtable and
/// back-link checks (`:80870`-`:80879`). Modelling each payload as the whole
/// state would emit tombstones for such a country and lose research state the
/// recorder still holds.
///
/// For a tag the payload *does* carry, everything is replaced: `per_tag` gives it
/// a fresh `in_progress_live: []` (`:13341`) before any active row is attached,
/// so its previous slots do not survive. [`fold_payload_at_hour`] implements
/// exactly that — upsert the country row, replace that tag's `(tag, *)` active
/// rows, leave every other tag alone.
///
/// **No other writer touches this accumulator.** `"research"` is written in
/// `world_fold.rs` at `world_fold.rs::fold_tech` (this fold) and at `world_fold.rs::project_frd_annotations`, which is d45's
/// projection-time `_r2_depth_in_progress_live` sidecar on a different key of the
/// same object; nothing removes a country or clears `research`.
#[derive(Clone, Debug, PartialEq)]
pub struct TechState {
    collections: Vec<KeyedState>,
}

/// **Written out rather than derived.** A derived `Default` builds an *empty*
/// collection list, and every accessor here indexes the two collections, so that
/// value would be a panic waiting for its first caller. `Default` is `new`.
impl Default for TechState {
    fn default() -> Self {
        Self::new()
    }
}

impl TechState {
    pub fn new() -> Self {
        Self {
            collections: vec![KeyedState::new(); COLLECTION_COUNT],
        }
    }

    /// The two collections in schema order — the shape `encode_baseline`,
    /// `encode_delta` and `decode_delta` take.
    pub fn as_slice(&self) -> &[KeyedState] {
        &self.collections
    }

    pub fn get(&self, index: usize) -> &KeyedState {
        &self.collections[index]
    }

    pub fn rows(&self) -> usize {
        self.collections.iter().map(KeyedState::len).sum()
    }
}

/// Fold one d13 DATA payload into `acc`, exactly as `fold_tech` folds it.
///
/// `frame_tick` is the recipe's fixed §6 input; d13 declares no derived field, so
/// it is taken and ignored, and — because a tick index is not the fold's
/// `game_hour_raw` — this entry point refuses a payload carrying an A29
/// provenance block. See [`record_from_active_row`] for the whole argument, and
/// [`fold_payload_at_hour`] for the entry that takes the fold's own clock.
pub fn fold_payload(
    schema: &Schema,
    acc: &mut TechState,
    payload: &[u8],
    frame_tick: u64,
) -> core::result::Result<(), String> {
    let _ = frame_tick;
    if payload_carries_a29_provenance(payload)? {
        return Err(
            "tech payload carries A29 provenance, whose same-hour authority gate needs the \
             frame's game_hour_raw; call fold_payload_at_hour"
                .into(),
        );
    }
    fold_payload_at_hour(schema, acc, payload, None)
}

/// [`fold_payload`] in the fold's own shape: `frame_game_hour` is `fold_tech`'s
/// `game_hour_raw` parameter (`world_fold.rs::fold_tech`), and `None` skips the A29
/// same-hour clause exactly as `game_hour_raw.is_some_and(..)` does at `world_fold.rs::fold_tech`.
///
/// **Upsert by country, never a wholesale replace.** See [`TechState`]. Each tag
/// the payload carries has its country record replaced and its whole `(tag, *)`
/// active-row set replaced; every other tag is left exactly as it was.
///
/// **Atomic.** Every row is decoded and checked before anything is written, so a
/// payload this function rejects leaves `acc` exactly as it found it — the
/// frame-level all-or-nothing `fold_tech` has by returning `Err` before it
/// publishes.
///
/// **Repeated country rows fold in payload order, last one winning.** The fold's
/// `per_tag.insert` (`world_fold.rs::fold_tech`) overwrites, and every active row is attached after
/// all country rows are read (`world_fold.rs::fold_tech`), so the surviving object collects them
/// all. A keyed set reproduces that exactly, and refusing the frame would refuse
/// one the shipping recorder folds.
///
/// **A repeated `(tag, slot_index)` is refused.** The fold `push`es onto an array
/// (`world_fold.rs::fold_tech`) rather than inserting into a map, so both rows survive there and a
/// keyed set can hold only one. Measuring such a frame would measure a state
/// nobody holds. The collector cannot produce one — it rejects a country whose
/// slot indices repeat (`attach_log.rs:80977`-`:80979`) — and none of the 364,487
/// measured rows does.
pub fn fold_payload_at_hour(
    schema: &Schema,
    acc: &mut TechState,
    payload: &[u8],
    frame_game_hour: Option<i32>,
) -> core::result::Result<(), String> {
    if schema.collections.len() != COLLECTION_COUNT {
        return Err(format!(
            "tech schema has {} collections, expected {COLLECTION_COUNT}",
            schema.collections.len()
        ));
    }
    // An accumulator that is not this domain's shape is a caller error, and every
    // line below indexes the two collections. Diagnosed, never a panic.
    if acc.collections.len() != COLLECTION_COUNT {
        return Err(format!(
            "tech accumulator has {} collections, expected {COLLECTION_COUNT}",
            acc.collections.len()
        ));
    }
    let countries_c = &schema.collections[COUNTRIES_INDEX];
    let active_c = &schema.collections[ACTIVE_RESEARCH_INDEX];
    let (header, tables) = split_payload(payload)?;

    // ---- Decode and check everything first. ----
    let mut countries = KeyedState::new();
    for row in tables.countries.chunks_exact(COUNTRY_ROW_LEN) {
        let key = key_from_country_row(row)?;
        let record = record_from_country_row(countries_c, row, 0)?;
        // Last writer wins, exactly like `per_tag.insert` (`:13342`).
        countries.insert(key, record);
    }

    let mut active = KeyedState::new();
    for span in active_row_spans(tables.active, header.active_count)? {
        let row = &tables.active[span];
        let key = key_from_active_row(row)?;
        // `world_fold.rs::fold_tech`.
        let tag = key.0[0];
        if !countries.contains_key(&Key(vec![tag])) {
            return Err(format!("tech active row references unknown tag {tag}"));
        }
        let record = record_from_active_row_at_hour(active_c, row, frame_game_hour)?;
        if active.insert(key, record).is_some() {
            return Err("tech payload repeats a (tag, slot_index) pair".into());
        }
    }

    // ---- Nothing below can fail, so `acc` is never left half-written. ----
    // Every carried tag's active rows go first, whole: `per_tag` hands each
    // carried country a fresh empty `in_progress_live` (`:13341`), so a slot the
    // payload does not repeat for that country is gone. A tag the payload does
    // not carry is not touched at all.
    acc.collections[ACTIVE_RESEARCH_INDEX]
        .retain(|key, _| !countries.contains_key(&Key(vec![key.first()])));
    acc.collections[ACTIVE_RESEARCH_INDEX].extend(active);
    acc.collections[COUNTRIES_INDEX].extend(countries);
    Ok(())
}

// --- Byte readers ------------------------------------------------------------

fn check_country_row_len(row: &[u8]) -> core::result::Result<(), String> {
    if row.len() != COUNTRY_ROW_LEN {
        return Err(format!(
            "tech country row length {} != {COUNTRY_ROW_LEN}",
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
    u32_at(b, off) as i32
}

fn u64_at(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().expect("8 bytes"))
}

fn i64_at(b: &[u8], off: usize) -> i64 {
    u64_at(b, off) as i64
}

#[cfg(test)]
mod tests;
