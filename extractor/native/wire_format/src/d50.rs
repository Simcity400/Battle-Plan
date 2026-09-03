//! d50 `naval_combat_members_live` — the ships and convoys inside each naval combat.
//!
//! Derived from `world_fold.rs:31017`-`:31107`
//! (`fold_naval_combat_members`), the D49 fold it depends on
// @world-fold-authority domain=50 functions=fold_naval_combat_members lines=31017-31107 sha256=4b0a3cce4ee4862f1519c4693203ca513e134edc70b423cc29c4bad6121f602b closure=60804ef8acf06dd81803fcf698f229401ae99b5e4e1f62fc57008f0f44511a35
//! (`:20040`-`:20128`, `fold_naval_combat_lifecycle`), the accumulator's JSON
//! projection (`:8839`-`:8887`), the lane codec that defines the payload grammar
//! (`extractor/native/lanes/naval_combat_live/src/domain49_50_naval_combat_live.rs`,
//! header `:1789`-`:1830`, rows `:1741`-`:1786`, row validation `:1967`-`:2214`),
//! and the collector read that produces the presence flags
//! (`extractor/native/collector/src/attach_log.rs:40095`-`:40177`,
//! `:40194`-`:40243`, `:40799`-`:40814`, `:40930`-`:40962`).
//! The fold, lane codec, and collector read are the field-level basis.
//!
//! # Payload grammar
//!
//! A 28-byte header (`PAYLOAD_HEADER_LEN`, lane `:25`), then `row_count` fixed
//! 88-byte rows (`MEMBER_ROW_LEN`, lane `:27`), then a string table of exactly
//! one empty entry. The decoder requires the walk to end exactly at the payload
//! end (lane `:1862`-`:1866`), so trailing bytes are a rejected payload.
//!
//! Header (`decode_payload_header`, lane `:1789`-`:1830`):
//!
//! | offset | width | field | check |
//! |---|---|---|---|
//! | `+0`  | u16 | `schema_version` | must be `3` (lane `:1801`) |
//! | `+2`  | u16 | `row_len`        | must be `88` (lane `:1806`) |
//! | `+4`  | u64 | `elapsed_wall_ms` | transport, unchecked |
//! | `+12` | u32 | `status_flags`   | — |
//! | `+16` | u32 | `row_count`      | `<= 100_000` (lane `:1817`) |
//! | `+20` | u32 | `string_count`   | must be `1` (lane `:1820`) |
//! | `+24` | u32 | `reserved`       | must be `0` (lane `:1811`) |
//!
//! Row (`decode_member_payload`, lane `:1762`-`:1783`):
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+0`  | u32 | `combat_id`                 | raw | none (key; zero rejected, lane `:1979`) |
//! | `+4`  | u64 | `generation`                | raw | none (key) |
//! | `+12` | u32 | `flags`                     | —   | unknown bits rejected (lane `:1972`) |
//! | `+16` | u32 | `side`                      | raw | none (key; must be `1` or `2`, lane `:1985`) |
//! | `+20` | u32 | `group_index`               | raw | none (`<= 64`, lane `:2003`) |
//! | `+24` | u32 | `member_index`              | raw | none (`<= 1000`, lane `:2009`) |
//! | `+28` | u32 | `unit_type_tag`             | raw | none (`51` for a ship, `0` for a convoy, lane `:2021`-`:2032`) |
//! | `+32` | u32 | `ship_instance_id`          | raw | `1<<1` |
//! | `+36` | u32 | `owner_tag_db_id`           | raw | `1<<2` |
//! | `+40` | u32 | `designed_max_strength_raw` | **raw, see below** | `1<<3` |
//! | `+44` | u32 | `battle_slot_index_raw`     | raw | none (key; `<= 4096`, lane `:2015`) |
//! | `+48` | u32 | `state_bitmask`             | enum | `1<<4` |
//! | `+52` | i32 | `engagement_weight_raw`     | **raw, see below** | `1<<5` |
//! | `+56` | u32 | `task_force_id_space`       | raw | `1<<8` (v3; was a must-be-zero reserved in v2) |
//! | `+60` | u32 | `task_force_id`             | raw | `1<<8` |
//! | `+64` | u32 | `fleet_id`                  | raw | `1<<9` |
//! | `+68` | u32 | `strength_raw`              | /100000 | `1<<10` |
//! | `+72` | u32 | `organisation_raw`          | /100000 | `1<<11` |
//! | `+76` | u32 | `sunk_convoys`              | raw (a count) | `1<<13` |
//! | `+80` | u32 | `experience_raw`            | /100000 | `1<<12` |
//! | `+84` | u32 | `reserved_tail`             | —   | must be `0` (lane `:1997`) |
//!
//! **`sunk_convoys` sits at `+76` and `experience_raw` at `+80`** — the row order
//! is not the flag order, which is exactly the transposition the per-offset test
//! exists to catch.
//!
//! Two flag bits are not gates: `1<<0` `DELETED` is the tombstone and `1<<6`
//! `CSHIP_JOIN_VALIDATED` / `1<<7` `IS_CONVOY` are witnesses (see below).
//!
//! # Shape: pure cumulative delta, one keyed set
//!
//! The fold never clears: every payload is a patch, there is no baseline record
//! and no mode word — the same grammar D49 documents in its own fold
//! (`world_fold.rs::fold_naval_combat_lifecycle`: *"the collector emits every active row on the
//! first frame (empty previous baseline) and only changes thereafter, so no
//! separate baseline record exists to gate on"*). That is the recipe's shape 3,
//! so **`changed_rows == row_count` is not an invariant** — the header carries no
//! `changed_rows` word at all. Deletes ride in `DELETED`.
//!
//! The key is the fold's own: `(combat_id, generation, side.wire_id(),
//! battle_slot_index_raw)` (`world_fold.rs:20153`-`:20158`, and the accumulator's
//! per-combat map key `(side.wire_id(), battle_slot_index_raw)` at `:20200`
//! under the combat's own `(combat_id, generation)` key at `world_fold.rs::fold_naval_combat_members`).
//!
//! # Three whole-hour refusals, validated before any mutation
//!
//! `fold_naval_combat_members` validates the **entire** delta before it touches
//! the accumulator, and three conditions make the whole hour unknown while
//! retaining the accumulator ([`FoldOutcome`]):
//!
//! 1. **Orphan member** — a live row whose `(combat_id, generation)` is not in
//!    D49's `naval_combats` accumulator (`world_fold.rs::fold_naval_combat_members`). Delete rows are
//!    exempt: a delete for an already-removed combat is a valid idempotent no-op.
//! 2. **Duplicate member identity** — the same key twice inside one payload
//!    (`world_fold.rs::fold_naval_combat_members`), live or deleted.
//! 3. **Retired-member resurrection** — a live row whose key is in the
//!    **persistent** `naval_retired_member_keys` set (`world_fold.rs::fold_naval_combat_members`). That set
//!    is never cleared anywhere in `world_fold.rs`: it is written only by this
//!    fold, one insert per delete row (`world_fold.rs::fold_naval_combat_members`), so it is fold state a replay
//!    harness must carry across hours, not payload state. [`State`] carries it.
//!
//! None of the three is a stream rejection: the accumulator survives and the next
//! hour folds normally. A payload that fails the lane's own decode or row
//! validation *is* a stream rejection, because `decode_member_rows` returns `Err`
//! and the fold propagates it (`world_fold.rs::fold_naval_combat_members`).
//!
//! # D49 is a hard parent, and it is MODELLED
//!
//! Two of this domain's semantics read D49's accumulator, and one D49 path writes
//! this domain's own state:
//!
//! * the orphan refusal reads D49's key set;
//! * `self.naval_combats.remove(&key)` on a D49 delete row
//!   (`world_fold.rs:20098`) drops the whole `NavalCombatFoldState`, **including
//!   its `members` map** — so a D49 tombstone silently removes every member of
//!   that combat. That is a second writer of this domain's accumulator, and it is
//!   not inert on any capture with a combat that ends.
//!
//! The recipe's three treatments are model / prove inert / assert inert. Neither
//! "inert" option is available for a writer the domain's own refusal reads, so
//! this is the **model it** case, and the model lives here rather than in the
//! measurement bin because it is part of d50's frame-level fold semantics.
//!
//! [`State::fold_lifecycle_payload`] is a **key-column mirror of D49, not a d50
//! port of D49**: it reproduces D49's structural validation and its three
//! non-folding paths, and keeps `(combat_id, generation)` and nothing else. No
//! D49 schema, no D49 field ids, no D49 record is produced anywhere in this
//! crate. When d49 is ported, this mirror must be replaced by that port's own
//! `State` — recorded as a retrofit item in the deliverable.
//!
//! # Four states
//!
//! **`is_convoy` decides absent-vs-unknown for every CShip-derived field**, and
//! it is the recipe's d28 kind-dependent gate with `is_convoy` in the role of
//! `kind`. The collector reads the member's packed handle at `CFEXMember+0x08`
//! (`attach_log.rs:40100`-`:40120`): a **zero** handle is a convoy — no CShip
//! identity exists at all — and any other handle must be a valid `(51, instance)`
//! pair or the frame is torn. So:
//!
//! * a convoy row can carry no `ship_instance_id`, no CShip join and none of the
//!   seven join-derived fields (six flags: since v3 `task_force_id` and
//!   `task_force_id_space` share `TASK_FORCE_PRESENT`, because half a reference
//!   is not one), and the lane *enforces* that (`:2046`-`:2056`,
//!   `:2098`-`:2107`). The concepts do not apply → **absent**.
//! * a non-convoy row always carries `ship_instance_id` — the lane rejects one
//!   that does not (`:2058`-`:2062`) — so that field is never empty on a ship.
//!
//! **`CSHIP_JOIN_VALIDATED` (`1<<6`) is the sibling status word for the seven
//! join-derived fields**, exactly the d28 `leader_status` pattern: the fields have
//! their own presence bits, but what a clear bit *means* is decided by the join.
//! `join = member.ship_instance_id.and_then(|ship| joins.get(&ship))` and every
//! one of `task_force_id`, `fleet_id`, `strength_raw`, `organisation_raw`,
//! `experience_raw`, `sunk_convoys` is `join.map(...)` / `join.and_then(...)`
//! (`attach_log.rs:40946`-`:40958`). A non-convoy row reaches `join == None` on
//! exactly one path: the ship's `state_bitmask` is `FEX_STATE_SUNK`, which
//! **excludes it from the set of ships the OOB join is even attempted for**
//! (`attach_log.rs:40801`-`:40813` — *"must not make the frame fail merely because
//! that handle has already disappeared from the live naval OOB"*). For every other
//! ship a missing join tears the whole frame (`:40239`-`:40241`,
//! `naval_member_cship_join_missing`). A sunk ship's task force and hull telemetry
//! still exist as concepts — the read was skipped, not answered — which is record-state contract's
//! **unknown**, "applicable, not captured". The four telemetry fields have a
//! second unknown path: the join succeeded but the value failed its range filter
//! (`attach_log.rs:40226`-`:40233`), which is a read rejection, unknown again.
//!
//! So the seven join-derived fields are `optional` (convoy) **and**
//! `unknown_capable` (join not answered), and never null. The lane's own
//! validation makes the two cases disjoint on every row it accepts, so the mapper
//! never has to guess which one a clear bit is.
//!
//! **This corrects the survey:** the d50 entry read *"`MEMBER_CSHIP_JOIN_VALIDATED` is the sibling-status word for
//! `ship_instance_id`. A clear join bit means the CShip read failed → **Unknown**,
//! not Null."* `ship_instance_id` is the one field the join bit does **not**
//! govern: the lane requires it present on every non-convoy row and forbids it on
//! every convoy row (`:2046`-`:2062`), so it is value-or-absent and can never be
//! unknown. The join bit governs the seven fields listed above. Measured on
//! `runs/batch3/deepwar_a`: 15,094 member rows, `SHIP_INSTANCE_PRESENT` set on
//! exactly the 14,786 non-convoy rows and clear on exactly the 308 convoy rows,
//! and the join bit clear on 14 non-convoy rows — all 14 with
//! `state_bitmask == FEX_STATE_SUNK`.
//!
//! **The four member-owned fields are unknown-or-value, and nothing else.**
//! `owner_tag_db_id`, `designed_max_strength_raw`, `state_bitmask` and
//! `engagement_weight_raw` are read directly off the `CFEXMember` and the
//! collector states the rule in the code: *"Ancillary fields
//! (owner/max-strength/state/weight) fail closed as badged-absent Options, never
//! coerced to 0"* (`attach_log.rs:40143`-`:40144`). Each is `read_*(…)` plus a
//! range filter, so `None` means unreadable memory or an out-of-range word — record-state contract's
//! unknown, word for word. They apply to a convoy exactly as much as to a ship
//! (all four are member-owned, not CShip-owned), so absent is wrong; nothing in
//! either authority assigns "the relation points at nothing" to any of them, so
//! null is wrong. Measured: `designed_max_strength_raw` is present **and zero** on
//! all 308 convoy rows — a real record-state contract zero, not an empty state.
//!
//! `owner_tag_db_id` is the one tie-break here: its filter is
//! `*owner > 0 && *owner <= MAX_OWNER_TAG_DB_ID` (`attach_log.rs:40147`), so a
//! genuine zero owner and an unreadable owner both clear the gate — the recipe's
//! value-derived-gate ambiguity (d32's `air_base_id`, d24). Undecidable → unknown,
//! and recorded as an open question.
//!
//! # Two witnesses that are not fields
//!
//! * **`IS_CONVOY` (`1<<7`) is a total function of `unit_type_tag`.** The lane
//!   rejects a non-convoy row whose tag is not `51` and a convoy row whose tag is
//!   not `0` (`:2021`-`:2032`), so over every row the fold accepts,
//!   `is_convoy <=> unit_type_tag == 0`. The recipe's rule for a value the fold
//!   hard-enforces as a function of carried state: enforce it in the mapper, test
//!   the identity, and leave it off the wire. [`UNIT_TYPE_TAG`](field::UNIT_TYPE_TAG)
//!   is carried; a reader reconstructs `is_convoy` as `unit_type_tag == 0`.
//! * **`CSHIP_JOIN_VALIDATED` is NOT derivable and IS carried.** On the collector's
//!   own rows it happens to coincide with `TASK_FORCE_PRESENT` (measured: 0
//!   disagreements in 15,094 rows), but the lane accepts a row with the join bit
//!   set and every derived bit clear — it only forbids the converse
//!   (`:2064`-`:2069`, `:2087`-`:2096`) — so the biconditional is a property of
//!   this producer, not of the grammar. The fold stores the bit and the snapshot
//!   publishes it (`world_fold.rs::build_naval_combats`), so it rides as a `Boolean`.
//!
//! # Scales — from the code that folds each field, not from the names
//!
//! The fold's member projection divides `strength_raw` and `organisation_raw` by
//! `FIXED_SCALE` = 100000 (`world_fold.rs:8876`-`:8877`). `experience_raw` is not
//! in the member projection at all, but it is the *same word from the same
//! producer* as d29's ship experience — both come from
//! `sample_compact_task_force_ships`' `row.experience_raw`
//! (`attach_log.rs:40230`, `:48008`) — and the fold divides that by `FIXED_SCALE`
//! (`world_fold.rs:8631`). All three are `Continuous` with `Quant::Fixed(5)`: an
//! integer over a `10^5` denominator, which is the source's own grid rather than a
//! precision budget, and which reproduces the raw integer downstream (the recipe's
//! d24 rule).
//!
//! **`designed_max_strength_raw` and `engagement_weight_raw` are carried RAW**,
//! as integers, because **no fold divides them**: they appear in no snapshot
//! projection anywhere in `world_fold.rs`, and the collector emits them raw in its
//! own JSON sample (`attach_log.rs:20588`, `:20591`). The recipe's rule is that a
//! field's scale comes from the code that folds *that field*, and for these two
//! there is none — so the schema makes no scale claim and carries the exact source
//! word, which is lossless whatever the scale turns out to be (d20's
//! `move_progress`, emitted raw, is the precedent). The empirical shape of both
//! fields is consistent with `x100000` and is recorded as an open question:
//! `engagement_weight_raw` takes exactly five values on the whole capture — 0,
//! 100000, 200000, 300000, 400000 — and `designed_max_strength_raw` takes 14
//! values, every one a multiple of 1000000 (320.0 … 2800.0 at `x100000`, the shape
//! of ship hull points). A `Fixed(5)` declaration would also be lossless, so
//! nothing is lost either way; what is avoided is asserting a grid no authority
//! states.
//!
//! `sunk_convoys` is a count with no denominator anywhere and is `Monotone`.
//!
//! # No derived field (derived-field contract)
//!
//! Nothing here changes deterministically with the clock: hull strength moves with
//! damage and repair, organisation with combat, a member's slot on reordering. The
//! mapper still takes `frame_tick` so every domain's boundary has one shape.

use std::collections::{BTreeMap, BTreeSet};

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Quant, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 50;
/// One row per `(combat, generation, side, battle slot)` membership.
pub const MEMBERS: u32 = 1;

/// Field ids are stable and append-only; schema-identity contract forbids reuse for a different field.
pub mod field {
    pub const COMBAT_ID: u32 = 1;
    pub const GENERATION: u32 = 2;
    pub const SIDE: u32 = 3;
    pub const BATTLE_SLOT_INDEX: u32 = 4;
    pub const GROUP_INDEX: u32 = 5;
    pub const MEMBER_INDEX: u32 = 6;
    pub const UNIT_TYPE_TAG: u32 = 7;
    pub const SHIP_INSTANCE_ID: u32 = 8;
    pub const CSHIP_JOIN_VALIDATED: u32 = 9;
    pub const OWNER_TAG_DB_ID: u32 = 10;
    pub const DESIGNED_MAX_STRENGTH_RAW: u32 = 11;
    pub const STATE: u32 = 12;
    pub const ENGAGEMENT_WEIGHT_RAW: u32 = 13;
    pub const TASK_FORCE_ID: u32 = 14;
    pub const FLEET_ID: u32 = 15;
    pub const STRENGTH: u32 = 16;
    pub const ORGANISATION: u32 = 17;
    pub const EXPERIENCE: u32 = 18;
    pub const SUNK_CONVOYS: u32 = 19;
    /// The clausewitz registry that minted [`TASK_FORCE_ID`] (v3). Appended,
    /// never reusing an id.
    pub const TASK_FORCE_ID_SPACE: u32 = 20;
}

// --- Payload grammar, from the lane codec ------------------------------------

/// `PAYLOAD_HEADER_LEN`, lane `:25`.
pub const HEADER_LEN: usize = 28;
/// `MEMBER_ROW_LEN`, lane `:27`.
pub const ROW_LEN: usize = 88;
/// `LIFECYCLE_ROW_LEN`, lane `:26` — the D49 parent mirror's row length.
pub const LIFECYCLE_ROW_LEN: usize = 72;
/// `SCHEMA_VERSION`, lane `:24`.
///
/// **v3 carries the task-force id space** (owner ruling 2026-08-27). The engine
/// mints task forces from two clausewitz registries -- the fleet-id minter
/// (type 61) and the `CShip` registry (type 51) -- and their numbers collide, so
/// a member's `task_force_id` alone stopped resolving once d28 began publishing
/// both. v3 spends the member row's `+56` reserved word, which v2 required to be
/// zero, so the row length is unchanged and a v2 payload is refused by version.
pub const SCHEMA_VERSION: u16 = 3;
/// `decode_payload_header`, lane `:1817`.
pub const MAX_ROW_COUNT: u32 = 100_000;

pub const ROW_DELETED: u32 = 1 << 0;
pub const SHIP_INSTANCE_PRESENT: u32 = 1 << 1;
pub const OWNER_PRESENT: u32 = 1 << 2;
pub const DESIGNED_MAX_STRENGTH_PRESENT: u32 = 1 << 3;
pub const STATE_BITMASK_PRESENT: u32 = 1 << 4;
pub const ENGAGEMENT_WEIGHT_PRESENT: u32 = 1 << 5;
pub const CSHIP_JOIN_VALIDATED: u32 = 1 << 6;
pub const IS_CONVOY: u32 = 1 << 7;
pub const TASK_FORCE_PRESENT: u32 = 1 << 8;
pub const FLEET_PRESENT: u32 = 1 << 9;
pub const STRENGTH_PRESENT: u32 = 1 << 10;
pub const ORGANISATION_PRESENT: u32 = 1 << 11;
pub const EXPERIENCE_PRESENT: u32 = 1 << 12;
pub const SUNK_CONVOYS_PRESENT: u32 = 1 << 13;

/// `MEMBER_KNOWN_FLAGS`, lane `:71`-`:85`. Any other bit means the row is not this
/// grammar (lane `:1972`).
pub const KNOWN_FLAGS: u32 = ROW_DELETED
    | SHIP_INSTANCE_PRESENT
    | OWNER_PRESENT
    | DESIGNED_MAX_STRENGTH_PRESENT
    | STATE_BITMASK_PRESENT
    | ENGAGEMENT_WEIGHT_PRESENT
    | CSHIP_JOIN_VALIDATED
    | IS_CONVOY
    | TASK_FORCE_PRESENT
    | FLEET_PRESENT
    | STRENGTH_PRESENT
    | ORGANISATION_PRESENT
    | EXPERIENCE_PRESENT
    | SUNK_CONVOYS_PRESENT;

/// The six GATES the CShip join produces, lane `:2072`-`:2078`, covering seven
/// fields: since v3 `TASK_FORCE_PRESENT` governs `task_force_id` and
/// `task_force_id_space` together, because a number with no registry cannot name
/// one of two task forces that share it. A row without a live join may carry
/// none of them.
pub const CSHIP_DERIVED_FLAGS: u32 = TASK_FORCE_PRESENT
    | FLEET_PRESENT
    | STRENGTH_PRESENT
    | ORGANISATION_PRESENT
    | EXPERIENCE_PRESENT
    | SUNK_CONVOYS_PRESENT;

/// `FIXED_SCALE`, `world_fold.rs::FIXED_SCALE`, used by the member projection at
/// `:8876`-`:8877`.
pub const FIXED_SCALE: f64 = 100_000.0;
/// `FIXED_SCALE` is `10^5`, so `Quant::Fixed(5)` is exactly the source's own grid.
pub const SCALE_DIGITS: u8 = 5;

pub const MAX_COMBAT_ID: u32 = 0x7fff_ffff;
pub const MAX_SHIP_INSTANCE_ID: u32 = 4_000_000;
/// `compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID` (lane `:88`),
/// mirrored rather than imported because d50 is not admitted to the production-
/// codec dependency surface. The measurement bin asserts the mirror — the same
/// arrangement d32 uses for its `DELTA_DATA` bit — and
/// `tests/wire_format_suite/producer_consumer_admission_agreement.rs` pins it to
/// `portable_admission::MAX_COMPACT_OWNER_TAG_DB_ID`.
///
/// Was `2_048`, a guess at the vanilla-plus-DLC country count (vanilla defines
/// 439 tags). ENGINE-STRUCTURAL now: a country-database id is admissible iff
/// `id < *(int32*)(db + 0x0C)` and that size is a signed `int`, so `i32::MAX - 1`
/// is the widest id any loadable database can hold.
pub const MAX_OWNER_TAG_DB_ID: u32 = (i32::MAX as u32) - 1;
pub const MAX_GROUP_INDEX: u32 = 64;
pub const MAX_MEMBER_INDEX: u32 = 1_000;
pub const MAX_BATTLE_SLOT_INDEX: u32 = 4_096;
pub const MAX_DESIGNED_STRENGTH_RAW: u32 = 1_000_000_000;
pub const MAX_FIXED_POINT_RAW: u32 = 1_000_000_000;
pub const MAX_SUNK_CONVOYS: u32 = 1_000_000;
pub const MAX_OOB_ID: u32 = 4_000_000;
/// `CSHIP_TYPE_TAG`, lane `:99`: the engine type word a live ship handle carries.
pub const CSHIP_TYPE_TAG: u32 = 51;

pub const SIDE_ATTACKER: u32 = 1;
pub const SIDE_DEFENDER: u32 = 2;

/// The exact `CFEXMember+0xE0` enum domain (lane `:101`-`:111`). The lane's own
/// comment: *"The legacy wire field and presence flag retain their
/// `state_bitmask` spelling for schema compatibility. The value is an enum, not a
/// bit set."* `fex_state_name` (lane `:113`) rejects everything else, so the
/// domain is exactly these seven raw words.
///
/// The wire carries the **dense code**, the index into this table, because
/// `Kind::Enum` rejects a code at or above its cardinality (schema-kind contract) and the raw words
/// are sparse (`0x20` would need `enum_card(33)`). The map is a bijection over the
/// accepted domain, so nothing is lost; [`state_code`] and [`state_raw`] are the
/// two directions and the tests pin both.
pub const FEX_STATES: [u32; 7] = [
    0x00, // FEX_STATE_UNINITIALIZED
    0x01, // FEX_STATE_WAITING
    0x02, // FEX_STATE_FIGHTING
    0x04, // FEX_STATE_ON_THE_WAY
    0x08, // FEX_STATE_ESCAPING
    0x10, // FEX_STATE_RUN_AWAY
    0x20, // FEX_STATE_SUNK
];
/// `FEX_STATE_SUNK`, lane `:111`. The terminal state, and the one that takes a
/// ship out of the CShip join set (`attach_log.rs:40804`-`:40808`).
pub const FEX_STATE_SUNK: u32 = 0x20;

/// The dense wire code for a raw `CFEXMember+0xE0` word, or `None` if the word is
/// not one of the seven the lane accepts.
pub fn state_code(raw: u32) -> Option<u32> {
    FEX_STATES.iter().position(|s| *s == raw).map(|i| i as u32)
}

/// The raw `CFEXMember+0xE0` word for a dense wire code.
pub fn state_raw(code: u32) -> Option<u32> {
    FEX_STATES.get(code as usize).copied()
}

pub fn schema() -> Schema {
    use field as f;
    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![Collection {
            collection_id: MEMBERS,
            // Members persist across hours under a stable identity and the frame
            // grammar is pure cumulative upsert / DELETED-removes
            // (`world_fold.rs::fold_naval_combat_members`) — a keyed set's grammar, and only a
            // keyed set has deltas (collection-shape contract).
            shape: Shape::KeyedSet,
            fields: vec![
                // The fold's key, `world_fold.rs::fold_naval_combat_members`.
                //
                // `combat_id` is a runtime engine instance identity created when a
                // battle starts; no manifest resolves it, so `Unsigned` (d20's
                // `div_id`, d32's `wing_id`). `generation` is the lane's own
                // body-reuse discriminator, stamped when a combat id appears on a
                // new body (`attach_log.rs:40899`-`:40906`) — purely runtime.
                Field::new(f::COMBAT_ID, Kind::Unsigned).key(0),
                Field::new(f::GENERATION, Kind::Unsigned).key(1),
                // `side.wire_id()` — 1 attacker, 2 defender (lane `:130`-`:135`).
                // An engine discriminator, and `Kind::Enum` is not a legal key
                // kind (`schema.rs:29`-`:31`), so it rides as the fold's own
                // integer.
                Field::new(f::SIDE, Kind::Unsigned).key(2),
                // The member's slot in the battle line. The fold's own identity
                // component, not a generated ordinal: the collector reads it from
                // `CFEXMember+0x120` and fails the frame closed rather than
                // coercing it (`attach_log.rs:40137`-`:40142`).
                Field::new(f::BATTLE_SLOT_INDEX, Kind::Unsigned).key(3),
                // Position of the member inside the combat's group array. Carried
                // because the fold stores it and the snapshot publishes it
                // (`world_fold.rs::build_naval_combats`); not a key, because the fold's
                // identity is the battle slot.
                Field::new(f::GROUP_INDEX, Kind::Unsigned),
                Field::new(f::MEMBER_INDEX, Kind::Unsigned),
                // The engine type word of the member's packed handle: `51` for a
                // ship, `0` for a convoy (lane `:2021`-`:2032`). An engine
                // constant, not content — no manifest resolves it — so `Unsigned`,
                // exactly as d20 classes `div_type`. This is also the carrier of
                // the `IS_CONVOY` witness: `is_convoy <=> unit_type_tag == 0`.
                Field::new(f::UNIT_TYPE_TAG, Kind::Unsigned),
                // A runtime CShip instance id (`Unsigned`, not `ContentId`).
                // `optional` and nothing else: a convoy has no ship identity at
                // all, and the lane requires every non-convoy row to carry one.
                Field::new(f::SHIP_INSTANCE_ID, Kind::Unsigned).optional(),
                // The join witness. Always present — it is a flag bit, not a
                // gated value — so no capability.
                Field::new(f::CSHIP_JOIN_VALIDATED, Kind::Boolean),
                // A country tag db id: content-defined (content-identity contract), so `ContentId`, the
                // same way d63 keys its countries and d20 classes its `owner`.
                // `unknown_capable` and nothing else — see the module docs; the
                // Null-vs-Unknown tie-break is an open question.
                Field::new(f::OWNER_TAG_DB_ID, Kind::ContentId).unknown_capable(),
                // Raw, unscaled: no fold divides it. See the module docs.
                Field::new(f::DESIGNED_MAX_STRENGTH_RAW, Kind::Unsigned).unknown_capable(),
                // The `CFEXMember+0xE0` enum, carried as a dense code over
                // [`FEX_STATES`].
                Field::new(f::STATE, Kind::Enum)
                    .enum_card(FEX_STATES.len() as u32)
                    .unknown_capable(),
                // An i32 cell (lane `:1777`), raw and unscaled: no fold divides it.
                Field::new(f::ENGAGEMENT_WEIGHT_RAW, Kind::Signed).unknown_capable(),
                // The SEVEN CShip-join-derived fields -- six, plus
                // `task_force_id_space` since v3, which shares the task-force
                // id's gate because half a reference is not one: `optional` on a
                // convoy (the concept does not apply), `unknown_capable` when
                // the join was not answered. Never null.
                Field::new(f::TASK_FORCE_ID, Kind::Unsigned)
                    .optional()
                    .unknown_capable(),
                // The other half of that reference (v3): which clausewitz
                // registry minted the number. Two do, and their numbers
                // collide, so the number alone names two different task forces.
                // Same gate as the id, hence the same capabilities.
                Field::new(f::TASK_FORCE_ID_SPACE, Kind::Unsigned)
                    .optional()
                    .unknown_capable(),
                Field::new(f::FLEET_ID, Kind::Unsigned)
                    .optional()
                    .unknown_capable(),
                // `Continuous` because the fold's own projection is
                // `raw / FIXED_SCALE` (`world_fold.rs::build_naval_combats`); `Fixed(5)`
                // because the source is an integer over a `10^5` denominator, so
                // the quantizer reproduces the source word bit for bit rather than
                // spending a precision budget (the recipe's d24 rule, proved in
                // the tests over the producer's whole admitted range).
                Field::new(f::STRENGTH, Kind::Continuous)
                    .quant(Quant::Fixed(SCALE_DIGITS))
                    .optional()
                    .unknown_capable(),
                Field::new(f::ORGANISATION, Kind::Continuous)
                    .quant(Quant::Fixed(SCALE_DIGITS))
                    .optional()
                    .unknown_capable(),
                // Same producer word as d29's ship experience, which the fold
                // divides by `FIXED_SCALE` (`world_fold.rs:8631`).
                Field::new(f::EXPERIENCE, Kind::Continuous)
                    .quant(Quant::Fixed(SCALE_DIGITS))
                    .optional()
                    .unknown_capable(),
                // A cumulative per-ship kill count. `Monotone` is an encoding hint
                // and never a validity constraint (schema-kind contract).
                Field::new(f::SUNK_CONVOYS, Kind::Monotone)
                    .optional()
                    .unknown_capable(),
            ],
        }],
        event_part: None,
    }
}

// --- The boundary mapping ----------------------------------------------------

/// The 28-byte payload header, shared by the D50 member payload and the D49
/// lifecycle payload the parent mirror reads (lane `:1789`-`:1830`).
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

/// One parsed 88-byte member row, with each gated field resolved to its `Option`
/// exactly as `decode_member_rows` resolves it (lane `:535`-`:584`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemberRow {
    pub combat_id: u32,
    pub generation: u64,
    pub flags: u32,
    /// `DELETED`: remove this key. Never a record; a wire tombstone.
    pub deleted: bool,
    pub side_wire_id: u32,
    pub group_index: u32,
    pub member_index: u32,
    pub unit_type_tag: u32,
    /// `IS_CONVOY`, which the lane forces equal to `unit_type_tag == 0`.
    pub is_convoy: bool,
    /// `CSHIP_JOIN_VALIDATED`, the sibling status word for the six join fields.
    pub cship_join_validated: bool,
    pub ship_instance_id: Option<u32>,
    pub owner_tag_db_id: Option<u32>,
    pub designed_max_strength_raw: Option<u32>,
    pub battle_slot_index_raw: u32,
    pub state_bitmask: Option<u32>,
    pub engagement_weight_raw: Option<i32>,
    pub task_force_id: Option<u32>,
    /// The clausewitz registry that minted `task_force_id` (v3), and the other
    /// half of the reference into d28's task-force rows. `Some` exactly when
    /// `task_force_id` is: one flag gates both words.
    pub task_force_id_space: Option<u32>,
    pub fleet_id: Option<u32>,
    pub strength_raw: Option<u32>,
    pub organisation_raw: Option<u32>,
    pub experience_raw: Option<u32>,
    pub sunk_convoys: Option<u32>,
}

impl MemberRow {
    /// This row's key, `(combat_id, generation, side, battle_slot_index_raw)`.
    pub fn key(&self) -> Key {
        member_key(
            self.combat_id,
            self.generation,
            self.side_wire_id,
            self.battle_slot_index_raw,
        )
    }
}

/// One parsed 72-byte D49 lifecycle row, reduced to what
/// `fold_naval_combat_lifecycle` needs to decide whether it folds and what it
/// then does to its key set. **Not a d49 port** — no value field survives here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LifecycleKeyRow {
    pub combat_id: u32,
    pub generation: u64,
    pub deleted: bool,
    /// `world_fold.rs::fold_naval_combat_lifecycle`: a live row missing either masks the hour.
    pub has_location: bool,
    pub has_region: bool,
    /// `world_fold.rs::fold_naval_combat_lifecycle`: a live row missing any of the four masks
    /// the hour.
    pub has_all_side_scalars: bool,
}

/// The `MEMBERS` key of one membership: the fold's own tuple
/// (`world_fold.rs::fold_naval_combat_members`).
pub fn member_key(combat_id: u32, generation: u64, side_wire_id: u32, battle_slot: u32) -> Key {
    key(&[
        combat_id as u64,
        generation,
        side_wire_id as u64,
        battle_slot as u64,
    ])
}

/// Read the 28-byte payload header, applying every check `decode_payload_header`
/// applies (lane `:1789`-`:1830`) for the given row length.
///
/// This is reachable without paying for the body on purpose: a caller that needs
/// the status word in order to classify a record — the failure-status exclusion is
/// `compact_contract`'s predicate, which this crate must not copy — reads it here.
/// The fold's own order is decode-then-status, so a caller replaying the fold
/// exactly runs [`split_payload`] first and classifies afterwards; both orders are
/// available because the header read is separable.
///
/// Fail-closed and total: no input of any length panics.
pub fn peek_header(
    payload: &[u8],
    expected_row_len: usize,
) -> core::result::Result<PayloadHeader, String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "naval_combat payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    let header = PayloadHeader {
        schema_version: u16_at(payload, 0),
        row_len: u16_at(payload, 2),
        elapsed_wall_ms: u64_at(payload, 4),
        status_flags: u32_at(payload, 12),
        row_count: u32_at(payload, 16),
        string_count: u32_at(payload, 20),
        reserved: u32_at(payload, 24),
    };
    // lane `:1799`-`:1803`.
    if header.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "naval_combat unsupported schema version {}",
            header.schema_version
        ));
    }
    // lane `:1804`-`:1808`.
    if header.row_len as usize != expected_row_len {
        return Err(format!(
            "naval_combat row length {} != expected {expected_row_len}",
            header.row_len
        ));
    }
    // lane `:1809`-`:1813`.
    if header.reserved != 0 {
        return Err("naval_combat payload reserved field is nonzero".into());
    }
    // lane `:1814`-`:1816`.
    if header.row_count > MAX_ROW_COUNT {
        return Err(format!(
            "naval_combat row count {} exceeds the {MAX_ROW_COUNT} limit",
            header.row_count
        ));
    }
    // lane `:1818`-`:1822`.
    if header.string_count != 1 {
        return Err(format!(
            "naval_combat lane expects exactly one empty string table entry, not {}",
            header.string_count
        ));
    }
    Ok(header)
}

/// Split a d50 DATA payload into its header and its rows slice, applying every
/// structural and per-row check the recorder applies before folding.
///
/// This is the whole of production's status-independent D50 payload authority:
/// `decode_compact_naval_combat_members_live_data_payload`
/// (`domain_data_authority.rs:8709`-`:8722`) calls
/// `naval4950::decode_members_data_payload` (lane `:625`-`:702`), which is
/// `decode_member_payload` plus `validate_decoded_member_row` on every row, run
/// whatever the status word says. `fold_naval_combat_members` reaches the same
/// rules through `decode_member_rows` (lane `:535`) and turns any of them into a
/// fold `Err` (`world_fold.rs::fold_naval_combat_members`) — a stream rejection, not an
/// excluded hour.
///
/// It lives here rather than in each caller so the payload grammar has exactly one
/// implementation: a measurement harness that re-derives it is a harness that can
/// drift from the recorder without anything failing. Fail-closed and total.
pub fn split_payload(payload: &[u8]) -> core::result::Result<(PayloadHeader, &[u8]), String> {
    let header = peek_header(payload, ROW_LEN)?;
    let rows = body_slice(payload, header, ROW_LEN)?;
    for (index, row) in rows.chunks_exact(ROW_LEN).enumerate() {
        parse_row(row).map_err(|e| format!("naval_combat_members row {index}: {e}"))?;
    }
    Ok((header, rows))
}

/// The rows slice of a payload whose header has already been read, with the
/// string table walked exactly as `validate_string_table` walks it (lane
/// `:1832`-`:1864`): `string_count` length-prefixed entries, index 0 empty, and
/// the walk must end exactly at the payload end.
fn body_slice<'a>(
    payload: &'a [u8],
    header: PayloadHeader,
    row_len: usize,
) -> core::result::Result<&'a [u8], String> {
    // Checked arithmetic: `row_count` is a word from a file on disk and the
    // product overflows `usize` for a large enough one.
    let rows_len = (header.row_count as usize)
        .checked_mul(row_len)
        .ok_or_else(|| format!("naval_combat row_count {} overflows", header.row_count))?;
    let rows_end = HEADER_LEN
        .checked_add(rows_len)
        .ok_or_else(|| "naval_combat payload length overflows".to_string())?;
    if payload.len() < rows_end {
        return Err(format!(
            "naval_combat payload {} shorter than {rows_end} bytes of header and rows",
            payload.len()
        ));
    }
    let mut cursor = rows_end;
    for index in 0..header.string_count as usize {
        if cursor.checked_add(2).is_none_or(|end| end > payload.len()) {
            return Err("naval_combat string length overruns the payload".into());
        }
        let len = u16_at(payload, cursor) as usize;
        cursor += 2;
        if cursor.checked_add(len).is_none_or(|end| end > payload.len()) {
            return Err("naval_combat string entry overruns the payload".into());
        }
        if index == 0 && len != 0 {
            return Err("naval_combat string table index 0 must be empty".into());
        }
        cursor += len;
    }
    if cursor != payload.len() {
        return Err(format!(
            "naval_combat payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
    Ok(&payload[HEADER_LEN..rows_end])
}

/// Parse one 88-byte member row, applying every check the lane applies in
/// `validate_decoded_member_row` (lane `:1967`-`:2214`) and resolving each gated
/// field the way `decode_member_rows` resolves it (lane `:551`-`:583`).
///
/// Mirrored, never tightened: every rule here is one the recorder's own decode
/// enforces, so no payload the recorder folds is refused by this function.
pub fn parse_row(row: &[u8]) -> core::result::Result<MemberRow, String> {
    if row.len() != ROW_LEN {
        return Err(format!("member row length {} != {ROW_LEN}", row.len()));
    }
    let combat_id = u32_at(row, 0);
    let generation = u64_at(row, 4);
    let flags = u32_at(row, 12);
    let side_wire_id = u32_at(row, 16);
    let group_index = u32_at(row, 20);
    let member_index = u32_at(row, 24);
    let unit_type_tag = u32_at(row, 28);
    let ship_instance_id = u32_at(row, 32);
    let owner_tag_db_id = u32_at(row, 36);
    let designed_max_strength_raw = u32_at(row, 40);
    let battle_slot_index_raw = u32_at(row, 44);
    let state_bitmask = u32_at(row, 48);
    let engagement_weight_raw = i32_at(row, 52);
    // `+56` was a must-be-zero reserved word in v2 and is the id space of
    // `task_force_id` in v3. It rides the same presence flag as the id, in both
    // directions.
    let task_force_id_space = u32_at(row, 56);
    let task_force_id = u32_at(row, 60);
    let fleet_id = u32_at(row, 64);
    let strength_raw = u32_at(row, 68);
    let organisation_raw = u32_at(row, 72);
    let sunk_convoys = u32_at(row, 76);
    let experience_raw = u32_at(row, 80);
    let reserved_tail = u32_at(row, 84);

    // lane `:1972`-`:1977`.
    if flags & !KNOWN_FLAGS != 0 {
        return Err(format!(
            "has unknown flags {:#010x}",
            flags & !KNOWN_FLAGS
        ));
    }
    // lane `:1978`-`:1983`.
    if combat_id == 0 || combat_id > MAX_COMBAT_ID {
        return Err(format!("invalid combat_id {combat_id}"));
    }
    // lane `:1984`-`:1989` and `:551`-`:553`.
    if side_wire_id != SIDE_ATTACKER && side_wire_id != SIDE_DEFENDER {
        return Err(format!("invalid side {side_wire_id}"));
    }
    // lane `:1990`-`:2001`.
    if flags & TASK_FORCE_PRESENT == 0 {
        if task_force_id_space != 0 {
            return Err("task_force_id_space present without flag".into());
        }
    } else if task_force_id_space == 0 {
        return Err("task_force reference carries no id space".into());
    }
    if reserved_tail != 0 {
        return Err("trailing reserved field is nonzero".into());
    }
    // lane `:2002`-`:2019`.
    if group_index > MAX_GROUP_INDEX {
        return Err(format!(
            "group_index {group_index} exceeds {MAX_GROUP_INDEX}"
        ));
    }
    if member_index > MAX_MEMBER_INDEX {
        return Err(format!(
            "member_index {member_index} exceeds {MAX_MEMBER_INDEX}"
        ));
    }
    if battle_slot_index_raw > MAX_BATTLE_SLOT_INDEX {
        return Err(format!(
            "battle_slot_index_raw {battle_slot_index_raw} exceeds {MAX_BATTLE_SLOT_INDEX}"
        ));
    }

    let is_convoy = flags & IS_CONVOY != 0;
    let cship_join_validated = flags & CSHIP_JOIN_VALIDATED != 0;
    let has_ship = flags & SHIP_INSTANCE_PRESENT != 0;

    // lane `:2020`-`:2032`: this is what makes `is_convoy` a total function of
    // `unit_type_tag`, which is why the flag itself is not a wire field.
    if !is_convoy && unit_type_tag != CSHIP_TYPE_TAG {
        return Err(format!(
            "non-convoy unit_type_tag {unit_type_tag} != {CSHIP_TYPE_TAG}"
        ));
    }
    if is_convoy && unit_type_tag != 0 {
        return Err(format!("convoy unit_type_tag {unit_type_tag} is not zero"));
    }
    // lane `:2033`-`:2044`.
    if has_ship {
        if ship_instance_id == 0 || ship_instance_id > MAX_SHIP_INSTANCE_ID {
            return Err(format!("invalid ship_instance_id {ship_instance_id}"));
        }
    } else if ship_instance_id != 0 {
        return Err("has ship_instance_id without flag".into());
    }
    // lane `:2045`-`:2050`.
    if cship_join_validated && !has_ship {
        return Err("has CShip validation without ship id".into());
    }
    // lane `:2051`-`:2062`.
    if is_convoy && (has_ship || ship_instance_id != 0) {
        return Err("convoy row must not carry ship-instance identity".into());
    }
    if is_convoy && cship_join_validated {
        return Err("convoy row must not carry CShip validation".into());
    }
    if !is_convoy && !has_ship {
        return Err("non-convoy ship lacks ship-instance identity".into());
    }
    // lane `:2063`-`:2069`: a non-convoy ship may only outlive its CShip join when
    // its exact state is terminal-sunk.
    let terminal_sunk =
        flags & STATE_BITMASK_PRESENT != 0 && state_bitmask == FEX_STATE_SUNK;
    if !is_convoy && !cship_join_validated && !terminal_sunk {
        return Err("non-terminal non-convoy ship lacks CShip validation".into());
    }
    // lane `:2070`-`:2096`: no join, no join-derived fields — the invariant the
    // absent/unknown mapping rests on.
    if !is_convoy && !cship_join_validated && flags & CSHIP_DERIVED_FLAGS != 0 {
        return Err(format!(
            "without a live CShip join carries CShip-derived flags {:#010x}",
            flags & CSHIP_DERIVED_FLAGS
        ));
    }
    if is_convoy && flags & (CSHIP_JOIN_VALIDATED | CSHIP_DERIVED_FLAGS) != 0 {
        return Err(format!(
            "convoy row carries CShip-derived flags {:#010x}",
            flags & (CSHIP_JOIN_VALIDATED | CSHIP_DERIVED_FLAGS)
        ));
    }
    // lane `:2097`-`:2124`.
    let owner = gated_u32(
        flags & OWNER_PRESENT != 0,
        owner_tag_db_id,
        None,
        Some(MAX_OWNER_TAG_DB_ID),
        "owner_tag_db_id",
    )?;
    let designed = gated_u32(
        flags & DESIGNED_MAX_STRENGTH_PRESENT != 0,
        designed_max_strength_raw,
        None,
        Some(MAX_DESIGNED_STRENGTH_RAW),
        "designed_max_strength_raw",
    )?;
    // lane `:2125`-`:2140`: the state word must be one of the seven when present.
    if flags & STATE_BITMASK_PRESENT == 0 && state_bitmask != 0 {
        return Err("has state bitmask without flag".into());
    }
    if flags & STATE_BITMASK_PRESENT != 0 && state_code(state_bitmask).is_none() {
        return Err(format!(
            "has unknown CFEX state enum {state_bitmask:#x}"
        ));
    }
    // lane `:2141`-`:2146`: the weight has NO range check of its own, only the
    // zero-packed rule. Mirrored: tightening would refuse a row the fold folds.
    if flags & ENGAGEMENT_WEIGHT_PRESENT == 0 && engagement_weight_raw != 0 {
        return Err("has engagement weight without flag".into());
    }
    // lane `:2147`-`:2172`: an OOB id is zero-rejected when present.
    let task_force = gated_u32(
        flags & TASK_FORCE_PRESENT != 0,
        task_force_id,
        Some(1),
        Some(MAX_OOB_ID),
        "task_force_id",
    )?;
    let fleet = gated_u32(
        flags & FLEET_PRESENT != 0,
        fleet_id,
        Some(1),
        Some(MAX_OOB_ID),
        "fleet_id",
    )?;
    // lane `:2173`-`:2214`.
    let strength = gated_u32(
        flags & STRENGTH_PRESENT != 0,
        strength_raw,
        None,
        Some(MAX_FIXED_POINT_RAW),
        "strength_raw",
    )?;
    let organisation = gated_u32(
        flags & ORGANISATION_PRESENT != 0,
        organisation_raw,
        None,
        Some(MAX_FIXED_POINT_RAW),
        "organisation_raw",
    )?;
    let experience = gated_u32(
        flags & EXPERIENCE_PRESENT != 0,
        experience_raw,
        None,
        Some(MAX_FIXED_POINT_RAW),
        "experience_raw",
    )?;
    let sunk = gated_u32(
        flags & SUNK_CONVOYS_PRESENT != 0,
        sunk_convoys,
        None,
        Some(MAX_SUNK_CONVOYS),
        "sunk_convoys",
    )?;

    Ok(MemberRow {
        combat_id,
        generation,
        flags,
        deleted: flags & ROW_DELETED != 0,
        side_wire_id,
        group_index,
        member_index,
        unit_type_tag,
        is_convoy,
        cship_join_validated,
        ship_instance_id: has_ship.then_some(ship_instance_id),
        owner_tag_db_id: owner,
        designed_max_strength_raw: designed,
        battle_slot_index_raw,
        state_bitmask: (flags & STATE_BITMASK_PRESENT != 0).then_some(state_bitmask),
        engagement_weight_raw: (flags & ENGAGEMENT_WEIGHT_PRESENT != 0)
            .then_some(engagement_weight_raw),
        task_force_id: task_force,
        task_force_id_space: (flags & TASK_FORCE_PRESENT != 0).then_some(task_force_id_space),
        fleet_id: fleet,
        strength_raw: strength,
        organisation_raw: organisation,
        experience_raw: experience,
        sunk_convoys: sunk,
    })
}

/// The lane's shape for a gated `u32`: a present value must be in range, and a
/// gated-off field must pack a zero. The second half is what makes an ungated
/// non-zero word a malformed row rather than data.
fn gated_u32(
    present: bool,
    value: u32,
    min_value: Option<u32>,
    max_value: Option<u32>,
    field: &str,
) -> core::result::Result<Option<u32>, String> {
    if !present {
        if value != 0 {
            return Err(format!("has {field} without flag"));
        }
        return Ok(None);
    }
    if min_value.is_some_and(|min| value < min) || max_value.is_some_and(|max| value > max) {
        return Err(format!("invalid {field} {value}"));
    }
    Ok(Some(value))
}

/// The `MEMBERS` key of one raw member row.
pub fn key_from_row(row: &[u8]) -> core::result::Result<Key, String> {
    Ok(parse_row(row)?.key())
}

/// One compact row of the `members` table, as the collector emits it today, onto
/// its stored Record. `row` is the raw 88-byte row slice (the caller has already
/// consumed the payload header via [`split_payload`]); d50 has exactly one row
/// form, so length selects nothing and a wrong length is rejected. `frame_tick` is
/// derived-field contract's derived-field input (stored = v + c * (frame_tick >> u)); d50 declares no
/// derived field, so it is taken and ignored — every domain's mapper has the same
/// shape.
///
/// Fail-closed throughout, mirroring the recorder's own decode: a row it would
/// reject is an `Err` here, never a coerced value. A **deleted** row is an `Err`
/// too — it is a key removal plus a retirement (`world_fold.rs::fold_naval_combat_members`)
/// and has no record; a caller must map it to a wire tombstone, and quietly
/// handing one back a record would turn a delete into an upsert. Nothing in this
/// function can panic on any input.
pub fn record_from_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    // d50 has no derived field; derived-field contract's transform input is unused. Named rather than
    // `_frame_tick` because the signature is the cross-domain contract.
    let _ = frame_tick;
    // A collection built for another domain would otherwise be filled field by
    // field with this domain's meanings under another domain's ids.
    if collection.collection_id != MEMBERS {
        return Err(format!(
            "naval_combat_members has no collection {}",
            collection.collection_id
        ));
    }
    let parsed = parse_row(row)?;
    if parsed.deleted {
        return Err(format!(
            "naval_combat_members combat={} generation={} side={} slot={} is a delete row, \
             not a record",
            parsed.combat_id, parsed.generation, parsed.side_wire_id, parsed.battle_slot_index_raw
        ));
    }
    record_from_parsed(collection, &parsed)
}

/// The value mapping itself, shared by [`record_from_row`] and [`State`] so a
/// record is built exactly one way.
fn record_from_parsed(
    collection: &Collection,
    parsed: &MemberRow,
) -> core::result::Result<Record, String> {
    // A convoy member has no CShip: the seven join-derived concepts do not apply
    // to it at all, which is record-state contract's `absent`. A ship whose join was not answered has
    // them applicable and uncaptured, which is `unknown`. See the module docs.
    let joined = |value: Option<u64>| -> Value {
        if parsed.is_convoy {
            Value::Absent
        } else {
            match value {
                Some(v) => Value::Unsigned(v),
                None => Value::Unknown,
            }
        }
    };
    let joined_fixed = |raw: Option<u32>| -> Value {
        if parsed.is_convoy {
            Value::Absent
        } else {
            match raw {
                Some(v) => Value::Continuous(f64::from(v) / FIXED_SCALE),
                None => Value::Unknown,
            }
        }
    };

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::GROUP_INDEX => Value::Unsigned(u64::from(parsed.group_index)),
            field::MEMBER_INDEX => Value::Unsigned(u64::from(parsed.member_index)),
            field::UNIT_TYPE_TAG => Value::Unsigned(u64::from(parsed.unit_type_tag)),
            // Absent on a convoy — no CShip identity exists — and always a value
            // on a ship, which the lane requires (`:2058`-`:2062`).
            field::SHIP_INSTANCE_ID => match parsed.ship_instance_id {
                Some(id) => Value::Unsigned(u64::from(id)),
                None => Value::Absent,
            },
            field::CSHIP_JOIN_VALIDATED => Value::Bool(parsed.cship_join_validated),
            // Member-owned reads: a clear gate is a failed or out-of-range read
            // (`attach_log.rs:40143`-`:40163`), never a fabricated zero. A present
            // zero is a real measured value.
            field::OWNER_TAG_DB_ID => match parsed.owner_tag_db_id {
                Some(id) => Value::ContentId(u64::from(id)),
                None => Value::Unknown,
            },
            field::DESIGNED_MAX_STRENGTH_RAW => match parsed.designed_max_strength_raw {
                Some(v) => Value::Unsigned(u64::from(v)),
                None => Value::Unknown,
            },
            field::STATE => match parsed.state_bitmask {
                // `parse_row` has already rejected any word outside the seven.
                Some(raw) => Value::Enum(state_code(raw).ok_or_else(|| {
                    format!("naval_combat_members unknown CFEX state {raw:#x}")
                })?),
                None => Value::Unknown,
            },
            field::ENGAGEMENT_WEIGHT_RAW => match parsed.engagement_weight_raw {
                Some(v) => Value::Signed(i64::from(v)),
                None => Value::Unknown,
            },
            field::TASK_FORCE_ID => joined(parsed.task_force_id.map(u64::from)),
            field::TASK_FORCE_ID_SPACE => joined(parsed.task_force_id_space.map(u64::from)),
            field::FLEET_ID => joined(parsed.fleet_id.map(u64::from)),
            field::STRENGTH => joined_fixed(parsed.strength_raw),
            field::ORGANISATION => joined_fixed(parsed.organisation_raw),
            field::EXPERIENCE => joined_fixed(parsed.experience_raw),
            // A count, not a fixed-point quantity: `Monotone` carries the word.
            field::SUNK_CONVOYS => joined(parsed.sunk_convoys.map(u64::from)),
            other => {
                // A collection this mapper does not know how to fill. Filling it
                // with a placeholder would publish invented data under a field id
                // that means something else.
                return Err(format!(
                    "naval_combat_members has no mapping for field id {other}"
                ));
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

// --- The frame-level fold ----------------------------------------------------

/// What [`State::fold_payload`] did with a D50 payload, mirroring the four paths
/// `fold_naval_combat_members` can take once its caller has applied the
/// failure-status exclusion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FoldOutcome {
    /// The delta was applied. `mark_fold_domain_success`, `world_fold.rs:20220`.
    Applied,
    /// A live row referenced a `(combat_id, generation)` D49 does not hold
    /// (`world_fold.rs::fold_naval_combat_members`). Nothing folded, accumulator retained,
    /// the domain-hour unknown.
    OrphanMemberReference,
    /// The payload repeated a `(combat, generation, side, slot)` identity
    /// (`world_fold.rs::fold_naval_combat_members`). Nothing folded, accumulator retained.
    DuplicateMemberIdentity,
    /// A live row re-used a key that a previous delete retired
    /// (`world_fold.rs::fold_naval_combat_members`). Nothing folded, accumulator retained.
    RetiredMemberResurrection,
}

/// What [`State::fold_lifecycle_payload`] did with a D49 payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleOutcome {
    /// The D49 delta was applied to the parent key set.
    Applied,
    /// The delta was applied although a D49 **value-completeness** gate would have
    /// masked the hour. Reachable only under [`ParentPolicy::IdentityOnly`], and
    /// counted separately wherever it is used.
    AppliedOverValueGate,
    /// Status bit 11 was set, or a live row lacked its promoted location/region
    /// pair (`world_fold.rs::fold_naval_combat_lifecycle`). Nothing folded, accumulator
    /// retained. **This is the 3a stream's 72-hour case.**
    LocationRegionIncomplete,
    /// The caller's failure-status exclusion fired
    /// (`handle_compact_failure`, `world_fold.rs:20069`-`:20075`).
    FailureStatus,
    /// A live row omitted one of the four side scalars
    /// (`world_fold.rs::fold_naval_combat_lifecycle`). Nothing folded, accumulator retained.
    IncompleteSideScalars,
}

/// How [`State::fold_lifecycle_payload`] treats D49's two **value-completeness**
/// gates — the promoted location/region pair and the four side scalars.
///
/// The default is the only one that describes the recorder. The second exists
/// because of a measured property of the batch-3 `deepwar_a` capture: **not one
/// of its 109 live D49 lifecycle rows carries the promoted location/region pair**,
/// and the collector badges every hour that emits one with lane-local status bit
/// 11. `fold_naval_combat_lifecycle` therefore folds no live row at all on that
/// stream, D49's `naval_combats` accumulator stays empty for all 514 hours, and
/// every d50 hour carrying a live member row is refused as an orphan. Under the
/// recorder's own semantics that capture publishes **no d50 state whatsoever** —
/// a true result, and a useless one to size a grammar against.
///
/// Both gates concern D49's own value fields: `location_id`, `region_id`,
/// positioning and anti-air. **d50 carries none of them.** What d50's orphan
/// refusal reads is the combat's *identity*, `(combat_id, generation)`, which
/// those gates say nothing about. So the second policy admits the identity write
/// while still refusing the hour on a failure status and still rejecting any
/// payload the lane cannot decode.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub enum ParentPolicy {
    /// Reproduce `fold_naval_combat_lifecycle` exactly. The only policy that
    /// describes the recorder, and the only one a production path may use.
    #[default]
    FoldFaithful,
    /// Let a D49 value-completeness gate mask D49's own values without masking the
    /// combat identity. **This is not what the recorder does.** It exists so a
    /// measurement can report a grammar comparison on a capture whose D49 lane
    /// never promoted a location, and every use of it must label the result as a
    /// size-accounting arm — the same standing d11's `ship_d11_no_pdq1.bin` has.
    IdentityOnly,
}

/// `STATUS_LOCATION_CANDIDATE_UNVALIDATED`, lane `:32`. A **lane-local** payload
/// status bit for domain 49, masked from global interpretation
/// (`compact_contract/src/lib.rs:93`-`:101`) but read directly by
/// `fold_naval_combat_lifecycle` (`world_fold.rs::fold_naval_combat_lifecycle`). Mirrored here because
/// the parent mirror must read it exactly where the fold does.
pub const D49_STATUS_LOCATION_CANDIDATE_UNVALIDATED: u32 = 1 << 11;

const D49_ROW_DELETED: u32 = 1 << 0;
const D49_LOCATION_PRESENT: u32 = 1 << 1;
const D49_ATTACKER_COUNT_PRESENT: u32 = 1 << 2;
const D49_DEFENDER_COUNT_PRESENT: u32 = 1 << 3;
const D49_ATTACKER_POSITIONING_PRESENT: u32 = 1 << 4;
const D49_DEFENDER_POSITIONING_PRESENT: u32 = 1 << 5;
const D49_REGION_PRESENT: u32 = 1 << 6;
const D49_ATTACKER_ANTI_AIR_PRESENT: u32 = 1 << 7;
const D49_DEFENDER_ANTI_AIR_PRESENT: u32 = 1 << 8;
const D49_KNOWN_FLAGS: u32 = D49_ROW_DELETED
    | D49_LOCATION_PRESENT
    | D49_ATTACKER_COUNT_PRESENT
    | D49_DEFENDER_COUNT_PRESENT
    | D49_ATTACKER_POSITIONING_PRESENT
    | D49_DEFENDER_POSITIONING_PRESENT
    | D49_REGION_PRESENT
    | D49_ATTACKER_ANTI_AIR_PRESENT
    | D49_DEFENDER_ANTI_AIR_PRESENT;
/// Province id space STRUCTURAL width (lane IA, `claim_347234c20a542ca79032`).
///
/// Was `1_000_000`. This constant is named for the FIELD (`location`) rather
/// than the SPACE, which is why lane IA's name-based census -- built around
/// `MAX_PROVINCE_ID` and `MAX_STATE_ID` -- did not see it for fourteen review
/// rounds. The value it bounds is read from an authenticated `CProvince` and is
/// a province id like any other.
const D49_MAX_LOCATION_ID: u32 = (i32::MAX as u32) - 1;
const D49_MAX_REGION_ID: u32 = 100_000;
const D49_MAX_POSITIONING_RAW: i32 = 100_000;

/// Parse one 72-byte D49 lifecycle row, applying every check
/// `validate_decoded_lifecycle_row` applies (lane `:1866`-`:1965`) and keeping
/// only the columns `fold_naval_combat_lifecycle` needs.
///
/// **This is not a d49 port.** See [`State::fold_lifecycle_payload`].
pub fn parse_lifecycle_row(row: &[u8]) -> core::result::Result<LifecycleKeyRow, String> {
    if row.len() != LIFECYCLE_ROW_LEN {
        return Err(format!(
            "lifecycle row length {} != {LIFECYCLE_ROW_LEN}",
            row.len()
        ));
    }
    let combat_id = u32_at(row, 0);
    let generation = u64_at(row, 4);
    // `+12` is `body_ptr`, a raw process address the lane's decoder does not even
    // read back (lane `:513` sets it to 0). It is a witness, never a field.
    let flags = u32_at(row, 20);
    let location_id = u32_at(row, 24);
    let region_id = u32_at(row, 28);
    let attacker_member_count = u32_at(row, 32);
    let defender_member_count = u32_at(row, 36);
    let attacker_positioning_raw = i32_at(row, 40);
    let defender_positioning_raw = i32_at(row, 44);
    let attacker_anti_air_raw = i64_at(row, 48);
    let defender_anti_air_raw = i64_at(row, 56);
    let reserved_tail = u64_at(row, 64);

    if flags & !D49_KNOWN_FLAGS != 0 {
        return Err(format!(
            "lifecycle row has unknown flags {:#010x}",
            flags & !D49_KNOWN_FLAGS
        ));
    }
    if combat_id == 0 || combat_id > MAX_COMBAT_ID {
        return Err(format!("lifecycle row invalid combat_id {combat_id}"));
    }
    let has_location = flags & D49_LOCATION_PRESENT != 0;
    if has_location && (location_id == 0 || location_id > D49_MAX_LOCATION_ID) {
        return Err(format!("lifecycle row invalid location_id {location_id}"));
    }
    if !has_location && location_id != 0 {
        return Err("lifecycle row has location without flag".into());
    }
    let has_region = flags & D49_REGION_PRESENT != 0;
    if has_region && (region_id == 0 || region_id > D49_MAX_REGION_ID) {
        return Err(format!("lifecycle row invalid region_id {region_id}"));
    }
    if !has_region && region_id != 0 {
        return Err("lifecycle row has region without flag".into());
    }
    if flags & D49_ATTACKER_COUNT_PRESENT == 0 && attacker_member_count != 0 {
        return Err("lifecycle row has attacker count without flag".into());
    }
    if flags & D49_DEFENDER_COUNT_PRESENT == 0 && defender_member_count != 0 {
        return Err("lifecycle row has defender count without flag".into());
    }
    let has_attacker_positioning = flags & D49_ATTACKER_POSITIONING_PRESENT != 0;
    let has_defender_positioning = flags & D49_DEFENDER_POSITIONING_PRESENT != 0;
    if !has_attacker_positioning && attacker_positioning_raw != 0 {
        return Err("lifecycle row has attacker positioning without flag".into());
    }
    if !has_defender_positioning && defender_positioning_raw != 0 {
        return Err("lifecycle row has defender positioning without flag".into());
    }
    if has_attacker_positioning
        && !(0..=D49_MAX_POSITIONING_RAW).contains(&attacker_positioning_raw)
    {
        return Err(format!(
            "lifecycle row attacker positioning raw {attacker_positioning_raw} is outside \
             0..={D49_MAX_POSITIONING_RAW}"
        ));
    }
    if has_defender_positioning
        && !(0..=D49_MAX_POSITIONING_RAW).contains(&defender_positioning_raw)
    {
        return Err(format!(
            "lifecycle row defender positioning raw {defender_positioning_raw} is outside \
             0..={D49_MAX_POSITIONING_RAW}"
        ));
    }
    let has_attacker_anti_air = flags & D49_ATTACKER_ANTI_AIR_PRESENT != 0;
    let has_defender_anti_air = flags & D49_DEFENDER_ANTI_AIR_PRESENT != 0;
    if !has_attacker_anti_air && attacker_anti_air_raw != 0 {
        return Err("lifecycle row has attacker anti-air without flag".into());
    }
    if !has_defender_anti_air && defender_anti_air_raw != 0 {
        return Err("lifecycle row has defender anti-air without flag".into());
    }
    if reserved_tail != 0 {
        return Err("lifecycle row trailing reserved field is nonzero".into());
    }

    Ok(LifecycleKeyRow {
        combat_id,
        generation,
        deleted: flags & D49_ROW_DELETED != 0,
        has_location,
        has_region,
        has_all_side_scalars: has_attacker_positioning
            && has_defender_positioning
            && has_attacker_anti_air
            && has_defender_anti_air,
    })
}

/// Split a D49 lifecycle payload into its header and its rows, applying the same
/// structural rules as [`split_payload`] for the 72-byte row form.
pub fn split_lifecycle_payload(
    payload: &[u8],
) -> core::result::Result<(PayloadHeader, &[u8]), String> {
    let header = peek_header(payload, LIFECYCLE_ROW_LEN)?;
    let rows = body_slice(payload, header, LIFECYCLE_ROW_LEN)?;
    for (index, row) in rows.chunks_exact(LIFECYCLE_ROW_LEN).enumerate() {
        parse_lifecycle_row(row).map_err(|e| format!("naval_combat_lifecycle row {index}: {e}"))?;
    }
    Ok((header, rows))
}

/// The keyed state d50 folds into, plus the two pieces of fold state its payload
/// grammar depends on.
///
/// One record is not one state for this domain: the grammar is pure cumulative
/// delta, so an hour's state is the whole history of the stream up to that hour.
/// Two further pieces of that history are not in the accumulator at all:
///
/// * `retired` mirrors `naval_retired_member_keys` (`world_fold.rs:1707`), which
///   is **never cleared** — a harness that replays per hour must carry it or the
///   resurrection refusal never fires;
/// * `parents` mirrors the KEY SET of D49's `naval_combats` (`:1702`), which the
///   orphan refusal reads and whose deletions cascade into this domain's own
///   accumulator.
///
/// This is the recipe's fourth public boundary, and it exists so the measurement
/// bin, the tests and any future caller share ONE replay of the shipping layer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    /// Collection [`MEMBERS`], keyed by [`member_key`].
    pub members: KeyedState,
    parents: BTreeSet<(u32, u64)>,
    retired: BTreeSet<(u32, u64, u32, u32)>,
    parent_deletes_applied: u64,
    members_dropped_by_parent_delete: u64,
}

impl State {
    pub fn new() -> Self {
        Self::default()
    }

    /// The D49 parent key set this state currently holds. Public for the
    /// measurement bin's reporting; nothing in the mapping reads it from outside.
    pub fn parents(&self) -> &BTreeSet<(u32, u64)> {
        &self.parents
    }

    /// The persistent retired-member key set (`naval_retired_member_keys`).
    pub fn retired(&self) -> &BTreeSet<(u32, u64, u32, u32)> {
        &self.retired
    }

    /// How many D49 tombstone **rows** have actually removed a parent, counted one
    /// row at a time inside the fold (`world_fold.rs::fold_naval_combat_lifecycle` processes each
    /// deleted row individually).
    ///
    /// A caller cannot recover this from the parent set's size: one payload may
    /// insert and delete, and the net change then hides both. Counting the net is
    /// exactly the defect this accessor exists to make impossible.
    pub fn parent_deletes_applied(&self) -> u64 {
        self.parent_deletes_applied
    }

    /// How many member rows those tombstones removed from this domain's own
    /// accumulator — the cross-domain write, measured rather than inferred.
    pub fn members_dropped_by_parent_delete(&self) -> u64 {
        self.members_dropped_by_parent_delete
    }

    /// Fold one D49 lifecycle payload into the parent key set, exactly as
    /// `fold_naval_combat_lifecycle` folds it (`world_fold.rs:20040`-`:20128`) —
    /// **reduced to the key columns**.
    ///
    /// This is deliberately not a d49 port: it produces no d49 record, declares no
    /// d49 field id, and keeps no d49 value. What it reproduces is (a) D49's
    /// structural validation, because a payload the recorder cannot decode is a
    /// stream rejection for the hour d50 folds in, (b) D49's three non-folding
    /// paths in the fold's own order, because each leaves the parent set
    /// unchanged, and (c) the two accumulator writes d50's semantics depend on:
    /// the key insert and the delete, whose cascade removes every member of that
    /// combat (`world_fold.rs:20098` drops the whole `NavalCombatFoldState`,
    /// `members` map included).
    ///
    /// `failure_status` is the caller's
    /// `compact_status_failure_flags_after_expected_partial_tolerance(49, …)`
    /// result. It is a parameter rather than a re-derivation because the predicate
    /// is `compact_contract`'s and this crate's library half must not copy its
    /// mask — the recipe's typed-mapper-input rule. Its position in the sequence
    /// is the fold's own: **after** the location/region check, **before** the
    /// side-scalar check (`world_fold.rs::fold_naval_combat_lifecycle`).
    ///
    /// Error contract: all-or-nothing. Every row is parsed and every refusal is
    /// decided before the first mutation, which is what `fold_naval_combat_lifecycle`
    /// does too — its own `Err` paths all precede its row loop.
    pub fn fold_lifecycle_payload(
        &mut self,
        payload: &[u8],
        failure_status: bool,
        policy: ParentPolicy,
    ) -> core::result::Result<LifecycleOutcome, String> {
        // `decode_lifecycle_rows` (`world_fold.rs::fold_naval_combat_lifecycle`) runs before anything
        // else, so a malformed payload is an `Err` even under a failure status,
        // and under either policy.
        let (header, rows) = split_lifecycle_payload(payload)?;
        let parsed: Vec<LifecycleKeyRow> = rows
            .chunks_exact(LIFECYCLE_ROW_LEN)
            .map(parse_lifecycle_row)
            .collect::<core::result::Result<_, _>>()?;

        // `world_fold.rs::fold_naval_combat_lifecycle`, and it is BEFORE the failure check.
        let location_region_incomplete = header.status_flags
            & D49_STATUS_LOCATION_CANDIDATE_UNVALIDATED
            != 0
            || parsed
                .iter()
                .any(|row| !row.deleted && (!row.has_location || !row.has_region));
        if location_region_incomplete && policy == ParentPolicy::FoldFaithful {
            return Ok(LifecycleOutcome::LocationRegionIncomplete);
        }
        // `handle_compact_failure`, `world_fold.rs:20069`-`:20075`. Never relaxed:
        // a failure status is a statement about the capture, not about D49's value
        // promotion, and its payload carries no rows anyway.
        if failure_status {
            return Ok(LifecycleOutcome::FailureStatus);
        }
        // `world_fold.rs::fold_naval_combat_lifecycle`.
        let side_scalars_incomplete = parsed
            .iter()
            .any(|row| !row.deleted && !row.has_all_side_scalars);
        if side_scalars_incomplete && policy == ParentPolicy::FoldFaithful {
            return Ok(LifecycleOutcome::IncompleteSideScalars);
        }

        for row in parsed {
            let combat = (row.combat_id, row.generation);
            if row.deleted {
                // `world_fold.rs::fold_naval_combat_lifecycle`: the whole combat state goes,
                // and its `members` map with it. Counted PER ROW, because a
                // payload that also inserts would otherwise hide the deletion in
                // the net size change.
                if self.parents.remove(&combat) {
                    self.parent_deletes_applied = self.parent_deletes_applied.saturating_add(1);
                }
                let dropped = self.drop_members_of(row.combat_id, row.generation);
                self.members_dropped_by_parent_delete = self
                    .members_dropped_by_parent_delete
                    .saturating_add(dropped as u64);
                continue;
            }
            // `world_fold.rs::fold_naval_combat_lifecycle`: `or_insert_with` keeps an existing
            // combat's members untouched, so a re-emitted live row changes
            // nothing here.
            self.parents.insert(combat);
        }
        Ok(
            if location_region_incomplete || side_scalars_incomplete {
                LifecycleOutcome::AppliedOverValueGate
            } else {
                LifecycleOutcome::Applied
            },
        )
    }

    /// Fold one D50 member payload exactly as `fold_naval_combat_members` folds it
    /// (`world_fold.rs:20131`-`:20222`).
    ///
    /// The caller is responsible for the **failure-status** exclusion, because the
    /// predicate is `compact_contract`'s and this crate's library half must not
    /// depend on that crate nor copy its mask. The fold consults it at `world_fold.rs::fold_naval_combat_members`,
    /// i.e. **after** `decode_member_rows` and **before** the three refusals — so a
    /// caller that gets the order right runs [`split_payload`] (production's
    /// status-independent payload authority), excludes on status, and only then
    /// calls this.
    ///
    /// Error contract: all-or-nothing, exactly like the fold. Every row is parsed
    /// and all three refusals are decided before the first mutation — the fold's
    /// own comment says so: *"Validate the whole current D50 delta before mutating
    /// any member accumulator"* (`world_fold.rs::fold_naval_combat_members`).
    pub fn fold_payload(
        &mut self,
        collection: &Collection,
        payload: &[u8],
    ) -> core::result::Result<FoldOutcome, String> {
        let (_, rows) = split_payload(payload)?;
        let parsed: Vec<MemberRow> = rows
            .chunks_exact(ROW_LEN)
            .map(parse_row)
            .collect::<core::result::Result<_, _>>()?;

        // `world_fold.rs::fold_naval_combat_members`. Delete rows are exempt: a delete for an
        // already-removed combat is a valid idempotent no-op.
        if parsed
            .iter()
            .any(|row| !row.deleted && !self.parents.contains(&(row.combat_id, row.generation)))
        {
            return Ok(FoldOutcome::OrphanMemberReference);
        }
        // `world_fold.rs::fold_naval_combat_members`. Over EVERY row, deleted included.
        let mut identities = BTreeSet::new();
        if parsed.iter().any(|row| {
            !identities.insert((
                row.combat_id,
                row.generation,
                row.side_wire_id,
                row.battle_slot_index_raw,
            ))
        }) {
            return Ok(FoldOutcome::DuplicateMemberIdentity);
        }
        // `world_fold.rs::fold_naval_combat_members`.
        if parsed.iter().any(|row| {
            !row.deleted
                && self.retired.contains(&(
                    row.combat_id,
                    row.generation,
                    row.side_wire_id,
                    row.battle_slot_index_raw,
                ))
        }) {
            return Ok(FoldOutcome::RetiredMemberResurrection);
        }

        // Build every record before mutating anything, so a mapping error cannot
        // leave the accumulator half-written.
        let mut pending: Vec<(MemberRow, Option<Record>)> = Vec::with_capacity(parsed.len());
        for row in parsed {
            let record = if row.deleted {
                None
            } else {
                Some(record_from_parsed(collection, &row)?)
            };
            pending.push((row, record));
        }

        for (row, record) in pending {
            let retired_key = (
                row.combat_id,
                row.generation,
                row.side_wire_id,
                row.battle_slot_index_raw,
            );
            // `world_fold.rs::fold_naval_combat_members`: the retirement is recorded BEFORE
            // the parent lookup, so a delete for a combat that is already gone
            // still retires the key.
            if row.deleted {
                self.retired.insert(retired_key);
            }
            // `world_fold.rs::fold_naval_combat_members`: a member whose combat has no active
            // lifecycle row is skipped rather than fabricating a parentless
            // combat. The orphan refusal above means only delete rows reach here.
            if !self.parents.contains(&(row.combat_id, row.generation)) {
                continue;
            }
            let member = row.key();
            if row.deleted {
                self.members.remove(&member);
            } else {
                // `world_fold.rs::fold_naval_combat_members`: a plain insert, so a repeated key inside
                // one payload would be last-writer-wins — except that the
                // duplicate-identity refusal above has already ruled that out.
                self.members
                    .insert(member, record.expect("a live row has a record"));
            }
        }
        Ok(FoldOutcome::Applied)
    }

    /// Remove every member key under one combat. Member keys are
    /// `[combat_id, generation, side, slot]` and `Key` orders lexicographically
    /// with a prefix sorting before any extension of itself, so a combat's members
    /// are exactly the half-open key range `[c, g] .. [c, g + 1]`.
    /// Returns how many member keys it removed.
    fn drop_members_of(&mut self, combat_id: u32, generation: u64) -> usize {
        let lower = Key(vec![combat_id as i128, generation as i128]);
        let upper = Key(vec![combat_id as i128, generation as i128 + 1]);
        let doomed: Vec<Key> = self
            .members
            .range(lower..upper)
            .map(|(k, _)| k.clone())
            .collect();
        let count = doomed.len();
        for k in doomed {
            self.members.remove(&k);
        }
        count
    }
}

/// The per-combat member counts the D49 accumulator would hold, for a harness that
/// wants to report the shape of the state. Keyed `(combat_id, generation)`.
pub fn members_per_combat(state: &State) -> BTreeMap<(u32, u64), usize> {
    let mut out = BTreeMap::new();
    for k in state.members.keys() {
        if k.0.len() >= 2 {
            let combat = (k.0[0] as u32, k.0[1] as u64);
            *out.entry(combat).or_insert(0usize) += 1;
        }
    }
    out
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
mod tests;
