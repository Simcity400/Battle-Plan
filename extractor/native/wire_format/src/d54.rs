//! d54 `army_battleplans_depth_live` — the R2 depth annotation over d27's
//! battle-plan orders: each order's path polyline, its type-5 target states, its
//! assigned divisions, and the complete registered `COrdersGroup` census that
//! authenticates the orders' typed owners.
//!
//! Derived from `world_fold.rs:32284`-`:32385`
//! (`fold_army_battleplans_depth`, dispatched at `:4729`), its payload decoder
// @world-fold-authority domain=54 functions=fold_army_battleplans_depth lines=32284-32385 sha256=5ee0c69ae21271799ce50e86479fb049f7e1c3e3b033533829c6a9173fcc1c50 closure=10e233d71485d73cfff1afa2efd82e6e9abb680f27755edfe9eee3ac31897389
//! `decode_army_battleplans_depth_payload` (`world_fold.rs::decode_army_battleplans_depth_payload`), the accumulator
//! types at `:23877`-`:23917` and their JSON projection (`:8282`-`:8414`), the
//! shipping row grammar in
//! `extractor/native/lanes/army_battleplans_depth_live/src/lib.rs` (constants at
//! `:39`-`:76`, the emitter `encode_depth_payload` at `:639`-`:788`, the
//! status-independent validator `decode_depth_payload` at `:797`-`:1136`), the
//! `OGS1` extension in that lane's `orders_group_live.rs` (`:374`-`:487`), and
//! the collector read that produces the flags and the statuses
//! (`extractor/native/collector/src/attach_log.rs:46402`-`:46620`). There is no
//! `extractor/documents/domains/d54_*.md` dossier.
//!
//! # The fold's call order, and where the two decoders sit
//!
//! `fold_army_battleplans_depth` runs, in this order (`world_fold.rs::fold_army_battleplans_depth`):
//!
//! 1. `battleplans54::decode_depth_payload` — the lane's own validator. Its
//!    `parse_errors` become the fold's `Err`, **before any status is read**, so a
//!    malformed payload refuses the stream whatever its status word (the d83/d56
//!    shape, not the d63 one). Production's status-independent DATA authority
//!    runs the *same* function on every d54 record
//!    (`recorder_host/src/domain_data_authority.rs:8317`-`:8325`), so the two
//!    authorities agree here and there is no d49-style split to document.
//! 2. the hard-failure gate — `COMPACT_STATUS_FAILURE_MASK & !PARTIAL_DATA`
//!    (`:21776`-`:21791`): domain-hour unknown, **accumulator retained**.
//! 3. the delta-before-baseline gate (`:21792`-`:21808`): same treatment.
//! 4. `decode_army_battleplans_depth_payload` (`:21810`), the world-fold's own
//!    second decode of the same bytes.
//! 5. the expected-partial tolerance gate (`:21813`-`:21823`) — d54 IS one of the
//!    four domains in `COMPACT_EXPECTED_PARTIAL_TOLERANCES`
//!    (`compact_contract/src/lib.rs:197`-`:201`), so this branch is **live**, and
//!    it sits AFTER the row decode. It returns `Ok` having folded no row, with the
//!    accumulator retained and the domain-hour unknown.
//! 6. the fold proper: install-or-clear the orders-group manager, clear the order
//!    map on a non-delta frame, then upsert/remove each row.
//!
//! **Step 4's decoder is a strict subset of step 1's**, checked check by check:
//! every refusal `decode_army_battleplans_depth_payload` makes (short payload,
//! bad `OGS1`, unknown owner kind, a child range past its table, a declared/actual
//! child-count mismatch) is also made by `decode_depth_payload`, which adds many
//! more. So one validation pass at position 1 reproduces both, and the positional
//! difference between them is unobservable. [`split_payload`] is that pass and
//! [`the_world_fold_decoder_adds_no_refusal_of_its_own`] pins the claim.
//!
//! # Payload grammar
//!
//! A 60-byte header (`lane lib.rs:639`-`:687`), then `row_count` fixed 88-byte
//! order rows, then three 8-byte child tables — `target_count` target-state rows,
//! `path_count` path-geometry rows, `assigned_division_count` assigned-division
//! rows — and then, optionally, the `OGS1` orders-group extension. The
//! **length up to the end of the assigned table is the legacy payload**; anything
//! past it is the extension, and its absence is a legacy frame
//! (`world_fold.rs::decode_army_battleplans_depth_payload`).
//!
//! | offset | width | header word | enforced by [`split_payload`] |
//! |---|---|---|---|
//! | `+0`  | u64 | `elapsed_wall_ms`                | parsed, never enforced (frame-level scalar, codec-measurement contract) |
//! | `+8`  | u32 | `row_count`                      | table arithmetic |
//! | `+12` | u32 | `changed_rows`                   | `<= row_count` (`lane lib.rs:853`-`:857`) |
//! | `+16` | u32 | `status_flags`                    | classification input |
//! | `+20` | u32 | `observed_army_rows`             | `== 0` implies `row_count == 0`; equals the `OGS1` army count |
//! | `+24` | u32 | `observed_order_rows`            | `>= row_count` on a non-delta frame |
//! | `+28` | u32 | `target_count`                   | table arithmetic + exact child accounting |
//! | `+32` | u32 | `unsupported_geometry_rows`      | parsed, never enforced (diagnostic) |
//! | `+36` | u32 | `execution_unknown_rows`         | parsed, never enforced (diagnostic) |
//! | `+40` | u32 | `omitted_non_exact_rows`         | parsed, never enforced (diagnostic) |
//! | `+44` | u32 | `path_geometry_omitted_rows`     | parsed, never enforced (diagnostic) |
//! | `+48` | u32 | `path_count`                     | table arithmetic + exact child accounting |
//! | `+52` | u32 | `assigned_division_count`        | table arithmetic + exact child accounting |
//! | `+56` | u32 | `identity_or_active_omitted_rows`| parsed, never enforced (diagnostic) |
//!
//! Order row, 88 bytes (`lane lib.rs:740`-`:772`, decoded at
//! `world_fold.rs::decode_army_battleplans_depth_payload`):
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+0`  | u32 | `owner_id`                    | raw | key; `1..=MAX_ARMY_ID` |
//! | `+4`  | u64 | `generation`                  | raw | key |
//! | `+12` | u32 | `flags`                       | —   | — |
//! | `+16` | u32 | `slot_index`                  | raw | `< MAX_SLOT_INDEX` |
//! | `+20` | i32 | `order_type`                  | raw | **signed** (`read_i32`, `:25702`) |
//! | `+24` | u32 | `child_first`                 | —   | transport, not state |
//! | `+28` | u32 | `child_count`                 | —   | transport, not state |
//! | `+32` | u32 | `target_declared_count`       | raw | `FLAG_TARGET_STATE_COUNT` |
//! | `+36` | u32 | `target_status`               | raw | `<= STATUS_FIXTURE_REQUIRED` |
//! | `+40` | u32 | `geometry_status`             | raw | `<= STATUS_FIXTURE_REQUIRED` |
//! | `+44` | u32 | `execution_status`            | raw | `<= STATUS_FIXTURE_REQUIRED` |
//! | `+48` | u32 | `assigned_units_status`       | raw | `EXACT` on every non-deleted row |
//! | `+52` | u32 | `order_linkage_status`        | raw | `EXACT` on every non-deleted row |
//! | `+56` | u32 | `stance_status`               | raw | `<= STATUS_COLD_PREFLIGHT_ONLY` |
//! | `+60` | u32 | `aggressiveness_status`       | raw | `<= STATUS_COLD_PREFLIGHT_ONLY` |
//! | `+64` | u32 | `engine_order_instance_id`    | raw | key; `1..=MAX_ORDER_INSTANCE_ID` |
//! | `+68` | u32 | `assigned_first`              | —   | transport, not state |
//! | `+72` | u32 | `assigned_count`              | —   | transport, not state |
//! | `+76` | u32 | `active`                      | raw | `<= 1` on every non-deleted row |
//! | `+80` | u32 | `active_status`               | raw | `EXACT` on every non-deleted row |
//! | `+84` | u32 | `owner_kind`                  | raw | key; `ARMY` or `ARMY_GROUP` |
//!
//! Child row, 8 bytes for all three tables: the id at `+0` (non-zero, bounded)
//! and a reserved word at `+4` that must be zero (`lane lib.rs:1107`-`:1133`).
//! The reserved word is never read by the fold, and the capture confirms the
//! rule: **0 non-zero reserved words in 514 records**.
//!
//! **No scales anywhere.** Every word is a raw id, count, index, flag or status
//! code; there is no `scale_raw` header word, `ensure_fixed_scale` is never
//! called for this domain, and the accumulator's JSON projection
//! (`world_fold.rs:8286`-`:8318`) emits every one verbatim. There is
//! consequently no `Continuous` field in this schema and nothing to quantize —
//! see [`the_schema_has_no_continuous_field_to_quantize`].
//!
//! # Shapes: the ordinal decision
//!
//! The order table is a keyed set on the fold's own key
//! (`ArmyBattleplanDepthKey`, `world_fold.rs:23877`-`:23883`): the typed owner,
//! the engine order instance and the generation, with the fold's own comment
//! recording that `slot_index` "is a mutable attribute on the value" and not
//! identity (`:23874`-`:23875`). Measured: `slot_index` moved on 216 of the 287
//! repeat observations of a key in `runs/batch3/deepwar_a`, so keying on it would
//! have churned three quarters of the repeats and split one order into several.
//!
//! The three child arrays are the decision. Each is a `Vec<u32>` inside the
//! order's value (`:23889`-`:23893`), addressed on the wire by a `(first, count)`
//! range the collector re-packs from emission order every frame, and **the fold
//! enforces no key inside any of them**: no uniqueness check, no ordering check,
//! nothing. Step 5's test — the order must be a total function of the carried
//! fields on every payload the FOLD accepts — fails for all three, so the id
//! cannot be the key. The position is therefore the identity and rides in the
//! key as a dense ordinal, which is d11's construction-queue case
//! (`d11.rs:162`-`:172`) and, under a `Vec`-valued fold with no key, the only
//! reading that reproduces the container element for element.
//!
//! **Both readings, with their evidence** (`deepwar_a`, payload level: every one
//! of the 514 records, because the fold folds almost none of them — see the
//! coverage caveat below):
//!
//! * *For the ordinal-keyed set.* 616 live order rows carry a path; **12 of them
//!   repeat a province inside one polyline** (e.g. key `(0, 10677, 2080, 1845)`
//!   carrying `[9632, 11609, 9632]`, and `(1, 8719, 2278, 958)` carrying `11456`
//!   three times). A province-id key would collapse those rows and silently drop
//!   waypoints — the exact data-loss defect this recipe exists to prevent, on
//!   input the producer demonstrably emits. Only 208 of the 616 polylines are
//!   ascending, so key order does not recover source order either. The ordinal is
//!   unique by construction, refuses nothing, and leads nothing (it is the last
//!   key component, after the parent's four), so the keyed set's own order **is**
//!   the fold's array order within each parent run.
//! * *Against — the case for stopping.* A polyline's positions shift when a
//!   waypoint is consumed or inserted, so `(order, ordinal)` is not the same
//!   waypoint across hours. Measured over the 287 repeat observations: 210 carry
//!   an identical path and 77 change; of the 77, 44 shrink, 18 grow, 28 keep their
//!   prefix and 19 keep their suffix. The positional key costs churn: 186 of 401
//!   positions differ across those 77 changes, against 148 id-set changes a
//!   (hypothetical, impossible) province key would have emitted — about 26% more
//!   changed rows.
//! * *What decides it.* The two measured stop-conditions do **not** fire.
//!   d11's `convoy_priority_reservations` stops because consecutive samples are
//!   *pure reorders* of an identical multiset, which a keyed set stores
//!   identically and emits no delta for; here **0 of the 77 path changes is a pure
//!   reorder** — every one is a structural edit. d16's `timed_live` stops because
//!   duplicates are legitimate *and* there is no key at all *and* no position
//!   persists; here the parent order has a fold-enforced key that persists — 225
//!   of the capture's 1,207 distinct order keys are serialized more than once, and
//!   287 of the 616 path observations are repeat observations of a key already
//!   seen — and the ordinal is scoped inside that key rather than standing on its
//!   own. The existing ordered-shape instances
//!   by name — d11's deployment queue, d56's naval supremacy, d64's delayed events
//!   — and does **not** name d54, so nothing is binding by name here.
//!   Stopping would drop every waypoint of every plan, which is the domain's
//!   entire geometric content, in exchange for avoiding a 26% churn premium on a
//!   collection this capture never folds at all. The ordinal-keyed set is taken,
//!   and the counter-evidence above is the record of what a future
//!   `OrderedOccurrenceList` would buy.
//!
//! **This departs from d27, deliberately.** d27's sibling `target_states` array
//! keeps the fold's key and carries its ordinal as a *value* field
//! (`d27.rs:66`-`:80`) because `fold_army_orders` proves each state id unique
//! within its owning row (`world_fold.rs:14296`-`:14298`). d54's decoders prove
//! no such thing about any of its three child tables, so step 5's second case is
//! unavailable here and the third applies. The difference is in the grammars, not
//! in the two ports' taste.
//!
//! # The four states, field by field
//!
//! Exactly one field in this domain reaches an empty state, and it is the same
//! one d27 analysed: `target_declared_count`. The fold stores it as an `Option`
//! gated by `FLAG_TARGET_STATE_COUNT` (`world_fold.rs::decode_army_battleplans_depth_payload`) and the
//! producer is `read_type5_target_states` (`lane lib.rs:350`-`:369`):
//!
//! * `order_type != ORDER_TYPE_AREA_DEFENSE` returns `(vec![], None, UNSUPPORTED)`
//!   (`:355`-`:357`). A non-area-defence order has no type-5 target list and no
//!   declared size for one: the concept is unsupported by the row, which is record-state contract's
//!   **absent**.
//! * every other `None` comes from `read_u32_vector`'s error arms — an address
//!   overflow, an unreadable `begin` or `size`, a count past `MAX_CHILD_ROWS` or
//!   `MAX_INLINE_CHILD_ROWS` — and carries `VALIDATION_FAILED` or `OVERFLOW`
//!   (`lane lib.rs:1150`-`:1183`). The engine holds a size; this sample could not
//!   read or accept it. That is record-state contract's **unknown**.
//! * a set gate is always a real value, including a real **zero**: `count == 0`
//!   returns `Ok((vec![], 0))` (`:1163`-`:1165`), so an empty target list under a
//!   set gate is measured emptiness.
//!
//! `target_status` is the disambiguator, it is a value field on the same record,
//! and the reader always has it. Nothing else in this domain is optional,
//! nullable or unknown-capable — see
//! [`only_target_declared_count_reaches_an_empty_state`].
//!
//! # Six words that are grammar, not fields (the d62/d24 rule)
//!
//! A value the accepted grammar hard-enforces as a function of carried state is
//! not a field. [`split_payload`] enforces each of these and
//! [`the_grammar_constants_are_enforced_not_encoded`] pins them:
//!
//! * `active_status`, `assigned_units_status` and `order_linkage_status` are
//!   `STATUS_EXACT` on **every non-deleted row** (`lane lib.rs:975`-`:990`), and a
//!   deleted row is a tombstone that never enters the accumulator. So all three
//!   are constant over everything this schema can hold.
//! * `FLAG_ACTIVE_VALUE` must be set on every non-deleted row (`:975`-`:979`).
//! * `FLAG_ASSIGNED_DIVISIONS` is set exactly when `assigned_count != 0`
//!   (`:1000`-`:1010`), and `FLAG_PATH_GEOMETRY` requires `child_count != 0`
//!   (`:1044`-`:1048`) — both are recoverable from whether the child collection
//!   carries a row for the order.
//! * `registered_entry_count` in the `OGS1` extension must equal
//!   `army_rows + army_group_count` (`orders_group_live.rs:436`-`:438`), and its
//!   `flags` word must be exactly `EXTENSION_FLAG_COMPLETE_MANAGER`
//!   (`:418`-`:420`).
//!
//! `FLAG_TARGET_STATES` is the one flag bit that is **not** recoverable and is
//! still not carried: the validator permits it with `child_count == 0`
//! (`:1017`-`:1032`), and the fold then stores the same empty `Vec` it stores
//! without the flag (`world_fold.rs::decode_army_battleplans_depth_payload`). The distinction does not
//! survive into the fold's state, so mirroring the fold loses nothing.
//!
//! The two flag bits that ARE independent data are carried as Booleans:
//! `FLAG_NON_TYPE5_GEOMETRY_BLOCKED` and `FLAG_EXECUTION_BLOCKED`. The emitter
//! sets each from a status code (`lane lib.rs:709`-`:714`) but **no validator
//! couples them**, so on the accepted grammar they are independent witness bits —
//! d56's rule, not d62's. Both are clear on all 514 records of the capture, which
//! is a coverage caveat, not a licence to drop them.
//!
//! # The orders-group manager
//!
//! The `OGS1` extension is a complete census of the engine's registered
//! type-53 reference table: one row per `COrdersGroup` army with three exact
//! scalars, plus the strict-sorted ids of every `CArmyGroup`
//! (`orders_group_live.rs:398`-`:487`). The fold keeps it in its own accumulator
//! (`world_fold.rs::fold_army_battleplans_depth`) and publishes it beside the orders
//! (`world_fold.rs::army_battleplans_depth_extra_json`). Both id lists are **strict-sorted and disjoint**, enforced
//! by the decoder, so both are ordinary keyed sets with no ordering problem.
//!
//! A **legacy frame carries no extension at all**, and the fold's rule is that it
//! must not inherit one: `army_orders_group_manager = None`, `known = false`
//! (`world_fold.rs::fold_army_battleplans_depth`). Three per-hour states therefore exist, and the
//! [`ORDERS_GROUP_MANAGER`] singleton (d52's `SINGLETON_KEY_COMPONENT`
//! convention, `d52.rs:401`-`:412`) is what keeps them apart in a keyed set:
//!
//! | frame | manager singleton | [`ORDERS_GROUP_ARMIES`] | [`ORDERS_GROUP_ARMY_GROUPS`] |
//! |---|---|---|---|
//! | legacy (no `OGS1`) | **row absent** | empty — unmeasured | empty — unmeasured |
//! | `OGS1` v1 | present, `owner_identities_complete = false` | measured | empty, `army_group_count` says how many were NOT carried |
//! | `OGS1` v2 | present, `owner_identities_complete = true` | measured | measured |
//!
//! Without the singleton, "no manager this hour" and "a measured empty census"
//! would be the same bytes — the `(domain, part)` gap scope gap contract does not have
//! (d17's war graph). With it, the distinction is in band
//! and costs two bytes an hour.
//!
//! # Coverage caveats — what `runs/batch3/deepwar_a` never exercised
//!
//! Measured over the capture's 514 d54 records: 266 hours fold, 241 are withheld
//! by the expected-partial tolerance and 7 are excluded by
//! `COMPACT_STATUS_SOURCE_UNAVAILABLE`. **610 of the capture's 616 live order rows
//! sit in records the fold withholds**, so the order table and all three child
//! collections are very nearly untouched by the measurement: 6 live rows across
//! the 266 measured hours, a peak accumulator of 2 rows, and rows at all in only
//! 4 of them. Concretely, on this capture:
//!
//! * the **path-geometry** collection is exercised by one folded row; the ordinal
//!   decision above therefore rests on payload-level evidence and on the grammar,
//!   not on folded state;
//! * the **target-state** and **assigned-division** collections have **zero**
//!   coverage at either level — no record in the capture sets
//!   `FLAG_TARGET_STATES` or `FLAG_ASSIGNED_DIVISIONS`, because every observed
//!   order is `ORDER_TYPE_MOVE` (84) or `ORDER_TYPE_LINE` (532) and neither
//!   carries type-5 targets;
//! * `target_declared_count`'s **absent** state is exercised on every one of the
//!   1,674 rows and its value and unknown states on none;
//! * both witness Booleans are clear on every row;
//! * the legacy (no-`OGS1`) frame occurs exactly 7 times and those are exactly the
//!   7 `SOURCE_UNAVAILABLE` records, so the fold **never folds a legacy frame
//!   here**: the manager's clear-on-legacy rule is implemented and tested under
//!   the mirror-the-fold rule and measured by nothing;
//! * `OGS1` v1 never occurs — all 507 extensions are v2 — so
//!   `owner_identities_complete = false` has no coverage.
//!
//! What would exercise them: a capture in which d27 and d28 stop reporting
//! tolerated partials (the survey's own conclusion — "d54 clears only when d27/d28
//! stop reporting partials"), and one containing area-defence orders.

use std::collections::BTreeSet;

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 54;

/// The battle-plan orders themselves, keyed by the fold's `ArmyBattleplanDepthKey`.
pub const ORDERS: u32 = 1;
/// One row per `(order, ordinal)` of the order's type-5 target-state vector.
pub const ORDER_TARGET_STATES: u32 = 2;
/// One row per `(order, ordinal)` of the order's path polyline.
pub const ORDER_PATH_GEOMETRY: u32 = 3;
/// One row per `(order, ordinal)` of the order's assigned-division vector.
pub const ORDER_ASSIGNED_DIVISIONS: u32 = 4;
/// The `OGS1` census header, as a singleton row (see [`SINGLETON_KEY_COMPONENT`]).
pub const ORDERS_GROUP_MANAGER: u32 = 5;
/// One row per registered `COrdersGroup` army.
pub const ORDERS_GROUP_ARMIES: u32 = 6;
/// One row per registered `CArmyGroup` id. Key-only.
pub const ORDERS_GROUP_ARMY_GROUPS: u32 = 7;

/// Field ids are stable and append-only; schema-identity contract forbids reuse for a different field.
/// They are unique across the whole domain, not merely within a collection, so a
/// misrouted record cannot silently land on a same-numbered field elsewhere.
pub mod field {
    // --- ORDERS ---
    pub const OWNER_KIND: u32 = 1;
    pub const OWNER_ID: u32 = 2;
    pub const ENGINE_ORDER_INSTANCE_ID: u32 = 3;
    pub const GENERATION: u32 = 4;
    pub const SLOT_INDEX: u32 = 5;
    pub const ORDER_TYPE: u32 = 6;
    pub const TARGET_DECLARED_COUNT: u32 = 7;
    pub const TARGET_STATUS: u32 = 8;
    pub const GEOMETRY_STATUS: u32 = 9;
    pub const EXECUTION_STATUS: u32 = 10;
    pub const STANCE_STATUS: u32 = 11;
    pub const AGGRESSIVENESS_STATUS: u32 = 12;
    pub const ACTIVE: u32 = 13;
    pub const NON_TYPE5_GEOMETRY_BLOCKED: u32 = 14;
    pub const EXECUTION_BLOCKED: u32 = 15;

    // --- ORDER_TARGET_STATES ---
    pub const TS_OWNER_KIND: u32 = 20;
    pub const TS_OWNER_ID: u32 = 21;
    pub const TS_ENGINE_ORDER_INSTANCE_ID: u32 = 22;
    pub const TS_GENERATION: u32 = 23;
    pub const TS_ORDINAL: u32 = 24;
    pub const TS_STATE_ID: u32 = 25;

    // --- ORDER_PATH_GEOMETRY ---
    pub const PG_OWNER_KIND: u32 = 30;
    pub const PG_OWNER_ID: u32 = 31;
    pub const PG_ENGINE_ORDER_INSTANCE_ID: u32 = 32;
    pub const PG_GENERATION: u32 = 33;
    pub const PG_ORDINAL: u32 = 34;
    pub const PG_PROVINCE_ID: u32 = 35;

    // --- ORDER_ASSIGNED_DIVISIONS ---
    pub const AD_OWNER_KIND: u32 = 40;
    pub const AD_OWNER_ID: u32 = 41;
    pub const AD_ENGINE_ORDER_INSTANCE_ID: u32 = 42;
    pub const AD_GENERATION: u32 = 43;
    pub const AD_ORDINAL: u32 = 44;
    pub const AD_DIVISION_ID: u32 = 45;

    // --- ORDERS_GROUP_MANAGER ---
    pub const MANAGER_SINGLETON: u32 = 50;
    pub const ARMY_GROUP_COUNT: u32 = 51;
    pub const OWNER_IDENTITIES_COMPLETE: u32 = 52;

    // --- ORDERS_GROUP_ARMIES ---
    pub const ARMY_ID: u32 = 60;
    pub const EXECUTION_TYPE: u32 = 61;
    pub const COHESION_TYPE: u32 = 62;
    pub const LEADER_PROXIMITY: u32 = 63;

    // --- ORDERS_GROUP_ARMY_GROUPS ---
    pub const ARMY_GROUP_ID: u32 = 70;
}

// --- Transcribed shipping constants -----------------------------------------
//
// d54 is not admitted to the recipe's production-codec dependency surface, and
// its codec lives in a capture-lane crate that the library may not depend on.
// So these are hand
// transcriptions of `lanes/army_battleplans_depth_live/src/lib.rs` and its
// `orders_group_live.rs`, in the d67/d70 class. Two machine checks hold the copy
// to the original, both in `bin/bench_real_d54.rs`: the status-bit values are
// asserted equal to `compact_contract`'s, and every one of the 514 real records
// must reproduce its own payload length exactly from these table strides, so a
// wrong stride fails the run on the first record rather than at review.

/// `battleplans54::HEADER_LEN` (`lane lib.rs:39`).
pub const HEADER_LEN: usize = 60;
/// `battleplans54::ROW_LEN` (`lane lib.rs:40`).
pub const ROW_LEN: usize = 88;
/// `battleplans54::TARGET_ROW_LEN` (`lane lib.rs:41`).
pub const TARGET_ROW_LEN: usize = 8;
/// `battleplans54::PATH_GEOMETRY_ROW_LEN` (`lane lib.rs:42`).
pub const PATH_GEOMETRY_ROW_LEN: usize = 8;
/// `battleplans54::ASSIGNED_DIVISION_ROW_LEN` (`lane lib.rs:43`).
pub const ASSIGNED_DIVISION_ROW_LEN: usize = 8;

/// `lane lib.rs:45`-`:52`.
pub const FLAG_DELETED: u32 = 1;
pub const FLAG_TARGET_STATES: u32 = 1 << 1;
pub const FLAG_TARGET_STATE_COUNT: u32 = 1 << 2;
pub const FLAG_NON_TYPE5_GEOMETRY_BLOCKED: u32 = 1 << 3;
pub const FLAG_EXECUTION_BLOCKED: u32 = 1 << 4;
pub const FLAG_PATH_GEOMETRY: u32 = 1 << 5;
pub const FLAG_ASSIGNED_DIVISIONS: u32 = 1 << 6;
pub const FLAG_ACTIVE_VALUE: u32 = 1 << 7;
/// Every bit the validator admits (`lane lib.rs:927`-`:941`).
pub const KNOWN_ROW_FLAGS: u32 = FLAG_DELETED
    | FLAG_TARGET_STATES
    | FLAG_TARGET_STATE_COUNT
    | FLAG_NON_TYPE5_GEOMETRY_BLOCKED
    | FLAG_EXECUTION_BLOCKED
    | FLAG_PATH_GEOMETRY
    | FLAG_ASSIGNED_DIVISIONS
    | FLAG_ACTIVE_VALUE;

/// `lane lib.rs:54`-`:61`. The whole status vocabulary is `0..=7`, which is this
/// schema's enum cardinality for every status field.
pub const STATUS_NONE: u32 = 0;
pub const STATUS_EXACT: u32 = 1;
pub const STATUS_UNKNOWN: u32 = 2;
pub const STATUS_UNSUPPORTED: u32 = 3;
pub const STATUS_OVERFLOW: u32 = 4;
pub const STATUS_VALIDATION_FAILED: u32 = 5;
pub const STATUS_FIXTURE_REQUIRED: u32 = 6;
pub const STATUS_COLD_PREFLIGHT_ONLY: u32 = 7;
/// schema-kind contract cardinality for every status field: codes `0..=7`.
pub const STATUS_CARD: u32 = 8;

/// `orders_group_live::ORDER_OWNER_KIND_ARMY` / `_ARMY_GROUP` (`:46`-`:47`).
pub const ORDER_OWNER_KIND_ARMY: u32 = 0;
pub const ORDER_OWNER_KIND_ARMY_GROUP: u32 = 1;

/// `lane lib.rs:72`-`:76`. Order types are `i32` and the validator constrains
/// them only through the child-table flags, so this is not an enum: a row with
/// neither flag may carry any `i32`.
pub const ORDER_TYPE_MOVE: i32 = 1;
pub const ORDER_TYPE_LINE: i32 = 2;
pub const ORDER_TYPE_INVASION: i32 = 3;
pub const ORDER_TYPE_PARADROP: i32 = 4;
pub const ORDER_TYPE_AREA_DEFENSE: i32 = 5;

/// `lane lib.rs:79`-`:86`. Only the bounds the *validator* applies are here; the
/// collector-side read bounds (`MAX_CHILD_ROWS`, `MAX_INLINE_CHILD_ROWS`) never
/// reach a payload and are named in the docs instead.
///
/// Both id bounds use the structural width of the signed ids accepted here.
/// The count bounds named above deliberately do not move with them — they
/// bound reads, not identities.
pub const MAX_STATE_ID: u32 = (i32::MAX as u32) - 1;
pub const MAX_PROVINCE_ID: u32 = (i32::MAX as u32) - 1;
pub const MAX_ARMY_ID: u32 = i32::MAX as u32;
pub const MAX_SLOT_INDEX: u32 = 64;
pub const MAX_ORDER_INSTANCE_ID: u32 = 100_000_000;
pub const MAX_DIVISION_ID: u32 = i32::MAX as u32;

/// `orders_group_live.rs:39`-`:47`.
pub const EXTENSION_MAGIC: [u8; 4] = *b"OGS1";
pub const EXTENSION_VERSION_V1: u16 = 1;
pub const EXTENSION_VERSION_V2: u16 = 2;
pub const EXTENSION_HEADER_LEN: usize = 24;
pub const EXTENSION_ROW_LEN: usize = 16;
pub const EXTENSION_FLAG_COMPLETE_MANAGER: u32 = 1;
/// `orders_group_live.rs:55`-`:56`. `MAX_TYPED_EXECUTION_TYPE` is deliberately
/// absent: the lane's own comment says the execution writer "copies a raw `u32`"
/// and that exact live objects "can retain an unlabelled raw ordinal outside this
/// UI subset" (`:49`-`:54`), the decoder bounds only the other two scalars
/// (`:450`-`:453`), and the capture proves it — 507 army rows carry
/// `execution_type = 4`.
pub const MAX_COHESION_TYPE: u32 = 3;
pub const MAX_LEADER_PROXIMITY: u32 = 4;

/// `COMPACT_STATUS_DELTA_DATA` (`compact_contract/src/status_flags_base.rs:16`).
/// Named here rather than depended on: the library half of this crate is the
/// reference implementation and must not pull in the shipping contract crate.
/// `bench_real_d54.rs`'s `assert_status_predicates_agree` pins the value.
pub const STATUS_DELTA_DATA: u32 = 1 << 10;

/// The constant singleton key [`ORDERS_GROUP_MANAGER`] uses. Zero, so a reader
/// that ever sees a second key there has seen a malformed frame rather than a
/// second census (d52's / d68's `SINGLETON_KEY_COMPONENT` convention).
pub const SINGLETON_KEY_COMPONENT: u64 = 0;

pub fn manager_singleton_key() -> Key {
    key(&[SINGLETON_KEY_COMPONENT])
}

// --- Schema ------------------------------------------------------------------

/// The four key components every child collection inherits from its order,
/// followed by the generated ordinal. Written once so the three child
/// collections cannot drift apart in their key shape.
fn child_key_fields(owner_kind: u32, owner_id: u32, instance: u32, generation: u32, ordinal: u32) -> Vec<Field> {
    vec![
        Field::new(owner_kind, Kind::Unsigned).key(0),
        Field::new(owner_id, Kind::Unsigned).key(1),
        Field::new(instance, Kind::Unsigned).key(2),
        Field::new(generation, Kind::Unsigned).key(3),
        // The generated dense position within this order's own child run. Step
        // 5's third case: the rows have no identity of their own and the
        // position is one. Last component, so the keyed set's own order is the
        // fold's array order inside each parent run.
        Field::new(ordinal, Kind::Unsigned).key(4),
    ]
}

pub fn schema() -> Schema {
    use field as f;

    let orders = Collection {
        collection_id: ORDERS,
        // The fold's own key persists across hours — 96 of the capture's 329
        // distinct order keys are observed more than once, one of them 24 times
        // — which is what a keyed set needs, and only a keyed set has a delta
        // grammar (collection-shape contract).
        shape: Shape::KeyedSet,
        fields: vec![
            // `ArmyBattleplanDepthKey` (`world_fold.rs:23877`-`:23883`). All four
            // are runtime instance identities or an engine discriminator: no
            // content manifest resolves any of them, so `Unsigned`, on d20's
            // `div_id`/`div_type` precedent.
            Field::new(f::OWNER_KIND, Kind::Unsigned).key(0),
            Field::new(f::OWNER_ID, Kind::Unsigned).key(1),
            Field::new(f::ENGINE_ORDER_INSTANCE_ID, Kind::Unsigned).key(2),
            Field::new(f::GENERATION, Kind::Unsigned).key(3),
            // "a mutable attribute on the value" (`world_fold.rs:23874`), and
            // measured as one: it changes on 216 of 287 key repeats.
            Field::new(f::SLOT_INDEX, Kind::Unsigned),
            // `read_i32` (`world_fold.rs:25702`). Not an `Enum`: a row carrying
            // neither child flag may hold any `i32` and the validator never
            // bounds it.
            Field::new(f::ORDER_TYPE, Kind::Signed),
            // The one field that reaches an empty state; see the module docs.
            Field::new(f::TARGET_DECLARED_COUNT, Kind::Unsigned)
                .optional()
                .unknown_capable(),
            // The status vocabulary is `0..=7` (`lane lib.rs:54`-`:61`), which is
            // what `battleplans_depth_status_name` maps (`world_fold.rs:25850`).
            // The validator bounds these three at `STATUS_FIXTURE_REQUIRED`, which
            // `split_payload` enforces; the cardinality stays the vocabulary's so
            // the enum means the same thing in every collection.
            Field::new(f::TARGET_STATUS, Kind::Enum).enum_card(STATUS_CARD),
            Field::new(f::GEOMETRY_STATUS, Kind::Enum).enum_card(STATUS_CARD),
            Field::new(f::EXECUTION_STATUS, Kind::Enum).enum_card(STATUS_CARD),
            // The collector hard-codes both to `STATUS_UNKNOWN`
            // (`attach_log.rs:46592`-`:46593`), but the validator admits the whole
            // vocabulary, so they are measured witnesses and not constants.
            Field::new(f::STANCE_STATUS, Kind::Enum).enum_card(STATUS_CARD),
            Field::new(f::AGGRESSIVENESS_STATUS, Kind::Enum).enum_card(STATUS_CARD),
            Field::new(f::ACTIVE, Kind::Boolean),
            // Two independent witness bits: the emitter derives each from a status
            // code (`lane lib.rs:709`-`:714`) but no validator couples them, so
            // dropping them would lose data on input the fold accepts (d56's rule).
            Field::new(f::NON_TYPE5_GEOMETRY_BLOCKED, Kind::Boolean),
            Field::new(f::EXECUTION_BLOCKED, Kind::Boolean),
        ],
    };

    let target_states = Collection {
        collection_id: ORDER_TARGET_STATES,
        shape: Shape::KeyedSet,
        fields: {
            let mut fields = child_key_fields(
                f::TS_OWNER_KIND,
                f::TS_OWNER_ID,
                f::TS_ENGINE_ORDER_INSTANCE_ID,
                f::TS_GENERATION,
                f::TS_ORDINAL,
            );
            // A state id is content-defined: state 1234 is whatever state the mod
            // set defines, so a manifest must resolve it (content-identity contract, d20's province
            // precedent). A `ContentId` **value**, not a key — the ordinal is the
            // key, so the heavier ContentId-key retrofit class does not apply here.
            fields.push(Field::new(f::TS_STATE_ID, Kind::ContentId));
            fields
        },
    };

    let path_geometry = Collection {
        collection_id: ORDER_PATH_GEOMETRY,
        shape: Shape::KeyedSet,
        fields: {
            let mut fields = child_key_fields(
                f::PG_OWNER_KIND,
                f::PG_OWNER_ID,
                f::PG_ENGINE_ORDER_INSTANCE_ID,
                f::PG_GENERATION,
                f::PG_ORDINAL,
            );
            fields.push(Field::new(f::PG_PROVINCE_ID, Kind::ContentId));
            fields
        },
    };

    let assigned_divisions = Collection {
        collection_id: ORDER_ASSIGNED_DIVISIONS,
        shape: Shape::KeyedSet,
        fields: {
            let mut fields = child_key_fields(
                f::AD_OWNER_KIND,
                f::AD_OWNER_ID,
                f::AD_ENGINE_ORDER_INSTANCE_ID,
                f::AD_GENERATION,
                f::AD_ORDINAL,
            );
            // A division id is a runtime instance identity created during play —
            // d20's `div_id` — so `Unsigned`, not `ContentId`.
            fields.push(Field::new(f::AD_DIVISION_ID, Kind::Unsigned));
            fields
        },
    };

    let orders_group_manager = Collection {
        collection_id: ORDERS_GROUP_MANAGER,
        shape: Shape::KeyedSet,
        fields: vec![
            // The constant singleton key. `Unsigned`: nothing content-defined
            // names it and no manifest resolves it.
            Field::new(f::MANAGER_SINGLETON, Kind::Unsigned).key(0),
            // Known even on an `OGS1` v1 extension, which carries the count but
            // none of the ids (`orders_group_live.rs:464`-`:478`).
            Field::new(f::ARMY_GROUP_COUNT, Kind::Unsigned),
            // v2 carries every typed owner identity; v1 does not
            // (`orders_group_live.rs:481`). Without this the reader cannot tell an
            // unmeasured army-group census from a measured empty one.
            Field::new(f::OWNER_IDENTITIES_COMPLETE, Kind::Boolean),
        ],
    };

    let orders_group_armies = Collection {
        collection_id: ORDERS_GROUP_ARMIES,
        shape: Shape::KeyedSet,
        fields: vec![
            // Strict-sorted, non-zero, `<= MAX_ARMY_ID`
            // (`orders_group_live.rs:447`-`:449`) — a fold-enforced key.
            Field::new(f::ARMY_ID, Kind::Unsigned).key(0),
            // Raw, unbounded by the decoder; see [`MAX_COHESION_TYPE`]'s comment.
            Field::new(f::EXECUTION_TYPE, Kind::Unsigned),
            // Bounded at 3 and 4 respectively (`orders_group_live.rs:450`-`:453`),
            // so the code space is closed and an `Enum` keeps it closed.
            Field::new(f::COHESION_TYPE, Kind::Enum).enum_card(MAX_COHESION_TYPE + 1),
            Field::new(f::LEADER_PROXIMITY, Kind::Enum).enum_card(MAX_LEADER_PROXIMITY + 1),
        ],
    };

    let orders_group_army_groups = Collection {
        collection_id: ORDERS_GROUP_ARMY_GROUPS,
        shape: Shape::KeyedSet,
        // Key-only: the census carries nothing but the id
        // (`orders_group_live.rs:466`-`:477`). Precedented (d28's `MEMBERS`,
        // d39/d40's `variants`), and its mapper checks `collection_id` explicitly
        // because a field loop over nothing cannot fail on the wrong collection.
        fields: vec![Field::new(f::ARMY_GROUP_ID, Kind::Unsigned).key(0)],
    };

    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact domain in this stream.
        cadence_log2: 0,
        collections: vec![
            orders,
            target_states,
            path_geometry,
            assigned_divisions,
            orders_group_manager,
            orders_group_armies,
            orders_group_army_groups,
        ],
        event_part: None,
    }
}

// --- Header ------------------------------------------------------------------

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("4 bytes"))
}

fn i32_at(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("4 bytes"))
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("8 bytes"))
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().expect("2 bytes"))
}

/// The 60-byte payload header. Every word is parsed; the module docs say which
/// are enforced and which are diagnostics the validator never reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub row_count: u32,
    pub changed_rows: u32,
    pub status_flags: u32,
    pub observed_army_rows: u32,
    pub observed_order_rows: u32,
    pub target_count: u32,
    pub unsupported_geometry_rows: u32,
    pub execution_unknown_rows: u32,
    pub omitted_non_exact_rows: u32,
    pub path_geometry_omitted_rows: u32,
    pub path_count: u32,
    pub assigned_division_count: u32,
    pub identity_or_active_omitted_rows: u32,
}

impl PayloadHeader {
    pub fn is_delta(&self) -> bool {
        self.status_flags & STATUS_DELTA_DATA != 0
    }
}

/// Read the header alone, without paying for the body.
///
/// A caller that needs to *classify* a record — delta or not, which status —
/// must be able to do it without the table arithmetic, because the fold's own
/// hard-failure and delta-before-baseline exits sit between the two.
pub fn peek_header(payload: &[u8]) -> core::result::Result<PayloadHeader, String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "battleplans payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    Ok(PayloadHeader {
        elapsed_wall_ms: u64_at(payload, 0),
        row_count: u32_at(payload, 8),
        changed_rows: u32_at(payload, 12),
        status_flags: u32_at(payload, 16),
        observed_army_rows: u32_at(payload, 20),
        observed_order_rows: u32_at(payload, 24),
        target_count: u32_at(payload, 28),
        unsupported_geometry_rows: u32_at(payload, 32),
        execution_unknown_rows: u32_at(payload, 36),
        omitted_non_exact_rows: u32_at(payload, 40),
        path_geometry_omitted_rows: u32_at(payload, 44),
        path_count: u32_at(payload, 48),
        assigned_division_count: u32_at(payload, 52),
        identity_or_active_omitted_rows: u32_at(payload, 56),
    })
}

// --- Rows --------------------------------------------------------------------

/// One decoded 88-byte order row. `child_first`/`child_count`/`assigned_first`
/// are transport into the shared child tables and are not carried here: the
/// ranges they name are resolved into the three child collections by
/// [`State::fold_payload`], exactly as d27 flattens its own.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OrderRow {
    pub owner_kind: u32,
    pub owner_id: u32,
    pub engine_order_instance_id: u32,
    pub generation: u64,
    pub flags: u32,
    pub slot_index: u32,
    pub order_type: i32,
    pub child_first: u32,
    pub child_count: u32,
    pub assigned_first: u32,
    pub assigned_count: u32,
    pub target_declared_count: Option<u32>,
    pub target_status: u32,
    pub geometry_status: u32,
    pub execution_status: u32,
    pub assigned_units_status: u32,
    pub order_linkage_status: u32,
    pub stance_status: u32,
    pub aggressiveness_status: u32,
    pub active: bool,
    pub active_status: u32,
}

impl OrderRow {
    pub fn deleted(&self) -> bool {
        self.flags & FLAG_DELETED != 0
    }

    pub fn has_targets(&self) -> bool {
        self.flags & FLAG_TARGET_STATES != 0
    }

    pub fn has_path(&self) -> bool {
        self.flags & FLAG_PATH_GEOMETRY != 0
    }
}

fn check_row_len(row: &[u8]) -> core::result::Result<(), String> {
    if row.len() != ROW_LEN {
        return Err(format!(
            "battleplans order row is {} bytes, not {ROW_LEN}",
            row.len()
        ));
    }
    Ok(())
}

/// Decode one 88-byte order row, applying every per-row rule the shipping
/// validator applies (`lane lib.rs:888`-`:1063`). The cross-row rules — child
/// range containment and the exact child accounting — need the header and live
/// in [`split_payload`].
pub fn parse_order_row(row: &[u8]) -> core::result::Result<OrderRow, String> {
    check_row_len(row)?;
    let owner_id = u32_at(row, 0);
    let generation = u64_at(row, 4);
    let flags = u32_at(row, 12);
    let slot_index = u32_at(row, 16);
    let order_type = i32_at(row, 20);
    let child_first = u32_at(row, 24);
    let child_count = u32_at(row, 28);
    let target_declared = u32_at(row, 32);
    let target_status = u32_at(row, 36);
    let geometry_status = u32_at(row, 40);
    let execution_status = u32_at(row, 44);
    let assigned_units_status = u32_at(row, 48);
    let order_linkage_status = u32_at(row, 52);
    let stance_status = u32_at(row, 56);
    let aggressiveness_status = u32_at(row, 60);
    let engine_order_instance_id = u32_at(row, 64);
    let assigned_first = u32_at(row, 68);
    let assigned_count = u32_at(row, 72);
    let active_value = u32_at(row, 76);
    let active_status = u32_at(row, 80);
    let owner_kind = u32_at(row, 84);

    // `lane lib.rs:919`-`:926`.
    if owner_id == 0 || owner_id > MAX_ARMY_ID {
        return Err(format!("battleplans row has bad owner id {owner_id}"));
    }
    if slot_index >= MAX_SLOT_INDEX {
        return Err(format!("battleplans row has bad slot index {slot_index}"));
    }
    // `:927`-`:941`.
    if flags & !KNOWN_ROW_FLAGS != 0 {
        return Err(format!("battleplans row has unknown flags {flags:#x}"));
    }
    let deleted = flags & FLAG_DELETED != 0;
    let has_targets = flags & FLAG_TARGET_STATES != 0;
    let has_path = flags & FLAG_PATH_GEOMETRY != 0;
    let has_assigned = flags & FLAG_ASSIGNED_DIVISIONS != 0;
    let has_active = flags & FLAG_ACTIVE_VALUE != 0;
    // `:942`-`:946`, and `world_fold.rs::decode_army_battleplans_depth_payload`: the two child ranges
    // share one `(first, count)` pair, so a row cannot claim both.
    if has_targets && has_path {
        return Err("battleplans row has both target and path children".into());
    }
    // `:947`-`:960`.
    if target_status > STATUS_FIXTURE_REQUIRED
        || geometry_status > STATUS_FIXTURE_REQUIRED
        || execution_status > STATUS_FIXTURE_REQUIRED
        || assigned_units_status > STATUS_COLD_PREFLIGHT_ONLY
        || order_linkage_status > STATUS_COLD_PREFLIGHT_ONLY
        || stance_status > STATUS_COLD_PREFLIGHT_ONLY
        || aggressiveness_status > STATUS_COLD_PREFLIGHT_ONLY
        || active_status > STATUS_COLD_PREFLIGHT_ONLY
    {
        return Err("battleplans row has a status code outside the vocabulary".into());
    }
    // `:961`-`:973`: a tombstone carries identity and nothing else.
    if deleted
        && (child_first != 0
            || child_count != 0
            || target_declared != 0
            || assigned_first != 0
            || assigned_count != 0
            || active_value != 0
            || active_status != STATUS_NONE)
    {
        return Err("battleplans deleted row carries child fields".into());
    }
    // `:974`-`:978`, and the fold's own second decode (`world_fold.rs::decode_army_battleplans_depth_payload`).
    if !matches!(
        owner_kind,
        ORDER_OWNER_KIND_ARMY | ORDER_OWNER_KIND_ARMY_GROUP
    ) {
        return Err(format!("battleplans row has unknown owner kind {owner_kind}"));
    }
    // `:979`-`:983`.
    if engine_order_instance_id == 0 || engine_order_instance_id > MAX_ORDER_INSTANCE_ID {
        return Err(format!(
            "battleplans row has bad engine order instance id {engine_order_instance_id}"
        ));
    }
    // `:984`-`:998`: the three grammar constants. Every row that reaches the
    // accumulator carries them, which is why they are enforced here and not
    // encoded.
    if !deleted && (!has_active || active_status != STATUS_EXACT || active_value > 1) {
        return Err("battleplans live row lacks an exact active state".into());
    }
    if !deleted && assigned_units_status != STATUS_EXACT {
        return Err("battleplans live row lacks an exact assigned-unit status".into());
    }
    if !deleted && order_linkage_status != STATUS_EXACT {
        return Err("battleplans live row lacks an exact order-linkage status".into());
    }
    // `:999`-`:1011`.
    if has_assigned {
        if assigned_count == 0 {
            return Err("battleplans row flags assigned divisions with a zero count".into());
        }
    } else if assigned_count != 0 {
        return Err("battleplans row has an assigned count without its flag".into());
    }
    // `:1013`-`:1017`.
    if !deleted && order_type == ORDER_TYPE_AREA_DEFENSE && target_status != STATUS_EXACT {
        return Err("battleplans type-5 row is not exact".into());
    }
    // `:1018`-`:1023`.
    if has_targets && order_type != ORDER_TYPE_AREA_DEFENSE {
        return Err("battleplans non-type-5 row carries target children".into());
    }
    // `:1032`-`:1051`.
    if has_path {
        if !matches!(
            order_type,
            ORDER_TYPE_MOVE | ORDER_TYPE_LINE | ORDER_TYPE_INVASION | ORDER_TYPE_PARADROP
        ) {
            return Err(format!(
                "battleplans row of order type {order_type} carries path geometry"
            ));
        }
        if order_type == ORDER_TYPE_PARADROP && child_count != 2 {
            return Err("battleplans paradrop row does not carry exactly source and target".into());
        }
        if child_count == 0 {
            return Err("battleplans row flags path geometry with a zero count".into());
        }
        if geometry_status != STATUS_EXACT {
            return Err("battleplans row carries path geometry without exact geometry".into());
        }
    }
    // `:1057`-`:1062`.
    let has_target_count = flags & FLAG_TARGET_STATE_COUNT != 0;
    if !has_target_count && target_declared != 0 {
        return Err("battleplans row has a target count without its flag".into());
    }

    Ok(OrderRow {
        owner_kind,
        owner_id,
        engine_order_instance_id,
        generation,
        flags,
        slot_index,
        order_type,
        child_first,
        child_count,
        assigned_first,
        assigned_count,
        target_declared_count: has_target_count.then_some(target_declared),
        target_status,
        geometry_status,
        execution_status,
        assigned_units_status,
        order_linkage_status,
        stance_status,
        aggressiveness_status,
        active: active_value != 0,
        active_status,
    })
}

/// One 8-byte child row: the id and the reserved word that must be zero
/// (`lane lib.rs:1107`-`:1133`). `max_value` is the table's own bound.
pub fn parse_child_row(row: &[u8], max_value: u32, what: &str) -> core::result::Result<u32, String> {
    if row.len() != TARGET_ROW_LEN {
        return Err(format!(
            "battleplans {what} child row is {} bytes, not {TARGET_ROW_LEN}",
            row.len()
        ));
    }
    let id = u32_at(row, 0);
    let reserved = u32_at(row, 4);
    if id == 0 || id > max_value {
        return Err(format!("battleplans {what} child row has bad id {id}"));
    }
    if reserved != 0 {
        return Err(format!(
            "battleplans {what} child row has a non-zero reserved word {reserved}"
        ));
    }
    Ok(id)
}

// --- The OGS1 extension ------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArmyOrdersGroupRow {
    pub army_id: u32,
    pub execution_type: u32,
    pub cohesion_type: u32,
    pub leader_proximity: u32,
}

/// The decoded `OGS1` extension (`orders_group_live.rs:398`-`:487`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrdersGroupExtension {
    /// v2 carries every typed owner identity; v1 carries only the count.
    pub owner_identities_complete: bool,
    pub army_group_count: u32,
    pub army_group_ids: Vec<u32>,
    pub registered_entry_count: u32,
    pub armies: Vec<ArmyOrdersGroupRow>,
}

/// Decode the extension, applying every rule
/// `decode_orders_group_extension` applies (`orders_group_live.rs:398`-`:487`).
pub fn decode_extension(bytes: &[u8]) -> core::result::Result<OrdersGroupExtension, String> {
    if bytes.len() < EXTENSION_HEADER_LEN {
        return Err("orders-group extension shorter than header".into());
    }
    if bytes[..4] != EXTENSION_MAGIC {
        return Err("orders-group extension has bad magic".into());
    }
    let version = u16_at(bytes, 4);
    if !matches!(version, EXTENSION_VERSION_V1 | EXTENSION_VERSION_V2) {
        return Err("orders-group extension has unsupported version".into());
    }
    if u16_at(bytes, 6) as usize != EXTENSION_HEADER_LEN {
        return Err("orders-group extension has bad header length".into());
    }
    let row_count = u32_at(bytes, 8) as usize;
    let registered_entry_count = u32_at(bytes, 12);
    let army_group_count = u32_at(bytes, 16);
    let flags = u32_at(bytes, 20);
    // The complete-manager flag is the only accepted value, so it is grammar
    // rather than a field.
    if flags != EXTENSION_FLAG_COMPLETE_MANAGER {
        return Err("orders-group extension is not complete-manager authority".into());
    }
    let army_rows_end = row_count
        .checked_mul(EXTENSION_ROW_LEN)
        .and_then(|rows| EXTENSION_HEADER_LEN.checked_add(rows))
        .ok_or("orders-group extension length overflow")?;
    let expected = if version == EXTENSION_VERSION_V2 {
        (army_group_count as usize)
            .checked_mul(4)
            .and_then(|bytes| army_rows_end.checked_add(bytes))
            .ok_or("orders-group extension length overflow")?
    } else {
        army_rows_end
    };
    if bytes.len() != expected {
        return Err("orders-group extension length mismatch".into());
    }
    // Derived, hard-enforced, therefore not a field.
    if registered_entry_count != (row_count as u32).saturating_add(army_group_count) {
        return Err("orders-group extension registered count mismatch".into());
    }
    let mut armies = Vec::with_capacity(row_count);
    let mut previous_id = 0u32;
    for index in 0..row_count {
        let base = EXTENSION_HEADER_LEN + index * EXTENSION_ROW_LEN;
        let army_id = u32_at(bytes, base);
        let execution_type = u32_at(bytes, base + 4);
        let cohesion_type = u32_at(bytes, base + 8);
        let leader_proximity = u32_at(bytes, base + 12);
        if army_id == 0 || army_id > MAX_ARMY_ID || army_id <= previous_id {
            return Err("orders-group extension army ids are not strict sorted ids".into());
        }
        if cohesion_type > MAX_COHESION_TYPE || leader_proximity > MAX_LEADER_PROXIMITY {
            return Err("orders-group extension scalar outside exact UI enum".into());
        }
        previous_id = army_id;
        armies.push(ArmyOrdersGroupRow {
            army_id,
            execution_type,
            cohesion_type,
            leader_proximity,
        });
    }
    let army_ids: BTreeSet<u32> = armies.iter().map(|row| row.army_id).collect();
    let mut army_group_ids = Vec::with_capacity(army_group_count as usize);
    if version == EXTENSION_VERSION_V2 {
        let mut previous_group_id = 0u32;
        for index in 0..army_group_count as usize {
            let army_group_id = u32_at(bytes, army_rows_end + index * 4);
            if army_group_id == 0
                || army_group_id > MAX_ARMY_ID
                || army_group_id <= previous_group_id
                || army_ids.contains(&army_group_id)
            {
                return Err("orders-group extension army-group ids are not strict disjoint ids".into());
            }
            previous_group_id = army_group_id;
            army_group_ids.push(army_group_id);
        }
    }
    Ok(OrdersGroupExtension {
        owner_identities_complete: version == EXTENSION_VERSION_V2,
        army_group_count,
        army_group_ids,
        registered_entry_count,
        armies,
    })
}

// --- split_payload -----------------------------------------------------------

/// A validated d54 payload, sliced into its four tables and its optional
/// extension.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Split<'a> {
    pub header: PayloadHeader,
    pub order_rows: &'a [u8],
    pub target_table: &'a [u8],
    pub path_table: &'a [u8],
    pub assigned_table: &'a [u8],
    /// `None` on a legacy frame — and a legacy frame must NOT inherit a prior
    /// record's manager (`world_fold.rs::fold_army_battleplans_depth`).
    pub extension: Option<OrdersGroupExtension>,
}

impl Split<'_> {
    pub fn order_row(&self, index: usize) -> &[u8] {
        &self.order_rows[index * ROW_LEN..(index + 1) * ROW_LEN]
    }

    pub fn target_row(&self, index: usize) -> &[u8] {
        &self.target_table[index * TARGET_ROW_LEN..(index + 1) * TARGET_ROW_LEN]
    }

    pub fn path_row(&self, index: usize) -> &[u8] {
        &self.path_table[index * PATH_GEOMETRY_ROW_LEN..(index + 1) * PATH_GEOMETRY_ROW_LEN]
    }

    pub fn assigned_row(&self, index: usize) -> &[u8] {
        &self.assigned_table
            [index * ASSIGNED_DIVISION_ROW_LEN..(index + 1) * ASSIGNED_DIVISION_ROW_LEN]
    }
}

/// Split and fully validate a d54 DATA payload.
///
/// This is `battleplans54::decode_depth_payload` (`lane lib.rs:797`-`:1136`),
/// which is BOTH the first thing the fold runs — before it reads any status word
/// (`world_fold.rs::fold_army_battleplans_depth`) — and production's status-independent DATA
/// authority (`domain_data_authority.rs:8317`-`:8325`). So it runs on every
/// record whatever the status, and any refusal here is a **stream rejection**,
/// never an excluded hour.
///
/// The fold's own second decoder (`world_fold.rs::decode_army_battleplans_depth_payload`) re-reads the same bytes
/// after the status gates; every refusal it makes is also made here, so its
/// later position is unobservable — see
/// [`the_world_fold_decoder_adds_no_refusal_of_its_own`].
pub fn split_payload(payload: &[u8]) -> core::result::Result<Split<'_>, String> {
    let header = peek_header(payload)?;
    let row_count = header.row_count as usize;
    let target_count = header.target_count as usize;
    let path_count = header.path_count as usize;
    let assigned_count = header.assigned_division_count as usize;

    // Checked arithmetic throughout: every count is an untrusted word off disk
    // and `count * stride` overflows `usize` for a large enough one. The shipping
    // validator saturates instead (`lane lib.rs:819`-`:824`), which produces the
    // same refusal because a saturated length is never `<= payload.len()`.
    let rows_end = row_count
        .checked_mul(ROW_LEN)
        .and_then(|len| HEADER_LEN.checked_add(len))
        .ok_or_else(|| format!("battleplans row_count {} overflows", header.row_count))?;
    let target_end = target_count
        .checked_mul(TARGET_ROW_LEN)
        .and_then(|len| rows_end.checked_add(len))
        .ok_or_else(|| format!("battleplans target_count {} overflows", header.target_count))?;
    let path_end = path_count
        .checked_mul(PATH_GEOMETRY_ROW_LEN)
        .and_then(|len| target_end.checked_add(len))
        .ok_or_else(|| format!("battleplans path_count {} overflows", header.path_count))?;
    let legacy_len = assigned_count
        .checked_mul(ASSIGNED_DIVISION_ROW_LEN)
        .and_then(|len| path_end.checked_add(len))
        .ok_or_else(|| {
            format!(
                "battleplans assigned_division_count {} overflows",
                header.assigned_division_count
            )
        })?;
    if payload.len() < legacy_len {
        return Err(format!(
            "battleplans payload length {} below {legacy_len}",
            payload.len()
        ));
    }

    // `lane lib.rs:828`-`:852`: the extension is decoded BEFORE the header
    // cross-checks, and its army count must equal the header's own census width.
    let extension = if payload.len() == legacy_len {
        None
    } else {
        let decoded = decode_extension(&payload[legacy_len..])
            .map_err(|reason| format!("battleplans orders-group extension: {reason}"))?;
        if decoded.armies.len() != header.observed_army_rows as usize {
            return Err("battleplans complete-manager army count differs from header".into());
        }
        Some(decoded)
    };

    // `:853`-`:874`. `changed_rows` IS read by this validator, so unlike d17/d34
    // it is an invariant here — but only as `<=`, never as equality.
    if header.changed_rows > header.row_count {
        return Err(format!(
            "battleplans changed rows {} exceed row count {}",
            header.changed_rows, header.row_count
        ));
    }
    if header.status_flags & STATUS_DELTA_DATA == 0 && header.observed_order_rows < header.row_count
    {
        return Err(format!(
            "battleplans observed order rows {} below emitted rows {} on a non-delta frame",
            header.observed_order_rows, header.row_count
        ));
    }
    if header.observed_army_rows == 0 && header.row_count != 0 {
        return Err("battleplans emitted rows with zero observed armies".into());
    }

    let split = Split {
        header,
        order_rows: &payload[HEADER_LEN..rows_end],
        target_table: &payload[rows_end..target_end],
        path_table: &payload[target_end..path_end],
        assigned_table: &payload[path_end..legacy_len],
        extension,
    };

    // Per-row rules, then the cross-row child accounting (`:1093`-`:1105`): the
    // three declared totals must exactly exhaust their tables, which is what makes
    // the ranges a partition and the ordinal a faithful reproduction of each
    // `Vec`.
    let mut declared_targets = 0usize;
    let mut declared_paths = 0usize;
    let mut declared_assigned = 0usize;
    for index in 0..row_count {
        let row = parse_order_row(split.order_row(index))
            .map_err(|reason| format!("battleplans row {index}: {reason}"))?;
        if row.has_targets() {
            let end = (row.child_first as usize)
                .checked_add(row.child_count as usize)
                .ok_or_else(|| format!("battleplans row {index}: target range overflow"))?;
            if end > target_count {
                return Err(format!(
                    "battleplans row {index}: target range exceeds the child table"
                ));
            }
            declared_targets += row.child_count as usize;
        }
        if row.has_path() {
            let end = (row.child_first as usize)
                .checked_add(row.child_count as usize)
                .ok_or_else(|| format!("battleplans row {index}: path range overflow"))?;
            if end > path_count {
                return Err(format!(
                    "battleplans row {index}: path range exceeds the child table"
                ));
            }
            declared_paths += row.child_count as usize;
        }
        if row.assigned_count != 0 {
            let end = (row.assigned_first as usize)
                .checked_add(row.assigned_count as usize)
                .ok_or_else(|| format!("battleplans row {index}: assigned range overflow"))?;
            if end > assigned_count {
                return Err(format!(
                    "battleplans row {index}: assigned range exceeds the child table"
                ));
            }
            declared_assigned += row.assigned_count as usize;
        }
    }
    if declared_targets != target_count {
        return Err("battleplans target child count mismatch".into());
    }
    if declared_paths != path_count {
        return Err("battleplans path child count mismatch".into());
    }
    if declared_assigned != assigned_count {
        return Err("battleplans assigned child count mismatch".into());
    }

    // Every child row of every table, id bound and reserved word
    // (`:1104`-`:1135`).
    for index in 0..target_count {
        parse_child_row(split.target_row(index), MAX_STATE_ID, "target")
            .map_err(|reason| format!("battleplans target child {index}: {reason}"))?;
    }
    for index in 0..path_count {
        parse_child_row(split.path_row(index), MAX_PROVINCE_ID, "path")
            .map_err(|reason| format!("battleplans path child {index}: {reason}"))?;
    }
    for index in 0..assigned_count {
        parse_child_row(split.assigned_row(index), MAX_DIVISION_ID, "assigned-division")
            .map_err(|reason| format!("battleplans assigned-division child {index}: {reason}"))?;
    }

    Ok(split)
}

// --- Keys --------------------------------------------------------------------

/// The order's key: `ArmyBattleplanDepthKey` (`world_fold.rs:23877`-`:23883`).
pub fn order_key(owner_kind: u32, owner_id: u32, instance: u32, generation: u64) -> Key {
    key(&[
        owner_kind as u64,
        owner_id as u64,
        instance as u64,
        generation,
    ])
}

pub fn key_from_order_row(row: &[u8]) -> core::result::Result<Key, String> {
    let parsed = parse_order_row(row)?;
    Ok(order_key(
        parsed.owner_kind,
        parsed.owner_id,
        parsed.engine_order_instance_id,
        parsed.generation,
    ))
}

/// A child row's key: its parent's, then the generated dense ordinal.
///
/// The parent key comes first so a caller that already parsed the order does not
/// parse it twice — d23's and d83's convention — and the ordinal is a parameter
/// rather than something a caller re-derives, which is step 5's counting rule.
pub fn key_from_child(parent: &Key, ordinal: u32) -> Key {
    let mut components = parent.0.clone();
    components.push(ordinal as i128);
    Key(components)
}

pub fn key_from_orders_group_army_row(row: &[u8]) -> core::result::Result<Key, String> {
    if row.len() != EXTENSION_ROW_LEN {
        return Err(format!(
            "orders-group army row is {} bytes, not {EXTENSION_ROW_LEN}",
            row.len()
        ));
    }
    Ok(key(&[u32_at(row, 0) as u64]))
}

pub fn key_from_army_group_id(army_group_id: u32) -> Key {
    key(&[army_group_id as u64])
}

// --- The boundary mapping ----------------------------------------------------

fn check_collection(collection: &Collection, expected: u32) -> core::result::Result<(), String> {
    if collection.collection_id != expected {
        return Err(format!(
            "d54 record built for collection {} but this mapper serves {expected}",
            collection.collection_id
        ));
    }
    Ok(())
}

/// One compact row of the order table, as the collector emits it today, onto its
/// stored Record. `row` is the raw 88-byte row slice (the caller has already
/// consumed the payload header via [`split_payload`]); d54 has exactly one order
/// row form, so length selects nothing and a wrong length is rejected.
/// `frame_tick` is derived-field contract's derived-field input; d54 declares no derived field, so it
/// is taken and ignored — every domain's mapper has the same shape.
///
/// A **deleted** row is a tombstone and has no record: it removes its key from
/// the accumulator (`world_fold.rs::decode_army_battleplans_depth_payload`). Calling this with one is a
/// caller error and is refused rather than mapped, because a tombstone's value
/// words are all forced to zero by the grammar and mapping them would publish
/// "slot 0, order type 0, status none" as measurement.
pub fn record_from_order_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    check_collection(collection, ORDERS)?;
    let parsed = parse_order_row(row)?;
    if parsed.deleted() {
        return Err("d54 deleted order row is a tombstone, not a record".into());
    }
    // The four-state decision, and the only one in this domain. `target_status`
    // is the disambiguator and it is on this same record.
    let target_declared_count = match parsed.target_declared_count {
        Some(value) => Value::Unsigned(value as u64),
        // The concept does not exist on a row whose order type has no type-5
        // target list at all (`lane lib.rs:355`-`:357`).
        None if parsed.target_status == STATUS_UNSUPPORTED => Value::Absent,
        // Every other clear gate is a size the engine holds and this sample could
        // not read or accept (`lane lib.rs:1150`-`:1183`).
        None => Value::Unknown,
    };
    Ok(Record::new(vec![
        Value::Unsigned(parsed.slot_index as u64),
        Value::Signed(parsed.order_type as i64),
        target_declared_count,
        Value::Enum(parsed.target_status),
        Value::Enum(parsed.geometry_status),
        Value::Enum(parsed.execution_status),
        Value::Enum(parsed.stance_status),
        Value::Enum(parsed.aggressiveness_status),
        Value::Bool(parsed.active),
        Value::Bool(parsed.flags & FLAG_NON_TYPE5_GEOMETRY_BLOCKED != 0),
        Value::Bool(parsed.flags & FLAG_EXECUTION_BLOCKED != 0),
    ]))
}

/// One 8-byte target-state child row onto its stored Record.
pub fn record_from_target_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    check_collection(collection, ORDER_TARGET_STATES)?;
    let state_id = parse_child_row(row, MAX_STATE_ID, "target")?;
    Ok(Record::new(vec![Value::ContentId(state_id as u64)]))
}

/// One 8-byte path-geometry child row onto its stored Record.
pub fn record_from_path_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    check_collection(collection, ORDER_PATH_GEOMETRY)?;
    let province_id = parse_child_row(row, MAX_PROVINCE_ID, "path")?;
    Ok(Record::new(vec![Value::ContentId(province_id as u64)]))
}

/// One 8-byte assigned-division child row onto its stored Record.
pub fn record_from_assigned_division_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    check_collection(collection, ORDER_ASSIGNED_DIVISIONS)?;
    let division_id = parse_child_row(row, MAX_DIVISION_ID, "assigned-division")?;
    Ok(Record::new(vec![Value::Unsigned(division_id as u64)]))
}

/// The `OGS1` census header onto the [`ORDERS_GROUP_MANAGER`] singleton's Record.
pub fn record_from_orders_group_manager(
    collection: &Collection,
    extension: &OrdersGroupExtension,
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    check_collection(collection, ORDERS_GROUP_MANAGER)?;
    // Mirrored rather than assumed: `decode_extension` enforces it, so a caller
    // that built an `OrdersGroupExtension` by hand cannot slip an inconsistent
    // census past this boundary either.
    if extension.registered_entry_count
        != (extension.armies.len() as u32).saturating_add(extension.army_group_count)
    {
        return Err("orders-group census registered count is not armies + army groups".into());
    }
    if extension.owner_identities_complete
        && extension.army_group_ids.len() != extension.army_group_count as usize
    {
        return Err("orders-group v2 census does not carry every army-group id".into());
    }
    if !extension.owner_identities_complete && !extension.army_group_ids.is_empty() {
        return Err("orders-group v1 census carries army-group ids it cannot have".into());
    }
    Ok(Record::new(vec![
        Value::Unsigned(extension.army_group_count as u64),
        Value::Bool(extension.owner_identities_complete),
    ]))
}

/// One 16-byte `OGS1` army row onto its stored Record.
pub fn record_from_orders_group_army_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    check_collection(collection, ORDERS_GROUP_ARMIES)?;
    if row.len() != EXTENSION_ROW_LEN {
        return Err(format!(
            "orders-group army row is {} bytes, not {EXTENSION_ROW_LEN}",
            row.len()
        ));
    }
    let army_id = u32_at(row, 0);
    let execution_type = u32_at(row, 4);
    let cohesion_type = u32_at(row, 8);
    let leader_proximity = u32_at(row, 12);
    if army_id == 0 || army_id > MAX_ARMY_ID {
        return Err(format!("orders-group army row has bad army id {army_id}"));
    }
    if cohesion_type > MAX_COHESION_TYPE || leader_proximity > MAX_LEADER_PROXIMITY {
        return Err("orders-group army row scalar outside exact UI enum".into());
    }
    Ok(Record::new(vec![
        Value::Unsigned(execution_type as u64),
        Value::Enum(cohesion_type),
        Value::Enum(leader_proximity),
    ]))
}

/// The key-only [`ORDERS_GROUP_ARMY_GROUPS`] record.
///
/// Its field loop iterates nothing, so a record built for the wrong collection
/// would come out silently empty instead of failing: the collection id is checked
/// explicitly (`variant_upgrades.rs:633`'s rule).
pub fn record_from_army_group_id(
    collection: &Collection,
    army_group_id: u32,
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    check_collection(collection, ORDERS_GROUP_ARMY_GROUPS)?;
    if army_group_id == 0 || army_group_id > MAX_ARMY_ID {
        return Err(format!(
            "orders-group army-group census has bad id {army_group_id}"
        ));
    }
    Ok(Record::new(vec![]))
}

// --- The frame-level fold ----------------------------------------------------

/// Why a d54 record did not become state.
///
/// Three of the four are **excluded hours**: the accumulator is retained, the
/// domain-hour is marked unknown, and the recorder carries on. None of them is a
/// stream rejection — those are [`split_payload`]'s `Err`, which happens one
/// layer earlier and on every record whatever its status.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FoldOutcome {
    /// The frame folded (`world_fold.rs::fold_army_battleplans_depth`).
    Applied,
    /// `COMPACT_STATUS_FAILURE_MASK & !PARTIAL_DATA` (`:21776`-`:21791`).
    FailureStatus,
    /// A delta arrived before any baseline (`:21792`-`:21808`).
    DeltaBeforeBaseline,
    /// The expected-partial tolerance withheld the hour (`:21813`-`:21823`).
    /// d54 IS in `COMPACT_EXPECTED_PARTIAL_TOLERANCES`, so this branch is live
    /// and it sits AFTER the row decode — the rows are validated in full and then
    /// abandoned, because the omitted order identities are not serialized and the
    /// retained accumulator cannot be aligned against them.
    ToleratedPartialWithheld,
}

/// The domain's frame-level state — the recipe's fourth public boundary, so the
/// measurement bin, the tests and any future caller share ONE replay of the
/// shipping layer.
///
/// Two accumulators, and they advance on **different** conditions, exactly as the
/// fold's do: the order map is cleared only by a non-delta frame
/// (`world_fold.rs::fold_army_battleplans_depth`), while the orders-group census is
/// **replaced or cleared on every folded frame** (`world_fold.rs::fold_army_battleplans_depth`) — a legacy
/// frame that carries no `OGS1` must not inherit the previous one's authority.
/// The three child collections are not separate accumulators: they are the order
/// values' own `Vec`s flattened, so they follow their parent exactly.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    /// Collection [`ORDERS`], keyed by [`key_from_order_row`].
    pub orders: KeyedState,
    /// Collection [`ORDER_TARGET_STATES`], keyed by the order's key + ordinal.
    pub target_states: KeyedState,
    /// Collection [`ORDER_PATH_GEOMETRY`], keyed by the order's key + ordinal.
    pub path_geometry: KeyedState,
    /// Collection [`ORDER_ASSIGNED_DIVISIONS`], keyed by the order's key + ordinal.
    pub assigned_divisions: KeyedState,
    /// Collection [`ORDERS_GROUP_MANAGER`] — at most the one singleton row.
    pub orders_group_manager: KeyedState,
    /// Collection [`ORDERS_GROUP_ARMIES`], keyed by `army_id`.
    pub orders_group_armies: KeyedState,
    /// Collection [`ORDERS_GROUP_ARMY_GROUPS`], keyed by `army_group_id`.
    pub orders_group_army_groups: KeyedState,
    /// `delta_baselines_seen.contains(&54)` (`world_fold.rs::fold_army_battleplans_depth`).
    baseline_seen: bool,
}

/// The seven per-collection states in schema order, which is the order
/// `encode_baseline`/`encode_delta` require.
impl State {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a baseline has landed. Public because the delta-before-baseline
    /// exclusion is decided from it.
    pub fn baseline_seen(&self) -> bool {
        self.baseline_seen
    }

    pub fn collections(&self) -> Vec<KeyedState> {
        vec![
            self.orders.clone(),
            self.target_states.clone(),
            self.path_geometry.clone(),
            self.assigned_divisions.clone(),
            self.orders_group_manager.clone(),
            self.orders_group_armies.clone(),
            self.orders_group_army_groups.clone(),
        ]
    }

    /// Drop every child row belonging to `parent`. The child key is the parent's
    /// four components plus the ordinal, and `Key` sorts lexicographically, so
    /// the parent's rows are exactly the contiguous run whose first four
    /// components match.
    fn drop_children_of(&mut self, parent: &Key) {
        for table in [
            &mut self.target_states,
            &mut self.path_geometry,
            &mut self.assigned_divisions,
        ] {
            let doomed: Vec<Key> = table
                .range(key_from_child(parent, 0)..)
                .take_while(|(k, _)| k.0[..parent.0.len()] == parent.0[..])
                .map(|(k, _)| k.clone())
                .collect();
            for k in doomed {
                table.remove(&k);
            }
        }
    }

    /// Fold one d54 DATA payload, reproducing `fold_army_battleplans_depth`
    /// (`world_fold.rs::fold_army_battleplans_depth`) exit for exit and in its own order.
    ///
    /// **Error contract**: all-or-nothing. `split_payload` runs first and its
    /// `Err` leaves this state untouched, which is what the fold does — its own
    /// first statement is the same decode, and it returns `Err` before it has
    /// written anything (`world_fold.rs::fold_army_battleplans_depth`).
    ///
    /// `failure_status` is the caller's classification of the record's status word
    /// against `compact_status_failure_flags_after_expected_partial_tolerance`,
    /// and `tolerated_partial` its `expected_partial_tolerance_applies` sibling.
    /// Both are typed inputs rather than something re-derived here: the library
    /// half of this crate must not depend on the shipping contract crate, and a
    /// hand-copied mask literal is exactly what the recipe forbids.
    pub fn fold_payload(
        &mut self,
        schema: &Schema,
        payload: &[u8],
        failure_status: bool,
        tolerated_partial: bool,
    ) -> core::result::Result<FoldOutcome, String> {
        // 1. The lane validator, status-independent and first.
        let split = split_payload(payload)?;

        // 2. The hard-failure gate (`:21776`-`:21791`). The accumulator is
        //    retained and the domain-hour is unknown.
        if failure_status {
            return Ok(FoldOutcome::FailureStatus);
        }

        // 3. The delta-before-baseline gate (`:21792`-`:21808`).
        if split.header.is_delta() && !self.baseline_seen {
            return Ok(FoldOutcome::DeltaBeforeBaseline);
        }

        // 4. `decode_army_battleplans_depth_payload` (`:21810`) — already done by
        //    `split_payload`, which is a superset of it.

        // 5. The expected-partial tolerance (`:21813`-`:21823`). Every row above
        //    has been validated in full, exactly as the recorder validates them,
        //    and now the frame is abandoned whole.
        if tolerated_partial {
            return Ok(FoldOutcome::ToleratedPartialWithheld);
        }

        let orders = collection_of(schema, ORDERS)?;
        let target_states = collection_of(schema, ORDER_TARGET_STATES)?;
        let path_geometry = collection_of(schema, ORDER_PATH_GEOMETRY)?;
        let assigned_divisions = collection_of(schema, ORDER_ASSIGNED_DIVISIONS)?;
        let manager = collection_of(schema, ORDERS_GROUP_MANAGER)?;
        let armies = collection_of(schema, ORDERS_GROUP_ARMIES)?;
        let army_groups = collection_of(schema, ORDERS_GROUP_ARMY_GROUPS)?;

        // Build every row before mutating anything, so the all-or-nothing error
        // contract holds even against a mapper refusal.
        let row_count = split.header.row_count as usize;
        let mut pending: Vec<(Key, Option<(Record, Vec<(u32, Record)>, Vec<(u32, Record)>, Vec<(u32, Record)>)>)> =
            Vec::with_capacity(row_count);
        for index in 0..row_count {
            let bytes = split.order_row(index);
            let parsed = parse_order_row(bytes)?;
            let row_key = order_key(
                parsed.owner_kind,
                parsed.owner_id,
                parsed.engine_order_instance_id,
                parsed.generation,
            );
            if parsed.deleted() {
                pending.push((row_key, None));
                continue;
            }
            let record = record_from_order_row(orders, bytes, 0)?;
            // The generated dense ordinal is counted in exactly one place, and it
            // is the position within THIS order's own run — never the shared
            // table index, which the collector re-packs from emission order every
            // frame.
            let mut targets = Vec::new();
            if parsed.has_targets() {
                for ordinal in 0..parsed.child_count {
                    let child = split.target_row((parsed.child_first + ordinal) as usize);
                    targets.push((ordinal, record_from_target_row(target_states, child, 0)?));
                }
            }
            let mut paths = Vec::new();
            if parsed.has_path() {
                for ordinal in 0..parsed.child_count {
                    let child = split.path_row((parsed.child_first + ordinal) as usize);
                    paths.push((ordinal, record_from_path_row(path_geometry, child, 0)?));
                }
            }
            let mut assigned = Vec::new();
            for ordinal in 0..parsed.assigned_count {
                let child = split.assigned_row((parsed.assigned_first + ordinal) as usize);
                assigned.push((
                    ordinal,
                    record_from_assigned_division_row(assigned_divisions, child, 0)?,
                ));
            }
            pending.push((row_key, Some((record, targets, paths, assigned))));
        }

        // The census is installed BEFORE the rows, as the fold installs it
        // (`:21825`-`:21832`), and a legacy frame CLEARS it rather than inheriting.
        let mut manager_state = KeyedState::new();
        let mut army_state = KeyedState::new();
        let mut group_state = KeyedState::new();
        if let Some(extension) = &split.extension {
            manager_state.insert(
                manager_singleton_key(),
                record_from_orders_group_manager(manager, extension, 0)?,
            );
            let bytes = &payload[payload.len() - extension_bytes_len(extension)..];
            for (index, row) in extension.armies.iter().enumerate() {
                let row_bytes =
                    &bytes[EXTENSION_HEADER_LEN + index * EXTENSION_ROW_LEN..][..EXTENSION_ROW_LEN];
                army_state.insert(
                    key_from_orders_group_army_row(row_bytes)?,
                    record_from_orders_group_army_row(armies, row_bytes, 0)?,
                );
                debug_assert_eq!(u32_at(row_bytes, 0), row.army_id);
            }
            for army_group_id in &extension.army_group_ids {
                group_state.insert(
                    key_from_army_group_id(*army_group_id),
                    record_from_army_group_id(army_groups, *army_group_id, 0)?,
                );
            }
        }
        self.orders_group_manager = manager_state;
        self.orders_group_armies = army_state;
        self.orders_group_army_groups = group_state;

        // `:21833`-`:21837`: a non-delta frame is a full replacement and is what
        // establishes the delta baseline.
        if !split.header.is_delta() {
            self.orders.clear();
            self.target_states.clear();
            self.path_geometry.clear();
            self.assigned_divisions.clear();
            self.baseline_seen = true;
        }

        // `:21839`-`:21846`: upsert or remove, in wire order, last writer wins —
        // the fold enforces no key uniqueness inside a frame, so this port must
        // not either.
        for (row_key, payload_rows) in pending {
            self.drop_children_of(&row_key);
            match payload_rows {
                None => {
                    self.orders.remove(&row_key);
                }
                Some((record, targets, paths, assigned)) => {
                    self.orders.insert(row_key.clone(), record);
                    for (ordinal, child) in targets {
                        self.target_states
                            .insert(key_from_child(&row_key, ordinal), child);
                    }
                    for (ordinal, child) in paths {
                        self.path_geometry
                            .insert(key_from_child(&row_key, ordinal), child);
                    }
                    for (ordinal, child) in assigned {
                        self.assigned_divisions
                            .insert(key_from_child(&row_key, ordinal), child);
                    }
                }
            }
        }

        Ok(FoldOutcome::Applied)
    }
}

/// The encoded length of an already-decoded extension, so its raw row bytes can
/// be re-sliced out of the payload without a second decode.
fn extension_bytes_len(extension: &OrdersGroupExtension) -> usize {
    EXTENSION_HEADER_LEN
        + extension.armies.len() * EXTENSION_ROW_LEN
        + extension.army_group_ids.len() * 4
}

fn collection_of(schema: &Schema, id: u32) -> core::result::Result<&Collection, String> {
    schema
        .collections
        .iter()
        .find(|c| c.collection_id == id)
        .ok_or_else(|| format!("d54 schema has no collection {id}"))
}

/// Every `ContentId` field in this domain, for the ContentId retrofit sweep.
///
/// Both are `ContentId` **values**, not key components, so the heavier
/// ContentId-key retrofit class (a manifest remap reorders keys and changes the
/// byte layout of every baseline) does not apply to d54. Today's mappers cast the
/// source `u32` straight into `Value::ContentId`, per the recipe's standing rule.
pub const CONTENT_ID_FIELDS: &[(u32, u32)] = &[
    (ORDER_TARGET_STATES, field::TS_STATE_ID),
    (ORDER_PATH_GEOMETRY, field::PG_PROVINCE_ID),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{
        apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta,
    };
    use crate::schema::Quant;

    const TICKS_PER_HOUR_LOG2: u8 = 0;

    // ---- Fixture builders ----------------------------------------------------

    #[derive(Clone, Debug)]
    struct RawOrder {
        owner_kind: u32,
        owner_id: u32,
        instance: u32,
        generation: u64,
        flags: u32,
        slot_index: u32,
        order_type: i32,
        child_first: u32,
        child_count: u32,
        target_declared: u32,
        target_status: u32,
        geometry_status: u32,
        execution_status: u32,
        assigned_units_status: u32,
        order_linkage_status: u32,
        stance_status: u32,
        aggressiveness_status: u32,
        assigned_first: u32,
        assigned_count: u32,
        active: u32,
        active_status: u32,
    }

    impl RawOrder {
        /// A live, exact, path-free MOVE order — the shape 532 of the capture's
        /// 616 live rows have, minus the path.
        fn live() -> Self {
            Self {
                owner_kind: ORDER_OWNER_KIND_ARMY,
                owner_id: 11_480,
                instance: 1_055,
                generation: 608,
                flags: FLAG_ACTIVE_VALUE,
                slot_index: 3,
                order_type: ORDER_TYPE_MOVE,
                child_first: 0,
                child_count: 0,
                target_declared: 0,
                target_status: STATUS_UNSUPPORTED,
                geometry_status: STATUS_EXACT,
                execution_status: STATUS_EXACT,
                assigned_units_status: STATUS_EXACT,
                order_linkage_status: STATUS_EXACT,
                stance_status: STATUS_UNKNOWN,
                aggressiveness_status: STATUS_UNKNOWN,
                assigned_first: 0,
                assigned_count: 0,
                active: 1,
                active_status: STATUS_EXACT,
            }
        }

        fn tombstone(owner_kind: u32, owner_id: u32, instance: u32, generation: u64) -> Self {
            Self {
                owner_kind,
                owner_id,
                instance,
                generation,
                flags: FLAG_DELETED,
                slot_index: 0,
                order_type: 0,
                child_first: 0,
                child_count: 0,
                target_declared: 0,
                target_status: STATUS_NONE,
                geometry_status: STATUS_NONE,
                execution_status: STATUS_NONE,
                assigned_units_status: STATUS_NONE,
                order_linkage_status: STATUS_NONE,
                stance_status: STATUS_NONE,
                aggressiveness_status: STATUS_NONE,
                assigned_first: 0,
                assigned_count: 0,
                active: 0,
                active_status: STATUS_NONE,
            }
        }

        fn encode(&self) -> Vec<u8> {
            let mut out = Vec::with_capacity(ROW_LEN);
            out.extend_from_slice(&self.owner_id.to_le_bytes());
            out.extend_from_slice(&self.generation.to_le_bytes());
            for word in [
                self.flags,
                self.slot_index,
                self.order_type as u32,
                self.child_first,
                self.child_count,
                self.target_declared,
                self.target_status,
                self.geometry_status,
                self.execution_status,
                self.assigned_units_status,
                self.order_linkage_status,
                self.stance_status,
                self.aggressiveness_status,
            ] {
                out.extend_from_slice(&word.to_le_bytes());
            }
            out.extend_from_slice(&self.instance.to_le_bytes());
            out.extend_from_slice(&self.assigned_first.to_le_bytes());
            out.extend_from_slice(&self.assigned_count.to_le_bytes());
            out.extend_from_slice(&self.active.to_le_bytes());
            out.extend_from_slice(&self.active_status.to_le_bytes());
            out.extend_from_slice(&self.owner_kind.to_le_bytes());
            assert_eq!(out.len(), ROW_LEN);
            out
        }
    }

    fn child(id: u32) -> Vec<u8> {
        let mut out = id.to_le_bytes().to_vec();
        out.extend_from_slice(&0u32.to_le_bytes());
        out
    }

    fn ogs_extension(version: u16, armies: &[(u32, u32, u32, u32)], groups: &[u32]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&EXTENSION_MAGIC);
        out.extend_from_slice(&version.to_le_bytes());
        out.extend_from_slice(&(EXTENSION_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&(armies.len() as u32).to_le_bytes());
        let group_count = groups.len() as u32;
        out.extend_from_slice(&((armies.len() as u32) + group_count).to_le_bytes());
        out.extend_from_slice(&group_count.to_le_bytes());
        out.extend_from_slice(&EXTENSION_FLAG_COMPLETE_MANAGER.to_le_bytes());
        for (id, execution, cohesion, proximity) in armies {
            for word in [id, execution, cohesion, proximity] {
                out.extend_from_slice(&word.to_le_bytes());
            }
        }
        if version == EXTENSION_VERSION_V2 {
            for id in groups {
                out.extend_from_slice(&id.to_le_bytes());
            }
        }
        out
    }

    struct Frame {
        status_flags: u32,
        rows: Vec<RawOrder>,
        targets: Vec<u32>,
        paths: Vec<u32>,
        assigned: Vec<u32>,
        extension: Option<Vec<u8>>,
        observed_army_rows: Option<u32>,
        observed_order_rows: Option<u32>,
        changed_rows: Option<u32>,
    }

    impl Frame {
        fn new() -> Self {
            Self {
                status_flags: 0,
                rows: Vec::new(),
                targets: Vec::new(),
                paths: Vec::new(),
                assigned: Vec::new(),
                extension: None,
                observed_army_rows: None,
                observed_order_rows: None,
                changed_rows: None,
            }
        }

        fn encode(&self) -> Vec<u8> {
            let mut out = Vec::new();
            out.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
            out.extend_from_slice(&(self.rows.len() as u32).to_le_bytes());
            out.extend_from_slice(
                &self
                    .changed_rows
                    .unwrap_or(self.rows.len() as u32)
                    .to_le_bytes(),
            );
            out.extend_from_slice(&self.status_flags.to_le_bytes());
            let armies = self.observed_army_rows.unwrap_or_else(|| {
                self.extension
                    .as_ref()
                    .map(|e| u32::from_le_bytes(e[8..12].try_into().unwrap()))
                    .unwrap_or(1)
            });
            out.extend_from_slice(&armies.to_le_bytes());
            out.extend_from_slice(
                &self
                    .observed_order_rows
                    .unwrap_or(self.rows.len() as u32)
                    .to_le_bytes(),
            );
            out.extend_from_slice(&(self.targets.len() as u32).to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes()); // unsupported_geometry_rows
            out.extend_from_slice(&0u32.to_le_bytes()); // execution_unknown_rows
            out.extend_from_slice(&0u32.to_le_bytes()); // omitted_non_exact_rows
            out.extend_from_slice(&0u32.to_le_bytes()); // path_geometry_omitted_rows
            out.extend_from_slice(&(self.paths.len() as u32).to_le_bytes());
            out.extend_from_slice(&(self.assigned.len() as u32).to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes()); // identity_or_active_omitted
            assert_eq!(out.len(), HEADER_LEN);
            for row in &self.rows {
                out.extend_from_slice(&row.encode());
            }
            for id in &self.targets {
                out.extend_from_slice(&child(*id));
            }
            for id in &self.paths {
                out.extend_from_slice(&child(*id));
            }
            for id in &self.assigned {
                out.extend_from_slice(&child(*id));
            }
            if let Some(extension) = &self.extension {
                out.extend_from_slice(extension);
            }
            out
        }
    }

    /// A frame carrying one live path-bearing order and a small v2 census — the
    /// workhorse fixture.
    fn full_frame() -> Frame {
        let mut row = RawOrder::live();
        row.flags = FLAG_ACTIVE_VALUE | FLAG_PATH_GEOMETRY;
        row.order_type = ORDER_TYPE_LINE;
        row.child_first = 0;
        row.child_count = 3;
        let mut frame = Frame::new();
        frame.rows = vec![row];
        // A polyline that revisits a province — 12 of the capture's 616
        // path-carrying rows do exactly this.
        frame.paths = vec![9_632, 11_609, 9_632];
        frame.extension = Some(ogs_extension(
            EXTENSION_VERSION_V2,
            &[(7, 4, 1, 2), (9, 0, 0, 0)],
            &[11, 12],
        ));
        frame.observed_army_rows = Some(2);
        frame
    }

    fn fold_ok(state: &mut State, schema: &Schema, payload: &[u8]) -> FoldOutcome {
        state
            .fold_payload(schema, payload, false, false)
            .expect("frame folds")
    }

    // ---- 1. Schema validity --------------------------------------------------

    #[test]
    fn the_schema_validates() {
        schema()
            .validate(TICKS_PER_HOUR_LOG2)
            .expect("d54 schema is well formed");
    }

    #[test]
    fn the_schema_hash_is_stable() {
        // canonical schema bytes; a changed hash means a changed grammar.
        assert_eq!(schema().schema_hash(), schema().schema_hash());
        assert_eq!(schema().collections.len(), 7);
    }

    // ---- 5. Shape and key ----------------------------------------------------

    #[test]
    fn every_collection_is_a_keyed_set_with_the_declared_key() {
        let s = schema();
        for collection in &s.collections {
            assert_eq!(
                collection.shape,
                Shape::KeyedSet,
                "collection {} shape",
                collection.collection_id
            );
            for f in collection.key_fields() {
                assert!(
                    f.kind.legal_as_key(),
                    "key field {} kind {:?}",
                    f.field_id,
                    f.kind
                );
                assert!(!f.optional && !f.nullable && !f.unknown_capable);
            }
        }

        let orders = collection_of(&s, ORDERS).unwrap();
        let ids: Vec<u32> = orders.key_fields().iter().map(|f| f.field_id).collect();
        assert_eq!(
            ids,
            vec![
                field::OWNER_KIND,
                field::OWNER_ID,
                field::ENGINE_ORDER_INSTANCE_ID,
                field::GENERATION
            ],
            "the order key is the fold's ArmyBattleplanDepthKey, in its order"
        );

        // Every child collection carries the parent's four components and then
        // the generated ordinal, LAST — so the keyed set's own order is the
        // fold's array order inside each parent run.
        for (id, expected) in [
            (
                ORDER_TARGET_STATES,
                vec![
                    field::TS_OWNER_KIND,
                    field::TS_OWNER_ID,
                    field::TS_ENGINE_ORDER_INSTANCE_ID,
                    field::TS_GENERATION,
                    field::TS_ORDINAL,
                ],
            ),
            (
                ORDER_PATH_GEOMETRY,
                vec![
                    field::PG_OWNER_KIND,
                    field::PG_OWNER_ID,
                    field::PG_ENGINE_ORDER_INSTANCE_ID,
                    field::PG_GENERATION,
                    field::PG_ORDINAL,
                ],
            ),
            (
                ORDER_ASSIGNED_DIVISIONS,
                vec![
                    field::AD_OWNER_KIND,
                    field::AD_OWNER_ID,
                    field::AD_ENGINE_ORDER_INSTANCE_ID,
                    field::AD_GENERATION,
                    field::AD_ORDINAL,
                ],
            ),
        ] {
            let collection = collection_of(&s, id).unwrap();
            let ids: Vec<u32> = collection.key_fields().iter().map(|f| f.field_id).collect();
            assert_eq!(ids, expected, "collection {id} key");
        }

        assert_eq!(
            collection_of(&s, ORDERS_GROUP_ARMY_GROUPS)
                .unwrap()
                .value_fields()
                .count(),
            0,
            "the army-group census is key-only"
        );
    }

    #[test]
    fn the_ordinal_reproduces_a_polyline_that_repeats_a_province() {
        // The measured case that rules out a province-id key: 12 of the
        // capture's 616 path-carrying rows repeat a province inside one
        // polyline. Under an id key this frame would store two rows, not three,
        // and the middle waypoint would be lost.
        let schema = schema();
        let mut state = State::new();
        assert_eq!(
            fold_ok(&mut state, &schema, &full_frame().encode()),
            FoldOutcome::Applied
        );
        assert_eq!(state.path_geometry.len(), 3);
        let parent = order_key(ORDER_OWNER_KIND_ARMY, 11_480, 1_055, 608);
        let ordered: Vec<u64> = state
            .path_geometry
            .range(key_from_child(&parent, 0)..)
            .take_while(|(k, _)| k.0[..4] == parent.0[..])
            .map(|(_, record)| match record.values[0] {
                Value::ContentId(id) => id,
                other => panic!("path province is {other:?}"),
            })
            .collect();
        assert_eq!(
            ordered,
            vec![9_632, 11_609, 9_632],
            "key order reproduces the fold's Vec element for element"
        );
    }

    // ---- 6. Every field's offset --------------------------------------------

    #[test]
    fn every_header_word_is_read_from_its_own_offset() {
        // A transposed header word mis-slices every table behind it and no
        // round-trip or replay assertion can see that: both arms agree on the
        // wrong slice (d31's rule).
        let mut payload = vec![0u8; HEADER_LEN];
        payload[0..8].copy_from_slice(&0x0102_0304_0506_0708u64.to_le_bytes());
        for (offset, value) in [
            (8usize, 11u32),
            (12, 12),
            (16, 13),
            (20, 14),
            (24, 15),
            (28, 16),
            (32, 17),
            (36, 18),
            (40, 19),
            (44, 20),
            (48, 21),
            (52, 22),
            (56, 23),
        ] {
            payload[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        let header = peek_header(&payload).expect("header parses");
        assert_eq!(header.elapsed_wall_ms, 0x0102_0304_0506_0708);
        assert_eq!(header.row_count, 11);
        assert_eq!(header.changed_rows, 12);
        assert_eq!(header.status_flags, 13);
        assert_eq!(header.observed_army_rows, 14);
        assert_eq!(header.observed_order_rows, 15);
        assert_eq!(header.target_count, 16);
        assert_eq!(header.unsupported_geometry_rows, 17);
        assert_eq!(header.execution_unknown_rows, 18);
        assert_eq!(header.omitted_non_exact_rows, 19);
        assert_eq!(header.path_geometry_omitted_rows, 20);
        assert_eq!(header.path_count, 21);
        assert_eq!(header.assigned_division_count, 22);
        assert_eq!(header.identity_or_active_omitted_rows, 23);
    }

    #[test]
    fn every_order_row_word_is_read_from_its_own_offset() {
        // Distinct values everywhere, so a transposition of any pair fails —
        // d62's `owner`/`actor` class, which no round-trip test can catch.
        let mut row = RawOrder::live();
        row.owner_kind = ORDER_OWNER_KIND_ARMY_GROUP;
        row.owner_id = 0x0011_2233;
        row.generation = 0x00AA_BBCC_DDEE_FF01;
        row.instance = 0x0055_6677;
        row.flags = FLAG_ACTIVE_VALUE
            | FLAG_TARGET_STATE_COUNT
            | FLAG_NON_TYPE5_GEOMETRY_BLOCKED
            | FLAG_EXECUTION_BLOCKED;
        row.slot_index = 41;
        row.order_type = -7;
        row.child_first = 0x1234;
        row.child_count = 0; // no child flag is set, so the range is transport only
        row.target_declared = 4_242;
        row.target_status = STATUS_OVERFLOW;
        row.geometry_status = STATUS_VALIDATION_FAILED;
        row.execution_status = STATUS_FIXTURE_REQUIRED;
        row.stance_status = STATUS_COLD_PREFLIGHT_ONLY;
        row.aggressiveness_status = STATUS_NONE;
        row.assigned_first = 0x99;
        row.assigned_count = 0;
        row.active = 1;

        let bytes = row.encode();
        let parsed = parse_order_row(&bytes).expect("row parses");
        assert_eq!(parsed.owner_id, 0x0011_2233);
        assert_eq!(parsed.generation, 0x00AA_BBCC_DDEE_FF01);
        assert_eq!(parsed.engine_order_instance_id, 0x0055_6677);
        assert_eq!(parsed.owner_kind, ORDER_OWNER_KIND_ARMY_GROUP);
        assert_eq!(parsed.slot_index, 41);
        assert_eq!(parsed.order_type, -7);
        assert_eq!(parsed.child_first, 0x1234);
        assert_eq!(parsed.assigned_first, 0x99);
        assert_eq!(parsed.target_declared_count, Some(4_242));
        assert_eq!(parsed.target_status, STATUS_OVERFLOW);
        assert_eq!(parsed.geometry_status, STATUS_VALIDATION_FAILED);
        assert_eq!(parsed.execution_status, STATUS_FIXTURE_REQUIRED);
        assert_eq!(parsed.assigned_units_status, STATUS_EXACT);
        assert_eq!(parsed.order_linkage_status, STATUS_EXACT);
        assert_eq!(parsed.stance_status, STATUS_COLD_PREFLIGHT_ONLY);
        assert_eq!(parsed.aggressiveness_status, STATUS_NONE);
        assert!(parsed.active);
        assert_eq!(parsed.active_status, STATUS_EXACT);

        let s = schema();
        let record = record_from_order_row(collection_of(&s, ORDERS).unwrap(), &bytes, 0)
            .expect("row maps");
        assert_eq!(
            record.values,
            vec![
                Value::Unsigned(41),
                Value::Signed(-7),
                Value::Unsigned(4_242),
                Value::Enum(STATUS_OVERFLOW),
                Value::Enum(STATUS_VALIDATION_FAILED),
                Value::Enum(STATUS_FIXTURE_REQUIRED),
                Value::Enum(STATUS_COLD_PREFLIGHT_ONLY),
                Value::Enum(STATUS_NONE),
                Value::Bool(true),
                Value::Bool(true),
                Value::Bool(true),
            ]
        );

        // The key extractor reads the same four words from the same offsets.
        assert_eq!(
            key_from_order_row(&bytes).unwrap(),
            order_key(
                ORDER_OWNER_KIND_ARMY_GROUP,
                0x0011_2233,
                0x0055_6677,
                0x00AA_BBCC_DDEE_FF01
            )
        );
    }

    #[test]
    fn every_child_and_census_row_word_is_read_from_its_own_offset() {
        let s = schema();
        assert_eq!(
            record_from_target_row(
                collection_of(&s, ORDER_TARGET_STATES).unwrap(),
                &child(65_535),
                0
            )
            .unwrap()
            .values,
            vec![Value::ContentId(65_535)]
        );
        assert_eq!(
            record_from_path_row(
                collection_of(&s, ORDER_PATH_GEOMETRY).unwrap(),
                &child(1),
                0
            )
            .unwrap()
            .values,
            vec![Value::ContentId(1)]
        );
        assert_eq!(
            record_from_assigned_division_row(
                collection_of(&s, ORDER_ASSIGNED_DIVISIONS).unwrap(),
                &child(2_147_483_647),
                0
            )
            .unwrap()
            .values,
            vec![Value::Unsigned(2_147_483_647)]
        );

        // The 16-byte census row: four distinct words at four distinct offsets.
        let mut row = Vec::new();
        for word in [77u32, 4u32, 3u32, 2u32] {
            row.extend_from_slice(&word.to_le_bytes());
        }
        assert_eq!(key_from_orders_group_army_row(&row).unwrap(), key(&[77]));
        assert_eq!(
            record_from_orders_group_army_row(
                collection_of(&s, ORDERS_GROUP_ARMIES).unwrap(),
                &row,
                0
            )
            .unwrap()
            .values,
            // execution_type 4 is above the labelled UI subset and is carried raw
            // — 507 of the capture's census rows do exactly this.
            vec![Value::Unsigned(4), Value::Enum(3), Value::Enum(2)]
        );

        let extension = decode_extension(&ogs_extension(
            EXTENSION_VERSION_V2,
            &[(7, 4, 1, 2)],
            &[11, 12],
        ))
        .expect("v2 extension decodes");
        assert_eq!(
            record_from_orders_group_manager(
                collection_of(&s, ORDERS_GROUP_MANAGER).unwrap(),
                &extension,
                0
            )
            .unwrap()
            .values,
            vec![Value::Unsigned(2), Value::Bool(true)]
        );
    }

    // ---- 3. States through the mapper ---------------------------------------

    #[test]
    fn only_target_declared_count_reaches_an_empty_state() {
        // Step 8.3: every capability the schema declares gets its own raw-bytes
        // case, and a collection that declares none says so. Here exactly one
        // field is `.optional().unknown_capable()` and NOTHING anywhere is
        // `.nullable()` — the fold never emits a record-state contract null for this domain, and
        // declaring one would cost a null bitmap bit for a state no input can
        // produce.
        let s = schema();
        let mut optional = Vec::new();
        for collection in &s.collections {
            for f in collection.value_fields() {
                assert!(
                    !f.nullable,
                    "collection {} field {} declares nullable",
                    collection.collection_id, f.field_id
                );
                if f.optional || f.unknown_capable {
                    optional.push((collection.collection_id, f.field_id, f.optional, f.unknown_capable));
                }
            }
        }
        assert_eq!(
            optional,
            vec![(ORDERS, field::TARGET_DECLARED_COUNT, true, true)],
            "one field, both capabilities, nothing else"
        );

        let orders = collection_of(&s, ORDERS).unwrap();

        // ABSENT: the gate is clear and the status says the concept is
        // unsupported by this order type.
        let mut row = RawOrder::live();
        row.target_status = STATUS_UNSUPPORTED;
        assert_eq!(
            record_from_order_row(orders, &row.encode(), 0).unwrap().values[2],
            Value::Absent
        );

        // UNKNOWN: the gate is clear and the status says the read failed, so a
        // size exists and was not captured.
        for status in [STATUS_VALIDATION_FAILED, STATUS_OVERFLOW, STATUS_NONE] {
            let mut row = RawOrder::live();
            row.target_status = status;
            assert_eq!(
                record_from_order_row(orders, &row.encode(), 0).unwrap().values[2],
                Value::Unknown,
                "target_status {status}"
            );
        }

        // A REAL ZERO: the gate is set and the engine reported an empty list —
        // `read_u32_vector` returns `Ok((vec![], 0))` for `count == 0`.
        let mut row = RawOrder::live();
        row.order_type = ORDER_TYPE_AREA_DEFENSE;
        row.target_status = STATUS_EXACT;
        row.flags |= FLAG_TARGET_STATE_COUNT;
        row.target_declared = 0;
        assert_eq!(
            record_from_order_row(orders, &row.encode(), 0).unwrap().values[2],
            Value::Unsigned(0)
        );

        // A REAL VALUE.
        row.target_declared = 5;
        assert_eq!(
            record_from_order_row(orders, &row.encode(), 0).unwrap().values[2],
            Value::Unsigned(5)
        );
    }

    #[test]
    fn a_non_zero_declared_count_under_a_clear_gate_is_refused() {
        // The captured value would be replaced by an empty state, which is data
        // loss — the shipping validator refuses it too (`lane lib.rs:1057`).
        let mut row = RawOrder::live();
        row.target_declared = 3;
        assert!(parse_order_row(&row.encode()).is_err());
    }

    #[test]
    fn a_tombstone_is_a_removal_not_a_record() {
        let s = schema();
        let orders = collection_of(&s, ORDERS).unwrap();
        let row = RawOrder::tombstone(ORDER_OWNER_KIND_ARMY, 5, 6, 7).encode();
        assert!(
            record_from_order_row(orders, &row, 0).is_err(),
            "mapping a tombstone would publish its forced zeros as measurement"
        );
        assert_eq!(
            key_from_order_row(&row).unwrap(),
            order_key(ORDER_OWNER_KIND_ARMY, 5, 6, 7)
        );
    }

    // ---- 2. Byte-identical round trip ---------------------------------------

    #[test]
    fn a_full_state_round_trips_byte_identically() {
        let schema = schema();
        let mut state = State::new();
        fold_ok(&mut state, &schema, &full_frame().encode());
        let states = state.collections();

        let mut first = Vec::new();
        encode_baseline(&schema, &states, &mut first).expect("baseline encodes");
        let (decoded, consumed) = decode_baseline(&schema, &first, 0).expect("baseline decodes");
        assert_eq!(consumed, first.len());
        let mut second = Vec::new();
        encode_baseline(&schema, &decoded, &mut second).expect("re-encode");
        assert_eq!(first, second, "encode -> decode -> re-encode is byte identical");
        for (i, collection) in schema.collections.iter().enumerate() {
            assert!(diff(collection, &decoded[i], &states[i]).is_empty());
        }
    }

    #[test]
    fn a_delta_round_trips_and_replays() {
        let schema = schema();
        let mut state = State::new();
        fold_ok(&mut state, &schema, &full_frame().encode());
        let before = state.collections();

        // A delta frame that moves the plan's middle waypoint and retires one
        // census army — the two shapes the capture's churn is made of.
        let mut row = RawOrder::live();
        row.flags = FLAG_ACTIVE_VALUE | FLAG_PATH_GEOMETRY;
        row.order_type = ORDER_TYPE_LINE;
        row.child_count = 3;
        let mut frame = Frame::new();
        frame.status_flags = STATUS_DELTA_DATA;
        frame.rows = vec![row];
        frame.paths = vec![9_632, 3_667, 9_632];
        frame.extension = Some(ogs_extension(EXTENSION_VERSION_V2, &[(7, 4, 1, 2)], &[11]));
        frame.observed_army_rows = Some(1);
        fold_ok(&mut state, &schema, &frame.encode());
        let after = state.collections();

        let deltas: Vec<Option<_>> = schema
            .collections
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let d = diff(c, &before[i], &after[i]);
                if d.is_empty() {
                    None
                } else {
                    Some(d)
                }
            })
            .collect();
        assert!(
            deltas[2].is_some(),
            "the middle waypoint changed, so the path collection emits a delta"
        );
        let mut bytes = Vec::new();
        encode_delta(&schema, &before, &deltas, &mut bytes).expect("delta encodes");
        let (read, consumed) = decode_delta(&schema, &before, &bytes, 0).expect("delta decodes");
        assert_eq!(consumed, bytes.len());
        let mut replayed = before.clone();
        for (i, d) in read.iter().enumerate() {
            if let Some(d) = d {
                apply(&mut replayed[i], d);
            }
        }
        for (i, collection) in schema.collections.iter().enumerate() {
            assert!(
                diff(collection, &replayed[i], &after[i]).is_empty(),
                "collection {} replays stored-equal",
                collection.collection_id
            );
        }
    }

    // ---- 4. Quantization -----------------------------------------------------

    #[test]
    fn the_schema_has_no_continuous_field_to_quantize() {
        // Step 8.4's conditional test: d54 carries no continuous field at all.
        // Every word in the payload is a raw id, count, index, flag or status
        // code; the fold divides nothing and there is no scale word in the
        // header. So there is no quantization saving to measure and no
        // drawn-rather-than-interpolated judgement to make.
        for collection in &schema().collections {
            for f in &collection.fields {
                assert_ne!(f.kind, Kind::Continuous, "field {}", f.field_id);
                assert_eq!(f.quant, Quant::None, "field {}", f.field_id);
            }
        }
    }

    // ---- Grammar constants ---------------------------------------------------

    #[test]
    fn the_grammar_constants_are_enforced_not_encoded() {
        // Six words the accepted grammar hard-enforces, and which are therefore
        // enforced here rather than carried (the d62/d24 rule). Each is checked
        // by mutating a frame that otherwise folds.
        for mutate in [
            |row: &mut RawOrder| row.active_status = STATUS_UNKNOWN,
            |row: &mut RawOrder| row.assigned_units_status = STATUS_UNKNOWN,
            |row: &mut RawOrder| row.order_linkage_status = STATUS_UNKNOWN,
            |row: &mut RawOrder| row.flags &= !FLAG_ACTIVE_VALUE,
            |row: &mut RawOrder| row.assigned_count = 3,
            |row: &mut RawOrder| {
                row.flags |= FLAG_PATH_GEOMETRY;
                row.child_count = 0;
            },
        ] {
            let mut row = RawOrder::live();
            mutate(&mut row);
            assert!(
                parse_order_row(&row.encode()).is_err(),
                "a grammar constant was not enforced"
            );
        }

        // And the two in the census header.
        let mut extension = ogs_extension(EXTENSION_VERSION_V2, &[(7, 0, 0, 0)], &[11]);
        extension[12..16].copy_from_slice(&99u32.to_le_bytes()); // registered_entry_count
        assert!(decode_extension(&extension).is_err());
        let mut extension = ogs_extension(EXTENSION_VERSION_V2, &[(7, 0, 0, 0)], &[11]);
        extension[20..24].copy_from_slice(&0u32.to_le_bytes()); // flags
        assert!(decode_extension(&extension).is_err());
    }

    #[test]
    fn the_witness_bits_are_carried_because_nothing_couples_them() {
        // The two "blocked" flags are set by the emitter from a status code, but
        // no validator couples them, so on the accepted grammar they are
        // independent data. A frame carrying one with an unrelated status folds,
        // and the boolean survives.
        let s = schema();
        let orders = collection_of(&s, ORDERS).unwrap();
        let mut row = RawOrder::live();
        row.flags |= FLAG_NON_TYPE5_GEOMETRY_BLOCKED;
        row.geometry_status = STATUS_EXACT;
        let record = record_from_order_row(orders, &row.encode(), 0).expect("folds");
        assert_eq!(record.values[9], Value::Bool(true));
        assert_eq!(record.values[5], Value::Enum(STATUS_EXACT));
    }

    // ---- split_payload -------------------------------------------------------

    #[test]
    fn the_world_fold_decoder_adds_no_refusal_of_its_own() {
        // `decode_army_battleplans_depth_payload` (`world_fold.rs::decode_army_battleplans_depth_payload`) runs
        // AFTER the status gates, so a refusal only it makes would be an excluded
        // hour rather than a stream rejection. It makes none: its five checks are
        // a short payload, a bad OGS1 extension, an unknown owner kind, a child
        // range past its table, and a declared/actual child-count mismatch — and
        // `split_payload` makes every one of them, at the earlier position.
        let base = full_frame();

        // short payload
        let mut bytes = base.encode();
        bytes.truncate(bytes.len() - 1);
        assert!(split_payload(&bytes).is_err());

        // bad OGS1
        let mut frame = full_frame();
        let mut extension = frame.extension.clone().unwrap();
        extension[0] = b'X';
        frame.extension = Some(extension);
        assert!(split_payload(&frame.encode()).is_err());

        // unknown owner kind
        let mut frame = full_frame();
        frame.rows[0].owner_kind = 2;
        assert!(split_payload(&frame.encode()).is_err());

        // child range past its table
        let mut frame = full_frame();
        frame.rows[0].child_first = 1;
        assert!(split_payload(&frame.encode()).is_err());

        // declared/actual child-count mismatch
        let mut frame = full_frame();
        frame.rows[0].child_count = 2;
        assert!(split_payload(&frame.encode()).is_err());
    }

    #[test]
    fn the_validator_rules_split_payload_owns() {
        let base = full_frame();
        assert!(split_payload(&base.encode()).is_ok());

        // `changed_rows <= row_count`, which THIS validator does read — unlike
        // d17/d34, where the same word is parsed and never enforced.
        let mut frame = full_frame();
        frame.changed_rows = Some(9);
        assert!(split_payload(&frame.encode()).is_err());

        // `observed_order_rows >= row_count` on a non-delta frame only.
        let mut frame = full_frame();
        frame.observed_order_rows = Some(0);
        assert!(split_payload(&frame.encode()).is_err());
        let mut frame = full_frame();
        frame.observed_order_rows = Some(0);
        frame.status_flags = STATUS_DELTA_DATA;
        assert!(
            split_payload(&frame.encode()).is_ok(),
            "a delta may emit more rows than the census observed"
        );

        // zero observed armies with rows emitted
        let mut frame = full_frame();
        frame.observed_army_rows = Some(0);
        assert!(split_payload(&frame.encode()).is_err());

        // the census width must equal the header's own
        let mut frame = full_frame();
        frame.observed_army_rows = Some(3);
        assert!(split_payload(&frame.encode()).is_err());

        // a non-zero reserved word in any child table
        let mut bytes = full_frame().encode();
        let child_start = HEADER_LEN + ROW_LEN;
        bytes[child_start + 4..child_start + 8].copy_from_slice(&1u32.to_le_bytes());
        assert!(split_payload(&bytes).is_err());

        // out-of-range child ids, per table
        for (max, offset) in [(MAX_PROVINCE_ID, 0usize)] {
            let mut bytes = full_frame().encode();
            let at = HEADER_LEN + ROW_LEN + offset;
            bytes[at..at + 4].copy_from_slice(&(max + 1).to_le_bytes());
            assert!(split_payload(&bytes).is_err());
        }
    }

    #[test]
    fn an_overlong_count_is_refused_rather_than_overflowing() {
        // Every count is an untrusted word off disk; `count * stride` must not
        // wrap. A public entry point may not panic on any byte string.
        let mut payload = full_frame().encode();
        payload[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(split_payload(&payload).is_err());
        let mut payload = full_frame().encode();
        payload[48..52].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(split_payload(&payload).is_err());
    }

    #[test]
    fn no_byte_string_panics_a_public_entry_point() {
        // A deterministic sweep over truncations and single-byte mutations of a
        // well-formed payload: every one must be an `Ok` or an `Err`, never a
        // panic.
        let good = full_frame().encode();
        for cut in 0..good.len() {
            let _ = split_payload(&good[..cut]);
            let _ = peek_header(&good[..cut]);
        }
        for index in 0..good.len() {
            for delta in [1u8, 0x80, 0xFF] {
                let mut bytes = good.clone();
                bytes[index] = bytes[index].wrapping_add(delta);
                let _ = split_payload(&bytes);
                if bytes.len() >= HEADER_LEN + ROW_LEN {
                    let _ = parse_order_row(&bytes[HEADER_LEN..HEADER_LEN + ROW_LEN]);
                }
            }
        }
        assert!(split_payload(&[]).is_err());
    }

    // ---- The fold ------------------------------------------------------------

    #[test]
    fn the_four_fold_outcomes_are_reproduced_in_the_folds_own_order() {
        let schema = schema();

        // Failure status: nothing folds, the accumulator is retained.
        let mut state = State::new();
        fold_ok(&mut state, &schema, &full_frame().encode());
        let before = state.collections();
        let mut frame = full_frame();
        frame.extension = Some(ogs_extension(EXTENSION_VERSION_V2, &[(3, 0, 0, 0)], &[]));
        frame.observed_army_rows = Some(1);
        assert_eq!(
            state
                .fold_payload(&schema, &frame.encode(), true, false)
                .unwrap(),
            FoldOutcome::FailureStatus
        );
        assert_eq!(state.collections(), before, "accumulator retained");

        // The tolerance withholds the hour AFTER validating every row.
        assert_eq!(
            state
                .fold_payload(&schema, &frame.encode(), false, true)
                .unwrap(),
            FoldOutcome::ToleratedPartialWithheld
        );
        assert_eq!(state.collections(), before, "accumulator retained");

        // ...but a malformed withheld frame is still a refused stream: the
        // recorder validates it in full before it withholds it.
        let mut malformed = full_frame();
        malformed.rows[0].slot_index = MAX_SLOT_INDEX;
        assert!(state
            .fold_payload(&schema, &malformed.encode(), false, true)
            .is_err());

        // Delta before baseline.
        let mut fresh = State::new();
        let mut delta = full_frame();
        delta.status_flags = STATUS_DELTA_DATA;
        assert_eq!(
            fresh.fold_payload(&schema, &delta.encode(), false, false).unwrap(),
            FoldOutcome::DeltaBeforeBaseline
        );
        assert!(!fresh.baseline_seen());
        assert_eq!(fresh.collections(), State::new().collections());
    }

    #[test]
    fn a_non_delta_frame_replaces_the_orders_and_a_delta_patches_them() {
        let schema = schema();
        let mut state = State::new();
        fold_ok(&mut state, &schema, &full_frame().encode());
        assert!(state.baseline_seen());
        assert_eq!(state.orders.len(), 1);

        // A delta adds a second order and leaves the first alone.
        let mut second = RawOrder::live();
        second.instance = 2_000;
        second.generation = 900;
        let mut frame = Frame::new();
        frame.status_flags = STATUS_DELTA_DATA;
        frame.rows = vec![second];
        frame.extension = full_frame().extension;
        frame.observed_army_rows = Some(2);
        fold_ok(&mut state, &schema, &frame.encode());
        assert_eq!(state.orders.len(), 2);
        assert_eq!(state.path_geometry.len(), 3, "the first order kept its path");

        // A clean frame with no rows wipes the table — measured on this capture,
        // where 248 of the 266 folded hours are exactly this.
        let mut empty = Frame::new();
        empty.extension = full_frame().extension;
        empty.observed_army_rows = Some(2);
        empty.observed_order_rows = Some(0);
        fold_ok(&mut state, &schema, &empty.encode());
        assert!(state.orders.is_empty());
        assert!(state.path_geometry.is_empty(), "children follow their parent");
        assert_eq!(
            state.orders_group_armies.len(),
            2,
            "the census is a separate accumulator and is untouched by the order clear"
        );
    }

    #[test]
    fn a_tombstone_removes_its_order_and_every_child_row() {
        let schema = schema();
        let mut state = State::new();
        fold_ok(&mut state, &schema, &full_frame().encode());
        assert_eq!(state.path_geometry.len(), 3);

        let mut frame = Frame::new();
        frame.status_flags = STATUS_DELTA_DATA;
        frame.rows = vec![RawOrder::tombstone(
            ORDER_OWNER_KIND_ARMY,
            11_480,
            1_055,
            608,
        )];
        frame.extension = full_frame().extension;
        frame.observed_army_rows = Some(2);
        fold_ok(&mut state, &schema, &frame.encode());
        assert!(state.orders.is_empty());
        assert!(
            state.path_geometry.is_empty(),
            "a removed order leaves no orphan waypoints"
        );
    }

    #[test]
    fn a_shortened_polyline_drops_exactly_its_tail_rows() {
        // The measured shape of path churn: 44 of the capture's 77 path changes
        // shrink. The child rows of an upserted order are replaced wholesale, so
        // a shorter polyline must not leave the old tail behind.
        let schema = schema();
        let mut state = State::new();
        fold_ok(&mut state, &schema, &full_frame().encode());
        assert_eq!(state.path_geometry.len(), 3);

        let mut row = RawOrder::live();
        row.flags = FLAG_ACTIVE_VALUE | FLAG_PATH_GEOMETRY;
        row.order_type = ORDER_TYPE_LINE;
        row.child_count = 1;
        let mut frame = Frame::new();
        frame.status_flags = STATUS_DELTA_DATA;
        frame.rows = vec![row];
        frame.paths = vec![9_632];
        frame.extension = full_frame().extension;
        frame.observed_army_rows = Some(2);
        fold_ok(&mut state, &schema, &frame.encode());
        assert_eq!(state.path_geometry.len(), 1);
    }

    #[test]
    fn a_legacy_frame_clears_the_census_rather_than_inheriting_it() {
        // `world_fold.rs::fold_army_battleplans_depth`. The three per-hour states stay apart:
        // the singleton row is simply absent, which is what distinguishes "no
        // manager this hour" from "a measured empty census".
        let schema = schema();
        let mut state = State::new();
        fold_ok(&mut state, &schema, &full_frame().encode());
        assert_eq!(state.orders_group_manager.len(), 1);
        assert_eq!(state.orders_group_armies.len(), 2);

        let mut legacy = full_frame();
        legacy.extension = None;
        legacy.observed_army_rows = Some(1);
        fold_ok(&mut state, &schema, &legacy.encode());
        assert!(state.orders_group_manager.is_empty());
        assert!(state.orders_group_armies.is_empty());
        assert!(state.orders_group_army_groups.is_empty());
    }

    #[test]
    fn a_v1_census_says_its_army_group_ids_were_not_carried() {
        let schema = schema();
        let mut state = State::new();
        let mut frame = full_frame();
        frame.extension = Some(ogs_extension(
            EXTENSION_VERSION_V1,
            &[(7, 0, 0, 0), (9, 0, 0, 0)],
            &[11, 12],
        ));
        frame.observed_army_rows = Some(2);
        fold_ok(&mut state, &schema, &frame.encode());
        let record = &state.orders_group_manager[&manager_singleton_key()];
        assert_eq!(
            record.values,
            vec![Value::Unsigned(2), Value::Bool(false)],
            "two army groups exist and neither id was carried"
        );
        assert!(
            state.orders_group_army_groups.is_empty(),
            "and the census collection is empty because they were not measured"
        );
    }

    #[test]
    fn a_duplicate_key_in_one_frame_is_last_writer_wins() {
        // The fold enforces no key uniqueness inside a frame
        // (`world_fold.rs::fold_army_battleplans_depth`), so this port must not either —
        // refusing would reject a payload the recorder folds.
        let schema = schema();
        let mut first = RawOrder::live();
        first.slot_index = 1;
        let mut second = RawOrder::live();
        second.slot_index = 2;
        let mut frame = full_frame();
        frame.rows = vec![first, second];
        frame.paths = Vec::new();
        frame.observed_order_rows = Some(2);
        let mut state = State::new();
        fold_ok(&mut state, &schema, &frame.encode());
        assert_eq!(state.orders.len(), 1);
        assert_eq!(
            state.orders[&order_key(ORDER_OWNER_KIND_ARMY, 11_480, 1_055, 608)].values[0],
            Value::Unsigned(2),
            "last writer wins, exactly as the fold's BTreeMap insert does"
        );
    }

    #[test]
    fn a_mapper_refuses_the_wrong_collection() {
        let s = schema();
        let orders = collection_of(&s, ORDERS).unwrap();
        let groups = collection_of(&s, ORDERS_GROUP_ARMY_GROUPS).unwrap();
        assert!(record_from_target_row(orders, &child(1), 0).is_err());
        // The key-only collection is the case that would otherwise fail
        // silently: its field loop iterates nothing.
        assert!(record_from_army_group_id(orders, 5, 0).is_err());
        assert!(record_from_army_group_id(groups, 5, 0).is_ok());
        assert!(record_from_army_group_id(groups, 0, 0).is_err());
    }

    #[test]
    fn every_content_id_field_is_listed_for_the_manifest_retrofit() {
        // The content-identity contract assignment exists; listing the fields is what lets the
        // pending mapper-retrofit sweep find them, and asserting the
        // list is what tells "checked" from "not checked" (d35's rule).
        let s = schema();
        let mut found = Vec::new();
        for collection in &s.collections {
            for f in &collection.fields {
                if f.kind == Kind::ContentId {
                    found.push((collection.collection_id, f.field_id));
                    assert!(!f.is_key(), "d54 declares no ContentId key");
                }
            }
        }
        assert_eq!(found, CONTENT_ID_FIELDS.to_vec());
    }

    #[test]
    fn the_enum_cardinalities_match_the_shipping_bounds() {
        let s = schema();
        let orders = collection_of(&s, ORDERS).unwrap();
        for f in orders.value_fields() {
            if f.kind == Kind::Enum {
                assert_eq!(f.enum_card, Some(STATUS_CARD), "field {}", f.field_id);
            }
        }
        let armies = collection_of(&s, ORDERS_GROUP_ARMIES).unwrap();
        for f in armies.value_fields() {
            match f.field_id {
                field::COHESION_TYPE => assert_eq!(f.enum_card, Some(MAX_COHESION_TYPE + 1)),
                field::LEADER_PROXIMITY => assert_eq!(f.enum_card, Some(MAX_LEADER_PROXIMITY + 1)),
                // execution_type is deliberately NOT an enum: the decoder does
                // not bound it and the capture carries 4.
                field::EXECUTION_TYPE => assert_eq!(f.kind, Kind::Unsigned),
                other => panic!("unexpected census field {other}"),
            }
        }
    }
}
