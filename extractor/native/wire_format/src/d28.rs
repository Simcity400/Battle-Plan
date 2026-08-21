//! d28 `oob_command_live` — the order-of-battle command tree: army groups,
//! armies, fleets and task forces, with their membership.
//!
//! Derived from `world_fold.rs:20297`-`:20506`
//! (`fold_oob_command`), its payload constants at `:970`-`:972` and `:1019`-`:1038`,
// @world-fold-authority domain=28 functions=fold_oob_command lines=20297-20506 sha256=6bb2b08571d4bddb43b5bfa98eebcfb8f617448e6ff499a3388cac8de585931e closure=4877b36fe37250431ccfdf9a0238bfb78e3bbf7070aa1ac5092d61b267cb474b
//! the accumulator's own JSON projection (`:7948`-`:8010`, `:23542`-`:23560`), and
//! the collector read that produces the flags, the status words and the member
//! child rows (`extractor/native/collector/src/attach_log.rs:46680`-`:47160`,
//! `:22236`-`:22345`, `:47625`-`:47700`, `:69638`-`:69737`).
//! `extractor/documents/domains/` carries no d28 dossier, so the fold and the
//! collector read are the whole basis.
//!
//! **Payload grammar** (`world_fold.rs::fold_oob_command`): a 48-byte header —
//! `elapsed_wall_ms` u64 at `+0`, `row_count` u32 at `+8`, `changed_rows` u32 at
//! `+12`, `status_flags` u32 at `+16`, `root_rows` u32 at `+20`,
//! `observed_army_rows` u32 at `+24`, `observed_army_group_rows` u32 at `+28`,
//! `member_count` u32 at `+32`, `deleted_rows` u32 at `+36`,
//! `observed_fleet_rows` u32 at `+40`, `observed_task_force_rows` u32 at `+44` —
//! followed by `row_count` fixed 80-byte **command rows** and then `member_count`
//! fixed 8-byte **member rows**. The fold requires the length to be exactly
//! `48 + row_count * 80 + member_count * 8` (`world_fold.rs::fold_oob_command`), so no legacy
//! short row exists and the length selects nothing.
//!
//! Command row (`world_fold.rs::fold_oob_command`):
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+0`  | u32 | `kind`                  | raw | none (key; must be `1..=4`, `world_fold.rs::fold_oob_command`) |
//! | `+4`  | u32 | `object_id`             | raw | none (key; zero rejected, `world_fold.rs::fold_oob_command`) |
//! | `+8`  | u32 | `flags`                 | —   | — |
//! | `+12` | u32 | `parent_id`             | raw | `PARENT` (`1<<1`) |
//! | `+16` | u32 | `owner_tag_db_id`       | raw | `OWNER` (`1<<2`) |
//! | `+20` | u32 | `leader_id`             | raw | `LEADER` (`1<<3`) |
//! | `+24` | u32 | `leader_skill`          | raw | `LEADER_SKILL` (`1<<4`), **validated** `world_fold.rs::fold_oob_command` |
//! | `+28` | u32 | `member_first`          | —   | transport, not state |
//! | `+32` | u32 | `member_count`          | —   | transport, not state |
//! | `+36` | u32 | `logical_member_count`  | raw | none |
//! | `+40` | u32 | `member_count_engine`   | raw | `MEMBER_COUNT_ENGINE` (`1<<5`) |
//! | `+44` | u32 | `missing_member_count`  | raw | `MISSING_MEMBER_COUNT` (`1<<6`) |
//! | `+48` | u32 | `leader_trait_count`    | raw | none on the wire — see below |
//! | `+52` | u32 | `parent_status`         | raw | none |
//! | `+56` | u32 | `member_status`         | raw | none |
//! | `+60` | u32 | `owner_status`          | raw | none |
//! | `+64` | u32 | `leader_status`         | raw | none |
//! | `+68` | u32 | `leader_trait_status`   | raw | none |
//! | `+72` | u32 | `location_province_id`  | raw | `LOCATION` (`1<<7`), **validated** `world_fold.rs::fold_oob_command` |
//! | `+76` | u32 | `reserved`              | —   | must be zero (`world_fold.rs::fold_oob_command`) |
//!
//! Member row (`world_fold.rs::fold_oob_command`): `member_id` u32 at `+0` (zero
//! rejected), `reserved` u32 at `+4` (must be zero).
//!
//! **No scales anywhere.** Every word is a raw id, count, skill or status code;
//! the fold divides nothing and the accumulator's JSON projection
//! (`world_fold.rs::build_order_of_battle`) emits every one of them verbatim. There is no
//! `scale_raw` word in the header and `ensure_fixed_scale` is never called for
//! this domain.
//!
//! **Two collections, because `(member_first, member_count)` is transport.** A
//! command addresses its members as a half-open range into one shared member
//! array; the fold proves those ranges non-overlapping (current
//! `world_fold.rs::fold_oob_command`) and total (`world_fold.rs::fold_oob_command`), and proves each member id
//! unique within its owning row (`world_fold.rs::fold_oob_command`).
//! The range *words* are re-packed by the collector on
//! every frame from emission order — `compact_oob_command_data_payload` recomputes
//! `member_first` from a running cursor over the row loop (current
//! `attach_log.rs:70025`-`:70085`) and then appends the member ids in that same
//! row order (`:70086`-`:70093`) — and carry no state a reader could ask about,
//! so they are flattened away here: collection
//! [`COMMANDS`] carries one row per command and collection [`MEMBERS`] one row
//! per membership edge, keyed by the command's key plus the member id.
//!
//! [`MEMBERS`] has **no value fields at all** — the membership edge is the whole
//! fact, and the member row's only other word is a must-be-zero reserved. That is
//! deliberate, not an oversight: adding the member id as a value beside the key
//! would be a second copy of state the decoder must keep consistent.
//!
//! # The member-ordering check is this layer's, not the fold's
//!
//! What the fold flattens the range into is **not** a set. It slices the frame's
//! shared member array and stores the slice as an ordered `Vec<u32>`
//! (`OobCommandState.members`, current `world_fold.rs:23623`, filled at `world_fold.rs::fold_oob_command`),
//! and the recorder's own projection emits that vector verbatim as an ordered
//! JSON array — `armies[].members` (`world_fold.rs::build_order_of_battle`) and `army_groups[].army_refs`
//! (`world_fold.rs::build_order_of_battle`). The fold enforces **no ordering at all** on the member rows: its
//! member loop rejects a zero id, an unowned child and a repeat within one row,
//! and nothing else (`world_fold.rs::fold_oob_command`). So the fold accepts every permutation of
//! a row's members and stores each one as a different vector, and [`MEMBERS`]'
//! `(kind, object_id, member_id)` key — whose own order is ascending `member_id`
//! — cannot express the ones that are not sorted.
//!
//! Mirroring would mean silently re-sorting the sequence, which is the data-loss
//! defect itself. So [`fold_payload`] **rejects** a command row whose member
//! range is not strictly ascending *at the moment that range becomes state*, and
//! that is a deliberate divergence: stricter than the fold, on input the fold
//! would accept. This is d32's precedent applied unchanged (`d32.rs:146`-`:171`),
//! and d32's rule — "tightening is justified only where mirroring would lose
//! information" — is why the rest of this schema mirrors and this one place does
//! not. Placing it there rather than in [`split_payload`] is what confines it to
//! the rows a frame leaves behind: a **delete** row's range is discarded with the
//! key (`world_fold.rs::fold_oob_command`, before the slice at `world_fold.rs::fold_oob_command`) and a **withheld**
//! tolerated-partial frame is abandoned whole (`world_fold.rs::fold_oob_command`), so neither can
//! lose a sequence and neither is refused — see [`validate_withheld_payload`].
//!
//! **It is not stricter than the producer, and the position is transport rather
//! than information.** An army's member list is not read from the army at all: it
//! is built by grouping the division roots by their parent pointer
//! (`attach_log.rs:46975`-`:47001`), so the pre-sort order is division-scan order,
//! not an engine member vector — and all four row builders then canonicalise it,
//! armies `sort_unstable`/`dedup` at `:47008`-`:47009`, army groups at
//! `:47058`-`:47059`, fleets at `:47164`-`:47165`, task-force ships at `:47884`
//! (which additionally fails the whole read on a duplicate, `:47885`-`:47887`).
//! `compact_oob_member_ids` (`:47198`) only truncates that sorted prefix, so even
//! the `OVERFLOW` path stays ascending. So no collector-produced frame can carry
//! an unsorted range. **Measured**: over the 1,519 command rows the 146 d28 DATA
//! records of `deepwar_t1` serialize, 1,000 live rows carry two or more members
//! and every one of them is strictly ascending, 472 live rows carry one or none
//! and have no ordering to check, and 47 are delete rows the check skips. Zero
//! refusals. The bench prints that split every run so the tightening stays
//! visible rather than becoming an invisible assumption.
//!
//! **Why not d27's ordinal, which is the same shape.** d27's `TARGET_STATES`
//! keeps the fold's key and carries `TARGET_ORDINAL` beside it (`d27.rs:363`),
//! and the same device would work here. The two domains split on what the
//! position *is*. d27's producer refuses to sort and says why — "Preserve engine
//! vector order. Rendering/replay consumers require the exact producer order;
//! sorting here would erase redraw semantics" (`attach_log.rs:19349`-`:19351`) —
//! so there the order is the engine's own and varies; it is information. d28 has
//! no such comment and could not have one: its order is the collector's own
//! canonicalisation of a scan, a total function of the member ids, re-derived
//! every frame. That is transport, which is exactly what d32 says about its
//! entry ordinal. The measurement agrees: carrying a range-local
//! `member_ordinal` was built and measured against this capture, and it costs the
//! wire arm its entire win — raw 185,749 → 218,494 B (90.2% → 106.2% of
//! shipping), zstd-3 whole-stream 35,110 → 50,791 B (95.8% → 138.6%), 48-hour
//! blocks 137,000 → 161,816 B, and member churn 5.54 → 28.14 changed rows/hour,
//! because an edge added or removed mid-range renumbers every edge after it. That
//! is a large, permanent price for a word that is redundant on every payload the
//! producer can emit and load-bearing on none of them. The alternative reading is
//! defensible and is recorded here with its numbers, per step 5's rule that where
//! two readings are both defensible the choice is a decision with its evidence.
//!
//! **No consumer indexes the array today**, which is why refusing rather than
//! carrying loses nothing downstream. `army_refs` has no reader anywhere in
//! `replay_interface/` — it appears only in the projection above, in the schema
//! wire contract and in the
//! independent save oracle, which builds its own from the save
//! (`extractor/save_oracle/parse_oob.py:229`, `:242`). The only reader of
//! `order_of_battle.armies[]` is `xui17_oob_assignments`
//! (`replay_interface/native/replay_core/src/world.rs:21664`-`:21730`), which
//! takes `id`, `owner` and `leader` and never touches `members`.
//!
//! ## The four states, field by field
//!
//! Seven flag gates (`world_fold.rs:1024`-`:1038`) and one status-driven field.
//! The fold folds a clear gate to `None` for five of them, and — the hazard worth
//! naming — to **`0`, not `None`**, for `owner_tag_db_id` (`world_fold.rs::fold_oob_command`).
//! The fold decides value-vs-nothing and nothing else;
//! the three-way choice below comes from the collector read that sets each flag.
//!
//! The decisive structure is that **which gate can be clear depends on the row's
//! `kind`**, because the collector builds a different row for each of the four
//! kinds and hard-codes `None` for the concepts a kind does not have
//! (`attach_log.rs:46855`-`:46913` for armies and army groups, `:46940`-`:47010`
//! for task forces and fleets). A gate the collector never even attempts for a
//! kind is record-state contract's **absent** ("not applicable, or unsupported by this schema"); a
//! gate it attempts and fails is record-state contract's **unknown** ("applicable, not captured").
//! `kind` is a key component, so the reader can always tell which case it is.
//!
//! | field | gate set | gate clear |
//! |---|---|---|
//! | `parent_id` | value | `parent_status == NONE` → **absent** (army group, fleet: no parent concept); otherwise **unknown** |
//! | `owner_tag_db_id` | value | `owner_status == MIXED` → **null**; otherwise **unknown** |
//! | `leader_id` | value | `leader_status == NONE` → **absent** (army group, fleet, task force); otherwise **unknown** |
//! | `leader_skill` | value | `leader_status == NONE` → **absent**; otherwise **unknown** |
//! | `member_count_engine` | value | `kind == ARMY_GROUP` → **absent**; otherwise **unknown** |
//! | `missing_member_count` | value | `kind ∈ {ARMY_GROUP, FLEET}` → **absent**; otherwise **unknown** |
//! | `location_province_id` | value | `kind != TASK_FORCE` → **absent**; otherwise **unknown** |
//! | `leader_trait_count` | (no gate) | `leader_trait_status == EXACT` → value; else `leader_status == NONE` → **absent**, otherwise **unknown** |
//!
//! The evidence, gate by gate:
//!
//! * **`parent_id`.** `read_compact_oob_army_group_parent` (`attach_log.rs:47060`)
//!   returns `(None, "validation_failed")` on an unreadable or invalid parent
//!   body and `(None, "unknown")` when the pointer is not a map-control heap
//!   pointer — which is also what a genuinely parentless army reads as, so null
//!   and unknown are indistinguishable there and the recipe's tie-break chooses
//!   unknown. Army-group and fleet rows set `parent_status: "none"` outright
//!   (`:46888`, `:47000`): those kinds are roots, the concept does not apply, and
//!   that is absent. Task-force rows always carry `Some(fleet_id)` with status
//!   `"exact"` (`:46967`).
//! * **`owner_tag_db_id`.** `compact_oob_owner_consensus` (`attach_log.rs:47036`)
//!   returns `(None, "unknown")` when no member division yielded a readable owner
//!   and `(None, "mixed")` when the members disagree. `"mixed"` is a *measured*
//!   fact: the single-owner relation genuinely resolves to nothing, which is record-state contract's
//!   null. `"unknown"` is an unread owner, which is record-state contract's unknown. The fold's `0`
//!   is a sentinel and not a country: the recorder's own projection renders it as
//!   JSON `Null` (`country_ref_json`, `world_fold.rs::country_ref_json`), and `0` is
//!   rejected as a `tag_db_id` everywhere else (d26 `:17404`, d63 `:9259`). So
//!   the `0` is an empty state that this schema is able to keep apart from a real
//!   value, and it is never emitted as `Unsigned(0)`/`ContentId(0)`.
//! * **`leader_id` / `leader_skill`.** `read_army_oob_leader_detail`
//!   (`attach_log.rs:22236`) returns `None` on an unreadable leader pointer, a
//!   non-heap pointer (again including a genuinely leaderless army), a vtable or
//!   handle-type mismatch, an unreadable id, or an out-of-range id — five read
//!   failures and one indistinguishable null, so unknown. Only army rows ever
//!   attempt a leader; army-group, fleet and task-force rows set
//!   `leader_status: "none"` (`:46903`, `:46982`, `:47006`), which is absent.
//!   `leader_skill` is read only inside the leader-present branch, so its gate
//!   can only be clear either because there is no leader (absent, mirroring
//!   `leader_id`) or because `read_unit_leader_skill` failed its pointer walk or
//!   its bound trip (`:22287`-`:22300`) — unknown.
//! * **`member_count_engine`.** Armies read it from the engine
//!   (`attach_log.rs:46836`), task forces carry the validated ship count and
//!   fleets carry their task-force count; **army groups hard-code `None`**
//!   (`:46884`) — the collector never attempts an engine-side count for that
//!   kind, so it is unsupported by this schema rather than uncaptured.
//! * **`missing_member_count`.** Armies compute
//!   `member_count_engine.checked_sub(live_member_count)` (`:46838`), which is
//!   `None` when the engine count itself is unknown or when the subtraction
//!   underflows — unknown either way. Task forces compute a filtered difference
//!   (`:46955`-`:46962`), whose `.filter(|missing| *missing > 0)` collapses a
//!   genuine **zero** into the same clear gate as an unread count; that is a
//!   real Null/Zero/Unknown ambiguity the row cannot resolve, so the tie-break
//!   applies and it is unknown (see the deliverable's open questions). Army
//!   groups and fleets hard-code `None` (`:46884`, `:47000`) — never attempted,
//!   so absent.
//! * **`location_province_id`.** Only task-force rows ever carry one; army,
//!   army-group and fleet rows hard-code `None`. The gate used to be a single
//!   bit, and a cleared gate was read as "the read failed — unknown". That was
//!   wrong, and expensively so: `read_task_force_location_province_with_window`
//!   cleared it for THREE different reasons, and one of them is a measurement.
//!   Zero at `CTaskForce+0x18` is the engine's own absence marker for a task
//!   force that is not deployed to a province, which the d56 lane had already
//!   established ("Zero is the engine's absence marker and is NOT passed to the
//!   bound check"). Collapsing that into "unknown" made a lawfully locationless
//!   task force look like a gap, and the viewer's fleets census — which requires
//!   every row to be accounted for — turned a Present collection into
//!   `DomainUnavailable` over one row, on every hour of every sealed recording.
//!   The row now carries `COMPACT_OOB_ROW_FLAG_LOCATION_UNREADABLE` beside
//!   `COMPACT_OOB_ROW_FLAG_LOCATION`, and the three states are:
//!   `LOCATION` set — a measured province id, inside map space; both clear on a
//!   task force — the engine's absence marker, measured; `LOCATION_UNREADABLE`
//!   set — the read failed or the id was outside map space, the one genuinely
//!   unknown case. Both clear on the other three kinds still means the schema
//!   never attempts a location there, which `kind` distinguishes. Measured on
//!   `captures/oz_tip`, 473 folded hours: 99,354 task-force rows measured, 473
//!   the engine's absence (one per hour — CHI task force 601, naval mission 8
//!   `RESERVE_FLEET`), and zero unreadable.
//! * **`leader_trait_count` has no flag, and that is the trap.** The collector's
//!   value is an `Option<u32>` that the packer writes as `unwrap_or(0)`
//!   (`attach_log.rs:69719`) with **no gate bit**, so a `None` becomes an
//!   indistinguishable `0` on the wire — and the fold stores that `0` as a
//!   value (`world_fold.rs::fold_oob_command`). What disambiguates it is `leader_trait_status`,
//!   which is `"none"` when the row has no leader at all, `"exact"` when the
//!   traits were read (including an honest empty vector, `attach_log.rs:22312`),
//!   and `"validation_failed"` when the walk failed — but **`"none"` alone does
//!   not mean absent**: `read_army_oob_leader_detail` returns the trait status
//!   as `None` on every one of its six leader-read failure paths
//!   (`attach_log.rs:22246`-`:22272`) and the row builder collapses that to
//!   `"none"` with `unwrap_or` (`:46878`), so an army whose leader could not be
//!   read carries `leader_trait_status == NONE` while the concept still applies
//!   (122 live army rows on deepwar_t1). Only `leader_status` separates "this
//!   kind has no leader" from "the leader read failed", so the empty state here
//!   is the same function of `leader_status` that `leader_id` and `leader_skill`
//!   use. The mapper uses that rather than publishing a fabricated zero. It is
//!   stricter than the fold in exactly one
//!   place: a non-`EXACT` status carrying a non-zero count is rejected rather
//!   than silently reduced to an empty state, because that combination is
//!   unproducible by the collector (`read_unit_leader_traits` returns
//!   `(Some(_), "exact")` or `(None, "validation_failed")`, nothing else) and
//!   dropping the word quietly would be the one way this mapping could lose data.
//!   Measured: 0 of 1,519 rows on deepwar_t1 carry it.
//!
//! **The five `*_status` words are `Enum`, with cardinality 6.** The vocabulary
//! is the collector's own `COMPACT_OOB_STATUS_*` (`attach_log.rs:11380`-`:11385`):
//! `NONE=0, EXACT=1, UNKNOWN=2, OVERFLOW=3, VALIDATION_FAILED=4, MIXED=5`, and
//! `compact_oob_status_code` (`:57633`) is total onto exactly that set. **This
//! corrects the batch-2 survey**, which cited `lanes/army_battleplans_depth_live`'s
//! `STATUS_EXACT`/`STATUS_UNKNOWN` — that is d54's vocabulary, which agrees on
//! `1` and `2` but assigns `3`, `4`, `5` differently
//! (`UNSUPPORTED`/`OVERFLOW`/`VALIDATION_FAILED`). Only `{0, 1, 2}` occur on
//! deepwar_t1, but `3`, `4` and `5` are all reachable
//! (`compact_oob_member_ids` `:47022`, `read_compact_oob_army_group_parent`
//! `:47060`, `compact_oob_owner_consensus` `:47044`), so the cardinality is the
//! producer's codomain and not the capture's histogram.
//!
//! **`logical_member_count`, the serialized member rows, `member_count_engine`
//! and `missing_member_count` are four different quantities.** The first is the
//! collector's own live count, the second is what fitted in the frame
//! (`compact_oob_member_ids` truncates at `MAX_OOB_INLINE_IDS` and reports
//! `"overflow"`, `attach_log.rs:47022`), the third is the engine's own word and
//! the fourth their difference. On deepwar_t1 the first two happen to be equal on
//! all 1,519 rows and `missing_member_count` is `0` wherever it is present — a
//! property of one capture, not of the grammar, so none of the four is derived
//! from another here.
//!
//! **Deleted rows are removals, not records** (`world_fold.rs::fold_oob_command`):
//! the low flag bit erases the key from the accumulator. They become wire
//! tombstones in *both* collections, derived by `body::diff` from the state the
//! removal produces. [`record_from_command_row`] refuses one rather than
//! inventing a record. Note the fold applies **no** emptiness requirement to a
//! delete row — unlike d26's (`world_fold.rs::fold_division_equipment`) — so a delete row may legally declare a
//! member range, and those member rows are validated and then discarded with the
//! key. [`fold_payload`] reproduces that.
//!
//! **The expected-partial withhold is a fold-nothing path, and it sits after the
//! row loop.** d28 is one of exactly four domains with an expected-partial
//! tolerance (`compact_contract/src/lib.rs:147`-`:151`). Unlike d26, which
//! applies a tolerated-partial frame's rows and then marks the domain-hour
//! unknown, d28 **withholds**: `fold_oob_command` decodes and validates every row
//! and every member row, and only then — at `:14414`-`:14423`, *before* the
//! accumulator is touched — marks the domain-hour unknown and returns `Ok`
//! without folding anything, because the omitted command identities are not
//! serialized. So a tolerated-partial frame is not d28 data and its hour belongs
//! in the measurement's exclusion set, positioned after the structural and
//! per-row validation, which is where [`fold_payload`]'s caller puts it. It also
//! never sets the fold's `delta_baselines_seen`, because
//! `replace_or_sparse_after_first` (`:14424`) is never reached.
//!
//! **No derived field (derived-field contract).** Nothing here changes deterministically with the
//! clock; `frame_tick` is taken and ignored so every domain's boundary has one
//! shape.

use std::collections::{BTreeMap, BTreeSet};

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 28;
/// One row per command object: army group, army, fleet or task force.
pub const COMMANDS: u32 = 1;
/// One row per membership edge, keyed by its command plus the member id.
pub const MEMBERS: u32 = 2;

/// Field ids are stable and append-only; schema-identity contract forbids reusing an id for a
/// different field. `KIND` and `OBJECT_ID` appear in both collections because
/// they are the same field in both — the command's identity — which is what schema-identity contract
/// forbids reusing an id *against*.
pub mod field {
    pub const KIND: u32 = 1;
    pub const OBJECT_ID: u32 = 2;
    pub const PARENT_ID: u32 = 3;
    pub const OWNER_TAG_DB_ID: u32 = 4;
    pub const LEADER_ID: u32 = 5;
    pub const LEADER_SKILL: u32 = 6;
    pub const LOGICAL_MEMBER_COUNT: u32 = 7;
    pub const MEMBER_COUNT_ENGINE: u32 = 8;
    pub const MISSING_MEMBER_COUNT: u32 = 9;
    pub const LEADER_TRAIT_COUNT: u32 = 10;
    pub const PARENT_STATUS: u32 = 11;
    pub const MEMBER_STATUS: u32 = 12;
    pub const OWNER_STATUS: u32 = 13;
    pub const LEADER_STATUS: u32 = 14;
    pub const LEADER_TRAIT_STATUS: u32 = 15;
    pub const LOCATION_PROVINCE_ID: u32 = 16;
    pub const MEMBER_ID: u32 = 17;
}

// --- Payload grammar, from `world_fold.rs:970`-`:972` and `:1019`-`:1038`. ----

/// `OOB_COMMAND_HEADER_LEN`, `world_fold.rs::OOB_COMMAND_HEADER_LEN`.
pub const HEADER_LEN: usize = 48;
/// `OOB_COMMAND_ROW_LEN`, `world_fold.rs::OOB_COMMAND_ROW_LEN`.
pub const ROW_LEN: usize = 80;
/// `OOB_COMMAND_MEMBER_ROW_LEN`, `world_fold.rs::OOB_COMMAND_MEMBER_ROW_LEN`.
pub const MEMBER_ROW_LEN: usize = 8;

/// `OOB_KIND_*`, `world_fold.rs:1019`-`:1022`. A `kind` outside this set is a
/// rejected row (`world_fold.rs::fold_oob_command`), which is why `kind` needs no enum
/// cardinality on the wire — see [`schema`].
pub const KIND_ARMY: u32 = 1;
pub const KIND_ARMY_GROUP: u32 = 2;
pub const KIND_FLEET: u32 = 3;
pub const KIND_TASK_FORCE: u32 = 4;

/// `ROW_FLAG_DELETED_LOW_BIT`, `world_fold.rs::ROW_FLAG_DELETED_LOW_BIT`.
pub const FLAG_DELETED: u32 = 1;
/// `OOB_ROW_FLAG_*`, `world_fold.rs:1024`-`:1030`.
pub const FLAG_PARENT: u32 = 1 << 1;
pub const FLAG_OWNER: u32 = 1 << 2;
pub const FLAG_LEADER: u32 = 1 << 3;
pub const FLAG_LEADER_SKILL: u32 = 1 << 4;
pub const FLAG_MEMBER_COUNT_ENGINE: u32 = 1 << 5;
pub const FLAG_MISSING_MEMBER_COUNT: u32 = 1 << 6;
pub const FLAG_LOCATION: u32 = 1 << 7;
/// Task-force rows only: `CTaskForce+0x18` could not be read, or its id fell
/// outside map space. Both location flags clear on a task-force row is the
/// engine's own absence marker for a task force not deployed to a province —
/// measured state, not a gap.
pub const FLAG_LOCATION_UNREADABLE: u32 = 1 << 8;
/// `OOB_KNOWN_FLAGS`, `world_fold.rs::OOB_KNOWN_FLAGS`. Any other bit is a hard
/// error (`:14342`) — an unrecognised flag word means the row is not this
/// grammar, so nothing in it can be trusted.
pub const KNOWN_FLAGS: u32 = FLAG_DELETED
    | FLAG_PARENT
    | FLAG_OWNER
    | FLAG_LEADER
    | FLAG_LEADER_SKILL
    | FLAG_MEMBER_COUNT_ENGINE
    | FLAG_MISSING_MEMBER_COUNT
    | FLAG_LOCATION
    | FLAG_LOCATION_UNREADABLE;

/// `COMPACT_OOB_STATUS_*`, `collector/src/attach_log.rs:11380`-`:11385`. The
/// producer, `compact_oob_status_code` (`:57633`), is total onto this set.
pub const STATUS_NONE: u32 = 0;
pub const STATUS_EXACT: u32 = 1;
pub const STATUS_UNKNOWN: u32 = 2;
pub const STATUS_OVERFLOW: u32 = 3;
pub const STATUS_VALIDATION_FAILED: u32 = 4;
pub const STATUS_MIXED: u32 = 5;
/// One past the highest status code; schema-kind contract rejects a code at or above this.
pub const STATUS_CARDINALITY: u32 = 6;

/// The five status words, in row order, with their byte offsets
/// (`world_fold.rs::fold_oob_command`).
pub const STATUS_WORDS: [(u32, usize); 5] = [
    (field::PARENT_STATUS, 52),
    (field::MEMBER_STATUS, 56),
    (field::OWNER_STATUS, 60),
    (field::LEADER_STATUS, 64),
    (field::LEADER_TRAIT_STATUS, 68),
];

pub fn schema() -> Schema {
    use field as f;
    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![
            Collection {
                collection_id: COMMANDS,
                // Commands persist across hours and the frame grammar is
                // first-frame-replaces / later-frames-upsert / low-bit-removes
                // (`world_fold.rs::fold_oob_command`), which is exactly a keyed
                // set's grammar — and only a keyed set has deltas (collection-shape contract).
                shape: Shape::KeyedSet,
                fields: vec![
                    // The fold's key is the `(kind, object_id)` pair
                    // (`world_fold.rs::fold_oob_command`); `kind` is checked
                    // against a four-value set and `object_id` is zero-rejected,
                    // so both are concrete in every row.
                    //
                    // `kind` is `Unsigned`, not `Enum`, for two reasons that
                    // point the same way. key-encoding contract admits only `Unsigned`, `Signed`
                    // and `ContentId` as key components, so an `Enum` key is not
                    // expressible at all; and content-identity contract classes an engine
                    // constant/discriminator as `Unsigned` regardless of how few
                    // values it takes — this is the same call d20 makes for
                    // `div_type`. The four-value set is enforced in the mapper,
                    // where the fold enforces it.
                    Field::new(f::KIND, Kind::Unsigned).key(0),
                    // A runtime instance identity created during play: army 5 is
                    // a different army next campaign and no manifest resolves
                    // it. `Unsigned`, as d20 classes `army_id`.
                    Field::new(f::OBJECT_ID, Kind::Unsigned).key(1),
                    // Another command's `object_id` — a runtime instance
                    // identity, so `Unsigned`. Absent for the kinds that have no
                    // parent concept, unknown when the parent read failed; see
                    // the module docs.
                    Field::new(f::PARENT_ID, Kind::Unsigned)
                        .optional()
                        .unknown_capable(),
                    // A country tag db id — content-defined (content-identity contract): a manifest
                    // must resolve it for it to keep its meaning under another
                    // mod set. d63 keys its countries the same way and d20
                    // classes its `owner` the same way. Nullable because
                    // `"mixed"` is a measured "no single owner", unknown-capable
                    // because `"unknown"` is an unread owner. Never absent: the
                    // concept applies to every command kind.
                    Field::new(f::OWNER_TAG_DB_ID, Kind::ContentId)
                        .nullable()
                        .unknown_capable(),
                    // A unit-leader instance identity, `Unsigned`.
                    Field::new(f::LEADER_ID, Kind::Unsigned)
                        .optional()
                        .unknown_capable(),
                    // A small engine-side skill word (`0..=MAX_UNIT_LEADER_SKILL`,
                    // bound-tripped at `attach_log.rs:22296`), emitted raw by the
                    // fold with no scale. `Unsigned`, not `Continuous`.
                    Field::new(f::LEADER_SKILL, Kind::Unsigned)
                        .optional()
                        .unknown_capable(),
                    // The collector's own live member count. Always concrete —
                    // it is a `u32`, not an `Option`, in every one of the four
                    // row builders — so it declares no capability at all, and a
                    // command with no members is a real `Zero`.
                    Field::new(f::LOGICAL_MEMBER_COUNT, Kind::Unsigned),
                    Field::new(f::MEMBER_COUNT_ENGINE, Kind::Unsigned)
                        .optional()
                        .unknown_capable(),
                    Field::new(f::MISSING_MEMBER_COUNT, Kind::Unsigned)
                        .optional()
                        .unknown_capable(),
                    Field::new(f::LEADER_TRAIT_COUNT, Kind::Unsigned)
                        .optional()
                        .unknown_capable(),
                    // The five capture-quality codes. `Enum` with the producer's
                    // full codomain (`STATUS_CARDINALITY`), never the histogram
                    // of one capture. Always concrete: the packer writes a code
                    // for every row, so none of the five declares a capability.
                    Field::new(f::PARENT_STATUS, Kind::Enum).enum_card(STATUS_CARDINALITY),
                    Field::new(f::MEMBER_STATUS, Kind::Enum).enum_card(STATUS_CARDINALITY),
                    Field::new(f::OWNER_STATUS, Kind::Enum).enum_card(STATUS_CARDINALITY),
                    Field::new(f::LEADER_STATUS, Kind::Enum).enum_card(STATUS_CARDINALITY),
                    Field::new(f::LEADER_TRAIT_STATUS, Kind::Enum).enum_card(STATUS_CARDINALITY),
                    // A province id: content-defined (content-identity contract), exactly as d20
                    // classes `location_province`. A ContentId **value**, not a
                    // key, so it is the lighter retrofit class.
                    Field::new(f::LOCATION_PROVINCE_ID, Kind::ContentId)
                        .optional()
                        .unknown_capable(),
                ],
            },
            Collection {
                collection_id: MEMBERS,
                shape: Shape::KeyedSet,
                // Key only. The membership edge IS the fact; the member row's
                // only other word is a must-be-zero reserved (current
                // `:14474`-`:14476`). The
                // keyed set's own order is ascending `member_id`, which is the
                // order every range this schema stores is already in — see the
                // module docs, and [`member_range_is_ordered`], which is where
                // the frames it does not accept are refused.
                fields: vec![
                    Field::new(f::KIND, Kind::Unsigned).key(0),
                    Field::new(f::OBJECT_ID, Kind::Unsigned).key(1),
                    // A division, army, task-force or ship instance id
                    // depending on the owning kind — a runtime instance
                    // identity in every case, so `Unsigned`.
                    Field::new(f::MEMBER_ID, Kind::Unsigned).key(2),
                ],
            },
        ],
        event_part: None,
    }
}

// --- The boundary mapping ----------------------------------------------------

/// The 48-byte payload header (`world_fold.rs::fold_oob_command`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub row_count: u32,
    pub changed_rows: u32,
    pub status_flags: u32,
    pub root_rows: u32,
    pub observed_army_rows: u32,
    pub observed_army_group_rows: u32,
    pub member_count: u32,
    pub deleted_rows: u32,
    pub observed_fleet_rows: u32,
    pub observed_task_force_rows: u32,
}

/// One parsed command row. `member_first`/`member_count` are kept because the
/// caller needs them to find the row's members, not because they are state: they
/// index the frame's shared member array and are re-packed from emission order
/// every frame (`attach_log.rs:69700`-`:69716`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandRow {
    pub kind: u32,
    pub object_id: u32,
    pub flags: u32,
    pub member_first: usize,
    pub member_count: usize,
    /// The low flag bit: remove this key. Never a record; a wire tombstone.
    pub deleted: bool,
}

/// Read the 48-byte payload header and nothing else
/// (`world_fold.rs::fold_oob_command`).
///
/// Exposing the header read is what keeps it from becoming a second, hand-rolled
/// copy of the header layout in every caller that needs `status_flags` before it
/// decides whether the record is measurable at all. It applies no invariant
/// beyond the length needed to read the words themselves.
pub fn peek_header(payload: &[u8]) -> core::result::Result<PayloadHeader, String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "oob_command payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    Ok(PayloadHeader {
        elapsed_wall_ms: u64::from_le_bytes(payload[0..8].try_into().expect("8 bytes")),
        row_count: u32_at(payload, 8),
        changed_rows: u32_at(payload, 12),
        status_flags: u32_at(payload, 16),
        root_rows: u32_at(payload, 20),
        observed_army_rows: u32_at(payload, 24),
        observed_army_group_rows: u32_at(payload, 28),
        member_count: u32_at(payload, 32),
        deleted_rows: u32_at(payload, 36),
        observed_fleet_rows: u32_at(payload, 40),
        observed_task_force_rows: u32_at(payload, 44),
    })
}

/// Split a d28 DATA payload into its header, its command rows and its member
/// rows, applying every structural check `fold_oob_command` applies before it
/// starts folding (`world_fold.rs::fold_oob_command`).
///
/// This lives here rather than in each caller so the header grammar has exactly
/// one implementation: a measurement harness that re-derives it is a harness
/// that can drift from the recorder without anything failing. It is fail-closed
/// and total — no input of any length panics.
///
/// **No mode parameter, deliberately.** d28's structural invariants are the
/// exact length and the two `ensure_count_at_most` bounds (`:14308`-`:14322`),
/// and none of them depends on the frame mode. In particular the fold does
/// **not** require `changed_rows == row_count` in any mode — d26's fold does, and
/// copying that here would reject healthy frames.
///
/// **This layer's own member-ordering check is NOT here.** It is not a
/// structural rule about the payload, it is a representability rule about the
/// state the frame leaves behind, so it belongs where that state is built —
/// [`fold_payload`]'s upsert loop, via [`member_range_is_ordered`].
pub fn split_payload(payload: &[u8]) -> core::result::Result<(PayloadHeader, &[u8], &[u8]), String> {
    let header = peek_header(payload)?;

    // Checked arithmetic throughout: both counts are attacker-shaped words from
    // a file on disk, and either product overflows `usize` for a large enough
    // one. `world_fold.rs::fold_oob_command` uses saturating arithmetic, which turns an
    // overflowing claim into a length that cannot match; rejecting is the same
    // outcome reached without relying on that.
    let rows_len = (header.row_count as usize)
        .checked_mul(ROW_LEN)
        .ok_or_else(|| format!("oob_command row_count {} overflows", header.row_count))?;
    let members_len = (header.member_count as usize)
        .checked_mul(MEMBER_ROW_LEN)
        .ok_or_else(|| format!("oob_command member_count {} overflows", header.member_count))?;
    let expected = HEADER_LEN
        .checked_add(rows_len)
        .and_then(|len| len.checked_add(members_len))
        .ok_or_else(|| "oob_command payload length overflows".to_string())?;
    if payload.len() != expected {
        return Err(format!("oob_command len {} != {expected}", payload.len()));
    }

    // `ensure_count_at_most`, `world_fold.rs:14308`-`:14322`. Both are `<=`,
    // unconditionally.
    if header.changed_rows > header.row_count {
        return Err(format!(
            "oob_command changed_rows {} > row_count {}",
            header.changed_rows, header.row_count
        ));
    }
    if header.deleted_rows > header.row_count {
        return Err(format!(
            "oob_command deleted_rows {} > row_count {}",
            header.deleted_rows, header.row_count
        ));
    }

    let rows = &payload[HEADER_LEN..HEADER_LEN + rows_len];
    let members = &payload[HEADER_LEN + rows_len..];
    Ok((header, rows, members))
}

/// This layer's own check on one command's member ids: they must be strictly
/// ascending, because [`MEMBERS`]' `(kind, object_id, member_id)` key orders them
/// that way and a keyed set can express no other order.
///
/// The fold enforces no ordering on a member range — its member loop rejects a
/// zero id, an unowned child and a repeat within one row, and nothing else
/// (current `world_fold.rs::fold_oob_command`) — and it stores the range as an
/// ordered `Vec<u32>` (`OobCommandState.members`, current `:23623`, filled at
/// `world_fold.rs::fold_oob_command`) that the recorder projects verbatim as an ordered JSON array
/// (`world_fold.rs::build_order_of_battle`). On a fold-accepted range that is *not* ascending,
/// mirroring would silently re-sort the sequence, which is the data-loss defect
/// itself. So this schema refuses that frame instead — d32's precedent exactly
/// (`d32.rs:146`-`:171`): stricter than the fold, on input the fold would accept,
/// and **not stricter than the producer**, which canonicalises every range before
/// packing it (`attach_log.rs:47008`-`:47009`, `:47058`-`:47059`,
/// `:47164`-`:47165`, `:47884`). Measured: 0 refusals across the 1,000 ranked
/// ranges of `deepwar_t1`.
///
/// **It takes the already-sliced ids, not the payload**, so it cannot be handed
/// a range and a member array that disagree, and it cannot index out of bounds
/// on any input. [`fold_payload`] is the only caller, and it calls this from its
/// upsert loop — which is what confines the rule to the rows the frame leaves
/// behind. Two classes are therefore exempt without a special case:
///
///   * **delete rows**, whose range the fold validates and then discards with the
///     key (its `continue` at `:14509`, before the slice at `:14526`);
///   * **withheld tolerated-partial frames**, which the fold validates in full
///     and then abandons without touching the accumulator (`:14488`-`:14496`) —
///     see [`validate_withheld_payload`].
///
/// Refusing either would reject a frame whose folded state this schema
/// represents perfectly well. Duplicate `(kind, object_id)` needs no carve-out
/// either: unlike d32's last-writer-wins wing key, the fold rejects a repeated
/// command identity outright (`:14411`-`:14413`), so every live row survives.
pub fn member_range_is_ordered(
    kind: u32,
    object_id: u32,
    member_ids: &[u32],
) -> core::result::Result<(), String> {
    for pair in member_ids.windows(2) {
        if pair[1] <= pair[0] {
            return Err(format!(
                "oob_command ({kind}, {object_id}) carries member {} after {}; the wire key \
                 (kind, object_id, member_id) orders this command's members ascending and \
                 cannot carry the frame's sequence",
                pair[1], pair[0]
            ));
        }
    }
    Ok(())
}

/// Parse one 80-byte command row, applying every per-row check the fold applies
/// (`world_fold.rs::fold_oob_command`) except the member-range bounds and the
/// cross-row uniqueness rules, which need the whole frame and live in
/// [`split_rows`] / [`fold_payload`].
pub fn parse_command_row(row: &[u8]) -> core::result::Result<CommandRow, String> {
    if row.len() != ROW_LEN {
        return Err(format!("oob_command row length {} != {ROW_LEN}", row.len()));
    }
    let kind = u32_at(row, 0);
    if !matches!(
        kind,
        KIND_ARMY | KIND_ARMY_GROUP | KIND_FLEET | KIND_TASK_FORCE
    ) {
        return Err(format!("oob_command row has invalid kind {kind}"));
    }
    let object_id = u32_at(row, 4);
    if object_id == 0 {
        return Err("oob_command row has zero object_id".into());
    }
    let flags = u32_at(row, 8);
    if flags & !KNOWN_FLAGS != 0 {
        return Err(format!("oob_command row has unknown flags {flags:#x}"));
    }
    // `world_fold.rs::fold_oob_command`: exactly two of the seven gates are
    // *validated* zero-when-clear. The other five are applied at insert only, so
    // this mapper does not invent a check the fold does not make — see the
    // deliverable's open questions on the asymmetry.
    if flags & FLAG_LEADER_SKILL == 0 && u32_at(row, 24) != 0 {
        return Err("oob_command leader_skill present without flag".into());
    }
    if flags & FLAG_LOCATION == 0 && u32_at(row, 72) != 0 {
        return Err("oob_command location present without flag".into());
    }
    // The two location flags are mutually exclusive and only a task force can be
    // unreadable, matching `world_fold.rs::fold_oob_command`.
    if flags & FLAG_LOCATION != 0 && flags & FLAG_LOCATION_UNREADABLE != 0 {
        return Err("oob_command location is both measured and unreadable".into());
    }
    if flags & FLAG_LOCATION_UNREADABLE != 0 && kind != KIND_TASK_FORCE {
        return Err("oob_command non-task-force row claims an unreadable location".into());
    }
    // `world_fold.rs::fold_oob_command`.
    if u32_at(row, 76) != 0 {
        return Err("oob_command row reserved field is nonzero".into());
    }
    Ok(CommandRow {
        kind,
        object_id,
        flags,
        member_first: u32_at(row, 28) as usize,
        member_count: u32_at(row, 32) as usize,
        deleted: flags & FLAG_DELETED != 0,
    })
}

/// Parse one 8-byte member row, applying the fold's per-row checks
/// (`world_fold.rs::fold_oob_command`), and return its `member_id`.
pub fn parse_member_row(row: &[u8]) -> core::result::Result<u32, String> {
    if row.len() != MEMBER_ROW_LEN {
        return Err(format!(
            "oob_command member row length {} != {MEMBER_ROW_LEN}",
            row.len()
        ));
    }
    let member_id = u32_at(row, 0);
    if u32_at(row, 4) != 0 {
        return Err("oob_command member reserved field is nonzero".into());
    }
    if member_id == 0 {
        return Err("oob_command member id is zero".into());
    }
    Ok(member_id)
}

/// The half-open index range of one command's member rows within the frame's
/// shared member array, checked exactly as the fold checks it
/// (`world_fold.rs::fold_oob_command`).
///
/// One derivation of the range rule, so the check and the use cannot disagree.
/// Unlike d26's ranges these may **not** overlap — the fold proves ownership is
/// injective (`world_fold.rs::fold_oob_command`) — but that is a whole-frame property and lives
/// in [`fold_payload`]; this is the per-row bound.
pub fn member_index_range(
    row: &CommandRow,
    member_count: usize,
) -> core::result::Result<core::ops::Range<usize>, String> {
    let end = row
        .member_first
        .checked_add(row.member_count)
        .ok_or_else(|| "oob_command member range overflow".to_string())?;
    if end > member_count {
        return Err(format!(
            "oob_command member range {}+{} exceeds {member_count}",
            row.member_first, row.member_count
        ));
    }
    Ok(row.member_first..end)
}

/// The [`COMMANDS`] key of one command: `(kind, object_id)`
/// (`world_fold.rs::fold_oob_command`).
pub fn command_key(kind: u32, object_id: u32) -> Key {
    key(&[kind as u64, object_id as u64])
}

/// The [`MEMBERS`] key of one membership edge: its command's key plus the member
/// id, which is the identity the fold's per-row uniqueness check is on
/// (`world_fold.rs::fold_oob_command`).
pub fn member_key(kind: u32, object_id: u32, member_id: u32) -> Key {
    key(&[kind as u64, object_id as u64, member_id as u64])
}

/// The [`COMMANDS`] key of one raw command row.
pub fn key_from_row(row: &[u8]) -> core::result::Result<Key, String> {
    let parsed = parse_command_row(row)?;
    Ok(command_key(parsed.kind, parsed.object_id))
}

/// The [`MEMBERS`] key of one raw member row under one raw command row.
pub fn key_from_member_row(
    command_row: &[u8],
    member_row: &[u8],
) -> core::result::Result<Key, String> {
    let parsed = parse_command_row(command_row)?;
    let member_id = parse_member_row(member_row)?;
    Ok(member_key(parsed.kind, parsed.object_id, member_id))
}

/// One compact row of the `commands` table, as the collector emits it today,
/// onto its stored Record. `row` is the raw 80-byte row slice (the caller has
/// already consumed the payload header via [`split_payload`]); d28 has exactly
/// one row form, so length selects nothing and a wrong length is rejected.
/// `frame_tick` is derived-field contract's derived-field input (stored = v + c * (frame_tick >> u));
/// d28 declares no derived field, so it is taken and ignored — every domain's
/// mapper has the same shape.
///
/// Fail-closed throughout, mirroring `fold_oob_command`: a row the fold would
/// reject is an `Err` here, never a coerced value. A **deleted** row is an `Err`
/// too — it is a removal (`world_fold.rs::fold_oob_command`) and has no record; a
/// caller must map it to a wire tombstone, and quietly handing one back a
/// gates-all-clear row would turn a delete into an upsert. Nothing in this
/// function can panic on any input.
pub fn record_from_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    // d28 has no derived field; derived-field contract's transform input is unused. Named rather
    // than `_frame_tick` because the signature is the cross-domain contract.
    let _ = frame_tick;
    let parsed = parse_command_row(row)?;
    if parsed.deleted {
        return Err(format!(
            "oob_command ({}, {}) is a delete row, not a record",
            parsed.kind, parsed.object_id
        ));
    }
    let flags = parsed.flags;
    let kind = parsed.kind;

    // The five status words first: they gate three of the four-state decisions
    // below, so an out-of-vocabulary code must stop the row before it is read as
    // a gate. schema-kind contract: "a value the writer cannot map into 0..n-1 is a failed sample
    // and a typed gap, never a coerced code."
    let mut statuses = [0u32; 5];
    for (i, (_, offset)) in STATUS_WORDS.into_iter().enumerate() {
        let code = u32_at(row, offset);
        if code >= STATUS_CARDINALITY {
            return Err(format!(
                "oob_command status word at +{offset} is {code}, outside the \
                 0..{STATUS_CARDINALITY} vocabulary"
            ));
        }
        statuses[i] = code;
    }
    // Indices are `STATUS_WORDS`' own order: parent, member, owner, leader,
    // leader_trait (`world_fold.rs::fold_oob_command`).
    let parent_status = statuses[0];
    let owner_status = statuses[2];
    let leader_status = statuses[3];
    let leader_trait_status = statuses[4];

    // `leader_trait_count` has no gate bit, so the status word is the only thing
    // that separates "no traits" from "not read" — see the module docs. A
    // non-zero count under a non-EXACT status is unproducible by the collector
    // and would be silently dropped by the mapping below, which is the one way
    // this boundary could lose data, so it is refused instead.
    let leader_trait_count = u32_at(row, 48);
    if leader_trait_status != STATUS_EXACT && leader_trait_count != 0 {
        return Err(format!(
            "oob_command leader_trait_count {leader_trait_count} with \
             leader_trait_status {leader_trait_status} != EXACT"
        ));
    }

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::PARENT_ID => gated(
                flags & FLAG_PARENT != 0,
                || Value::Unsigned(u32_at(row, 12) as u64),
                // Army groups and fleets are roots: `parent_status: "none"`
                // (`attach_log.rs:46888`, `:47000`). Anything else is a failed
                // or indistinguishable read.
                if parent_status == STATUS_NONE {
                    Value::Absent
                } else {
                    Value::Unknown
                },
            ),
            field::OWNER_TAG_DB_ID => gated(
                flags & FLAG_OWNER != 0,
                || Value::ContentId(u32_at(row, 16) as u64),
                // `"mixed"` is a measured "there is no single owner"; anything
                // else is an unread owner. The fold's `0` sentinel
                // (`world_fold.rs::fold_oob_command`) is never emitted as a value.
                if owner_status == STATUS_MIXED {
                    Value::Null
                } else {
                    Value::Unknown
                },
            ),
            field::LEADER_ID => gated(
                flags & FLAG_LEADER != 0,
                || Value::Unsigned(u32_at(row, 20) as u64),
                leader_absent_or_unknown(leader_status),
            ),
            field::LEADER_SKILL => gated(
                flags & FLAG_LEADER_SKILL != 0,
                || Value::Unsigned(u32_at(row, 24) as u64),
                // No leader means no skill; a leader whose skill read failed
                // means an uncaptured skill.
                leader_absent_or_unknown(leader_status),
            ),
            // Always concrete in every one of the four row builders.
            field::LOGICAL_MEMBER_COUNT => Value::Unsigned(u32_at(row, 36) as u64),
            field::MEMBER_COUNT_ENGINE => gated(
                flags & FLAG_MEMBER_COUNT_ENGINE != 0,
                || Value::Unsigned(u32_at(row, 40) as u64),
                // Army groups never attempt it (`attach_log.rs:46884`).
                if kind == KIND_ARMY_GROUP {
                    Value::Absent
                } else {
                    Value::Unknown
                },
            ),
            field::MISSING_MEMBER_COUNT => gated(
                flags & FLAG_MISSING_MEMBER_COUNT != 0,
                || Value::Unsigned(u32_at(row, 44) as u64),
                // Army groups and fleets never attempt it (`:46884`, `:47000`).
                if kind == KIND_ARMY_GROUP || kind == KIND_FLEET {
                    Value::Absent
                } else {
                    Value::Unknown
                },
            ),
            field::LEADER_TRAIT_COUNT => {
                if leader_trait_status == STATUS_EXACT {
                    Value::Unsigned(leader_trait_count as u64)
                } else {
                    // NOT `leader_trait_status == NONE => Absent`.
                    // `read_army_oob_leader_detail` returns the trait status as
                    // `None` on **every** leader-read failure path
                    // (`attach_log.rs:22246`-`:22272`), and the row builder turns
                    // that into `"none"` with `unwrap_or("none")` (`:46878`). So
                    // a `"none"` trait status on an army means "the leader read
                    // failed", not "this kind has no leader" — 122 live army
                    // rows on deepwar_t1 are exactly that. Only `leader_status`
                    // separates the two, which is why this is the same decision
                    // as `leader_id` and `leader_skill`.
                    leader_absent_or_unknown(leader_status)
                }
            }
            field::LOCATION_PROVINCE_ID => gated(
                flags & FLAG_LOCATION != 0,
                || Value::ContentId(u32_at(row, 72) as u64),
                // Only task-force rows ever carry a location; the other three
                // kinds never attempt one, so the concept is unsupported for
                // them. For a task force the cleared gate is no longer one
                // state: `FLAG_LOCATION_UNREADABLE` is the read that failed or
                // the id outside map space (unknown), and both flags clear is
                // the engine's own absence marker for a task force not deployed
                // to a province — a measurement, so `Absent`. Mapping that to
                // `Unknown` is what made the viewer's fleets census treat a
                // lawfully locationless task force as a gap.
                if kind == KIND_TASK_FORCE && flags & FLAG_LOCATION_UNREADABLE != 0 {
                    Value::Unknown
                } else {
                    Value::Absent
                },
            ),
            other => {
                let Some(index) = STATUS_WORDS.iter().position(|(id, _)| *id == other) else {
                    // A collection this mapper does not know how to fill.
                    // Filling it with a placeholder would publish invented data
                    // under a field id that means something else.
                    return Err(format!(
                        "oob_command commands has no mapping for field id {other}"
                    ));
                };
                Value::Enum(statuses[index])
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// One compact member row onto its stored Record. [`MEMBERS`] carries no value
/// fields — the membership edge is the whole fact and its key holds it — so the
/// record is empty and this exists to keep the boundary uniform: the row is
/// still validated (`world_fold.rs::fold_oob_command`) and a collection carrying a
/// value field this mapper cannot fill is still refused.
///
/// The *order* of a command's member range is not this function's business: it
/// is a whole-range property, checked once by [`member_range_is_ordered`] where
/// [`fold_payload`] turns the range into state.
pub fn record_from_member_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    let _ = parse_member_row(row)?;
    if let Some(f) = collection.value_fields().next() {
        return Err(format!(
            "oob_command members has no mapping for field id {}",
            f.field_id
        ));
    }
    Ok(Record::new(Vec::new()))
}

// --- The frame-level fold ----------------------------------------------------

/// One command in the accumulator: its [`COMMANDS`] record and its member ids in
/// the fold's own stored order. This is `OobCommandState` (current
/// `world_fold.rs:23616`) with the fields the wire schema carries, plus the
/// member vector the child table flattens into.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandEntry {
    pub record: Record,
    /// Wire order, exactly as the fold's slice stores it (current
    /// `world_fold.rs::fold_oob_command`) — which, on every frame this schema accepts, is
    /// ascending: see [`member_range_is_ordered`].
    pub members: Vec<u32>,
}

/// The d28 accumulator: `world_fold.rs:1677`'s
/// `BTreeMap<OobCommandKey, OobCommandState>`.
///
/// One d28 record is **not** one state — the domain is a full first frame
/// followed by sparse upserts (`world_fold.rs::fold_oob_command`) — so the frame-level fold is a
/// public, tested boundary here rather than a private copy in each caller.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub commands: BTreeMap<(u32, u32), CommandEntry>,
}

impl State {
    pub fn new() -> Self {
        Self::default()
    }

    /// The two keyed states this accumulator represents, in schema order.
    pub fn keyed_states(&self) -> Vec<KeyedState> {
        let mut commands = KeyedState::new();
        let mut members = KeyedState::new();
        for ((kind, object_id), entry) in &self.commands {
            commands.insert(command_key(*kind, *object_id), entry.record.clone());
            for member_id in &entry.members {
                members.insert(
                    member_key(*kind, *object_id, *member_id),
                    Record::new(Vec::new()),
                );
            }
        }
        vec![commands, members]
    }
}

/// `replace_or_sparse_after_first` for the frames that reach the fold
/// (`world_fold.rs::replace_or_sparse_after_first`).
///
/// A tolerated-partial frame never reaches it — d28's withhold at `:14414`
/// returns first — so the expected-partial arm of that helper is unreachable
/// here and is deliberately not modelled. `baseline_seen` is the caller's
/// `delta_baselines_seen` membership for domain 28.
pub fn frame_is_sparse(delta: bool, baseline_seen: bool) -> bool {
    delta || baseline_seen
}

/// Apply one d28 payload to the accumulator exactly as `fold_oob_command` does
/// (`world_fold.rs::fold_oob_command`), returning its header.
///
/// `sparse` is [`frame_is_sparse`]'s answer: `false` clears the accumulator
/// first (`world_fold.rs::fold_oob_command`), `true` upserts into it.
///
/// The caller is responsible for the four fold-nothing paths, all of which
/// return before this function's effect: failure status (`world_fold.rs::fold_oob_command`),
/// expected-partial-before-baseline (`world_fold.rs::fold_oob_command`), delta-before-baseline (`world_fold.rs::fold_oob_command`)
/// and — d28's own — the expected-partial **withhold** at `world_fold.rs::fold_oob_command`, which sits
/// *after* every row has been decoded and validated. That ordering matters and
/// is why this function is fail-closed on row content regardless of status: a
/// tolerated-partial frame whose rows are malformed is a corrupt frame in the
/// recorder too. A caller validating a **withheld** frame must therefore use
/// [`validate_withheld_payload`] rather than this function with a throwaway
/// accumulator: this one additionally applies [`member_range_is_ordered`], and a
/// withheld frame's rows never become state for that rule to protect.
///
/// Fail-closed: every rejection the fold makes is an `Err` here, and a frame this
/// rejects leaves the accumulator exactly as it was. Total: no payload of any
/// content panics.
pub fn fold_payload(
    state: &mut State,
    payload: &[u8],
    sparse: bool,
    commands: &Collection,
    members: &Collection,
) -> core::result::Result<PayloadHeader, String> {
    fold_payload_inner(state, payload, sparse, commands, members, true)
}

/// Everything [`fold_payload`] validates, on a frame the recorder validates in
/// full and then **withholds**: `fold_oob_command` decodes and checks every row
/// and every member row, and only then — at `world_fold.rs::fold_oob_command`,
/// before the accumulator is touched — marks the domain-hour unknown and returns
/// without folding anything. A harness must still run that validation, because a
/// malformed tolerated-partial frame is a corrupt stream in the recorder too.
///
/// It differs from [`fold_payload`] in exactly one way: it does **not** apply
/// [`member_range_is_ordered`]. That rule exists because mirroring an unsorted
/// range into a keyed set would silently re-sort a sequence the accumulator
/// would otherwise hold — and a withheld frame's rows never reach the
/// accumulator, so there is no sequence to lose. Refusing one would reject a
/// frame whose folded state (nothing) this schema represents perfectly well.
/// This is the same reasoning that exempts a delete row's range.
///
/// The frame has no effect, so there is no accumulator parameter and no `sparse`:
/// the fold's `replace_or_sparse_after_first` is never reached on this path
/// either (`:14498`).
pub fn validate_withheld_payload(
    payload: &[u8],
    commands: &Collection,
    members: &Collection,
) -> core::result::Result<PayloadHeader, String> {
    let mut discarded = State::new();
    fold_payload_inner(&mut discarded, payload, true, commands, members, false)
}

/// `tighten` is whether this layer's own [`member_range_is_ordered`] rule
/// applies — true for a frame that is folded, false for one that is validated and
/// withheld. Everything else is identical, so the two public entry points cannot
/// drift apart in their reading of the fold.
fn fold_payload_inner(
    state: &mut State,
    payload: &[u8],
    sparse: bool,
    commands: &Collection,
    members: &Collection,
    tighten: bool,
) -> core::result::Result<PayloadHeader, String> {
    let (header, row_bytes, member_bytes) = split_payload(payload)?;
    let member_count = header.member_count as usize;

    // Pass 1 over the command rows, in the fold's own order
    // (`world_fold.rs::fold_oob_command`): per-row validity, key uniqueness across
    // the frame, and member-range ownership.
    let mut rows = Vec::with_capacity(header.row_count as usize);
    let mut row_keys = BTreeSet::new();
    let mut member_owners: Vec<Option<usize>> = vec![None; member_count];
    for (index, bytes) in row_bytes.chunks_exact(ROW_LEN).enumerate() {
        let row = parse_command_row(bytes).map_err(|e| format!("row {index}: {e}"))?;
        // `world_fold.rs::fold_oob_command`: a key serialized twice in one frame is
        // a rejected frame, never a last-writer-wins overwrite.
        if !row_keys.insert((row.kind, row.object_id)) {
            return Err(format!(
                "oob_command row {index} duplicates identity ({}, {})",
                row.kind, row.object_id
            ));
        }
        let range = member_index_range(&row, member_count).map_err(|e| format!("row {index}: {e}"))?;
        // `:14355`-`:14359`: ownership is injective, so the ranges do not
        // overlap. This runs for **every** row including a delete row, exactly
        // as the fold does — the fold's delete branch is after the row loop.
        for owner in &mut member_owners[range] {
            if owner.replace(index).is_some() {
                return Err("oob_command member ranges overlap".into());
            }
        }
        rows.push(row);
    }
    if rows.len() != header.row_count as usize {
        return Err(format!(
            "oob_command decoded {} rows, header claims {}",
            rows.len(),
            header.row_count
        ));
    }

    // Pass 2 over the member rows (`world_fold.rs::fold_oob_command`): every member
    // row is decoded and checked before any row is applied, so a single invalid
    // member rejects the whole frame — and every member row must be owned
    // (totality) and unique within its owner.
    let mut member_ids = Vec::with_capacity(member_count);
    let mut ids_by_row: Vec<BTreeSet<u32>> = vec![BTreeSet::new(); rows.len()];
    for (index, bytes) in member_bytes.chunks_exact(MEMBER_ROW_LEN).enumerate() {
        let member_id = parse_member_row(bytes).map_err(|e| format!("member row {index}: {e}"))?;
        let Some(owner) = member_owners[index] else {
            return Err(format!("oob_command member child {index} is unowned"));
        };
        if !ids_by_row[owner].insert(member_id) {
            return Err(format!(
                "oob_command row {owner} duplicates member id {member_id}"
            ));
        }
        // Validating through the member collection's own mapper keeps the
        // schema's arity check on this path too.
        let _ = record_from_member_row(members, bytes, 0)
            .map_err(|e| format!("member row {index}: {e}"))?;
        member_ids.push(member_id);
    }
    if member_ids.len() != member_count {
        return Err(format!(
            "oob_command decoded {} member rows, header claims {member_count}",
            member_ids.len()
        ));
    }

    // Build every surviving command before touching the accumulator, so a frame
    // this rejects leaves it exactly as it was.
    let mut removals: Vec<(u32, u32)> = Vec::new();
    let mut upserts: Vec<((u32, u32), CommandEntry)> = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let key = (row.kind, row.object_id);
        if row.deleted {
            // `world_fold.rs::fold_oob_command`. The fold imposes no emptiness
            // requirement on a delete row, so a member range it declared is
            // validated above and then discarded with the key.
            removals.push(key);
            continue;
        }
        let bytes = &row_bytes[index * ROW_LEN..(index + 1) * ROW_LEN];
        let record =
            record_from_row(commands, bytes, 0).map_err(|e| format!("row {index}: {e}"))?;
        let range = member_index_range(row, member_count).map_err(|e| format!("row {index}: {e}"))?;
        let row_members = &member_ids[range];
        // This layer's own rule, applied here and nowhere else: this is the one
        // place a member sequence becomes state, so delete rows (which `continue`
        // above) and withheld frames (`tighten == false`) are exempt without a
        // special case. See `member_range_is_ordered`.
        if tighten {
            member_range_is_ordered(row.kind, row.object_id, row_members)
                .map_err(|e| format!("row {index}: {e}"))?;
        }
        upserts.push((
            key,
            CommandEntry {
                record,
                members: row_members.to_vec(),
            },
        ));
    }

    // `world_fold.rs::fold_oob_command`: only a non-sparse frame replaces the
    // world. Keys are unique within the frame, so `removals` and `upserts` name
    // disjoint keys and their order does not matter.
    if !sparse {
        state.commands.clear();
    }
    for key in removals {
        state.commands.remove(&key);
    }
    for (key, entry) in upserts {
        state.commands.insert(key, entry);
    }

    Ok(header)
}

// --- helpers -----------------------------------------------------------------

/// A gated field: the value when the gate is set, the caller's chosen empty
/// state when it is clear. The value side is a closure so the word is never read
/// on a clear gate — the fold does not read it either.
fn gated(present: bool, value: impl FnOnce() -> Value, empty: Value) -> Value {
    if present {
        value()
    } else {
        empty
    }
}

/// The empty state of a leader-derived field on a clear gate: `"none"` means the
/// kind has no leader concept at all (absent), anything else is a read that was
/// attempted and failed (unknown).
fn leader_absent_or_unknown(leader_status: u32) -> Value {
    if leader_status == STATUS_NONE {
        Value::Absent
    } else {
        Value::Unknown
    }
}

/// Callers reach these only after a length check, so the slices are in bounds.
fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{diff, KeyedState};
    use crate::TickGrid;

    fn grid() -> TickGrid {
        TickGrid::new(3).expect("3 is a legal grid")
    }

    fn commands() -> Collection {
        schema().collections[0].clone()
    }

    fn members() -> Collection {
        schema().collections[1].clone()
    }

    fn index_of(c: &Collection, id: u32) -> usize {
        c.value_fields()
            .position(|f| f.field_id == id)
            .expect("field is in the schema")
    }

    /// The twenty words of one command row, in row order
    /// (`world_fold.rs::fold_oob_command`).
    #[derive(Clone, Copy)]
    struct RawRow {
        kind: u32,
        object_id: u32,
        flags: u32,
        parent_id: u32,
        owner_tag_db_id: u32,
        leader_id: u32,
        leader_skill: u32,
        member_first: u32,
        member_count: u32,
        logical_member_count: u32,
        member_count_engine: u32,
        missing_member_count: u32,
        leader_trait_count: u32,
        parent_status: u32,
        member_status: u32,
        owner_status: u32,
        leader_status: u32,
        leader_trait_status: u32,
        location_province_id: u32,
    }

    impl Default for RawRow {
        fn default() -> Self {
            Self {
                kind: KIND_ARMY,
                object_id: 1,
                flags: 0,
                parent_id: 0,
                owner_tag_db_id: 0,
                leader_id: 0,
                leader_skill: 0,
                member_first: 0,
                member_count: 0,
                logical_member_count: 0,
                member_count_engine: 0,
                missing_member_count: 0,
                leader_trait_count: 0,
                parent_status: STATUS_NONE,
                member_status: STATUS_NONE,
                owner_status: STATUS_NONE,
                leader_status: STATUS_NONE,
                leader_trait_status: STATUS_NONE,
                location_province_id: 0,
            }
        }
    }

    impl RawRow {
        fn bytes(&self) -> Vec<u8> {
            let words = [
                self.kind,
                self.object_id,
                self.flags,
                self.parent_id,
                self.owner_tag_db_id,
                self.leader_id,
                self.leader_skill,
                self.member_first,
                self.member_count,
                self.logical_member_count,
                self.member_count_engine,
                self.missing_member_count,
                self.leader_trait_count,
                self.parent_status,
                self.member_status,
                self.owner_status,
                self.leader_status,
                self.leader_trait_status,
                self.location_province_id,
                0, // reserved
            ];
            let mut out = Vec::with_capacity(ROW_LEN);
            for w in words {
                out.extend_from_slice(&w.to_le_bytes());
            }
            assert_eq!(out.len(), ROW_LEN);
            out
        }
    }

    /// An army row with every gate set and a distinct value in every word — the
    /// shape the offset test needs.
    fn full_army() -> RawRow {
        RawRow {
            kind: KIND_ARMY,
            object_id: 1001,
            flags: KNOWN_FLAGS & !FLAG_DELETED & !FLAG_LOCATION_UNREADABLE,
            parent_id: 2002,
            owner_tag_db_id: 303,
            leader_id: 40404,
            leader_skill: 5,
            member_first: 0,
            member_count: 0,
            logical_member_count: 17,
            member_count_engine: 19,
            missing_member_count: 2,
            leader_trait_count: 3,
            parent_status: STATUS_EXACT,
            member_status: STATUS_OVERFLOW,
            owner_status: STATUS_EXACT,
            leader_status: STATUS_EXACT,
            leader_trait_status: STATUS_EXACT,
            location_province_id: 13087,
        }
    }

    fn member_row(member_id: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity(MEMBER_ROW_LEN);
        out.extend_from_slice(&member_id.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out
    }

    /// A whole payload: header words, then rows, then member rows.
    fn payload(rows: &[RawRow], member_ids: &[u32], status_flags: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // row_count
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // changed_rows
        out.extend_from_slice(&status_flags.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // root_rows
        out.extend_from_slice(&0u32.to_le_bytes()); // observed_army_rows
        out.extend_from_slice(&0u32.to_le_bytes()); // observed_army_group_rows
        out.extend_from_slice(&(member_ids.len() as u32).to_le_bytes()); // member_count
        let deleted = rows.iter().filter(|r| r.flags & FLAG_DELETED != 0).count();
        out.extend_from_slice(&(deleted as u32).to_le_bytes()); // deleted_rows
        out.extend_from_slice(&0u32.to_le_bytes()); // observed_fleet_rows
        out.extend_from_slice(&0u32.to_le_bytes()); // observed_task_force_rows
        assert_eq!(out.len(), HEADER_LEN);
        for row in rows {
            out.extend_from_slice(&row.bytes());
        }
        for id in member_ids {
            out.extend_from_slice(&member_row(*id));
        }
        out
    }

    #[test]
    fn the_schema_is_valid() {
        schema()
            .validate(grid().log2())
            .expect("d28 schema is well-formed");
    }

    #[test]
    fn both_collections_are_keyed_sets_with_the_folds_keys() {
        let c = commands();
        assert_eq!(c.shape, Shape::KeyedSet, "first frame replaces, later upsert");
        let keys = c.key_fields();
        assert_eq!(keys.len(), 2, "the fold keys commands by (kind, object_id)");
        assert_eq!(keys[0].field_id, field::KIND);
        assert_eq!(keys[1].field_id, field::OBJECT_ID);
        for k in &keys {
            assert_eq!(
                k.kind,
                Kind::Unsigned,
                "engine discriminator and runtime instance id are both Unsigned (content-identity contract)"
            );
            assert!(k.kind.legal_as_key());
            assert!(!k.optional && !k.nullable && !k.unknown_capable);
        }
        assert_eq!(
            c.fields.len(),
            16,
            "2 key + 8 gated/status-driven + 5 status + logical_member_count"
        );
        assert_eq!(c.value_fields().count(), 14);

        let m = members();
        assert_eq!(m.shape, Shape::KeyedSet);
        let mkeys = m.key_fields();
        assert_eq!(mkeys.len(), 3, "members are keyed by (kind, object_id, member_id)");
        assert_eq!(mkeys[2].field_id, field::MEMBER_ID);
        for k in &mkeys {
            assert_eq!(k.kind, Kind::Unsigned);
            assert!(k.kind.legal_as_key());
        }
        assert_eq!(
            m.value_fields().count(),
            0,
            "the membership edge is the whole fact; nothing beside the key"
        );
    }

    #[test]
    fn the_schema_declares_exactly_the_states_the_mapper_emits() {
        // Every declared capability costs bits, so declaration and mapper must
        // match exactly. The mapper's reachable states, from the module docs.
        let c = commands();
        let optional: Vec<u32> = c
            .value_fields()
            .filter(|f| f.optional)
            .map(|f| f.field_id)
            .collect();
        assert_eq!(
            optional,
            vec![
                field::PARENT_ID,
                field::LEADER_ID,
                field::LEADER_SKILL,
                field::MEMBER_COUNT_ENGINE,
                field::MISSING_MEMBER_COUNT,
                field::LEADER_TRAIT_COUNT,
                field::LOCATION_PROVINCE_ID,
            ]
        );
        let nullable: Vec<u32> = c
            .value_fields()
            .filter(|f| f.nullable)
            .map(|f| f.field_id)
            .collect();
        assert_eq!(
            nullable,
            vec![field::OWNER_TAG_DB_ID],
            "only the owner relation can genuinely resolve to nothing (\"mixed\")"
        );
        let unknown: Vec<u32> = c
            .value_fields()
            .filter(|f| f.unknown_capable)
            .map(|f| f.field_id)
            .collect();
        assert_eq!(
            unknown,
            vec![
                field::PARENT_ID,
                field::OWNER_TAG_DB_ID,
                field::LEADER_ID,
                field::LEADER_SKILL,
                field::MEMBER_COUNT_ENGINE,
                field::MISSING_MEMBER_COUNT,
                field::LEADER_TRAIT_COUNT,
                field::LOCATION_PROVINCE_ID,
            ]
        );
        // `logical_member_count` and the five status words are always concrete.
        for id in [
            field::LOGICAL_MEMBER_COUNT,
            field::PARENT_STATUS,
            field::MEMBER_STATUS,
            field::OWNER_STATUS,
            field::LEADER_STATUS,
            field::LEADER_TRAIT_STATUS,
        ] {
            let f = c
                .value_fields()
                .find(|f| f.field_id == id)
                .expect("in schema");
            assert!(!f.optional && !f.nullable && !f.unknown_capable, "field {id}");
        }
        // Members declares nothing at all.
        assert_eq!(members().optional_count(), 0);
    }

    #[test]
    fn no_field_is_continuous_so_no_quantization_test_applies() {
        // Step 4's "quantization saves bytes" test does not apply: d28 has no
        // continuous field at all. Every word in the row is a raw id, count,
        // skill or status code and the fold divides none of them
        // (`world_fold.rs::fold_oob_command`, projection `world_fold.rs::build_order_of_battle`), so there
        // is nothing to quantize and nothing to under- or over-quantize.
        for c in schema().collections {
            assert!(
                c.fields.iter().all(|f| !matches!(f.kind, Kind::Continuous)),
                "collection {} has a continuous field",
                c.collection_id
            );
        }
    }

    #[test]
    fn no_field_is_derived() {
        // derived-field contract derived fields are clock-linear integers; nothing in d28 changes
        // deterministically with the clock, so `frame_tick` never enters a value.
        for c in schema().collections {
            assert!(c.fields.iter().all(|f| f.derived.is_none()));
        }
    }

    /// kills: collapsing the task-force location back to one gate bit, or letting
    /// an army carry `FLAG_LOCATION_UNREADABLE`, or accepting both location flags
    /// at once.
    ///
    /// The producer now distinguishes three things at `CTaskForce+0x18`: a
    /// measured province, the engine's own absence marker for a task force not
    /// deployed to one, and a read that failed. Only the last is a gap; mapping
    /// the middle one to `Unknown` is what made the viewer's fleets census refuse
    /// every hour of every sealed recording over a single row.
    #[test]
    fn a_task_force_location_carries_three_distinguishable_states() {
        let c = commands();
        let task_force = |flags: u32, province: u32| RawRow {
            kind: KIND_TASK_FORCE,
            object_id: 4,
            flags: FLAG_PARENT | FLAG_OWNER | flags,
            parent_id: 30,
            owner_tag_db_id: 12,
            parent_status: STATUS_EXACT,
            member_status: STATUS_EXACT,
            owner_status: STATUS_EXACT,
            location_province_id: province,
            ..RawRow::default()
        };

        let measured = record_from_row(&c, &task_force(FLAG_LOCATION, 118).bytes(), 0)
            .expect("a measured location maps");
        assert_eq!(
            measured.values[index_of(&c, field::LOCATION_PROVINCE_ID)],
            Value::ContentId(118)
        );

        let engine_absent =
            record_from_row(&c, &task_force(0, 0).bytes(), 0).expect("engine absence maps");
        assert_eq!(
            engine_absent.values[index_of(&c, field::LOCATION_PROVINCE_ID)],
            Value::Absent
        );

        let unreadable = record_from_row(&c, &task_force(FLAG_LOCATION_UNREADABLE, 0).bytes(), 0)
            .expect("an unreadable location maps");
        assert_eq!(
            unreadable.values[index_of(&c, field::LOCATION_PROVINCE_ID)],
            Value::Unknown
        );

        // The two flags are mutually exclusive...
        assert!(record_from_row(
            &c,
            &task_force(FLAG_LOCATION | FLAG_LOCATION_UNREADABLE, 118).bytes(),
            0
        )
        .is_err());

        // ...and only a task force ever attempts a location at all.
        let army = RawRow {
            kind: KIND_ARMY,
            object_id: 7,
            flags: FLAG_LOCATION_UNREADABLE,
            ..RawRow::default()
        };
        assert!(record_from_row(&c, &army.bytes(), 0).is_err());
    }

    #[test]
    fn every_field_reads_from_its_own_offset() {
        // The transposition test: one row with a distinct value in every word,
        // each field checked at its own offset. d28 has no scale anywhere, so
        // this is the unscaled form of step 8's offset test — every value must
        // come back raw and from the right place.
        let c = commands();
        let r = record_from_row(&c, &full_army().bytes(), 0).expect("maps");
        for (id, expected) in [
            (field::PARENT_ID, Value::Unsigned(2002)),
            (field::OWNER_TAG_DB_ID, Value::ContentId(303)),
            (field::LEADER_ID, Value::Unsigned(40404)),
            (field::LEADER_SKILL, Value::Unsigned(5)),
            (field::LOGICAL_MEMBER_COUNT, Value::Unsigned(17)),
            (field::MEMBER_COUNT_ENGINE, Value::Unsigned(19)),
            (field::MISSING_MEMBER_COUNT, Value::Unsigned(2)),
            (field::LEADER_TRAIT_COUNT, Value::Unsigned(3)),
            (field::PARENT_STATUS, Value::Enum(STATUS_EXACT)),
            (field::MEMBER_STATUS, Value::Enum(STATUS_OVERFLOW)),
            (field::OWNER_STATUS, Value::Enum(STATUS_EXACT)),
            (field::LEADER_STATUS, Value::Enum(STATUS_EXACT)),
            (field::LEADER_TRAIT_STATUS, Value::Enum(STATUS_EXACT)),
            (field::LOCATION_PROVINCE_ID, Value::ContentId(13087)),
        ] {
            assert_eq!(r.values[index_of(&c, id)], expected, "field {id}");
        }
        // The five status words are adjacent in the row and separated in the
        // record, so check them against a row where all five differ.
        let mut row = full_army();
        row.parent_status = STATUS_EXACT;
        row.member_status = STATUS_UNKNOWN;
        row.owner_status = STATUS_OVERFLOW;
        row.leader_status = STATUS_VALIDATION_FAILED;
        row.leader_trait_status = STATUS_MIXED;
        // A non-EXACT trait status may not carry a count.
        row.leader_trait_count = 0;
        // `leader_status != NONE` keeps the leader fields at their gated values.
        let r = record_from_row(&c, &row.bytes(), 0).expect("maps");
        for (id, code) in [
            (field::PARENT_STATUS, STATUS_EXACT),
            (field::MEMBER_STATUS, STATUS_UNKNOWN),
            (field::OWNER_STATUS, STATUS_OVERFLOW),
            (field::LEADER_STATUS, STATUS_VALIDATION_FAILED),
            (field::LEADER_TRAIT_STATUS, STATUS_MIXED),
        ] {
            assert_eq!(r.values[index_of(&c, id)], Value::Enum(code), "status {id}");
        }
    }

    #[test]
    fn a_real_zero_is_a_value_not_an_empty_state() {
        // Every gate set, every value zero: record-state contract keeps zero out of the three empty
        // states. `owner_tag_db_id` is the pointed case — the fold folds a
        // *clear* owner gate to `0` (`world_fold.rs::fold_oob_command`), so a `0` under a
        // SET gate must still be a value here and not be confused with it.
        let c = commands();
        let row = RawRow {
            kind: KIND_ARMY,
            object_id: 9,
            flags: KNOWN_FLAGS & !FLAG_DELETED & !FLAG_LOCATION_UNREADABLE,
            leader_trait_status: STATUS_EXACT,
            ..RawRow::default()
        };
        let r = record_from_row(&c, &row.bytes(), 0).expect("maps");
        assert_eq!(r.values[index_of(&c, field::PARENT_ID)], Value::Unsigned(0));
        assert_eq!(
            r.values[index_of(&c, field::OWNER_TAG_DB_ID)],
            Value::ContentId(0),
            "a zero under a SET owner gate is a measured value"
        );
        assert_eq!(r.values[index_of(&c, field::LEADER_ID)], Value::Unsigned(0));
        assert_eq!(r.values[index_of(&c, field::LEADER_SKILL)], Value::Unsigned(0));
        assert_eq!(
            r.values[index_of(&c, field::LOGICAL_MEMBER_COUNT)],
            Value::Unsigned(0),
            "a command with no members has a real zero, not an empty state"
        );
        assert_eq!(
            r.values[index_of(&c, field::MISSING_MEMBER_COUNT)],
            Value::Unsigned(0)
        );
        assert_eq!(
            r.values[index_of(&c, field::LEADER_TRAIT_COUNT)],
            Value::Unsigned(0),
            "an EXACT trait status with an empty trait vector is a real zero"
        );
        assert_eq!(
            r.values[index_of(&c, field::LOCATION_PROVINCE_ID)],
            Value::ContentId(0)
        );
    }

    #[test]
    fn a_clear_gate_maps_to_the_state_its_evidence_names() {
        let c = commands();

        // 1. An army whose reads all failed: every gate clear, every status a
        //    failure code. Nothing is absent — the concepts all apply to an army.
        let row = RawRow {
            kind: KIND_ARMY,
            object_id: 1,
            flags: 0,
            parent_status: STATUS_UNKNOWN,
            member_status: STATUS_EXACT,
            owner_status: STATUS_UNKNOWN,
            leader_status: STATUS_VALIDATION_FAILED,
            leader_trait_status: STATUS_VALIDATION_FAILED,
            ..RawRow::default()
        };
        let r = record_from_row(&c, &row.bytes(), 0).expect("maps");
        for id in [
            field::PARENT_ID,
            field::OWNER_TAG_DB_ID,
            field::LEADER_ID,
            field::LEADER_SKILL,
            field::MEMBER_COUNT_ENGINE,
            field::MISSING_MEMBER_COUNT,
            field::LEADER_TRAIT_COUNT,
        ] {
            assert_eq!(r.values[index_of(&c, id)], Value::Unknown, "field {id}");
        }
        // An army is not a task force, so its location is absent, not unknown.
        assert_eq!(
            r.values[index_of(&c, field::LOCATION_PROVINCE_ID)],
            Value::Absent
        );

        // 2. An army group, as the collector builds one (`attach_log.rs:46884`):
        //    owner gate set, everything else structurally clear.
        let row = RawRow {
            kind: KIND_ARMY_GROUP,
            object_id: 2,
            flags: FLAG_OWNER,
            owner_tag_db_id: 77,
            logical_member_count: 4,
            member_status: STATUS_EXACT,
            owner_status: STATUS_EXACT,
            ..RawRow::default()
        };
        let r = record_from_row(&c, &row.bytes(), 0).expect("maps");
        for id in [
            field::PARENT_ID,
            field::LEADER_ID,
            field::LEADER_SKILL,
            field::MEMBER_COUNT_ENGINE,
            field::MISSING_MEMBER_COUNT,
            field::LEADER_TRAIT_COUNT,
            field::LOCATION_PROVINCE_ID,
        ] {
            assert_eq!(
                r.values[index_of(&c, id)],
                Value::Absent,
                "an army group has no {id}"
            );
        }
        assert_eq!(
            r.values[index_of(&c, field::OWNER_TAG_DB_ID)],
            Value::ContentId(77)
        );

        // 3. A fleet (`:47000`): engine count present, missing count never
        //    attempted, no parent, no leader, no location.
        let row = RawRow {
            kind: KIND_FLEET,
            object_id: 3,
            flags: FLAG_OWNER | FLAG_MEMBER_COUNT_ENGINE,
            owner_tag_db_id: 12,
            member_count_engine: 6,
            logical_member_count: 6,
            member_status: STATUS_EXACT,
            owner_status: STATUS_EXACT,
            ..RawRow::default()
        };
        let r = record_from_row(&c, &row.bytes(), 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::MEMBER_COUNT_ENGINE)],
            Value::Unsigned(6)
        );
        for id in [
            field::PARENT_ID,
            field::LEADER_ID,
            field::LEADER_SKILL,
            field::MISSING_MEMBER_COUNT,
            field::LEADER_TRAIT_COUNT,
            field::LOCATION_PROVINCE_ID,
        ] {
            assert_eq!(r.values[index_of(&c, id)], Value::Absent, "a fleet has no {id}");
        }

        // 4. A task force whose location read failed (3 such rows on
        //    deepwar_t1): location is UNKNOWN here, not absent, because the
        //    concept applies to this kind.
        let row = RawRow {
            kind: KIND_TASK_FORCE,
            object_id: 4,
            flags: FLAG_PARENT | FLAG_OWNER | FLAG_MEMBER_COUNT_ENGINE,
            parent_id: 30,
            owner_tag_db_id: 12,
            member_count_engine: 5,
            logical_member_count: 5,
            parent_status: STATUS_EXACT,
            member_status: STATUS_EXACT,
            owner_status: STATUS_EXACT,
            ..RawRow::default()
        };
        let r = record_from_row(&c, &row.bytes(), 0).expect("maps");
        // Both location flags clear on a TASK FORCE is now the engine's own
        // absence marker at `CTaskForce+0x18` — a task force that is not deployed
        // to a province — which is a measurement, not a gap. The read that failed
        // carries `FLAG_LOCATION_UNREADABLE` instead; see the three-state test.
        assert_eq!(
            r.values[index_of(&c, field::LOCATION_PROVINCE_ID)],
            Value::Absent
        );
        // A task force's missing count is attempted and filtered, so a clear
        // gate is unknown — the recipe's tie-break, see the open questions.
        assert_eq!(
            r.values[index_of(&c, field::MISSING_MEMBER_COUNT)],
            Value::Unknown
        );
        // It has no leader concept at all.
        assert_eq!(r.values[index_of(&c, field::LEADER_ID)], Value::Absent);
        assert_eq!(r.values[index_of(&c, field::LEADER_SKILL)], Value::Absent);

        // 5. A `"mixed"` owner is NULL, not unknown: the single-owner relation
        //    was measured and genuinely resolves to nothing.
        let row = RawRow {
            kind: KIND_ARMY,
            object_id: 5,
            flags: 0,
            owner_status: STATUS_MIXED,
            member_status: STATUS_EXACT,
            parent_status: STATUS_UNKNOWN,
            leader_status: STATUS_UNKNOWN,
            leader_trait_status: STATUS_NONE,
            ..RawRow::default()
        };
        let r = record_from_row(&c, &row.bytes(), 0).expect("maps");
        assert_eq!(r.values[index_of(&c, field::OWNER_TAG_DB_ID)], Value::Null);
        // `leader_trait_status == NONE` on an ARMY whose leader read failed is
        // UNKNOWN, not absent — see the regression test below.
        assert_eq!(
            r.values[index_of(&c, field::LEADER_TRAIT_COUNT)],
            Value::Unknown
        );
    }

    #[test]
    fn leader_trait_count_follows_leader_status_not_the_trait_status_alone() {
        // carried in as a regression: the reviewer's own
        // input is the deepwar_t1 row at record offset 453616, sequence 14, hour
        // 60831018, row 1 — `kind=ARMY`, `object_id=13`, `flags=0x66`,
        // `leader_status=UNKNOWN`, `leader_trait_status=NONE`,
        // `leader_trait_count=0`. 122 live army rows on that capture share it.
        // `read_army_oob_leader_detail` returns NO trait status on every leader
        // failure path (`attach_log.rs:22246`-`:22272`) and the row builder turns
        // that into `"none"` (`:46878`), so a `"none"` trait status is only
        // "absent" when the KIND has no leader — which `leader_status == NONE`
        // is what names.
        let c = commands();
        let reviewer_row = RawRow {
            kind: KIND_ARMY,
            object_id: 13,
            flags: 0x66,
            member_count: 0,
            logical_member_count: 0,
            member_count_engine: 4,
            missing_member_count: 0,
            leader_trait_count: 0,
            parent_status: STATUS_EXACT,
            member_status: STATUS_EXACT,
            owner_status: STATUS_EXACT,
            leader_status: STATUS_UNKNOWN,
            leader_trait_status: STATUS_NONE,
            parent_id: 0,
            owner_tag_db_id: 0,
            ..RawRow::default()
        };
        let r = record_from_row(&c, &reviewer_row.bytes(), 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::LEADER_TRAIT_COUNT)],
            Value::Unknown,
            "an army whose leader read failed has an uncaptured trait count"
        );
        assert_eq!(r.values[index_of(&c, field::LEADER_ID)], Value::Unknown);
        assert_eq!(r.values[index_of(&c, field::LEADER_SKILL)], Value::Unknown);

        // The whole (leader_status x leader_trait_status) space, not one input:
        // the count is a value exactly when the trait status is EXACT, absent
        // exactly when the leader status is NONE, and unknown everywhere else —
        // the same function `leader_id` and `leader_skill` use.
        for leader_status in 0..STATUS_CARDINALITY {
            for trait_status in 0..STATUS_CARDINALITY {
                let row = RawRow {
                    kind: KIND_ARMY,
                    object_id: 1,
                    flags: 0,
                    leader_status,
                    leader_trait_status: trait_status,
                    leader_trait_count: 0,
                    ..RawRow::default()
                };
                let r = record_from_row(&c, &row.bytes(), 0)
                    .unwrap_or_else(|e| panic!("{leader_status}/{trait_status}: {e}"));
                let expected = if trait_status == STATUS_EXACT {
                    Value::Unsigned(0)
                } else if leader_status == STATUS_NONE {
                    Value::Absent
                } else {
                    Value::Unknown
                };
                assert_eq!(
                    r.values[index_of(&c, field::LEADER_TRAIT_COUNT)],
                    expected,
                    "leader_status {leader_status}, leader_trait_status {trait_status}"
                );
                // And the two leader fields agree on the absent/unknown split
                // whenever their gates are clear.
                let leader_empty = if leader_status == STATUS_NONE {
                    Value::Absent
                } else {
                    Value::Unknown
                };
                assert_eq!(r.values[index_of(&c, field::LEADER_ID)], leader_empty);
                assert_eq!(r.values[index_of(&c, field::LEADER_SKILL)], leader_empty);
            }
        }
    }

    #[test]
    fn every_gate_is_exercised_one_at_a_time() {
        // The whole gate space, not one sampled case: for each of the seven
        // gates, a task-force-kind row (the kind for which every concept
        // applies except the leader) with that one gate cleared.
        let c = commands();
        let base = RawRow {
            kind: KIND_TASK_FORCE,
            object_id: 7,
            flags: KNOWN_FLAGS
                & !FLAG_DELETED
                & !FLAG_LEADER
                & !FLAG_LEADER_SKILL
                & !FLAG_LOCATION_UNREADABLE,
            parent_id: 8,
            owner_tag_db_id: 9,
            member_count_engine: 10,
            missing_member_count: 11,
            logical_member_count: 12,
            location_province_id: 13,
            parent_status: STATUS_EXACT,
            member_status: STATUS_EXACT,
            owner_status: STATUS_EXACT,
            leader_status: STATUS_NONE,
            leader_trait_status: STATUS_NONE,
            ..RawRow::default()
        };
        for (gate, field_id, zero_the_word, also_set) in [
            (FLAG_PARENT, field::PARENT_ID, false, 0),
            (FLAG_OWNER, field::OWNER_TAG_DB_ID, false, 0),
            (FLAG_MEMBER_COUNT_ENGINE, field::MEMBER_COUNT_ENGINE, false, 0),
            (FLAG_MISSING_MEMBER_COUNT, field::MISSING_MEMBER_COUNT, false, 0),
            // The two validated gates must pack a zero when clear. Clearing the
            // location gate alone no longer means "unknown" -- that is the
            // engine's absence marker now -- so the unreadable flag comes with
            // it to keep this row saying what this loop is testing.
            (
                FLAG_LOCATION,
                field::LOCATION_PROVINCE_ID,
                true,
                FLAG_LOCATION_UNREADABLE,
            ),
        ] {
            let mut row = base;
            row.flags &= !gate;
            row.flags |= also_set;
            if zero_the_word {
                row.location_province_id = 0;
            }
            let r = record_from_row(&c, &row.bytes(), 0)
                .unwrap_or_else(|e| panic!("gate {gate:#x}: {e}"));
            assert_eq!(
                r.values[index_of(&c, field_id)],
                Value::Unknown,
                "field {field_id} with gate {gate:#x} clear on a task force"
            );
        }
        // The leader gates, on the kind that has a leader.
        let army = RawRow {
            kind: KIND_ARMY,
            object_id: 8,
            flags: FLAG_LEADER | FLAG_LEADER_SKILL,
            leader_id: 5,
            leader_skill: 6,
            leader_status: STATUS_EXACT,
            member_status: STATUS_EXACT,
            leader_trait_status: STATUS_EXACT,
            ..RawRow::default()
        };
        let r = record_from_row(&c, &army.bytes(), 0).expect("maps");
        assert_eq!(r.values[index_of(&c, field::LEADER_ID)], Value::Unsigned(5));
        assert_eq!(r.values[index_of(&c, field::LEADER_SKILL)], Value::Unsigned(6));
        // Skill gate clear but leader present and read: unknown, not absent.
        let mut skill_clear = army;
        skill_clear.flags = FLAG_LEADER;
        skill_clear.leader_skill = 0;
        let r = record_from_row(&c, &skill_clear.bytes(), 0).expect("maps");
        assert_eq!(r.values[index_of(&c, field::LEADER_SKILL)], Value::Unknown);
        assert_eq!(r.values[index_of(&c, field::LEADER_ID)], Value::Unsigned(5));
        // Leader gate clear with a failed read: both unknown.
        let mut leader_failed = army;
        leader_failed.flags = 0;
        leader_failed.leader_id = 0;
        leader_failed.leader_skill = 0;
        leader_failed.leader_status = STATUS_VALIDATION_FAILED;
        leader_failed.leader_trait_status = STATUS_VALIDATION_FAILED;
        leader_failed.leader_trait_count = 0;
        let r = record_from_row(&c, &leader_failed.bytes(), 0).expect("maps");
        assert_eq!(r.values[index_of(&c, field::LEADER_ID)], Value::Unknown);
        assert_eq!(r.values[index_of(&c, field::LEADER_SKILL)], Value::Unknown);
        assert_eq!(
            r.values[index_of(&c, field::LEADER_TRAIT_COUNT)],
            Value::Unknown
        );
    }

    #[test]
    fn a_full_row_roundtrips_byte_for_byte() {
        let c = commands();
        let record = record_from_row(&c, &full_army().bytes(), 0).expect("maps");
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
    fn every_empty_state_survives_a_roundtrip() {
        // A row reaching absent, null and unknown at once, plus a member record
        // that has no values at all.
        let c = commands();
        let row = RawRow {
            kind: KIND_ARMY,
            object_id: 1,
            flags: 0,
            parent_status: STATUS_UNKNOWN,
            member_status: STATUS_EXACT,
            owner_status: STATUS_MIXED,
            leader_status: STATUS_NONE,
            leader_trait_status: STATUS_NONE,
            ..RawRow::default()
        };
        let record = record_from_row(&c, &row.bytes(), 0).expect("maps");
        assert_eq!(record.values[index_of(&c, field::OWNER_TAG_DB_ID)], Value::Null);
        assert_eq!(record.values[index_of(&c, field::LEADER_ID)], Value::Absent);
        assert_eq!(record.values[index_of(&c, field::PARENT_ID)], Value::Unknown);
        let mut bytes = Vec::new();
        record.encode(&c, &mut bytes).expect("encodes");
        let (back, pos) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(pos, bytes.len());
        assert_eq!(back, record);
        let mut reencoded = Vec::new();
        back.encode(&c, &mut reencoded).expect("re-encodes");
        assert_eq!(reencoded, bytes);

        let m = members();
        let mrecord = record_from_member_row(&m, &member_row(42), 0).expect("maps");
        assert!(mrecord.values.is_empty());
        let mut mbytes = Vec::new();
        mrecord.encode(&m, &mut mbytes).expect("encodes");
        let (mback, mpos) = Record::decode(&m, &mbytes, 0).expect("decodes");
        assert_eq!(mpos, mbytes.len());
        assert_eq!(mback, mrecord);
    }

    #[test]
    fn the_mapper_rejects_every_row_the_fold_rejects() {
        let c = commands();
        // Wrong length, both directions, including the empty slice.
        for len in [0usize, 1, ROW_LEN - 1, ROW_LEN + 1, 48] {
            assert!(record_from_row(&c, &vec![0u8; len], 0).is_err(), "length {len}");
        }
        // An invalid kind (`world_fold.rs::fold_oob_command`), across the whole edge.
        for kind in [0u32, 5, 6, u32::MAX] {
            let row = RawRow { kind, ..full_army() };
            assert!(record_from_row(&c, &row.bytes(), 0).is_err(), "kind {kind}");
        }
        // Zero object_id (`:14334`).
        let row = RawRow { object_id: 0, ..full_army() };
        assert!(record_from_row(&c, &row.bytes(), 0).is_err());
        // Every unknown flag bit (`:14342`), not just one sample.
        for bit in 8..32u32 {
            let row = RawRow {
                flags: full_army().flags | (1 << bit),
                ..full_army()
            };
            assert!(record_from_row(&c, &row.bytes(), 0).is_err(), "flag bit {bit}");
        }
        // The two validated gates carrying a value without their flag
        // (`:14361`, `:14366`).
        let row = RawRow {
            flags: full_army().flags & !FLAG_LEADER_SKILL,
            ..full_army()
        };
        assert!(record_from_row(&c, &row.bytes(), 0).is_err(), "leader_skill without flag");
        let row = RawRow {
            flags: full_army().flags & !FLAG_LOCATION,
            ..full_army()
        };
        assert!(record_from_row(&c, &row.bytes(), 0).is_err(), "location without flag");
        // A non-zero reserved word (`:14369`).
        let mut bytes = full_army().bytes();
        bytes[76..80].copy_from_slice(&1u32.to_le_bytes());
        assert!(record_from_row(&c, &bytes, 0).is_err(), "reserved nonzero");
        // A delete row is a removal, not a record (`:14433`).
        let row = RawRow {
            flags: FLAG_DELETED,
            leader_skill: 0,
            location_province_id: 0,
            ..full_army()
        };
        assert!(record_from_row(&c, &row.bytes(), 0).is_err(), "delete row");
        // A status code outside the producer's vocabulary, every one of the five
        // words and the whole out-of-range edge.
        for (i, (_, offset)) in STATUS_WORDS.into_iter().enumerate() {
            for code in [STATUS_CARDINALITY, STATUS_CARDINALITY + 1, u32::MAX] {
                let mut bytes = full_army().bytes();
                bytes[offset..offset + 4].copy_from_slice(&code.to_le_bytes());
                assert!(
                    record_from_row(&c, &bytes, 0).is_err(),
                    "status word {i} code {code}"
                );
            }
        }
        // `leader_trait_count` carrying a value without an EXACT status — the
        // one place this mapper is stricter than the fold, swept over every
        // non-EXACT code rather than one sample.
        for status in [
            STATUS_NONE,
            STATUS_UNKNOWN,
            STATUS_OVERFLOW,
            STATUS_VALIDATION_FAILED,
            STATUS_MIXED,
        ] {
            let row = RawRow {
                leader_trait_status: status,
                leader_trait_count: 1,
                ..full_army()
            };
            assert!(
                record_from_row(&c, &row.bytes(), 0).is_err(),
                "leader_trait_count under status {status}"
            );
            // And zero under the same status is accepted.
            let row = RawRow {
                leader_trait_status: status,
                leader_trait_count: 0,
                ..full_army()
            };
            assert!(record_from_row(&c, &row.bytes(), 0).is_ok(), "status {status}");
        }
        // Member rows.
        let m = members();
        assert!(
            record_from_member_row(&m, &member_row(0), 0).is_err(),
            "zero member id"
        );
        let mut bad = member_row(5);
        bad[4..8].copy_from_slice(&1u32.to_le_bytes());
        assert!(
            record_from_member_row(&m, &bad, 0).is_err(),
            "member reserved nonzero"
        );
        for len in [0usize, 4, 7, 9] {
            assert!(
                record_from_member_row(&m, &vec![0u8; len], 0).is_err(),
                "length {len}"
            );
        }
    }

    #[test]
    fn the_mapper_refuses_a_collection_it_cannot_fill() {
        // A caller handing over another domain's collection must not get a
        // record full of placeholders under this domain's field ids.
        let mut c = commands();
        c.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_row(&c, &full_army().bytes(), 0).is_err());
        let mut m = members();
        m.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_member_row(&m, &member_row(1), 0).is_err());
    }

    #[test]
    fn split_payload_enforces_the_folds_structural_rules() {
        let rows = [full_army()];
        let p = payload(&rows, &[], 0);
        let (header, row_bytes, member_bytes) = split_payload(&p).expect("well-formed");
        assert_eq!(header.row_count, 1);
        assert_eq!(row_bytes.len(), ROW_LEN);
        assert!(member_bytes.is_empty());

        // Short of the header.
        for len in 0..HEADER_LEN {
            assert!(split_payload(&p[..len]).is_err(), "len {len}");
        }
        // A length that does not match the declared counts, both directions.
        let mut short = p.clone();
        short.pop();
        assert!(split_payload(&short).is_err());
        let mut long = p.clone();
        long.push(0);
        assert!(split_payload(&long).is_err());
        // changed_rows > row_count (`world_fold.rs::fold_oob_command`).
        let mut over = p.clone();
        over[12..16].copy_from_slice(&2u32.to_le_bytes());
        assert!(split_payload(&over).is_err());
        // changed_rows < row_count is LEGAL for d28 — the fold's check is `<=`
        // and it never requires equality in any mode.
        let mut under = p.clone();
        under[12..16].copy_from_slice(&0u32.to_le_bytes());
        assert!(split_payload(&under).is_ok());
        // deleted_rows > row_count (`:14315`).
        let mut deleted = p.clone();
        deleted[36..40].copy_from_slice(&2u32.to_le_bytes());
        assert!(split_payload(&deleted).is_err());
        // Counts whose byte lengths overflow `usize` must be rejected, not
        // wrapped into a plausible small length.
        for offset in [8usize, 32] {
            let mut huge = p.clone();
            huge[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            huge[12..16].copy_from_slice(&0u32.to_le_bytes());
            huge[36..40].copy_from_slice(&0u32.to_le_bytes());
            assert!(split_payload(&huge).is_err(), "offset {offset}");
        }
    }

    #[test]
    fn the_keys_are_the_folds_keys() {
        assert_eq!(
            key_from_row(&full_army().bytes()).expect("keys"),
            command_key(KIND_ARMY, 1001)
        );
        assert!(key_from_row(&[]).is_err());
        assert_eq!(
            key_from_member_row(&full_army().bytes(), &member_row(77)).expect("keys"),
            member_key(KIND_ARMY, 1001, 77)
        );
        assert!(key_from_member_row(&full_army().bytes(), &member_row(0)).is_err());
    }

    fn fold_one(
        state: &mut State,
        rows: &[RawRow],
        member_ids: &[u32],
        sparse: bool,
    ) -> core::result::Result<PayloadHeader, String> {
        let s = schema();
        fold_payload(
            state,
            &payload(rows, member_ids, 0),
            sparse,
            &s.collections[0],
            &s.collections[1],
        )
    }

    #[test]
    fn fold_payload_replaces_then_upserts_and_removes() {
        let army = RawRow {
            kind: KIND_ARMY,
            object_id: 1,
            flags: FLAG_OWNER,
            owner_tag_db_id: 5,
            member_first: 0,
            member_count: 2,
            logical_member_count: 2,
            member_status: STATUS_EXACT,
            owner_status: STATUS_EXACT,
            ..RawRow::default()
        };
        let group = RawRow {
            kind: KIND_ARMY_GROUP,
            object_id: 2,
            flags: FLAG_OWNER,
            owner_tag_db_id: 5,
            member_first: 2,
            member_count: 1,
            logical_member_count: 1,
            member_status: STATUS_EXACT,
            owner_status: STATUS_EXACT,
            ..RawRow::default()
        };
        let mut state = State::new();
        // First frame: not sparse, so it replaces.
        fold_one(&mut state, &[army, group], &[10, 11, 1], false).expect("folds");
        assert_eq!(state.commands.len(), 2);
        let states = state.keyed_states();
        assert_eq!(states[0].len(), 2, "two commands");
        assert_eq!(states[1].len(), 3, "three membership edges");
        assert!(states[1].contains_key(&member_key(KIND_ARMY, 1, 10)));
        assert!(states[1].contains_key(&member_key(KIND_ARMY_GROUP, 2, 1)));

        // Sparse frame touching only the army: the group survives untouched.
        let mut moved = army;
        moved.member_count = 1;
        moved.logical_member_count = 1;
        fold_one(&mut state, &[moved], &[12], true).expect("folds");
        assert_eq!(state.commands.len(), 2);
        let states = state.keyed_states();
        assert_eq!(states[1].len(), 2, "army now has one member, group still one");
        assert!(states[1].contains_key(&member_key(KIND_ARMY, 1, 12)));
        assert!(!states[1].contains_key(&member_key(KIND_ARMY, 1, 10)));

        // A delete row removes the key and every edge under it.
        let tombstone = RawRow {
            kind: KIND_ARMY,
            object_id: 1,
            flags: FLAG_DELETED,
            ..RawRow::default()
        };
        fold_one(&mut state, &[tombstone], &[], true).expect("folds");
        assert_eq!(state.commands.len(), 1);
        let states = state.keyed_states();
        assert_eq!(states[0].len(), 1);
        assert_eq!(states[1].len(), 1, "the army's edges went with it");

        // A non-sparse frame replaces the world.
        fold_one(&mut state, &[], &[], false).expect("folds");
        assert!(state.commands.is_empty());
        let states = state.keyed_states();
        assert!(states[0].is_empty() && states[1].is_empty());
    }

    #[test]
    fn fold_payload_rejects_what_the_fold_rejects_and_leaves_the_state_alone() {
        let good = RawRow {
            kind: KIND_ARMY,
            object_id: 1,
            flags: 0,
            member_count: 1,
            logical_member_count: 1,
            member_status: STATUS_EXACT,
            parent_status: STATUS_UNKNOWN,
            owner_status: STATUS_UNKNOWN,
            leader_status: STATUS_UNKNOWN,
            leader_trait_status: STATUS_VALIDATION_FAILED,
            ..RawRow::default()
        };
        let mut state = State::new();
        fold_one(&mut state, &[good], &[10], false).expect("folds");
        let before = state.clone();

        // A duplicated identity in one frame (`world_fold.rs::fold_oob_command`).
        let mut dup = good;
        dup.member_first = 1;
        assert!(fold_one(&mut state, &[good, dup], &[10, 11], true).is_err());
        // Overlapping member ranges (`:14355`).
        let mut other = good;
        other.object_id = 2;
        assert!(fold_one(&mut state, &[good, other], &[10], true).is_err());
        // An unowned member child (`:14403`).
        let mut no_members = good;
        no_members.member_count = 0;
        assert!(fold_one(&mut state, &[no_members], &[10], true).is_err());
        // A member id repeated under one row (`:14409`).
        let mut two = good;
        two.member_count = 2;
        assert!(fold_one(&mut state, &[two], &[10, 10], true).is_err());
        // A member range past the end (`:14350`).
        let mut past = good;
        past.member_count = 2;
        assert!(fold_one(&mut state, &[past], &[10], true).is_err());
        // A zero member id (`:14400`).
        assert!(fold_one(&mut state, &[good], &[0], true).is_err());

        assert_eq!(state, before, "a rejected frame leaves the accumulator alone");
    }

    #[test]
    fn a_delete_row_may_declare_a_member_range_and_the_fold_still_drops_it() {
        // The fold applies NO emptiness requirement to a delete row (unlike
        // d26's `:17427`): the range is validated in the row loop and then
        // discarded with the key. A mapper that assumed emptiness would reject a
        // frame the recorder folds.
        let live = RawRow {
            kind: KIND_ARMY,
            object_id: 1,
            flags: 0,
            member_count: 1,
            logical_member_count: 1,
            member_status: STATUS_EXACT,
            parent_status: STATUS_UNKNOWN,
            owner_status: STATUS_UNKNOWN,
            leader_status: STATUS_UNKNOWN,
            leader_trait_status: STATUS_VALIDATION_FAILED,
            ..RawRow::default()
        };
        let mut state = State::new();
        fold_one(&mut state, &[live], &[10], false).expect("folds");
        let deleted = RawRow {
            kind: KIND_ARMY,
            object_id: 1,
            flags: FLAG_DELETED,
            member_first: 0,
            member_count: 1,
            ..RawRow::default()
        };
        fold_one(&mut state, &[deleted], &[99], true).expect("a delete row may own members");
        assert!(state.commands.is_empty());
    }

    #[test]
    fn frame_is_sparse_matches_the_folds_rule() {
        // `world_fold.rs::replace_or_sparse_after_first`: only the first non-delta frame replaces.
        assert!(!frame_is_sparse(false, false));
        assert!(frame_is_sparse(true, false));
        assert!(frame_is_sparse(false, true));
        assert!(frame_is_sparse(true, true));
    }

    #[test]
    fn the_member_collection_diffs_as_pure_key_membership() {
        // A collection with no value fields still has to behave: identical keys
        // must produce an empty diff, and an added or removed key must show up
        // as exactly one change or one tombstone.
        let m = members();
        let mut a = KeyedState::new();
        a.insert(member_key(KIND_ARMY, 1, 10), Record::new(Vec::new()));
        let mut b = a.clone();
        assert!(diff(&m, &a, &b).is_empty(), "identical membership is no delta");
        b.insert(member_key(KIND_ARMY, 1, 11), Record::new(Vec::new()));
        let d = diff(&m, &a, &b);
        assert_eq!(d.changed.len(), 1);
        assert_eq!(d.tombstones.len(), 0);
        let d = diff(&m, &b, &a);
        assert_eq!(d.changed.len(), 0);
        assert_eq!(d.tombstones.len(), 1);
    }

    // --- the member-ordering divergence -------------------------------------

    #[test]
    fn an_unsorted_member_range_is_refused_rather_than_re_sorted() {
        // The exact payload the audit is about: descending member ids. The FOLD
        // accepts this — its member loop checks only zero, ownership and per-row
        // uniqueness (`world_fold.rs::fold_oob_command`) — and stores the vector
        // [900, 431, 12]. The MEMBERS key orders them 12 < 431 < 900, so this
        // schema cannot carry it and must say so instead of silently re-sorting.
        let s = schema();
        let row = RawRow {
            member_first: 0,
            member_count: 3,
            ..full_army()
        };
        let p = payload(&[row], &[900, 431, 12], 0);
        let mut state = State::new();
        let err = fold_payload(&mut state, &p, false, &s.collections[0], &s.collections[1])
            .expect_err("refused");
        assert!(err.contains("431") && err.contains("900"), "{err}");
        // Nothing partial is left behind: the refusal happens while the upserts
        // are being built, before the accumulator is touched.
        assert!(state.commands.is_empty());
        // And it is refused on an upsert into a populated accumulator too, not
        // just on a replacing first frame.
        let seed = payload(&[full_army()], &[], 0);
        let mut state = State::new();
        fold_payload(&mut state, &seed, false, &s.collections[0], &s.collections[1])
            .expect("seed folds");
        let before = state.clone();
        assert!(fold_payload(&mut state, &p, true, &s.collections[0], &s.collections[1]).is_err());
        assert_eq!(state, before, "a refused frame leaves the accumulator alone");

        // Ascending is accepted, and is the only ordering of these three ids
        // that is.
        let ok = payload(&[row], &[12, 431, 900], 0);
        let mut state = State::new();
        fold_payload(&mut state, &ok, false, &s.collections[0], &s.collections[1]).expect("folds");
        assert_eq!(state.commands[&(KIND_ARMY, 1001)].members, vec![12, 431, 900]);

        // `split_payload` is purely structural and does NOT refuse: the rule is
        // about the state a frame leaves behind, not about the payload's shape.
        split_payload(&p).expect("structurally well-formed");
    }

    #[test]
    fn a_withheld_frame_is_validated_in_full_but_not_ordering_checked() {
        // The recorder validates a tolerated-partial frame's every row and member
        // row and then abandons it without touching the accumulator
        // (`world_fold.rs::fold_oob_command`). Its member ranges never become state,
        // so the ordering rule has nothing to protect and must not refuse it —
        // otherwise this port rejects a frame class the recorder tolerates.
        let s = schema();
        let row = RawRow {
            member_first: 0,
            member_count: 3,
            ..full_army()
        };
        let p = payload(&[row], &[900, 431, 12], 0);
        let header = validate_withheld_payload(&p, &s.collections[0], &s.collections[1])
            .expect("a withheld frame's order is not state");
        assert_eq!(header.row_count, 1);
        assert_eq!(header.member_count, 3);
        // It is still the FULL validation: a malformed member row in a withheld
        // frame is a corrupt stream in the recorder too.
        let mut corrupt = p.clone();
        let member0 = HEADER_LEN + ROW_LEN;
        corrupt[member0..member0 + 4].copy_from_slice(&0u32.to_le_bytes()); // zero id
        assert!(
            validate_withheld_payload(&corrupt, &s.collections[0], &s.collections[1]).is_err(),
            "a zero member id is rejected on the withheld path too"
        );
        let mut corrupt = p.clone();
        corrupt[member0 + 4..member0 + 8].copy_from_slice(&1u32.to_le_bytes()); // reserved
        assert!(
            validate_withheld_payload(&corrupt, &s.collections[0], &s.collections[1]).is_err(),
            "a non-zero member reserved word is rejected on the withheld path too"
        );
        // And the ONLY difference from the folding path is the ordering rule.
        let mut state = State::new();
        assert!(
            fold_payload(&mut state, &p, false, &s.collections[0], &s.collections[1]).is_err(),
            "the same payload IS refused when it would become state"
        );
    }

    #[test]
    fn the_refusal_sweeps_every_kind_every_position_and_every_arity() {
        // By property, not by the one reported input: every command kind, every
        // adjacent inversion in a range, and a range whose only defect is at its
        // very end. A one-member range has no ordering and is always accepted.
        let s = schema();
        let fold = |p: &[u8]| {
            let mut state = State::new();
            fold_payload(&mut state, p, false, &s.collections[0], &s.collections[1]).map(|_| ())
        };
        for kind in [KIND_ARMY, KIND_ARMY_GROUP, KIND_FLEET, KIND_TASK_FORCE] {
            let base = RawRow {
                kind,
                object_id: 7,
                flags: FLAG_OWNER,
                owner_tag_db_id: 3,
                parent_status: STATUS_NONE,
                member_status: STATUS_EXACT,
                owner_status: STATUS_EXACT,
                leader_status: STATUS_NONE,
                leader_trait_status: STATUS_NONE,
                ..RawRow::default()
            };
            for len in 1usize..=5 {
                let ascending: Vec<u32> = (1..=len as u32).map(|i| i * 10).collect();
                let row = RawRow {
                    member_first: 0,
                    member_count: len as u32,
                    ..base
                };
                fold(&payload(&[row], &ascending, 0))
                    .unwrap_or_else(|e| panic!("kind {kind} len {len} ascending: {e}"));
                // Swap each adjacent pair in turn.
                for swap in 0..len.saturating_sub(1) {
                    let mut ids = ascending.clone();
                    ids.swap(swap, swap + 1);
                    assert!(
                        fold(&payload(&[row], &ids, 0)).is_err(),
                        "kind {kind} len {len} swap at {swap} must be refused"
                    );
                }
            }
        }
    }

    #[test]
    fn the_refusal_runs_per_range_not_across_the_shared_array() {
        // Two commands sharing one frame's member array. Each range is ascending
        // in itself, but the second starts below the first's last id — which is
        // fine, because a member range is only ever compared with itself. A
        // check written against the flat array would reject this healthy frame.
        let first = RawRow {
            object_id: 1,
            member_first: 0,
            member_count: 2,
            ..full_army()
        };
        let second = RawRow {
            object_id: 2,
            member_first: 2,
            member_count: 3,
            ..full_army()
        };
        let p = payload(&[first, second], &[400, 900, 12, 431, 800], 0);
        let s = schema();
        let mut state = State::new();
        fold_payload(&mut state, &p, false, &s.collections[0], &s.collections[1])
            .expect("each range is ascending in itself");
        assert_eq!(state.commands[&(KIND_ARMY, 1)].members, vec![400, 900]);
        assert_eq!(state.commands[&(KIND_ARMY, 2)].members, vec![12, 431, 800]);
    }

    #[test]
    fn a_delete_rows_range_is_not_subject_to_the_ordering_check() {
        // The tightening runs on the rows the frame leaves behind. A delete row's
        // range never reaches the accumulator — the fold `continue`s at
        // `world_fold.rs::fold_oob_command`, before the slice at `world_fold.rs::fold_oob_command` — so refusing it
        // would reject a frame whose folded state this schema represents
        // perfectly well. Same carve-out as d32's (`d32.rs:575`-`:581`).
        let s = schema();
        let dead = RawRow {
            object_id: 5,
            flags: FLAG_DELETED,
            member_first: 0,
            member_count: 3,
            ..RawRow::default()
        };
        let p = payload(&[dead], &[900, 431, 12], 0);
        let mut state = State::new();
        fold_payload(&mut state, &p, false, &s.collections[0], &s.collections[1])
            .expect("a delete row's order is not state");
        assert!(state.commands.is_empty(), "the delete removed nothing and added nothing");
    }

    #[test]
    fn member_range_is_ordered_is_total_on_any_slice() {
        // The rule takes the already-sliced ids, so there is no range and no
        // member array that can disagree and nothing to index out of bounds.
        assert!(member_range_is_ordered(KIND_ARMY, 1, &[]).is_ok(), "empty");
        assert!(member_range_is_ordered(KIND_ARMY, 1, &[7]).is_ok(), "single");
        assert!(member_range_is_ordered(KIND_ARMY, 1, &[1, 2, u32::MAX]).is_ok());
        assert!(member_range_is_ordered(KIND_ARMY, 1, &[2, 1]).is_err());
        assert!(
            member_range_is_ordered(KIND_ARMY, 1, &[5, 5]).is_err(),
            "equal is not ascending either, and the fold rejects it as a duplicate"
        );
        assert!(member_range_is_ordered(KIND_ARMY, 1, &[u32::MAX, 0]).is_err());
    }

    #[test]
    fn nothing_panics_on_arbitrary_payload_bytes() {
        // The public boundary reads a file from disk and must return a
        // diagnosis, never unwind.
        let s = schema();
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        for _ in 0..3_000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let len = (seed as usize) % 300;
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                *byte = seed as u8;
            }
            let _ = peek_header(&bytes);
            let _ = split_payload(&bytes);
            let _ = record_from_row(&s.collections[0], &bytes, 0);
            let _ = record_from_member_row(&s.collections[1], &bytes, 0);
            let mut state = State::new();
            let _ = fold_payload(
                &mut state,
                &bytes,
                true,
                &s.collections[0],
                &s.collections[1],
            );
        }
        // And corruption of a genuinely well-formed payload, byte by byte.
        let good = payload(&[full_army()], &[], 0);
        for i in 0..good.len() {
            let mut p = good.clone();
            p[i] ^= 0xFF;
            let mut state = State::new();
            let _ = fold_payload(&mut state, &p, false, &s.collections[0], &s.collections[1]);
        }
    }
}
