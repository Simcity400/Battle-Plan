//! d27 `army_orders` — the battle-plan orders held by armies and army groups,
//! with the target states an area-defence order covers.
//!
//! Derived from `world_fold.rs:17804`-`:17985`
//! (`fold_army_orders`), its payload constants at `:967`-`:969` and
// @world-fold-authority domain=27 functions=fold_army_orders lines=17804-17985 sha256=648d691c782cbd794ebb2848dc82aaf4e1ec16f0b0e4cdd9ace7e393ba22ce5d closure=9a40c3270b04367dda2ab1c25eadbb69947295bd7f5cf5b2401e02354e2a464f
//! `:1019`-`:1023`, the accumulator's own JSON projection (`:7915`-`:7952`,
//! `:23532`-`:23541`), and the collector read that produces the flags, the
//! status word and the target-state child rows
//! (`extractor/native/collector/src/attach_log.rs:19154`-`:19352`,
//! `:46181`-`:46335`, `:69931`-`:69999`). `extractor/documents/domains/` carries
//! no d27 dossier, so the fold and the collector read are the whole basis.
//!
//! **Payload grammar** (`world_fold.rs::fold_army_orders`): a 40-byte header —
//! `elapsed_wall_ms` u64 at `+0`, `row_count` u32 at `+8`, `changed_rows` u32 at
//! `+12`, `status_flags` u32 at `+16`, `root_rows` u32 at `+20`,
//! `observed_army_rows` u32 at `+24`, `observed_order_rows` u32 at `+28`,
//! `target_state_count` u32 at `+32`, `deleted_rows` u32 at `+36` — followed by
//! `row_count` fixed 48-byte **order rows** and then `target_state_count` fixed
//! 8-byte **target rows**. The fold requires the length to be exactly
//! `40 + row_count * 48 + target_state_count * 8` (`world_fold.rs::fold_army_orders`), so no
//! legacy short row exists and the length selects nothing.
//!
//! Order row (`world_fold.rs::fold_army_orders`):
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+0`  | u32 | `owner_id` (`army_id`)          | raw | none (key; zero rejected, `world_fold.rs::fold_army_orders`) |
//! | `+4`  | u64 | `generation`                    | raw | none (key; zero rejected, `world_fold.rs::fold_army_orders`) |
//! | `+12` | u32 | `flags`                         | —   | — |
//! | `+16` | u32 | `slot_index`                    | raw | none |
//! | `+20` | i32 | `order_type`                    | raw | none — **signed**, `read_i32` at `:14274` |
//! | `+24` | u32 | `target_first`                  | —   | transport, not state |
//! | `+28` | u32 | `target_count`                  | —   | transport, not state |
//! | `+32` | u32 | `target_state_declared_count`   | raw | `TARGET_STATE_COUNT` (`1<<2`) — **collector-side only**, see below |
//! | `+36` | u32 | `target_status_code`            | raw | none |
//! | `+40` | u32 | `owner_kind`                    | raw | none (key; must be `0` or `1`, `world_fold.rs::fold_army_orders`) |
//! | `+44` | u32 | `reserved1`                     | —   | must be zero (`world_fold.rs::fold_army_orders`) |
//!
//! Target row (`world_fold.rs::fold_army_orders`): `state_id` u32 at `+0` (zero
//! rejected), `reserved` u32 at `+4` (must be zero).
//!
//! **No scales anywhere.** Every word is a raw id, count, index or code; the
//! fold divides nothing, there is no `scale_raw` word in the header and
//! `ensure_fixed_scale` is never called for this domain. The accumulator's JSON
//! projection (`world_fold.rs:7929`-`:7949`) emits every one of them verbatim.
//!
//! **Two collections, because `(target_first, target_count)` is transport.** An
//! order addresses its target states as a half-open range into one shared target
//! array; the fold proves those ranges non-overlapping (`world_fold.rs::fold_army_orders`) and
//! total (`world_fold.rs::fold_army_orders`), and proves each state id unique within its owning
//! row (`world_fold.rs::fold_army_orders`). The ranges are re-packed by the collector on every
//! frame from emission order (`attach_log.rs:69971`-`:69991`). So the ranges are
//! flattened away here: collection [`ORDERS`] carries one row per order and
//! collection [`TARGET_STATES`] one row per (order, state) pair, keyed by the
//! order's key plus the state id.
//!
//! **[`TARGET_STATES`] carries an ordinal, and that is the one place this port
//! departs from its d28 sibling.** The fold stores `target_states` as a
//! `Vec<u32>` in **producer order** (`world_fold.rs::fold_army_orders`, struct at `:23538`)
//! and the recorder's own projection emits that vector as an ordered JSON array
//! (`world_fold.rs::build_order_of_battle`). The producer refuses to sort it and says why:
//! "Preserve engine vector order. Rendering/replay consumers require the exact
//! producer order; sorting here would erase redraw semantics"
//! (`attach_log.rs:19349`-`:19351`). A keyed set orders its rows by key, so
//! keying on `state_id` alone would reproduce the *set* and silently drop the
//! sequence. This is step 5's second case — rows with a fold-enforced identity
//! that nothing ranks — so the fold's key is kept and the dense position within
//! the row's target range rides beside it as a required value field,
//! [`field::TARGET_ORDINAL`], exactly as d11's `line_ordinal` does
//! (`d11.rs:537`-`:542`). Under a `Vec`-valued fold a position-to-element map is
//! the faithful reading, and with the ordinal carried the pair
//! (key order, ordinal) reproduces the vector element for element.
//! **Measured, and reported rather than relied on**: on `deepwar_t1` all 167
//! target-carrying rows happen to be ascending in `state_id`, so the ordinal is
//! redundant with key order on *this* stream — which is a property of one
//! capture, not of the payloads the fold accepts, and the fold enforces no
//! ordering at all.
//!
//! ## The four states, field by field
//!
//! Two flag gates reach a field (`world_fold.rs:1019`-`:1023`): `TARGET_STATES`
//! (`1<<1`) and `TARGET_STATE_COUNT` (`1<<2`). The third known bit is the
//! deleted low bit, which is a removal rather than a gate.
//!
//! | field | gate set | gate clear |
//! |---|---|---|
//! | `slot_index` | always a value | — (no gate; always concrete) |
//! | `order_type` | always a value | — (no gate; always concrete) |
//! | `target_status_code` | always a value | — (no gate; always concrete) |
//! | `target_state_declared_count` | value | `target_status_code == UNSUPPORTED` → **absent**; otherwise **unknown** |
//!
//! **`target_state_declared_count` is the four-state decision the fold alone
//! gets wrong.** The fold reads `+32` unconditionally (`world_fold.rs::fold_army_orders`)
//! and stores whatever is there, so a fold-only reading publishes the packer's
//! `unwrap_or(0)` filler (`attach_log.rs:69984`) as a measured **zero**. The
//! gate is real, it is just on the collector side: the packer sets
//! `TARGET_STATE_COUNT` exactly when `row.target_state_count.is_some()`
//! (`:69974`-`:69976`). `read_army_order_target_states`
//! (`attach_log.rs:19291`-`:19352`) is the only producer of that `Option`, and
//! its branches split cleanly:
//!
//! * `order_type != ORDER_TYPE_AREA_DEFENSE` → `(None, None, "unsupported")`
//!   (`:19295`). An order that is not an area-defence order has no target-state
//!   list *and no declared size for one*: the concept is unsupported by the
//!   row, which is §3's **absent**.
//! * every other `None` size — an address overflow, an unreadable `begin` or
//!   `size` word, or `size > MAX_ARMY_ORDER_TARGET_STATES` — returns
//!   `"validation_failed"` (`:19297`-`:19315`). The engine holds a size; this
//!   sample could not read or accept it. That is §3's **unknown**.
//! * every branch that *does* know a size returns `Some(size)`, including the
//!   `"overflow"` path that reports the size while refusing to read the list
//!   (`:19319`) — so a set gate is always a real value, and `0` under a set gate
//!   is a genuinely empty target list (`:19311`), a real **zero**.
//!
//! `target_status_code` is therefore the disambiguator, it is a value field on
//! the same record, and the reader always has it. The mapper is stricter than
//! the fold in exactly one place: a **non-zero** declared count under a clear
//! gate is refused rather than silently replaced by an empty state, because the
//! packer writes `unwrap_or(0)` there and a non-zero word would be a captured
//! value this mapping would drop (step 7's second divergence shape; the same
//! call d28 makes for `leader_trait_count`). Measured: 0 of the 888 clear-gate
//! rows on `deepwar_t1` carry a non-zero word.
//!
//! **The child collection's emptiness is readable, which is why this is a port
//! and not a d17-class stop.** A cleared `TARGET_STATES` gate contributes no
//! [`TARGET_STATES`] rows, and so does a set gate over an empty list — and a
//! keyed set cannot say "this membership was not measured". Here it does not
//! have to: `target_status_code` is carried on the parent row and separates the
//! two. `target_states.is_some()` holds in exactly the two `"exact"` branches
//! (`attach_log.rs:19311`, `:19351`), so `EXACT` means the membership was
//! measured — possibly as a real empty set — and `UNSUPPORTED`, `OVERFLOW` and
//! `VALIDATION_FAILED` each name a different reason it was not. The fold itself
//! is lossier: it fabricates an empty `Vec` for an unread list
//! (`world_fold.rs::fold_army_orders` over an empty range) and keeps the status word beside
//! it, which is the same information in a shape the reader has to know to
//! consult.
//!
//! **`target_status_code` is `Enum`, with cardinality 5.** The vocabulary is the
//! collector's own `COMPACT_ARMY_ORDER_TARGET_STATUS_*`
//! (`attach_log.rs:10977`, `:11314`-`:11317`): `NONE=0, EXACT=1, UNSUPPORTED=2,
//! OVERFLOW=3, VALIDATION_FAILED=4`, and `compact_army_order_target_status_code`
//! (`:46329`-`:46335`) is total onto exactly that set — its `_ =>` arm maps
//! anything else to `NONE`. Only `{0, 1, 2}` occur on `deepwar_t1`, but `3` and
//! `4` are both reachable (`:19315`-`:19347`), so the cardinality is the
//! producer's codomain and not the capture's histogram. `NONE` reaches the wire
//! only through `deleted_compact_army_order_row`, whose `target_status` is the
//! literal `"deleted"` (`:46312`) — and a deleted row is a removal, not a
//! record, so this mapper never emits `Enum(NONE)` from a capture shaped like
//! today's. It stays in the cardinality because the fold accepts the word and a
//! future non-deleted `NONE` must map rather than fail.
//!
//! **`order_type` is `Signed`, NOT an `Enum` — this corrects the batch-2
//! survey.** The survey points at `lanes/army_battleplans_depth_live`'s
//! `ORDER_TYPE_MOVE=1 … ORDER_TYPE_AREA_DEFENSE=5` (`lib.rs:72`-`:76`) as an
//! enum vocabulary. Those constants are real, but nothing enforces them on this
//! path: `read_army_order_sample_header` reads the word as a raw `i32` straight
//! out of the order object (`attach_log.rs:19274`-`:19288`), no caller filters
//! on it — `read_army_order_target_states` and
//! `read_army_battleplans_depth_path_provinces` only *branch* on it, returning
//! `"unsupported"` for a value they do not know (`:19295`, `:19362`) — and both
//! `compact_army_order_row` (`:46287`) and the packer (`:69980`) copy it
//! verbatim. The fold reads it with `read_i32` and validates nothing
//! (`world_fold.rs:14274`). So the codomain is all of `i32`, an `Enum(6)` would
//! reject rows the fold folds — recipe step 2's "never gate against the fold" —
//! and the histogram `{1, 2, 3, 5}` measured on `deepwar_t1` is a histogram, not
//! a codomain. `PARADROP=4` never occurs on this capture at all.
//!
//! **`owner_kind` is different, and it is a key.** The fold hard-rejects any
//! value outside `{ORDER_OWNER_KIND_ARMY=0, ORDER_OWNER_KIND_ARMY_GROUP=1}`
//! (`world_fold.rs::fold_army_orders`, constants at
//! `lanes/army_battleplans_depth_live/src/orders_group_live.rs:45`-`:46`), so
//! unlike `order_type` its two-value set *is* enforced. It is still `Unsigned`
//! rather than `Enum`: §7.4 admits only `Unsigned`, `Signed` and `ContentId` as
//! key components, so an `Enum` key is not expressible, and §5 classes an engine
//! discriminator as `Unsigned` anyway — the same call d20 makes for `div_type`
//! and d28 for `kind`. The two-value set is enforced in the mapper, where the
//! fold enforces it.
//!
//! **`state_id` is a `ContentId` and it is a KEY component.** §5 names state ids
//! as content-defined ("Country tags, equipment names, province and state ids
//! are content-defined", `WIRE_FORMAT.md:346`-`:351`): state 42 is whatever
//! state the mod set defines. It is the only `ContentId` in this domain, and
//! because it sorts and delta-encodes by raw value it is the heavier retrofit
//! class flagged in §13 — see the deliverable's open questions.
//!
//! **`generation` and `owner_id` are runtime instance identities.**
//! `generation` is the collector's own tracker counter, handed out at
//! `attach_log.rs:46208`-`:46224` and stable only within one capture;
//! `owner_id` is an engine army/army-group id. Neither is resolved by a content
//! manifest, so both are `Unsigned` (§5's campaign test). The fold's own comment
//! at `world_fold.rs:23507`-`:23512` explains why `slot_index` is *not* in the
//! key: it is mutable, and including it stranded a stale twin under the old key
//! with no tombstone able to remove it.
//!
//! **Deleted rows are removals, not records** (`world_fold.rs::fold_army_orders`):
//! the low flag bit erases the key from the accumulator. They become wire
//! tombstones in *both* collections, derived by `body::diff` from the state the
//! removal produces. [`record_from_row`] refuses one rather than inventing a
//! record. The fold applies **no** emptiness requirement to a delete row, so a
//! delete row may legally declare a target range; those target rows are
//! validated and then discarded with the key, and [`fold_payload`] reproduces
//! that. (The collector's own delete rows carry no range —
//! `deleted_compact_army_order_row` sets `target_states: None` (`:46308`) — but
//! "the emitter never does this" is not a reason to refuse it.)
//!
//! **The expected-partial withhold is a fold-nothing path, and it sits after the
//! row loop.** d27 is one of exactly four domains with an expected-partial
//! tolerance (`compact_contract/src/lib.rs:141`-`:146`). `fold_army_orders`
//! decodes and validates every order row and every target row, and only then —
//! at `:14301`-`:14310`, *before* the accumulator is touched — marks the
//! domain-hour unknown and returns `Ok` without folding anything, because the
//! omitted order identities are not serialized. So a tolerated-partial frame is
//! not d27 data and its hour belongs in the measurement's exclusion set,
//! positioned after the structural and per-row validation, which is where
//! [`fold_payload`]'s caller puts it. It also never sets the fold's
//! `delta_baselines_seen`, because `replace_or_sparse_after_first` (`:14311`) is
//! never reached.
//!
//! **No derived field (§6).** Nothing here changes deterministically with the
//! clock; `frame_tick` is taken and ignored so every domain's boundary has one
//! shape.

use std::collections::{BTreeMap, BTreeSet};

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 27;
/// One row per battle-plan order, keyed by its typed owner and generation.
pub const ORDERS: u32 = 1;
/// One row per (order, target state) pair.
pub const TARGET_STATES: u32 = 2;

/// Field ids are stable and append-only; §11 forbids reusing an id for a
/// different field. The three key ids appear in both collections because they
/// are the same field in both — the order's identity — which is what §11
/// forbids reusing an id *against*.
pub mod field {
    pub const OWNER_KIND: u32 = 1;
    pub const OWNER_ID: u32 = 2;
    pub const GENERATION: u32 = 3;
    pub const SLOT_INDEX: u32 = 4;
    pub const ORDER_TYPE: u32 = 5;
    pub const TARGET_STATE_DECLARED_COUNT: u32 = 6;
    pub const TARGET_STATUS_CODE: u32 = 7;
    pub const STATE_ID: u32 = 8;
    pub const TARGET_ORDINAL: u32 = 9;
}

// --- Payload grammar, from `world_fold.rs:967`-`:969` and `:1019`-`:1023`. ---

/// `ARMY_ORDER_HEADER_LEN`, `world_fold.rs::ARMY_ORDER_HEADER_LEN`.
pub const HEADER_LEN: usize = 40;
/// `ARMY_ORDER_ROW_LEN`, `world_fold.rs::ARMY_ORDER_ROW_LEN`.
pub const ROW_LEN: usize = 48;
/// `ARMY_ORDER_TARGET_ROW_LEN`, `world_fold.rs::ARMY_ORDER_TARGET_ROW_LEN`.
pub const TARGET_ROW_LEN: usize = 8;

/// `battleplans54::ORDER_OWNER_KIND_*`, re-exported at
/// `lanes/army_battleplans_depth_live/src/lib.rs:62`-`:63` from
/// `orders_group_live.rs:45`-`:46`. A value outside this pair is a rejected row
/// (`world_fold.rs::fold_army_orders`), which is why `owner_kind` needs no enum
/// cardinality on the wire — see [`schema`].
pub const OWNER_KIND_ARMY: u32 = 0;
pub const OWNER_KIND_ARMY_GROUP: u32 = 1;

/// `ROW_FLAG_DELETED_LOW_BIT`, `world_fold.rs::ROW_FLAG_DELETED_LOW_BIT`.
pub const FLAG_DELETED: u32 = 1;
/// `ARMY_ORDER_ROW_FLAG_TARGET_STATES`, `world_fold.rs::ARMY_ORDER_ROW_FLAG_TARGET_STATES`.
pub const FLAG_TARGET_STATES: u32 = 1 << 1;
/// `ARMY_ORDER_ROW_FLAG_TARGET_STATE_COUNT`, `world_fold.rs::ARMY_ORDER_ROW_FLAG_TARGET_STATE_COUNT`.
pub const FLAG_TARGET_STATE_COUNT: u32 = 1 << 2;
/// `ARMY_ORDER_KNOWN_FLAGS`, `world_fold.rs::ARMY_ORDER_KNOWN_FLAGS`. Any other bit is a
/// hard error (`:14241`-`:14243`) — an unrecognised flag word means the row is
/// not this grammar, so nothing in it can be trusted.
pub const KNOWN_FLAGS: u32 = FLAG_DELETED | FLAG_TARGET_STATES | FLAG_TARGET_STATE_COUNT;

/// `COMPACT_ARMY_ORDER_TARGET_STATUS_*`, `collector/src/attach_log.rs:10977`
/// and `:11314`-`:11317`. The producer,
/// `compact_army_order_target_status_code` (`:46329`), is total onto this set.
pub const TARGET_STATUS_NONE: u32 = 0;
pub const TARGET_STATUS_EXACT: u32 = 1;
pub const TARGET_STATUS_UNSUPPORTED: u32 = 2;
pub const TARGET_STATUS_OVERFLOW: u32 = 3;
pub const TARGET_STATUS_VALIDATION_FAILED: u32 = 4;
/// One past the highest status code; §4 rejects a code at or above this.
pub const TARGET_STATUS_CARDINALITY: u32 = 5;

pub fn schema() -> Schema {
    use field as f;
    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![
            Collection {
                collection_id: ORDERS,
                // Orders persist across hours and the frame grammar is
                // first-frame-replaces / later-frames-upsert / low-bit-removes
                // (`world_fold.rs::fold_army_orders`), which is exactly a keyed
                // set's grammar — and only a keyed set has deltas (§7.3).
                shape: Shape::KeyedSet,
                fields: vec![
                    // The fold's key is the typed identity
                    // `(owner_kind, owner_id, generation)`
                    // (`world_fold.rs::fold_army_orders`); all three are validated
                    // in every row (`world_fold.rs::fold_army_orders`), so all three are
                    // concrete. `owner_kind` is `Unsigned`, not `Enum`: §7.4
                    // admits no `Enum` key, and §5 classes an engine
                    // discriminator as `Unsigned` — d20's call for `div_type`.
                    Field::new(f::OWNER_KIND, Kind::Unsigned).key(0),
                    // An engine army / army-group id: a runtime instance
                    // identity created during play, so `Unsigned` (§5).
                    Field::new(f::OWNER_ID, Kind::Unsigned).key(1),
                    // The collector's own generation counter
                    // (`attach_log.rs:46208`-`:46224`) — a runtime instance
                    // identity, `Unsigned`. Not `Monotone`: it is an identity,
                    // and §7.4 admits no `Monotone` key.
                    Field::new(f::GENERATION, Kind::Unsigned).key(2),
                    // The order's position in its owner's order walk
                    // (`attach_log.rs:19080`-`:19081`) — always a value, never
                    // gated, and deliberately not a key component
                    // (`world_fold.rs:23507`-`:23512`). Declares no capability.
                    Field::new(f::SLOT_INDEX, Kind::Unsigned),
                    // `read_i32` at `world_fold.rs:14274`, and the collector
                    // validates nothing — see the module docs on why this is
                    // `Signed` and not the survey's `Enum`.
                    Field::new(f::ORDER_TYPE, Kind::Signed),
                    // The engine's own size for the target-state list, gated by
                    // the collector's `TARGET_STATE_COUNT` flag. Absent when the
                    // order type has no target-state concept, unknown when the
                    // size could not be read or accepted; never null, because no
                    // producer branch means "there is genuinely no size".
                    Field::new(f::TARGET_STATE_DECLARED_COUNT, Kind::Unsigned)
                        .optional()
                        .unknown_capable(),
                    // The capture-quality code for the target-state read.
                    // `Enum` with the producer's full codomain, never the
                    // histogram of one capture. Always concrete: the packer
                    // writes a code for every row (`attach_log.rs:69986`).
                    Field::new(f::TARGET_STATUS_CODE, Kind::Enum)
                        .enum_card(TARGET_STATUS_CARDINALITY),
                ],
            },
            Collection {
                collection_id: TARGET_STATES,
                shape: Shape::KeyedSet,
                fields: vec![
                    Field::new(f::OWNER_KIND, Kind::Unsigned).key(0),
                    Field::new(f::OWNER_ID, Kind::Unsigned).key(1),
                    Field::new(f::GENERATION, Kind::Unsigned).key(2),
                    // A the game state id: content-defined (§5). A `ContentId`
                    // **key**, which is the heavier §13 retrofit class.
                    Field::new(f::STATE_ID, Kind::ContentId).key(3),
                    // The dense position of this state within its order's target
                    // range. The fold stores the targets as an ordered `Vec`
                    // and the producer refuses to sort it
                    // (`attach_log.rs:19349`-`:19351`), so the sequence is state
                    // and rides beside the key — step 5's `line_ordinal` case.
                    // Always concrete.
                    Field::new(f::TARGET_ORDINAL, Kind::Unsigned),
                ],
            },
        ],
        event_part: None,
    }
}

// --- The boundary mapping ----------------------------------------------------

/// The 40-byte payload header (`world_fold.rs::fold_army_orders`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub row_count: u32,
    pub changed_rows: u32,
    pub status_flags: u32,
    pub root_rows: u32,
    pub observed_army_rows: u32,
    pub observed_order_rows: u32,
    pub target_state_count: u32,
    pub deleted_rows: u32,
}

/// One parsed order row. `target_first`/`target_count` are kept because the
/// caller needs them to find the row's target states, not because they are
/// state: they index the frame's shared target array and are re-packed from
/// emission order every frame (`attach_log.rs:69971`-`:69991`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OrderRow {
    pub owner_kind: u32,
    pub owner_id: u32,
    pub generation: u64,
    pub flags: u32,
    pub target_first: usize,
    pub target_count: usize,
    /// The low flag bit: remove this key. Never a record; a wire tombstone.
    pub deleted: bool,
}

/// Read the 40-byte payload header and nothing else
/// (`world_fold.rs::fold_army_orders`).
///
/// Exposing the header read is what keeps it from becoming a second, hand-rolled
/// copy of the header layout in every caller that needs `status_flags` before it
/// decides whether the record is measurable at all. It applies no invariant
/// beyond the length needed to read the words themselves — which is exactly the
/// fold's own call order: `need(payload, ARMY_ORDER_HEADER_LEN)` at `world_fold.rs::fold_army_orders`,
/// then the three status guards at `world_fold.rs::fold_army_orders`, and only then the payload
/// length check at `world_fold.rs::fold_army_orders`. Enforcing the body length here would refuse records
/// the recorder accepts and folds nothing from.
pub fn peek_header(payload: &[u8]) -> core::result::Result<PayloadHeader, String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "army_orders payload {} shorter than the {HEADER_LEN}-byte header",
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
        observed_order_rows: u32_at(payload, 28),
        target_state_count: u32_at(payload, 32),
        deleted_rows: u32_at(payload, 36),
    })
}

/// Split a d27 DATA payload into its header, its order rows and its target rows,
/// applying every structural check `fold_army_orders` applies before it starts
/// reading rows (`world_fold.rs::fold_army_orders`).
///
/// This lives here rather than in each caller so the header grammar has exactly
/// one implementation: a measurement harness that re-derives it is a harness
/// that can drift from the recorder without anything failing. It is fail-closed
/// and total — no input of any length panics.
///
/// **No mode parameter, deliberately.** d27's structural invariants are the
/// exact length and three `ensure_count_at_most` bounds (`:14194`-`:14212`), and
/// none of them depends on the frame mode. In particular the fold does **not**
/// require `changed_rows == row_count` in any mode — d26's fold does, and
/// copying that here would reject healthy frames (47 of the 146 d27 hours on
/// `deepwar_t1` are `DELTA_DATA`, and `changed_rows` tracks the patch, not the
/// table).
pub fn split_payload(payload: &[u8]) -> core::result::Result<(PayloadHeader, &[u8], &[u8]), String> {
    let header = peek_header(payload)?;

    // Checked arithmetic throughout: both counts are attacker-shaped words from
    // a file on disk, and either product overflows `usize` for a large enough
    // one. `world_fold.rs::fold_army_orders` uses saturating arithmetic, which
    // turns an overflowing claim into a length that cannot match; rejecting is
    // the same outcome reached without relying on that.
    let rows_len = (header.row_count as usize)
        .checked_mul(ROW_LEN)
        .ok_or_else(|| format!("army_orders row_count {} overflows", header.row_count))?;
    let targets_len = (header.target_state_count as usize)
        .checked_mul(TARGET_ROW_LEN)
        .ok_or_else(|| {
            format!(
                "army_orders target_state_count {} overflows",
                header.target_state_count
            )
        })?;
    let expected = HEADER_LEN
        .checked_add(rows_len)
        .and_then(|len| len.checked_add(targets_len))
        .ok_or_else(|| "army_orders payload length overflows".to_string())?;
    if payload.len() != expected {
        return Err(format!("army_orders len {} != {expected}", payload.len()));
    }

    // `ensure_count_at_most`, `world_fold.rs:14194`-`:14212`. Three bounds, all
    // `<=`, all unconditional — and the third relates two header words to each
    // other rather than to `row_count`, which is why it belongs here beside them
    // and not in the row loop.
    if header.changed_rows > header.row_count {
        return Err(format!(
            "army_orders changed_rows {} > row_count {}",
            header.changed_rows, header.row_count
        ));
    }
    if header.deleted_rows > header.row_count {
        return Err(format!(
            "army_orders deleted_rows {} > row_count {}",
            header.deleted_rows, header.row_count
        ));
    }
    if header.observed_army_rows > header.root_rows {
        return Err(format!(
            "army_orders observed_army_rows {} > root_rows {}",
            header.observed_army_rows, header.root_rows
        ));
    }

    let rows = &payload[HEADER_LEN..HEADER_LEN + rows_len];
    let targets = &payload[HEADER_LEN + rows_len..];
    Ok((header, rows, targets))
}

/// Parse one 48-byte order row, applying every per-row check the fold applies
/// (`world_fold.rs::fold_army_orders`) except the target-range bounds and the
/// cross-row uniqueness rules, which need the whole frame and live in
/// [`target_index_range`] / [`fold_payload`].
pub fn parse_row(row: &[u8]) -> core::result::Result<OrderRow, String> {
    if row.len() != ROW_LEN {
        return Err(format!("army_orders row length {} != {ROW_LEN}", row.len()));
    }
    let owner_id = u32_at(row, 0);
    let generation = u64::from_le_bytes(row[4..12].try_into().expect("8 bytes"));
    let owner_kind = u32_at(row, 40);
    // `world_fold.rs::fold_army_orders`: one combined typed-identity check.
    if owner_id == 0
        || generation == 0
        || !matches!(owner_kind, OWNER_KIND_ARMY | OWNER_KIND_ARMY_GROUP)
    {
        return Err(format!(
            "army_orders row has invalid typed identity owner_kind={owner_kind} \
             owner_id={owner_id} generation={generation}"
        ));
    }
    let flags = u32_at(row, 12);
    if flags & !KNOWN_FLAGS != 0 {
        return Err(format!("army_orders row has unknown flags {flags:#x}"));
    }
    let target_first = u32_at(row, 24) as usize;
    let target_count = u32_at(row, 28) as usize;
    // `world_fold.rs::fold_army_orders`: the range words must be zero when the
    // gate is clear. This is the only gate the fold itself validates.
    if flags & FLAG_TARGET_STATES == 0 && (target_first != 0 || target_count != 0) {
        return Err("army_orders target range present without flag".into());
    }
    // `world_fold.rs::fold_army_orders`.
    if u32_at(row, 44) != 0 {
        return Err("army_orders row reserved field is nonzero".into());
    }
    Ok(OrderRow {
        owner_kind,
        owner_id,
        generation,
        flags,
        target_first,
        target_count,
        deleted: flags & FLAG_DELETED != 0,
    })
}

/// Parse one 8-byte target row, applying the fold's per-row checks
/// (`world_fold.rs::fold_army_orders`), and return its `state_id`.
pub fn parse_target_row(row: &[u8]) -> core::result::Result<u32, String> {
    if row.len() != TARGET_ROW_LEN {
        return Err(format!(
            "army_orders target row length {} != {TARGET_ROW_LEN}",
            row.len()
        ));
    }
    let state_id = u32_at(row, 0);
    if u32_at(row, 4) != 0 {
        return Err("army_orders target reserved field is nonzero".into());
    }
    if state_id == 0 {
        return Err("army_orders target state id is zero".into());
    }
    Ok(state_id)
}

/// The half-open index range of one order's target rows within the frame's
/// shared target array, checked exactly as the fold checks it
/// (`world_fold.rs::fold_army_orders`).
///
/// One derivation of the range rule, so the check and the use cannot disagree.
/// The ranges may not overlap — the fold proves ownership is injective
/// (`world_fold.rs::fold_army_orders`) — but that is a whole-frame property and lives in
/// [`fold_payload`]; this is the per-row bound.
pub fn target_index_range(
    row: &OrderRow,
    target_state_count: usize,
) -> core::result::Result<core::ops::Range<usize>, String> {
    let end = row
        .target_first
        .checked_add(row.target_count)
        .ok_or_else(|| "army_orders target range overflow".to_string())?;
    if end > target_state_count {
        return Err(format!(
            "army_orders target range {}+{} exceeds {target_state_count}",
            row.target_first, row.target_count
        ));
    }
    Ok(row.target_first..end)
}

/// The [`ORDERS`] key of one order: `(owner_kind, owner_id, generation)`
/// (`world_fold.rs::fold_army_orders`).
pub fn order_key(owner_kind: u32, owner_id: u32, generation: u64) -> Key {
    key(&[owner_kind as u64, owner_id as u64, generation])
}

/// The [`TARGET_STATES`] key of one (order, state) pair: its order's key plus
/// the state id, which is the identity the fold's per-row uniqueness check is on
/// (`world_fold.rs::fold_army_orders`).
pub fn target_key(owner_kind: u32, owner_id: u32, generation: u64, state_id: u32) -> Key {
    key(&[
        owner_kind as u64,
        owner_id as u64,
        generation,
        state_id as u64,
    ])
}

/// The [`ORDERS`] key of one raw order row.
pub fn key_from_row(row: &[u8]) -> core::result::Result<Key, String> {
    let parsed = parse_row(row)?;
    Ok(order_key(parsed.owner_kind, parsed.owner_id, parsed.generation))
}

/// The [`TARGET_STATES`] key of one raw target row under one raw order row.
///
/// The parent's identity comes first, d23's and d83's convention, so a caller
/// that already parsed the order row does not parse it twice — though this form
/// takes the raw bytes for callers that have not.
pub fn key_from_target_row(
    order_row: &[u8],
    target_row: &[u8],
) -> core::result::Result<Key, String> {
    let parsed = parse_row(order_row)?;
    let state_id = parse_target_row(target_row)?;
    Ok(target_key(
        parsed.owner_kind,
        parsed.owner_id,
        parsed.generation,
        state_id,
    ))
}

/// One compact row of the `orders` table, as the collector emits it today, onto
/// its stored Record. `row` is the raw 48-byte row slice (the caller has already
/// consumed the payload header via [`split_payload`]); d27 has exactly one row
/// form, so length selects nothing and a wrong length is rejected. `frame_tick`
/// is §6's derived-field input (stored = v + c * (frame_tick >> u)); d27
/// declares no derived field, so it is taken and ignored — every domain's mapper
/// has the same shape.
///
/// Fail-closed throughout, mirroring `fold_army_orders`: a row the fold would
/// reject is an `Err` here, never a coerced value. A **deleted** row is an `Err`
/// too — it is a removal (`world_fold.rs::fold_army_orders`) and has no record; a
/// caller must map it to a wire tombstone, and quietly handing one back a
/// gates-all-clear row would turn a delete into an upsert. Nothing in this
/// function can panic on any input.
pub fn record_from_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    // d27 has no derived field; §6's transform input is unused. Named rather
    // than `_frame_tick` because the signature is the cross-domain contract.
    let _ = frame_tick;
    let parsed = parse_row(row)?;
    if parsed.deleted {
        return Err(format!(
            "army_orders ({}, {}, {}) is a delete row, not a record",
            parsed.owner_kind, parsed.owner_id, parsed.generation
        ));
    }
    let flags = parsed.flags;

    // The status word first: it decides the four-state answer for
    // `target_state_declared_count`, so an out-of-vocabulary code must stop the
    // row before it is read as a gate. §4: "a value the writer cannot map into
    // 0..n-1 is a failed sample and a typed gap, never a coerced code." The
    // producer is total onto `0..=4` (`attach_log.rs:46329`-`:46335`), so this
    // refuses nothing the collector can emit.
    let target_status_code = u32_at(row, 36);
    if target_status_code >= TARGET_STATUS_CARDINALITY {
        return Err(format!(
            "army_orders target_status_code {target_status_code} is outside the \
             0..{TARGET_STATUS_CARDINALITY} vocabulary"
        ));
    }

    // `target_state_declared_count` is packed `unwrap_or(0)` under a clear gate
    // (`attach_log.rs:69984`), so a non-zero word there is a captured value the
    // empty-state mapping below would silently drop — the one way this boundary
    // could lose data. Refused rather than mapped. Measured: 0 of 888 clear-gate
    // rows on deepwar_t1.
    let declared_count = u32_at(row, 32);
    if flags & FLAG_TARGET_STATE_COUNT == 0 && declared_count != 0 {
        return Err(format!(
            "army_orders target_state_declared_count {declared_count} present \
             without the TARGET_STATE_COUNT flag"
        ));
    }

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            // Always a value: `slot_index` is `out.len()` in the order walk
            // (`attach_log.rs:19080`) and the fold reads it unconditionally
            // (`world_fold.rs::fold_army_orders`).
            field::SLOT_INDEX => Value::Unsigned(u32_at(row, 16) as u64),
            // `read_i32` (`world_fold.rs:14274`); no producer validates the
            // vocabulary, so the codomain is all of `i32`.
            field::ORDER_TYPE => Value::Signed(i32_at(row, 20) as i64),
            field::TARGET_STATE_DECLARED_COUNT => {
                if flags & FLAG_TARGET_STATE_COUNT != 0 {
                    Value::Unsigned(declared_count as u64)
                } else if target_status_code == TARGET_STATUS_UNSUPPORTED {
                    // `read_army_order_target_states` returns
                    // `(None, None, "unsupported")` for every order type that is
                    // not AREA_DEFENSE (`attach_log.rs:19295`): the row has no
                    // target-state list and no declared size for one, so the
                    // concept is unsupported by this row.
                    Value::Absent
                } else {
                    // Every other `None` size is a read that was attempted and
                    // failed — an address overflow, an unreadable `begin`/`size`
                    // word, or a size past `MAX_ARMY_ORDER_TARGET_STATES`
                    // (`attach_log.rs:19297`-`:19315`), all `"validation_failed"`.
                    // Never `Null`: no producer branch means "there is genuinely
                    // no size".
                    Value::Unknown
                }
            }
            field::TARGET_STATUS_CODE => Value::Enum(target_status_code),
            other => {
                // A collection this mapper does not know how to fill. Filling it
                // with a placeholder would publish invented data under a field
                // id that means something else.
                return Err(format!(
                    "army_orders orders has no mapping for field id {other}"
                ));
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// One compact target row onto its stored [`TARGET_STATES`] Record.
///
/// `ordinal` is the row's dense position within its order's target range — the
/// order witness the keyed set cannot otherwise carry (see the module docs). It
/// is a parameter rather than something this function derives, because step 5's
/// rule is that a generated ordinal is counted at exactly one place: the caller
/// that walks the range. `frame_tick` is §6's input and is unused here.
///
/// The state id itself is **not** a value field — it is key component 3, and
/// carrying it beside the key would be a second copy of state the decoder must
/// then keep consistent. The row is still validated
/// (`world_fold.rs::fold_army_orders`), and a collection carrying a value field
/// this mapper cannot fill is still refused.
pub fn record_from_target_row(
    collection: &Collection,
    row: &[u8],
    ordinal: u32,
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    let _ = parse_target_row(row)?;
    // A key-only-looking collection whose field loop iterates almost nothing
    // would come out silently near-empty if built for the wrong collection, so
    // the id is checked before the loop (`variant_upgrades.rs:633`).
    if collection.collection_id != TARGET_STATES {
        return Err(format!(
            "army_orders target row built for collection {} (expected {TARGET_STATES})",
            collection.collection_id
        ));
    }
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::TARGET_ORDINAL => Value::Unsigned(ordinal as u64),
            other => {
                return Err(format!(
                    "army_orders target_states has no mapping for field id {other}"
                ));
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

// --- The frame-level fold ----------------------------------------------------

/// One order in the accumulator: its [`ORDERS`] record and its target states in
/// the fold's own stored order. This is `world_fold.rs:23532`'s
/// `ArmyOrderState` with the fields the wire schema carries, plus the target
/// vector the child table flattens into.
#[derive(Clone, Debug, PartialEq)]
pub struct OrderEntry {
    pub record: Record,
    /// Producer order, exactly as `world_fold.rs::fold_army_orders` stores it. The child
    /// collection's ordinal is this vector's index.
    pub targets: Vec<u32>,
}

/// The d27 accumulator: `world_fold.rs:1696`'s
/// `BTreeMap<ArmyOrderKey, ArmyOrderState>`, keyed by
/// `(owner_kind, owner_id, generation)`.
///
/// One d27 record is **not** one state — the domain is a full first frame
/// followed by sparse upserts (`world_fold.rs::fold_army_orders`) — so the frame-level fold is a
/// public, tested boundary here rather than a private copy in each caller.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub orders: BTreeMap<(u32, u32, u64), OrderEntry>,
}

impl State {
    pub fn new() -> Self {
        Self::default()
    }

    /// The two keyed states this accumulator represents, in schema order.
    pub fn keyed_states(&self) -> Vec<KeyedState> {
        let mut orders = KeyedState::new();
        let mut targets = KeyedState::new();
        for ((owner_kind, owner_id, generation), entry) in &self.orders {
            orders.insert(
                order_key(*owner_kind, *owner_id, *generation),
                entry.record.clone(),
            );
            for (ordinal, state_id) in entry.targets.iter().enumerate() {
                targets.insert(
                    target_key(*owner_kind, *owner_id, *generation, *state_id),
                    Record::new(vec![Value::Unsigned(ordinal as u64)]),
                );
            }
        }
        vec![orders, targets]
    }
}

/// `replace_or_sparse_after_first` for the frames that reach the fold
/// (`world_fold.rs::replace_or_sparse_after_first`).
///
/// A tolerated-partial frame never reaches it — d27's withhold at `:14301`
/// returns first — so the expected-partial arm of that helper is unreachable
/// here and is deliberately not modelled. `baseline_seen` is the caller's
/// `delta_baselines_seen` membership for domain 27.
pub fn frame_is_sparse(delta: bool, baseline_seen: bool) -> bool {
    delta || baseline_seen
}

/// Apply one d27 payload to the accumulator exactly as `fold_army_orders` does
/// (`world_fold.rs::fold_army_orders`), returning its header.
///
/// `sparse` is [`frame_is_sparse`]'s answer: `false` clears the accumulator
/// first (`world_fold.rs::fold_army_orders`), `true` upserts into it.
///
/// The caller is responsible for the four fold-nothing paths, all of which
/// return before this function's effect: failure status (`world_fold.rs::fold_army_orders`),
/// expected-partial-before-baseline (`world_fold.rs::fold_army_orders`), delta-before-baseline (`world_fold.rs::fold_army_orders`)
/// and — d27's own — the expected-partial **withhold** at `world_fold.rs::fold_army_orders`, which sits
/// *after* every order row and every target row has been decoded and validated.
/// That ordering matters and is why this function is fail-closed on row content
/// regardless of status: a tolerated-partial frame whose rows are malformed is a
/// corrupt frame in the recorder too.
///
/// Fail-closed: every rejection the fold makes is an `Err` here, and a frame this
/// rejects leaves the accumulator exactly as it was — `fold_army_orders` builds
/// its whole `rows`/`targets` picture before its first `clear`, so the
/// all-or-nothing error contract is the fold's own. Total: no payload of any
/// content panics.
pub fn fold_payload(
    state: &mut State,
    payload: &[u8],
    sparse: bool,
    orders: &Collection,
    target_states: &Collection,
) -> core::result::Result<PayloadHeader, String> {
    let (header, row_bytes, target_bytes) = split_payload(payload)?;
    let target_state_count = header.target_state_count as usize;

    // Pass 1 over the order rows, in the fold's own order
    // (`world_fold.rs::fold_army_orders`): per-row validity, key uniqueness across
    // the frame, and target-range ownership.
    let mut rows = Vec::with_capacity(header.row_count as usize);
    let mut row_keys = BTreeSet::new();
    let mut target_owners: Vec<Option<usize>> = vec![None; target_state_count];
    for (index, bytes) in row_bytes.chunks_exact(ROW_LEN).enumerate() {
        let row = parse_row(bytes).map_err(|e| format!("row {index}: {e}"))?;
        // `world_fold.rs::fold_army_orders`: a key serialized twice in one frame is
        // a rejected frame, never a last-writer-wins overwrite.
        if !row_keys.insert((row.owner_kind, row.owner_id, row.generation)) {
            return Err(format!(
                "army_orders row {index} duplicates typed identity ({}, {}, {})",
                row.owner_kind, row.owner_id, row.generation
            ));
        }
        let range = target_index_range(&row, target_state_count)
            .map_err(|e| format!("row {index}: {e}"))?;
        // `:14258`-`:14263`: ownership is injective, so the ranges do not
        // overlap. This runs for **every** row including a delete row, exactly
        // as the fold does — the fold's delete branch is after the row loop.
        for owner in &mut target_owners[range] {
            if owner.replace(index).is_some() {
                return Err("army_orders target ranges overlap".into());
            }
        }
        rows.push(row);
    }
    if rows.len() != header.row_count as usize {
        return Err(format!(
            "army_orders decoded {} rows, header claims {}",
            rows.len(),
            header.row_count
        ));
    }

    // Pass 2 over the target rows (`world_fold.rs::fold_army_orders`): every target
    // row is decoded and checked before any row is applied, so a single invalid
    // target rejects the whole frame — and every target row must be owned
    // (totality) and unique within its owner.
    let mut target_ids = Vec::with_capacity(target_state_count);
    let mut ids_by_row: Vec<BTreeSet<u32>> = vec![BTreeSet::new(); rows.len()];
    for (index, bytes) in target_bytes.chunks_exact(TARGET_ROW_LEN).enumerate() {
        let state_id = parse_target_row(bytes).map_err(|e| format!("target row {index}: {e}"))?;
        let Some(owner) = target_owners[index] else {
            return Err(format!("army_orders target child {index} is unowned"));
        };
        if !ids_by_row[owner].insert(state_id) {
            return Err(format!(
                "army_orders row {owner} duplicates a target state id {state_id}"
            ));
        }
        // Validating through the child collection's own mapper keeps the
        // schema's arity check on this path too. The ordinal is the row's
        // position within its owner's range, filled in below where the range is
        // walked; zero here is only the validation call.
        let _ = record_from_target_row(target_states, bytes, 0, 0)
            .map_err(|e| format!("target row {index}: {e}"))?;
        target_ids.push(state_id);
    }
    if target_ids.len() != target_state_count {
        return Err(format!(
            "army_orders decoded {} target rows, header claims {target_state_count}",
            target_ids.len()
        ));
    }

    // Build every surviving order before touching the accumulator, so a frame
    // this rejects leaves it exactly as it was.
    let mut removals: Vec<(u32, u32, u64)> = Vec::new();
    let mut upserts: Vec<((u32, u32, u64), OrderEntry)> = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let key = (row.owner_kind, row.owner_id, row.generation);
        if row.deleted {
            // `world_fold.rs::fold_army_orders`. The fold imposes no emptiness
            // requirement on a delete row, so a target range it declared is
            // validated above and then discarded with the key.
            removals.push(key);
            continue;
        }
        let bytes = &row_bytes[index * ROW_LEN..(index + 1) * ROW_LEN];
        let record = record_from_row(orders, bytes, 0).map_err(|e| format!("row {index}: {e}"))?;
        let range =
            target_index_range(row, target_state_count).map_err(|e| format!("row {index}: {e}"))?;
        upserts.push((
            key,
            OrderEntry {
                record,
                // `world_fold.rs::fold_army_orders`: the fold's own slice, in producer
                // order. The child collection's ordinal is this vector's index.
                targets: target_ids[range].to_vec(),
            },
        ));
    }

    // `world_fold.rs::fold_army_orders`: only a non-sparse frame replaces the
    // world. Keys are unique within the frame, so `removals` and `upserts` name
    // disjoint keys and their order does not matter.
    if !sparse {
        state.orders.clear();
    }
    for key in removals {
        state.orders.remove(&key);
    }
    for (key, entry) in upserts {
        state.orders.insert(key, entry);
    }

    Ok(header)
}

// --- helpers -----------------------------------------------------------------

/// Callers reach these only after a length check, so the slices are in bounds.
fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn i32_at(b: &[u8], off: usize) -> i32 {
    i32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{diff, KeyedState};
    use crate::TickGrid;

    fn grid() -> TickGrid {
        TickGrid::new(3).expect("3 is a legal grid")
    }

    fn orders() -> Collection {
        schema().collections[0].clone()
    }

    fn targets() -> Collection {
        schema().collections[1].clone()
    }

    fn index_of(c: &Collection, id: u32) -> usize {
        c.value_fields()
            .position(|f| f.field_id == id)
            .unwrap_or_else(|| panic!("field {id} is a value field"))
    }

    /// The eleven words of one order row, in row order
    /// (`world_fold.rs::fold_army_orders`). `generation` is a u64 at `+4`, so the
    /// row is built explicitly rather than from a `[u32; 12]`.
    #[allow(clippy::too_many_arguments)]
    fn order_row(
        owner_id: u32,
        generation: u64,
        flags: u32,
        slot_index: u32,
        order_type: i32,
        target_first: u32,
        target_count: u32,
        declared_count: u32,
        status: u32,
        owner_kind: u32,
        reserved: u32,
    ) -> Vec<u8> {
        let mut r = Vec::with_capacity(ROW_LEN);
        r.extend_from_slice(&owner_id.to_le_bytes());
        r.extend_from_slice(&generation.to_le_bytes());
        r.extend_from_slice(&flags.to_le_bytes());
        r.extend_from_slice(&slot_index.to_le_bytes());
        r.extend_from_slice(&order_type.to_le_bytes());
        r.extend_from_slice(&target_first.to_le_bytes());
        r.extend_from_slice(&target_count.to_le_bytes());
        r.extend_from_slice(&declared_count.to_le_bytes());
        r.extend_from_slice(&status.to_le_bytes());
        r.extend_from_slice(&owner_kind.to_le_bytes());
        r.extend_from_slice(&reserved.to_le_bytes());
        assert_eq!(r.len(), ROW_LEN);
        r
    }

    fn target_row(state_id: u32) -> Vec<u8> {
        let mut r = Vec::with_capacity(TARGET_ROW_LEN);
        r.extend_from_slice(&state_id.to_le_bytes());
        r.extend_from_slice(&0u32.to_le_bytes());
        r
    }

    /// A whole payload: the 40-byte header, then the rows, then the targets.
    #[allow(clippy::too_many_arguments)]
    fn payload(
        status_flags: u32,
        root_rows: u32,
        observed_army_rows: u32,
        observed_order_rows: u32,
        deleted_rows: u32,
        rows: &[Vec<u8>],
        targets: &[u32],
    ) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // changed_rows
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&root_rows.to_le_bytes());
        p.extend_from_slice(&observed_army_rows.to_le_bytes());
        p.extend_from_slice(&observed_order_rows.to_le_bytes());
        p.extend_from_slice(&(targets.len() as u32).to_le_bytes());
        p.extend_from_slice(&deleted_rows.to_le_bytes());
        assert_eq!(p.len(), HEADER_LEN);
        for r in rows {
            p.extend_from_slice(r);
        }
        for t in targets {
            p.extend_from_slice(&target_row(*t));
        }
        p
    }

    /// A live area-defence row: both gates set, three target states.
    fn area_defence_row(owner_id: u32, generation: u64, first: u32, count: u32) -> Vec<u8> {
        order_row(
            owner_id,
            generation,
            FLAG_TARGET_STATES | FLAG_TARGET_STATE_COUNT,
            2,
            5,
            first,
            count,
            count,
            TARGET_STATUS_EXACT,
            OWNER_KIND_ARMY,
            0,
        )
    }

    // --- step 8.1 -----------------------------------------------------------

    #[test]
    fn schema_validates() {
        schema().validate(grid().log2()).expect("d27 schema is valid");
    }

    #[test]
    fn schema_hash_is_stable_across_calls() {
        assert_eq!(schema().schema_hash(), schema().schema_hash());
    }

    // --- step 8.5: shape and key -------------------------------------------

    #[test]
    fn shape_and_key_are_the_folds() {
        let s = schema();
        assert_eq!(s.domain, 27);
        assert_eq!(s.collections.len(), 2);
        assert!(s.event_part.is_none(), "d27 carries no event part");

        let o = &s.collections[0];
        assert_eq!(o.collection_id, ORDERS);
        assert!(matches!(o.shape, Shape::KeyedSet));
        let okeys: Vec<_> = o
            .key_fields()
            .into_iter()
            .map(|f| (f.field_id, f.kind, f.key_position))
            .collect();
        assert_eq!(
            okeys,
            vec![
                (field::OWNER_KIND, Kind::Unsigned, Some(0)),
                (field::OWNER_ID, Kind::Unsigned, Some(1)),
                (field::GENERATION, Kind::Unsigned, Some(2)),
            ]
        );

        let t = &s.collections[1];
        assert_eq!(t.collection_id, TARGET_STATES);
        assert!(matches!(t.shape, Shape::KeyedSet));
        let tkeys: Vec<_> = t
            .key_fields()
            .into_iter()
            .map(|f| (f.field_id, f.kind, f.key_position))
            .collect();
        assert_eq!(
            tkeys,
            vec![
                (field::OWNER_KIND, Kind::Unsigned, Some(0)),
                (field::OWNER_ID, Kind::Unsigned, Some(1)),
                (field::GENERATION, Kind::Unsigned, Some(2)),
                (field::STATE_ID, Kind::ContentId, Some(3)),
            ]
        );

        // Key components are concrete everywhere: never optional, nullable,
        // unknown-capable or derived (§7.4).
        for c in &s.collections {
            for f in c.key_fields() {
                assert!(!f.optional && !f.nullable && !f.unknown_capable);
                assert!(f.derived.is_none());
                assert!(f.kind.legal_as_key());
            }
        }
    }

    /// §13's retrofit sweep needs "checked, none here" to be distinguishable
    /// from "not checked". d27 has exactly one `ContentId`, and it is a KEY.
    #[test]
    fn the_only_content_id_is_the_state_id_key() {
        let s = schema();
        let content: Vec<_> = s
            .collections
            .iter()
            .flat_map(|c| {
                c.fields
                    .iter()
                    .filter(|f| matches!(f.kind, Kind::ContentId))
                    .map(move |f| (c.collection_id, f.field_id, f.key_position))
            })
            .collect();
        assert_eq!(content, vec![(TARGET_STATES, field::STATE_ID, Some(3))]);
    }

    /// Step 8.4 does not apply: d27 has no `Continuous` field at all — every
    /// word in the payload is a raw id, count, index or code and the fold
    /// divides nothing (`world_fold.rs::fold_army_orders`) — so there is nothing to
    /// quantize and no all-`Exact` variant to compare against.
    #[test]
    fn no_field_is_continuous_so_quantization_does_not_apply() {
        for c in &schema().collections {
            for f in &c.fields {
                assert!(!matches!(f.kind, Kind::Continuous), "field {}", f.field_id);
                assert!(!f.quant.is_quantized(), "field {}", f.field_id);
            }
        }
    }

    // --- step 8.6: every offset, and every header word ----------------------

    #[test]
    fn every_header_word_reads_from_its_own_offset() {
        // Distinct values per word, so a transposition cannot pass.
        let mut p = Vec::new();
        p.extend_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
        for w in [3u32, 2, 0x400, 9, 8, 7, 4, 1] {
            p.extend_from_slice(&w.to_le_bytes());
        }
        assert_eq!(p.len(), HEADER_LEN);
        let h = peek_header(&p).expect("header reads");
        assert_eq!(h.elapsed_wall_ms, 0x1122_3344_5566_7788);
        assert_eq!(h.row_count, 3);
        assert_eq!(h.changed_rows, 2);
        assert_eq!(h.status_flags, 0x400);
        assert_eq!(h.root_rows, 9);
        assert_eq!(h.observed_army_rows, 8);
        assert_eq!(h.observed_order_rows, 7);
        assert_eq!(h.target_state_count, 4);
        assert_eq!(h.deleted_rows, 1);
    }

    #[test]
    fn a_short_payload_is_refused_not_read() {
        for len in 0..HEADER_LEN {
            assert!(peek_header(&vec![0u8; len]).is_err(), "len {len}");
        }
    }

    #[test]
    fn every_row_word_reads_from_its_own_offset() {
        // Every word distinct, and none of them equal to another's value, so a
        // swapped pair of offsets fails.
        let row = order_row(
            0x0000_1111, // owner_id       +0
            0x2222_3333_4444_5555,
            FLAG_TARGET_STATES | FLAG_TARGET_STATE_COUNT, // flags +12
            0x6666,                                       // slot_index     +16
            -0x0777_7777,                                 // order_type     +20 (signed!)
            0,                                            // target_first   +24
            2,                                            // target_count   +28
            0x0888_8888,                                  // declared_count +32
            TARGET_STATUS_OVERFLOW,                       // status         +36
            OWNER_KIND_ARMY_GROUP,                        // owner_kind     +40
            0,                                            // reserved       +44
        );
        let parsed = parse_row(&row).expect("row parses");
        assert_eq!(parsed.owner_id, 0x0000_1111);
        assert_eq!(parsed.generation, 0x2222_3333_4444_5555);
        assert_eq!(parsed.flags, FLAG_TARGET_STATES | FLAG_TARGET_STATE_COUNT);
        assert_eq!(parsed.target_first, 0);
        assert_eq!(parsed.target_count, 2);
        assert_eq!(parsed.owner_kind, OWNER_KIND_ARMY_GROUP);
        assert!(!parsed.deleted);

        let c = orders();
        let rec = record_from_row(&c, &row, 0).expect("row maps");
        assert_eq!(
            rec.values[index_of(&c, field::SLOT_INDEX)],
            Value::Unsigned(0x6666)
        );
        assert_eq!(
            rec.values[index_of(&c, field::ORDER_TYPE)],
            Value::Signed(-0x0777_7777),
            "order_type is signed and comes from +20"
        );
        assert_eq!(
            rec.values[index_of(&c, field::TARGET_STATE_DECLARED_COUNT)],
            Value::Unsigned(0x0888_8888)
        );
        assert_eq!(
            rec.values[index_of(&c, field::TARGET_STATUS_CODE)],
            Value::Enum(TARGET_STATUS_OVERFLOW)
        );

        // And the key comes from +40, +0, +4 in that order — `owner_kind` and
        // `owner_id` are adjacent in neither the row nor the key, which is the
        // transposition this asserts against.
        assert_eq!(
            key_from_row(&row).expect("key"),
            order_key(
                OWNER_KIND_ARMY_GROUP,
                0x0000_1111,
                0x2222_3333_4444_5555
            )
        );
    }

    #[test]
    fn every_target_row_word_reads_from_its_own_offset() {
        assert_eq!(parse_target_row(&target_row(1234)).expect("parses"), 1234);
        // A non-zero reserved word is a rejected row (`world_fold.rs::fold_army_orders`).
        let mut bad = target_row(1234);
        bad[4] = 1;
        assert!(parse_target_row(&bad).is_err());
        // And the state id is zero-rejected (`:14290`).
        assert!(parse_target_row(&target_row(0)).is_err());
    }

    // --- step 8.3: states through the mapper, from raw bytes ----------------

    /// [`ORDERS`] declares exactly two capabilities and only on one field:
    /// `.optional()` and `.unknown_capable()` on
    /// `target_state_declared_count`. No field is nullable anywhere in this
    /// domain, so there is no Null case to build — and the states test below
    /// asserts that, so a later edit that adds `.nullable()` without a mapper
    /// path fails here.
    #[test]
    fn declared_capabilities_match_what_the_mapper_emits() {
        for c in &schema().collections {
            for f in &c.fields {
                assert!(!f.nullable, "d27 has no nullable field: {}", f.field_id);
                if f.field_id == field::TARGET_STATE_DECLARED_COUNT {
                    assert!(f.optional && f.unknown_capable);
                } else {
                    assert!(
                        !f.optional && !f.unknown_capable,
                        "field {} declares a capability its mapper never emits",
                        f.field_id
                    );
                }
            }
        }
    }

    #[test]
    fn a_real_zero_is_a_real_zero() {
        // A set TARGET_STATE_COUNT gate over a genuinely empty list
        // (`attach_log.rs:19311`: `size == 0` returns `Some(0)` with `"exact"`).
        let row = order_row(
            7,
            1,
            FLAG_TARGET_STATES | FLAG_TARGET_STATE_COUNT,
            0,
            5,
            0,
            0,
            0,
            TARGET_STATUS_EXACT,
            OWNER_KIND_ARMY,
            0,
        );
        let c = orders();
        let rec = record_from_row(&c, &row, 0).expect("maps");
        assert_eq!(
            rec.values[index_of(&c, field::TARGET_STATE_DECLARED_COUNT)],
            Value::Unsigned(0),
            "a measured zero is never collapsed into an empty state"
        );
        assert_eq!(
            rec.values[index_of(&c, field::SLOT_INDEX)],
            Value::Unsigned(0),
            "slot 0 is a real slot"
        );
    }

    #[test]
    fn a_clear_count_gate_under_unsupported_is_absent() {
        // The 594+238+23 rows/hour class on deepwar_t1: a non-area-defence
        // order, both gates clear, status UNSUPPORTED.
        let row = order_row(
            7,
            1,
            0,
            1,
            2,
            0,
            0,
            0,
            TARGET_STATUS_UNSUPPORTED,
            OWNER_KIND_ARMY,
            0,
        );
        let c = orders();
        let rec = record_from_row(&c, &row, 0).expect("maps");
        assert_eq!(
            rec.values[index_of(&c, field::TARGET_STATE_DECLARED_COUNT)],
            Value::Absent
        );
    }

    #[test]
    fn a_clear_count_gate_under_a_failed_read_is_unknown() {
        for status in [TARGET_STATUS_VALIDATION_FAILED, TARGET_STATUS_OVERFLOW] {
            let row = order_row(7, 1, 0, 1, 5, 0, 0, 0, status, OWNER_KIND_ARMY, 0);
            let c = orders();
            let rec = record_from_row(&c, &row, 0).expect("maps");
            assert_eq!(
                rec.values[index_of(&c, field::TARGET_STATE_DECLARED_COUNT)],
                Value::Unknown,
                "status {status}"
            );
        }
        // NONE is the `_ =>` arm (`attach_log.rs:46334`), reachable today only
        // on a delete row. On a live row it is still not "unsupported", so it
        // takes the conservative arm.
        let row = order_row(7, 1, 0, 1, 5, 0, 0, 0, TARGET_STATUS_NONE, OWNER_KIND_ARMY, 0);
        let c = orders();
        let rec = record_from_row(&c, &row, 0).expect("maps");
        assert_eq!(
            rec.values[index_of(&c, field::TARGET_STATE_DECLARED_COUNT)],
            Value::Unknown
        );
    }

    /// The whole `(status, gate)` space, swept rather than sampled: every code
    /// in the vocabulary, both gate states, and the answer pinned per cell.
    #[test]
    fn the_declared_count_four_state_is_a_total_function_of_status_and_gate() {
        let c = orders();
        for status in 0..TARGET_STATUS_CARDINALITY {
            for gate in [false, true] {
                let flags = if gate { FLAG_TARGET_STATE_COUNT } else { 0 };
                let word = if gate { 11 } else { 0 };
                let row = order_row(7, 1, flags, 0, 5, 0, 0, word, status, OWNER_KIND_ARMY, 0);
                let rec = record_from_row(&c, &row, 0).expect("maps");
                let got = rec.values[index_of(&c, field::TARGET_STATE_DECLARED_COUNT)];
                let want = if gate {
                    Value::Unsigned(11)
                } else if status == TARGET_STATUS_UNSUPPORTED {
                    Value::Absent
                } else {
                    Value::Unknown
                };
                assert_eq!(got, want, "status {status} gate {gate}");
            }
        }
    }

    /// The divergence from the fold, pinned: the fold reads `+32`
    /// unconditionally, so a non-zero word under a clear gate would have its
    /// captured value replaced by an empty state. Refused instead.
    #[test]
    fn a_nonzero_declared_count_without_its_gate_is_refused() {
        let row = order_row(
            7,
            1,
            0,
            0,
            5,
            0,
            0,
            3, // the fold would store this 3; the mapping would drop it
            TARGET_STATUS_VALIDATION_FAILED,
            OWNER_KIND_ARMY,
            0,
        );
        let err = record_from_row(&orders(), &row, 0).expect_err("refused");
        assert!(err.contains("without the TARGET_STATE_COUNT flag"), "{err}");
    }

    #[test]
    fn an_out_of_vocabulary_status_code_is_refused() {
        for status in [TARGET_STATUS_CARDINALITY, 99, u32::MAX] {
            let row = order_row(7, 1, 0, 0, 5, 0, 0, 0, status, OWNER_KIND_ARMY, 0);
            let err = record_from_row(&orders(), &row, 0).expect_err("refused");
            assert!(err.contains("outside the"), "{err}");
        }
    }

    #[test]
    fn a_delete_row_is_a_removal_not_a_record() {
        let row = order_row(
            7,
            1,
            FLAG_DELETED,
            0,
            2,
            0,
            0,
            0,
            TARGET_STATUS_NONE,
            OWNER_KIND_ARMY,
            0,
        );
        let err = record_from_row(&orders(), &row, 0).expect_err("refused");
        assert!(err.contains("delete row"), "{err}");
        // But its key is still readable — that is what a tombstone needs.
        assert_eq!(
            key_from_row(&row).expect("key"),
            order_key(OWNER_KIND_ARMY, 7, 1)
        );
    }

    #[test]
    fn the_fold_s_row_rejections_are_this_mappers_rejections() {
        // owner_id 0, generation 0, an out-of-set owner_kind, an unknown flag
        // bit, a non-zero reserved word, and a target range without its flag.
        let bad = [
            order_row(0, 1, 0, 0, 2, 0, 0, 0, 2, OWNER_KIND_ARMY, 0),
            order_row(7, 0, 0, 0, 2, 0, 0, 0, 2, OWNER_KIND_ARMY, 0),
            order_row(7, 1, 0, 0, 2, 0, 0, 0, 2, 2, 0),
            order_row(7, 1, 0, 0, 2, 0, 0, 0, 2, u32::MAX, 0),
            order_row(7, 1, 1 << 3, 0, 2, 0, 0, 0, 2, OWNER_KIND_ARMY, 0),
            order_row(7, 1, 0, 0, 2, 0, 0, 0, 2, OWNER_KIND_ARMY, 1),
            order_row(7, 1, 0, 0, 2, 1, 0, 0, 2, OWNER_KIND_ARMY, 0),
            order_row(7, 1, 0, 0, 2, 0, 1, 0, 2, OWNER_KIND_ARMY, 0),
        ];
        for (i, row) in bad.iter().enumerate() {
            assert!(parse_row(row).is_err(), "case {i}");
        }
        // A wrong row length is refused rather than read out of bounds.
        assert!(parse_row(&[]).is_err());
        assert!(parse_row(&vec![0u8; ROW_LEN - 1]).is_err());
        assert!(parse_row(&vec![0u8; ROW_LEN + 1]).is_err());
    }

    #[test]
    fn the_target_mapper_refuses_a_foreign_collection() {
        let err = record_from_target_row(&orders(), &target_row(5), 0, 0).expect_err("refused");
        assert!(err.contains("expected"), "{err}");
    }

    #[test]
    fn the_target_record_carries_its_ordinal() {
        let c = targets();
        for ordinal in [0u32, 1, 7, u32::MAX] {
            let rec = record_from_target_row(&c, &target_row(5), ordinal, 0).expect("maps");
            assert_eq!(
                rec.values[index_of(&c, field::TARGET_ORDINAL)],
                Value::Unsigned(ordinal as u64)
            );
        }
    }

    // --- step 8.2: byte-identical round trip --------------------------------

    #[test]
    fn a_full_state_round_trips_byte_identically() {
        let s = schema();
        let rows = vec![
            area_defence_row(11, 5, 0, 3),
            order_row(
                12,
                6,
                0,
                1,
                -3,
                0,
                0,
                0,
                TARGET_STATUS_VALIDATION_FAILED,
                OWNER_KIND_ARMY_GROUP,
                0,
            ),
            order_row(13, 7, 0, 4, 2, 0, 0, 0, TARGET_STATUS_UNSUPPORTED, OWNER_KIND_ARMY, 0),
        ];
        let p = payload(0, 3, 3, 3, 0, &rows, &[900, 12, 431]);
        let mut state = State::new();
        fold_payload(&mut state, &p, false, &s.collections[0], &s.collections[1])
            .expect("folds");
        let states = state.keyed_states();
        assert_eq!(states[0].len(), 3);
        assert_eq!(states[1].len(), 3);

        let mut a = Vec::new();
        crate::body::encode_baseline(&s, &states, &mut a).expect("encodes");
        let (decoded, consumed) = crate::body::decode_baseline(&s, &a, 0).expect("decodes");
        assert_eq!(consumed, a.len());
        let mut b = Vec::new();
        crate::body::encode_baseline(&s, &decoded, &mut b).expect("re-encodes");
        assert_eq!(a, b, "encode -> decode -> encode is byte-identical");
    }

    #[test]
    fn a_delta_round_trips_and_replays() {
        let s = schema();
        let first = payload(
            0,
            2,
            2,
            2,
            0,
            &[area_defence_row(11, 5, 0, 2), area_defence_row(12, 6, 2, 1)],
            &[900, 431, 77],
        );
        // Hour two: order 11 loses a target and gains one, order 12 is deleted.
        // The status word is the caller's classification input, not something
        // `fold_payload` reads — sparseness is the explicit parameter — so the
        // header carries 0 here and the sparse decision is passed directly.
        let second = payload(
            0,
            2,
            2,
            2,
            1,
            &[
                area_defence_row(11, 5, 0, 2),
                order_row(
                    12,
                    6,
                    FLAG_DELETED,
                    0,
                    5,
                    0,
                    0,
                    0,
                    TARGET_STATUS_NONE,
                    OWNER_KIND_ARMY,
                    0,
                ),
            ],
            &[900, 555],
        );

        let mut state = State::new();
        fold_payload(&mut state, &first, false, &s.collections[0], &s.collections[1])
            .expect("baseline folds");
        let before = state.keyed_states();
        fold_payload(&mut state, &second, true, &s.collections[0], &s.collections[1])
            .expect("delta folds");
        let after = state.keyed_states();
        assert_eq!(after[0].len(), 1, "order 12 was removed");
        assert_eq!(after[1].len(), 2, "and so were its targets");

        let deltas: Vec<Option<crate::body::KeyedDelta>> = s
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
        assert!(deltas[1].as_ref().expect("targets changed").tombstones.len() >= 2);

        let mut bytes = Vec::new();
        crate::body::encode_delta(&s, &before, &deltas, &mut bytes).expect("encodes");
        let mut reader: Vec<KeyedState> = before.clone();
        let (decoded, consumed) =
            crate::body::decode_delta(&s, &reader, &bytes, 0).expect("decodes");
        assert_eq!(consumed, bytes.len());
        for (i, d) in decoded.iter().enumerate() {
            if let Some(d) = d {
                crate::body::apply(&mut reader[i], d);
            }
        }
        for (i, c) in s.collections.iter().enumerate() {
            assert!(
                diff(c, &reader[i], &after[i]).is_empty(),
                "collection {} replays stored-equal",
                c.collection_id
            );
        }
    }

    // --- the frame-level fold ----------------------------------------------

    #[test]
    fn the_ordinal_reproduces_producer_order_not_key_order() {
        // Descending state ids: the keyed set orders them 12 < 431 < 900 while
        // the fold's vector is [900, 431, 12]. Only the ordinal recovers it.
        let s = schema();
        let p = payload(0, 1, 1, 1, 0, &[area_defence_row(11, 5, 0, 3)], &[900, 431, 12]);
        let mut state = State::new();
        fold_payload(&mut state, &p, false, &s.collections[0], &s.collections[1]).expect("folds");
        assert_eq!(
            state.orders[&(OWNER_KIND_ARMY, 11, 5)].targets,
            vec![900, 431, 12],
            "the entry holds the producer's order"
        );
        let states = state.keyed_states();
        let c = targets();
        let ordinal_of = |state_id: u32| -> u64 {
            match states[1][&target_key(OWNER_KIND_ARMY, 11, 5, state_id)].values
                [index_of(&c, field::TARGET_ORDINAL)]
            {
                Value::Unsigned(n) => n,
                other => panic!("ordinal is unsigned, got {other:?}"),
            }
        };
        assert_eq!(ordinal_of(900), 0);
        assert_eq!(ordinal_of(431), 1);
        assert_eq!(ordinal_of(12), 2);
    }

    #[test]
    fn a_first_frame_replaces_and_later_frames_upsert() {
        assert!(!frame_is_sparse(false, false), "the first non-delta frame replaces");
        assert!(frame_is_sparse(true, false), "a delta is always sparse");
        assert!(frame_is_sparse(false, true), "and so is everything after the first");

        let s = schema();
        let mut state = State::new();
        fold_payload(
            &mut state,
            &payload(0, 1, 1, 1, 0, &[area_defence_row(11, 5, 0, 1)], &[900]),
            false,
            &s.collections[0],
            &s.collections[1],
        )
        .expect("folds");
        // A sparse frame naming a different order keeps the first one.
        fold_payload(
            &mut state,
            &payload(0, 1, 1, 1, 0, &[area_defence_row(12, 6, 0, 1)], &[431]),
            true,
            &s.collections[0],
            &s.collections[1],
        )
        .expect("folds");
        assert_eq!(state.orders.len(), 2);
        // A replacing frame drops it.
        fold_payload(
            &mut state,
            &payload(0, 1, 1, 1, 0, &[area_defence_row(12, 6, 0, 1)], &[431]),
            false,
            &s.collections[0],
            &s.collections[1],
        )
        .expect("folds");
        assert_eq!(state.orders.len(), 1);
    }

    #[test]
    fn a_delete_row_may_declare_a_target_range_and_it_is_discarded() {
        // The fold applies no emptiness requirement to a delete row
        // (`world_fold.rs::fold_army_orders`), so this must fold, not fail.
        let s = schema();
        let mut state = State::new();
        let deleted_with_range = order_row(
            11,
            5,
            FLAG_DELETED | FLAG_TARGET_STATES,
            0,
            5,
            0,
            1,
            0,
            TARGET_STATUS_NONE,
            OWNER_KIND_ARMY,
            0,
        );
        fold_payload(
            &mut state,
            &payload(0, 1, 1, 1, 1, &[deleted_with_range], &[900]),
            false,
            &s.collections[0],
            &s.collections[1],
        )
        .expect("a delete row with a range still folds");
        assert!(state.orders.is_empty());
    }

    #[test]
    fn the_folds_frame_level_rejections_are_reproduced() {
        let s = schema();
        let (o, t) = (&s.collections[0], &s.collections[1]);
        let mut state = State::new();

        // Duplicate identity in one frame.
        let dup = payload(
            0,
            2,
            2,
            2,
            0,
            &[area_defence_row(11, 5, 0, 1), area_defence_row(11, 5, 1, 1)],
            &[900, 431],
        );
        assert!(fold_payload(&mut state, &dup, false, o, t).is_err());

        // Overlapping ranges.
        let overlap = payload(
            0,
            2,
            2,
            2,
            0,
            &[area_defence_row(11, 5, 0, 2), area_defence_row(12, 6, 1, 1)],
            &[900, 431],
        );
        assert!(fold_payload(&mut state, &overlap, false, o, t).is_err());

        // An unowned target child (totality).
        let unowned = payload(0, 1, 1, 1, 0, &[area_defence_row(11, 5, 0, 1)], &[900, 431]);
        assert!(fold_payload(&mut state, &unowned, false, o, t).is_err());

        // A duplicate state id within one row.
        let dupstate = payload(0, 1, 1, 1, 0, &[area_defence_row(11, 5, 0, 2)], &[900, 900]);
        assert!(fold_payload(&mut state, &dupstate, false, o, t).is_err());

        // A range past the end of the target array.
        let past = payload(0, 1, 1, 1, 0, &[area_defence_row(11, 5, 0, 3)], &[900, 431]);
        assert!(fold_payload(&mut state, &past, false, o, t).is_err());

        // And every one of those left the accumulator untouched.
        assert!(state.orders.is_empty());
    }

    #[test]
    fn the_header_bounds_are_the_folds_three() {
        let rows = [area_defence_row(11, 5, 0, 1)];
        let good = payload(0, 1, 1, 1, 0, &rows, &[900]);
        assert!(split_payload(&good).is_ok());

        let mut changed_too_big = good.clone();
        changed_too_big[12..16].copy_from_slice(&2u32.to_le_bytes());
        assert!(split_payload(&changed_too_big).is_err());

        let mut deleted_too_big = good.clone();
        deleted_too_big[36..40].copy_from_slice(&2u32.to_le_bytes());
        assert!(split_payload(&deleted_too_big).is_err());

        let mut observed_too_big = good.clone();
        observed_too_big[24..28].copy_from_slice(&2u32.to_le_bytes());
        assert!(split_payload(&observed_too_big).is_err());

        // `changed_rows < row_count` is legal — the fold requires `<=`, never
        // equality, in any mode.
        let mut fewer_changed = good.clone();
        fewer_changed[12..16].copy_from_slice(&0u32.to_le_bytes());
        assert!(split_payload(&fewer_changed).is_ok());

        // A trailing byte is a rejected frame.
        let mut trailing = good.clone();
        trailing.push(0);
        assert!(split_payload(&trailing).is_err());
    }

    #[test]
    fn an_overflowing_count_is_refused_not_multiplied() {
        let mut p = vec![0u8; HEADER_LEN];
        p[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(split_payload(&p).is_err());
        let mut q = vec![0u8; HEADER_LEN];
        q[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(split_payload(&q).is_err());
    }

    /// Total on every input: no payload of any length or content panics.
    #[test]
    fn the_boundary_is_total() {
        let seeds: Vec<Vec<u8>> = vec![
            vec![],
            vec![0u8; 1],
            vec![0xffu8; HEADER_LEN],
            vec![0xffu8; HEADER_LEN + ROW_LEN],
            vec![0u8; HEADER_LEN + ROW_LEN + TARGET_ROW_LEN],
            vec![0x55u8; 200],
        ];
        let s = schema();
        for p in &seeds {
            let _ = peek_header(p);
            let _ = split_payload(p);
            let mut state = State::new();
            let _ = fold_payload(&mut state, p, false, &s.collections[0], &s.collections[1]);
            let _ = fold_payload(&mut state, p, true, &s.collections[0], &s.collections[1]);
        }
        for r in [vec![], vec![0u8; ROW_LEN], vec![0xffu8; ROW_LEN]] {
            let _ = parse_row(&r);
            let _ = key_from_row(&r);
            let _ = record_from_row(&orders(), &r, 0);
        }
        for r in [vec![], vec![0u8; TARGET_ROW_LEN], vec![0xffu8; TARGET_ROW_LEN]] {
            let _ = parse_target_row(&r);
            let _ = record_from_target_row(&targets(), &r, 0, 0);
        }
    }
}
