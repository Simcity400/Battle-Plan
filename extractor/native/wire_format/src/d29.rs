//! d29 `ships_live` — one row per live ship instance.
//!
//! Derived from `world_fold.rs:22286`-`:22418`
//! (`fold_ships`), its payload constants at `:973`-`:974` and `:1039`-`:1045`, the
// @world-fold-authority domain=29 functions=fold_ships lines=22286-22418 sha256=e8bba8e1d6cc0e37c5bab4d911f65646bb03ceb0e371030a1bf14ca42c7717f1 closure=3bdd4c5100d8d0c58baf13b81ef9d8ae3951f9a0065c566f4ad9ad1c42869486
//! render path that consumes the folded state (`:8620`-`:8634`, `:8539`-`:8573`),
//! and the collector read that produces the presence flags
//! (`extractor/native/collector/src/attach_log.rs:47977`-`:48031`,
//! `:47449`-`:47473`, `:47550`-`:47568`).
//! `domain-29::Overview` in `extractor/documents/technical_evidence.json`
//! describes the traversal that finds ships, not the row layout, so the fold
//! and collector read are the field-level basis.
//!
//! **Payload grammar** (`world_fold.rs::fold_ships`): a 40-byte header —
//! `elapsed_wall_ms` u64 at `+0`, `row_count` u32 at `+8`, `changed_rows` u32 at
//! `+12`, `status_flags` u32 at `+16`, `observed_ship_rows` u32 at `+20`,
//! `observed_task_force_rows` u32 at `+24`, `deleted_rows` u32 at `+28`,
//! `scale_raw` u32 at `+32`, `payload_version` u32 at `+36` (must be
//! [`PAYLOAD_VERSION`]; v1 wrote a must-be-zero `reserved0` there, so an old
//! payload reads `0` and is refused by version) — followed by `row_count`
//! **variable-length** rows.
//!
//! **Rows are variable-length and are therefore walked, never index-addressed.**
//! Each row is 40 fixed bytes (`SHIP_FIXED_ROW_LEN`, `world_fold.rs::SHIP_FIXED_ROW_LEN`) followed by a
//! u16-length-prefixed UTF-8 `definition` string (`read_string`, `world_fold.rs::read_string`),
//! whose length is `0` exactly when `FLAG_DEFINITION` is clear (`:14613`-`:14618`
//! reject both asymmetries). [`split_payload`] walks them the same way the fold
//! does and enforces the fold's trailing-byte rule (`:14621`).
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+0`  | u32 | `instance_id`      | raw       | none (key) |
//! | `+4`  | u32 | `flags`            | —         | — |
//! | `+8`  | u32 | `task_force_id`    | raw       | `1<<1` |
//! | `+12` | u32 | `category_enum`    | raw       | none |
//! | `+16` | u32 | `strength_raw`     | /100000   | none |
//! | `+20` | u32 | `organisation_raw` | /100000   | none |
//! | `+24` | u32 | `experience_raw`   | /100000   | none |
//! | `+28` | u32 | `sunk_convoys`     | raw       | none |
//! | `+32` | u32 | `owner_tag_db_id`  | raw       | `1<<3` |
//! | `+36` | u32 | `task_force_id_space` | raw    | `1<<1` — the SAME gate as `task_force_id`, in both directions (v1 had a must-be-zero reserved here) |
//! | `+40` | u16+n | `definition`     | symbol    | `1<<2` |
//!
//! **The scale is a frame invariant, not an assumption.** The header's
//! `scale_raw` is asserted equal to `FIXED_SCALE` (`ensure_fixed_scale`,
//! `:14578`/`:30245`), so a frame declaring another denominator is rejected
//! rather than read at its own scale.
//!
//! **The `/FIXED_SCALE` division belongs at this boundary.** `fold_ships` stores
//! `strength_raw`/`organisation_raw`/`experience_raw` as the raw u32 words
//! (`world_fold.rs::fold_ships`) and the division happens one layer later, in `ship_json`
//! (`world_fold.rs::ship_json`), which is the **only** consumer of the folded d29
//! accumulator's numeric channels — the naval-combat lane reads raw strength from
//! the *collector's* row (`attach_log.rs:40229`), never from this accumulator.
//! So the value a d29 consumer sees is the divided one, and that is what the
//! record carries. `Quant::Fixed(5)` then costs nothing for the raw-integer
//! reading either: the source is an int32 over a 100_000 denominator, so
//! `fixed(5)` is the source's own grid and `zigzag(scale(v, 10^5))` recovers the
//! raw word exactly (verified over every raw value in a 903-hour capture — see
//! the tests). Choosing `Unsigned(raw)` instead would have moved the fold's
//! projection out of the boundary and made every consumer re-derive a
//! denominator the frame already declares.
//!
//! **Four states.** FOUR fields are gated -- `task_force_id` and its
//! `task_force_id_space` share one gate, which is the point of the pair -- and
//! for all four a clear gate is a **failed read**, never a relation pointing at
//! nothing and never an inapplicable concept, so all four are `unknown` and no
//! field in this domain is `optional` or `nullable`:
//!
//! - `task_force_id` (`FLAG_TASK_FORCE`). The collector reaches a ship only by
//!   walking a task force's ship vector and sets the field unconditionally on
//!   every live row it builds (`attach_log.rs:48021`); the only `None` it
//!   produces is on a tombstone (`:48077`), which carries no fields at all. A
//!   live row with the gate clear is therefore not "this ship has no task force"
//!   — it is a row the current read path cannot construct, i.e. uncaptured.
//! - `owner_tag_db_id` (`FLAG_OWNER`). The owner is the ship's task force's
//!   fleet's country (`attach_log.rs:47787`, `:47646`), read as
//!   `snapshot_u32_or_read(...).and_then(valid_division_owner_tag_db_id).or_else(
//!   || valid_division_owner_tag_db_id(index))` (`:47567`) — `None` only when both
//!   the object's tag word and its country-table index fail the validity filter,
//!   which is an unreadable/out-of-range read, not an unowned ship. Every ship
//!   belongs to a fleet and every fleet to a country, so `null` would assert
//!   something false.
//! - `definition` (`FLAG_DEFINITION`). `read_msvc_string_with_snapshot`
//!   (`attach_log.rs:86539`) returns `None` on unreadable memory, a size past
//!   `MAX_SHIP_DEFINITION_LEN`, an inconsistent capacity, a missing NUL, or a
//!   non-printable byte, and the caller additionally discards a value that looks
//!   like a formatted pointer (`:48017`). Every one of those is record-state contract's "applicable,
//!   not captured".
//!
//! Corroborated empirically: across the 53,970 live rows of
//! `runs/batch1/deepwar_t1` (903 hours, D04 deep-war) the row flag word is
//! `0b1110` on **every** live row and `0b0001` on every one of the 22 tombstones
//! — no gate is ever clear on a live row, which is what a read-failure path looks
//! like in a healthy capture.
//!
//! **The fold's own flag asymmetry is mirrored in behaviour, not in semantics.**
//! `fold_ships` folds `task_force_id` as an `Option` (`:14637`) but folds a clear
//! owner gate to the sentinel `owner_tag_db_id = 0` (`:14644`-`:14648`), and
//! `ship_owner_key` (`:8606`) then reads that `0` as *unowned* and drops the ship
//! from its per-country counts. `0` is not a legal tag
//! (`valid_division_owner_tag_db_id` requires `> 0`, `attach_log.rs:16689`), so
//! the sentinel is at least unambiguous — but collapsing "unreadable" into
//! "unowned" is the value-vs-nothing decision record-state contract explicitly does not delegate to
//! the fold. This schema keeps them apart. Recorded as an open question.
//!
//! **`definition` is a content-identity contract symbol, and it is kept.** content-identity contract names "equipment names"
//! as content-defined and requires that "the stream carries manifest ids as
//! varints, never text" — that is why schema-kind contract has no string kind: text is a
//! `ContentId`. The field is a real reference into mod-set content (the
//! recorder already packs a `ship_definitions_static` catalog whose `ship_hulls`
//! keys are exactly this vocabulary,
//! `recorder_host/src/main.rs:7621`), it is the join key `naval_missions_status`
//! groups task-force composition by (`world_fold.rs::naval_missions_status_ship_summary`), and it is
//! read from the ship's own memory independently of `category_enum`. On this
//! capture the two happen to be bijective (7 definitions, 7 categories, matching
//! counts) — but nothing enforces that: `ship_category_label`
//! (`attach_log.rs:21649`) derives a label from an **engine enum** while
//! `definition` is a heap string a mod is free to name anything, so deriving one
//! from the other would be a vanilla-only assumption and exactly the "silently
//! drop captured data" failure. See [`Symbols`] for how the id is minted and what
//! codec-measurement contract will replace.
//!
//! **No derived field (derived-field contract).** Nothing here changes deterministically with the
//! clock; `sunk_convoys` accumulates on events, not on ticks. The mapper still
//! takes `frame_tick` so every domain's boundary has one shape.

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Quant, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 29;
pub const SHIPS: u32 = 1;

/// Field ids are stable and append-only; schema-identity contract forbids reuse for a different field.
pub mod field {
    pub const INSTANCE_ID: u32 = 1;
    pub const TASK_FORCE_ID: u32 = 2;
    pub const DEFINITION: u32 = 3;
    pub const CATEGORY: u32 = 4;
    pub const STRENGTH: u32 = 5;
    pub const ORGANISATION: u32 = 6;
    pub const EXPERIENCE: u32 = 7;
    pub const SUNK_CONVOYS: u32 = 8;
    pub const OWNER_TAG_DB_ID: u32 = 9;
    /// The clausewitz id SPACE of [`TASK_FORCE_ID`], and the other half of the
    /// join into d28's task-force rows (owner ruling 2026-08-27). Appended,
    /// never reusing an id.
    pub const TASK_FORCE_ID_SPACE: u32 = 10;
}

/// The only d29 payload version this schema reads.
/// `world_fold.rs::SHIP_PAYLOAD_VERSION`. v1 wrote a hard zero into the header
/// word this occupies (`+36`, and the fold refused a non-zero there), so a v1
/// payload reads `0` and is refused by name.
pub const PAYLOAD_VERSION: u32 = 2;

// --- Payload grammar, from `world_fold.rs:973`-`:974`, `:1039`-`:1045`. -------

/// `SHIP_HEADER_LEN`, `world_fold.rs::SHIP_HEADER_LEN`.
pub const HEADER_LEN: usize = 40;
/// `SHIP_FIXED_ROW_LEN`, `world_fold.rs::SHIP_FIXED_ROW_LEN`. The **fixed** part only: a row is
/// this plus a u16-length-prefixed `definition` string.
pub const FIXED_ROW_LEN: usize = 40;
/// The u16 length prefix that follows the fixed part (`read_string`, `:30256`).
pub const DEFINITION_LEN_PREFIX: usize = 2;

/// `ROW_FLAG_DELETED_LOW_BIT`, `world_fold.rs::ROW_FLAG_DELETED_LOW_BIT`. A set bit removes the key
/// (`:14629`-`:14632`) — a wire tombstone, not a record.
pub const FLAG_DELETED: u32 = 1 << 0;
/// `SHIP_ROW_FLAG_TASK_FORCE`, `world_fold.rs::SHIP_ROW_FLAG_TASK_FORCE`.
pub const FLAG_TASK_FORCE: u32 = 1 << 1;
/// `SHIP_ROW_FLAG_DEFINITION`, `world_fold.rs::SHIP_ROW_FLAG_DEFINITION`.
pub const FLAG_DEFINITION: u32 = 1 << 2;
/// `SHIP_ROW_FLAG_OWNER`, `world_fold.rs::SHIP_ROW_FLAG_OWNER`.
pub const FLAG_OWNER: u32 = 1 << 3;
/// `SHIP_KNOWN_FLAGS`, `world_fold.rs::SHIP_KNOWN_FLAGS`. Any other bit is a hard error
/// (`:14593`) — an unrecognised flag word means the row is not this grammar.
pub const KNOWN_FLAGS: u32 = FLAG_DELETED | FLAG_TASK_FORCE | FLAG_DEFINITION | FLAG_OWNER;

/// `FIXED_SCALE`, `world_fold.rs::FIXED_SCALE`. The header's `scale_raw` is asserted equal
/// to it (`:14578`).
pub const FIXED_SCALE: f64 = 100_000.0;
/// The same constant as the integer word the header carries, so the check is
/// exact rather than a float comparison.
pub const FIXED_SCALE_RAW: u32 = 100_000;
/// `100_000 == 10^5`, so `Quant::Fixed(5)` is the source's own grid.
pub const FIXED_SCALE_DIGITS: u8 = 5;

// --- content-identity contract symbols --------------------------------------------------------------

/// The definition vocabulary, as a symbol table.
///
/// content-identity contract: symbol ids are **file-global**, assigned from the content set, and
/// "in-stream interning is forbidden — it must be fully read before any seek".
/// So the table is an **input** to this boundary, built once from the whole
/// content set before any record is mapped; it is never grown by the mapper.
///
/// The production assignment now lives in the neutral content-manifest crate's
/// `symbol_manifest` module; this domain-local mapper has not yet been
/// retrofitted with that file-global resolver.
/// [`Symbols::from_vocabulary`] therefore remains a measurement stand-in that is
/// deterministic, order-independent and derived only from the
/// content set — the 0-based index into the sorted, deduplicated symbol list —
/// and it is labelled as a stand-in, not as an answer. Two properties make that
/// safe for this deliverable:
///
/// - Nothing downstream of the id is computed from its value: `ContentId` is a
///   plain varint (schema-kind contract) and the key of this domain is not a `ContentId`, so no
///   ordering or delta layout depends on the choice.
/// - The measurement is invariant to it. Any assignment over a vocabulary of
///   fewer than 128 symbols yields a one-byte varint for every row, so the byte
///   sizes reported for this domain are the same under the production rule.
///
/// When this mapper is integrated with content-identity contract, this type is replaced by the
/// container's manifest resolver and every call site changes with it — the
/// retrofit the recipe predicts for every `ContentId`-emitting mapper.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Symbols {
    /// Sorted and deduplicated; the id of a symbol is its index.
    vocabulary: Vec<String>,
}

impl Symbols {
    /// A table with no symbols. A row carrying a definition is rejected against
    /// it — fail-closed, so an unresolvable symbol is a diagnosed error and never
    /// a record with the definition quietly dropped.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build from every definition the content set can produce. The input is
    /// sorted and deduplicated here, so callers may pass it in any order and two
    /// callers over the same content set always mint the same ids.
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

    pub fn len(&self) -> usize {
        self.vocabulary.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vocabulary.is_empty()
    }

    /// The symbol id of `symbol`, or `None` if the content set does not contain
    /// it. `None` is an error at the boundary, never an `Unknown` value: the
    /// string WAS captured, so claiming it was not would be a false statement
    /// about the capture.
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
}

pub fn schema() -> Schema {
    use field as f;
    let fields = vec![
        // Key: the ship's `instance_id` (`world_fold.rs::fold_ships`, keyed at
        // `world_fold.rs::fold_ships`). `Unsigned`, not `ContentId`: it is the high half of
        // the engine's runtime object handle (`attach_log.rs:47994`), minted
        // during play. No content manifest resolves it and it names a different
        // ship next campaign — d20's `div_id`/`army_id` case, not its `owner`
        // case.
        Field::new(f::INSTANCE_ID, Kind::Unsigned).key(0),
        // A runtime instance identity too (the task force's engine id), and
        // genuinely mutable state: 34 ships changed task force during the
        // measured capture.
        Field::new(f::TASK_FORCE_ID, Kind::Unsigned).unknown_capable(),
        // The other half of that reference (owner ruling 2026-08-27): which
        // clausewitz registry minted the task force's number. Two do -- the
        // fleet-id minter (61) and the `CShip` registry (51) -- and their
        // numbers collide, so `task_force_id` alone names two different task
        // forces. `Unsigned` for the same reason d28's `id_space` is: the set of
        // so an enum cardinality would be an invented bound. Rides the same gate
        // as the id, hence the same `unknown_capable` and no `optional`.
        Field::new(f::TASK_FORCE_ID_SPACE, Kind::Unsigned).unknown_capable(),
        // content-identity contract: an equipment/hull name is content-defined, so it travels as a
        // symbol id. See `Symbols` for this mapper's pending content-identity contract retrofit.
        Field::new(f::DEFINITION, Kind::ContentId).unknown_capable(),
        // The engine's ship-category discriminator. `Unsigned`, not `Enum`:
        // `fold_ships` applies **no** bound to it (`:14604` reads the word and
        // stores it verbatim at `:14639`), so an `Enum(n)` would reject a frame
        // the recorder folds — schema-kind contract makes a code `>= n` corrupt. It is also not
        // `ContentId`: like d20's `div_type` it is an engine constant that is
        // identical under every mod set, and stability is not what makes an id
        // content. See the open questions for what would justify an `Enum`.
        Field::new(f::CATEGORY, Kind::Unsigned),
        // The three fixed-point channels. `Continuous` because the fold's
        // consumer divides them (`:8629`-`:8631`), `Fixed(5)` because the source
        // is an int32 over a 100_000 denominator and `10^5` is that grid exactly
        // — lossless, and it recovers the raw word a raw-reading consumer wants
        // without spending `Exact`'s 8 bytes. This is d24's decimal-fixed-point
        // capture's strength values and 82% of its experience values.
        Field::new(f::STRENGTH, Kind::Continuous).quant(Quant::Fixed(FIXED_SCALE_DIGITS)),
        Field::new(f::ORGANISATION, Kind::Continuous).quant(Quant::Fixed(FIXED_SCALE_DIGITS)),
        Field::new(f::EXPERIENCE, Kind::Continuous).quant(Quant::Fixed(FIXED_SCALE_DIGITS)),
        // A lifetime per-ship counter. schema-kind contract's own worked example says monotone is a
        // hint and never a validity constraint, naming this very quantity.
        Field::new(f::SUNK_CONVOYS, Kind::Monotone),
        // The owning country tag — content-defined (content-identity contract), the same identity d63
        // keys by and d20 classes as `ContentId`. Not a key here, so it is the
        // lighter of the two `ContentId` retrofit classes.
        Field::new(f::OWNER_TAG_DB_ID, Kind::ContentId).unknown_capable(),
    ];

    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![Collection {
            collection_id: SHIPS,
            // `replace_or_sparse_after_first` (`world_fold.rs:14624`, `:2947`):
            // the first record clears and repopulates, every later one upserts,
            // and a `FLAG_DELETED` row removes. That is a keyed set with a delta
            // grammar, which is the only shape collection-shape contract implements anyway.
            shape: Shape::KeyedSet,
            fields,
        }],
        event_part: None,
    }
}

// --- The boundary mapping ----------------------------------------------------

/// The 40-byte payload header (`world_fold.rs::fold_ships`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub row_count: u32,
    pub changed_rows: u32,
    pub status_flags: u32,
    pub observed_ship_rows: u32,
    /// Read by the collector's own gate but **not** by `fold_ships`, which never
    /// touches `+24`. Carried so a caller can see it without re-reading the
    /// header; no invariant here uses it, because the fold applies none.
    pub observed_task_force_rows: u32,
    pub deleted_rows: u32,
    pub scale_raw: u32,
    /// `+36`, v1's must-be-zero `reserved0`. Always [`PAYLOAD_VERSION`] on a
    /// payload this schema accepts.
    pub payload_version: u32,
}

/// One parsed row: the fixed words, the flags, and the definition text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShipRow {
    pub instance_id: u32,
    pub flags: u32,
    pub task_force_id: u32,
    /// `+36`, v1's must-be-zero reserved word. The id space of
    /// `task_force_id`: non-zero exactly when `FLAG_TASK_FORCE` is set, because
    /// a d28 task-force row always names a registry and a reference that does
    /// not name one cannot pick between two same-numbered task forces.
    pub task_force_id_space: u32,
    pub category_enum: u32,
    pub strength_raw: u32,
    pub organisation_raw: u32,
    pub experience_raw: u32,
    pub sunk_convoys: u32,
    pub owner_tag_db_id: u32,
    /// `Some` exactly when `FLAG_DEFINITION` is set (`world_fold.rs::fold_ships`).
    pub definition: Option<String>,
}

impl ShipRow {
    /// `world_fold.rs::fold_ships`: a set low bit removes the key and nothing else in
    /// the row is read.
    pub fn is_deleted(&self) -> bool {
        self.flags & FLAG_DELETED != 0
    }
}

/// Read and fully validate the 40-byte payload header, **without walking the
/// rows** (`world_fold.rs::fold_ships`).
///
/// This split exists because the fold's own call order demands it. `fold_ships`
/// applies every header check *before* it consults the status word
/// (`handle_compact_failure` at `:14582`, `reject_delta_before_baseline` at
/// `:14585`) and walks rows only *after* both
/// (`:14589`). A caller that has to decide whether a record folds at all must
/// therefore be able to validate the header without touching a row — otherwise
/// it would apply row-level checks to a failure-status record the recorder never
/// reads, and refuse a stream the recorder accepts.
///
/// Every header invariant is here, none is deferred: unlike d26, d29 has no
/// frame mode, so no header rule depends on anything but the header.
pub fn peek_header(payload: &[u8]) -> core::result::Result<PayloadHeader, String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "ships payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    let header = PayloadHeader {
        elapsed_wall_ms: u64::from_le_bytes(payload[0..8].try_into().expect("8 bytes")),
        row_count: u32_at(payload, 8),
        changed_rows: u32_at(payload, 12),
        status_flags: u32_at(payload, 16),
        observed_ship_rows: u32_at(payload, 20),
        observed_task_force_rows: u32_at(payload, 24),
        deleted_rows: u32_at(payload, 28),
        scale_raw: u32_at(payload, 32),
        payload_version: u32_at(payload, 36),
    };
    // `:14557`-`:14570`, `ensure_count_at_most`.
    if header.changed_rows > header.row_count {
        return Err(format!(
            "ships changed_rows {} > row_count {}",
            header.changed_rows, header.row_count
        ));
    }
    if header.deleted_rows > header.row_count {
        return Err(format!(
            "ships deleted_rows {} > row_count {}",
            header.deleted_rows, header.row_count
        ));
    }
    // `:14571`-`:14577`, `ensure_row_count_with_deletes`. Saturating on the right
    // exactly as the fold's helper does (`:30234`), so a header claiming huge
    // counts cannot wrap into a passing comparison.
    let allowed = (header.observed_ship_rows as u64).saturating_add(header.deleted_rows as u64);
    if header.row_count as u64 > allowed {
        return Err(format!(
            "ships row_count {} > observed_ship_rows {} + deleted_rows {}",
            header.row_count, header.observed_ship_rows, header.deleted_rows
        ));
    }
    // `:14578`, `ensure_fixed_scale`. The denominator is declared per frame, so
    // this is what makes `FIXED_SCALE` a fact about the payload rather than an
    // assumption about the domain.
    if header.scale_raw != FIXED_SCALE_RAW {
        return Err(format!(
            "ships scale_raw {} != {FIXED_SCALE_RAW}",
            header.scale_raw
        ));
    }
    // `+36` is the payload version since v2 (owner ruling 2026-08-27), in place
    // of v1's must-be-zero `reserved0`. A v1 payload therefore reads `0` here,
    // on every input, and is refused by name.
    if header.payload_version != PAYLOAD_VERSION {
        return Err(format!(
            "ships payload version {} is refused: only v{PAYLOAD_VERSION} is legal, and v1 (which wrote 0 here) carries no task-force id space",
            header.payload_version
        ));
    }
    Ok(header)
}

/// Split a d29 DATA payload into its header and its rows, applying the fold's
/// structural checks (`world_fold.rs::fold_ships`).
///
/// **Rows are variable-length, so this walks them** — 40 fixed bytes then a
/// u16-length-prefixed string — exactly as `fold_ships` does, and returns one
/// slice per row covering the whole row including its string. Index arithmetic
/// over a fixed stride would silently mis-frame every row after the first
/// definition. The walk ends with the fold's trailing-byte rule (`world_fold.rs::fold_ships`).
///
/// The header invariants are [`peek_header`]'s, which are the fold's own and no
/// others. d29 is **not** full-replace: `fold_ships` requires
/// `changed_rows <= row_count` and `deleted_rows <= row_count`
/// (`ensure_count_at_most`, `:14557`-`:14570`) and
/// `row_count <= observed_ship_rows + deleted_rows`
/// (`ensure_row_count_with_deletes`, `:14571`-`:14577`), never an equality —
/// asserting `changed_rows == row_count` here would reject the sparse frames this
/// domain is made of.
pub fn split_payload(payload: &[u8]) -> core::result::Result<(PayloadHeader, Vec<&[u8]>), String> {
    let header = peek_header(payload)?;

    // The walk. `row_count` is input from a file, so every step is bounds- and
    // overflow-checked; nothing here can panic or wrap on any byte string.
    let mut rows = Vec::new();
    let mut cursor = HEADER_LEN;
    for index in 0..header.row_count as usize {
        let fixed_end = cursor
            .checked_add(FIXED_ROW_LEN)
            .ok_or_else(|| format!("ships row {index}: offset overflows"))?;
        if fixed_end > payload.len() {
            return Err(format!(
                "ships row {index}: need {FIXED_ROW_LEN} fixed bytes at {cursor}, payload is {}",
                payload.len()
            ));
        }
        let len_end = fixed_end
            .checked_add(DEFINITION_LEN_PREFIX)
            .ok_or_else(|| format!("ships row {index}: offset overflows"))?;
        if len_end > payload.len() {
            return Err(format!(
                "ships row {index}: definition length prefix runs past the payload"
            ));
        }
        let definition_len = u16::from_le_bytes([payload[fixed_end], payload[fixed_end + 1]])
            as usize;
        let row_end = len_end
            .checked_add(definition_len)
            .ok_or_else(|| format!("ships row {index}: offset overflows"))?;
        if row_end > payload.len() {
            return Err(format!(
                "ships row {index}: definition of {definition_len} byte(s) runs past the payload"
            ));
        }
        rows.push(&payload[cursor..row_end]);
        cursor = row_end;
    }
    // `:14621`.
    if cursor != payload.len() {
        return Err(format!(
            "ships payload has {} trailing byte(s)",
            payload.len() - cursor
        ));
    }
    Ok((header, rows))
}

/// Parse one whole row slice, applying every per-row check `fold_ships` applies
/// and **no others**.
///
/// The fold's row rules are exactly four (`:14591`-`:14619`): the fixed part must
/// be present, an unknown flag bit is fatal, `+36` (`task_force_id_space` since
/// v2, a must-be-zero reserved in v1) must agree with `FLAG_TASK_FORCE` in both
/// directions, and the `FLAG_DEFINITION` bit and a non-empty string must agree
/// in both directions. It applies **no range check** to any value word and **no
/// zero-check** to `instance_id`. The collector and the naval gate lane do
/// (`attach_log.rs:47993`-`:48011`,
/// `lanes/naval_oob_fleets_ships_live/domain28_29_compact.rs:1300`-`:1345`), but
/// those are producer-side contracts; enforcing them here would reject frames the
/// shipping recorder folds, which is the one thing this boundary must not do.
/// Recorded as an open question.
pub fn parse_row(row: &[u8]) -> core::result::Result<ShipRow, String> {
    if row.len() < FIXED_ROW_LEN + DEFINITION_LEN_PREFIX {
        return Err(format!(
            "ships row length {} < {} (fixed part plus length prefix)",
            row.len(),
            FIXED_ROW_LEN + DEFINITION_LEN_PREFIX
        ));
    }
    let flags = u32_at(row, 4);
    // `:14593`-`:14595`.
    if flags & !KNOWN_FLAGS != 0 {
        return Err(format!(
            "ships row {} has unknown flags {flags:#x}",
            u32_at(row, 0)
        ));
    }
    // `+36` is `task_force_id_space` since v2, in place of v1's must-be-zero
    // reserved word. It rides the same presence flag as the id, in both
    // directions, so the reference can never be half present.
    let task_force_id_space = u32_at(row, 36);
    if flags & FLAG_TASK_FORCE == 0 {
        if task_force_id_space != 0 {
            return Err("ships task_force_id_space present without flag".into());
        }
    } else if task_force_id_space == 0 {
        return Err("ships task_force reference carries no id space".into());
    }
    let definition_len =
        u16::from_le_bytes([row[FIXED_ROW_LEN], row[FIXED_ROW_LEN + 1]]) as usize;
    let text_start = FIXED_ROW_LEN + DEFINITION_LEN_PREFIX;
    if text_start + definition_len != row.len() {
        return Err(format!(
            "ships row declares a {definition_len}-byte definition but carries {}",
            row.len().saturating_sub(text_start)
        ));
    }
    // `read_string`, `:30263`-`:30266`: invalid UTF-8 is a rejected frame.
    let text = std::str::from_utf8(&row[text_start..])
        .map_err(|e| format!("ships definition: invalid utf-8: {e}"))?;
    // `:14613`-`:14618`, both directions.
    let has_definition = flags & FLAG_DEFINITION != 0;
    if text.is_empty() && has_definition {
        return Err("ships definition flag with empty string".into());
    }
    if !text.is_empty() && !has_definition {
        return Err("ships definition string without flag".into());
    }
    Ok(ShipRow {
        instance_id: u32_at(row, 0),
        flags,
        task_force_id: u32_at(row, 8),
        task_force_id_space,
        category_enum: u32_at(row, 12),
        strength_raw: u32_at(row, 16),
        organisation_raw: u32_at(row, 20),
        experience_raw: u32_at(row, 24),
        sunk_convoys: u32_at(row, 28),
        owner_tag_db_id: u32_at(row, 32),
        definition: has_definition.then(|| text.to_string()),
    })
}

/// The key of one row: its `instance_id` (`world_fold.rs::fold_ships`).
///
/// Valid on a tombstone too — that is the whole point of a tombstone, and
/// `fold_ships` reads exactly this word before removing (`world_fold.rs::fold_ships`).
pub fn key_from_row(row: &[u8]) -> core::result::Result<Key, String> {
    Ok(ship_key(parse_row(row)?.instance_id))
}

/// The key for an `instance_id`, so callers never rebuild the tuple by hand.
pub fn ship_key(instance_id: u32) -> Key {
    key(&[instance_id as u64])
}

/// One compact row of the `ships` table, as the collector emits it today, onto
/// its stored Record.
///
/// `row` is the whole raw row slice — 40 fixed bytes plus the length-prefixed
/// definition — as [`split_payload`] yields it; d29 has one row form and length
/// selects nothing, but a row's length is its own and is checked against its
/// declared string length. `frame_tick` is derived-field contract's derived-field input
/// (stored = v + c * (frame_tick >> u)); d29 declares no derived field, so it is
/// taken and ignored — every domain's mapper has the same shape.
///
/// **`symbols` is the deviation from that shape, and it is deliberate.** schema-kind contract has
/// no string kind because content-identity contract makes text a symbol id, and this domain is the first
/// ported one whose source carries a symbol as *text* rather than as a `u32` a
/// mapper can pun into a `ContentId`. The recipe already records that every
/// `ContentId`-emitting mapper gains the production resolver input during its
/// content-identity contract retrofit; d29 cannot defer the abstraction, so it takes the input now. Dropping the field
/// instead would lose captured data, and minting an id inside the mapper would be
/// the in-stream interning content-identity contract forbids.
///
/// A **tombstone row is not a record** and is rejected here: a `FLAG_DELETED` row
/// removes its key (`world_fold.rs:14629`) and carries no fields. Callers reach
/// it through [`ShipRow::is_deleted`] and [`fold_payload`]. Same shape as d26's
/// delete rows.
///
/// Fail-closed throughout: a row the fold would reject is an `Err`, never a
/// coerced value, and nothing here can panic on any byte string.
pub fn record_from_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
    symbols: &Symbols,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    let parsed = parse_row(row)?;
    record_from_parsed_row(collection, &parsed, symbols)
}

/// [`record_from_row`] over an already-parsed row, so a caller that has to walk
/// the payload once does not parse every row twice.
pub fn record_from_parsed_row(
    collection: &Collection,
    parsed: &ShipRow,
    symbols: &Symbols,
) -> core::result::Result<Record, String> {
    if parsed.is_deleted() {
        return Err(format!(
            "ships instance_id {} is a delete row, not a record",
            parsed.instance_id
        ));
    }
    let has_task_force = parsed.flags & FLAG_TASK_FORCE != 0;
    let has_owner = parsed.flags & FLAG_OWNER != 0;

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            // A clear gate is a failed read, not an unlinked ship — see the
            // module docs. record-state contract calls that `unknown`.
            field::TASK_FORCE_ID => {
                if has_task_force {
                    Value::Unsigned(parsed.task_force_id as u64)
                } else {
                    Value::Unknown
                }
            }
            // The space rides the same gate as the id, because the two words
            // are one reference: `task_force_id` alone stopped resolving when
            // d28 began publishing both clausewitz registries.
            field::TASK_FORCE_ID_SPACE => {
                if has_task_force {
                    Value::Unsigned(parsed.task_force_id_space as u64)
                } else {
                    Value::Unknown
                }
            }
            field::DEFINITION => match parsed.definition.as_deref() {
                None => Value::Unknown,
                Some(text) => match symbols.id(text) {
                    Some(id) => Value::ContentId(id),
                    // Refused, not weakened to `Unknown`: the string is right
                    // here, so "not captured" would be false. A symbol outside
                    // the content set means the caller built the table from the
                    // wrong content, and that is a diagnosis, not data.
                    None => {
                        return Err(format!(
                            "ships definition {text:?} is not in the symbol table \
                             ({} symbol(s))",
                            symbols.len()
                        ))
                    }
                },
            },
            field::CATEGORY => Value::Unsigned(parsed.category_enum as u64),
            field::STRENGTH => Value::Continuous(parsed.strength_raw as f64 / FIXED_SCALE),
            field::ORGANISATION => {
                Value::Continuous(parsed.organisation_raw as f64 / FIXED_SCALE)
            }
            field::EXPERIENCE => Value::Continuous(parsed.experience_raw as f64 / FIXED_SCALE),
            field::SUNK_CONVOYS => Value::Unsigned(parsed.sunk_convoys as u64),
            field::OWNER_TAG_DB_ID => {
                if has_owner {
                    Value::ContentId(parsed.owner_tag_db_id as u64)
                } else {
                    Value::Unknown
                }
            }
            other => {
                // A collection this mapper does not know how to fill. Filling it
                // with a placeholder would publish invented data under a field id
                // that means something else.
                return Err(format!("ships has no mapping for field id {other}"));
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// Every definition string one payload carries, for building a [`Symbols`] table
/// over a whole capture before any record is mapped (content-identity contract: fully read first).
///
/// Shares [`split_payload`]/[`parse_row`] rather than re-walking the payload, so
/// the vocabulary and the records can never disagree about what a row says.
pub fn definitions_in_payload(payload: &[u8]) -> core::result::Result<Vec<String>, String> {
    let (_, rows) = split_payload(payload)?;
    let mut out = Vec::new();
    for row in rows {
        if let Some(definition) = parse_row(row)?.definition {
            out.push(definition);
        }
    }
    Ok(out)
}

// --- The domain fold ---------------------------------------------------------

/// The accumulated ship set.
///
/// d29 is incremental — `replace_or_sparse_after_first` (`world_fold.rs:14624`,
/// `:2947`) clears only on the first record and upserts thereafter — so a single
/// record is **not** a state. This lives beside the mapper for the same reason
/// [`split_payload`] does: the measurement harness, the tests and any future
/// caller must share one implementation of the accumulation rules, because a
/// re-derived fold drifts from the recorder without anything failing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    ships: KeyedState,
}

impl State {
    pub fn new() -> Self {
        Self::default()
    }

    /// The single collection, in [`schema`] order — the shape `encode_baseline`,
    /// `encode_delta` and `decode_delta` take.
    pub fn as_slice(&self) -> &[KeyedState] {
        std::slice::from_ref(&self.ships)
    }

    pub fn ships(&self) -> &KeyedState {
        &self.ships
    }

    pub fn len(&self) -> usize {
        self.ships.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ships.is_empty()
    }
}

/// Fold one d29 DATA payload into `acc`, exactly as `fold_ships` folds it.
///
/// `sparse` is the result of `replace_or_sparse_after_first(29, status_flags)`
/// (`world_fold.rs::fold_ships`, defined at `:2947`): **false only for the first
/// non-delta record of the stream**, true for every record after it and for any
/// record carrying `COMPACT_STATUS_DELTA_DATA`. The caller supplies it rather
/// than this function reading the bit, because that constant belongs to
/// `compact_contract` and `wire_format`'s library half deliberately does not
/// depend on that crate — copying the literal here is exactly the drift the
/// shared-contract rule forbids. The caller also owns the `delta_baselines_seen`
/// side effect that makes the decision stateful (`world_fold.rs::replace_or_sparse_after_first`).
///
/// Two payloads the fold returns from **without folding** are likewise the
/// caller's to exclude before calling: a failure status
/// (`handle_compact_failure`, `:14582`) and a delta before any baseline
/// (`reject_delta_before_baseline`, `:14585`). Both are checked *after* the
/// header validation above and *before* the row walk, which is why this function
/// still validates the header on every call — a malformed header is a rejected
/// frame whatever the status word says.
///
/// **Atomic.** Every row is walked and checked before anything is written, so a
/// payload this function rejects leaves `acc` exactly as it found it.
///
/// **Rows apply in payload order, last one winning.** `fold_ships` has no
/// duplicate rejection: a second row for one `instance_id` overwrites the first
/// in its map (`world_fold.rs::fold_ships`), and a delete followed by an insert of the same id in
/// one frame resurrects it. Refusing such a frame would refuse one the shipping
/// recorder folds.
pub fn fold_payload(
    schema: &Schema,
    acc: &mut State,
    payload: &[u8],
    sparse: bool,
    frame_tick: u64,
    symbols: &Symbols,
) -> core::result::Result<(), String> {
    if schema.collections.len() != 1 {
        return Err(format!(
            "ships schema has {} collections, expected 1",
            schema.collections.len()
        ));
    }
    let collection = &schema.collections[0];
    let (_, rows) = split_payload(payload)?;

    // ---- Decode and check everything first. ----
    enum Applied {
        Upsert(Key, Record),
        Remove(Key),
    }
    let mut applied = Vec::with_capacity(rows.len());
    for row in rows {
        let parsed = parse_row(row)?;
        let key = ship_key(parsed.instance_id);
        if parsed.is_deleted() {
            applied.push(Applied::Remove(key));
        } else {
            let record = record_from_parsed_row(collection, &parsed, symbols)?;
            let _ = frame_tick;
            applied.push(Applied::Upsert(key, record));
        }
    }

    // ---- Nothing below can fail, so `acc` is never left half-written. ----
    // `:14625`-`:14627`: only a non-sparse record clears.
    if !sparse {
        acc.ships.clear();
    }
    for action in applied {
        match action {
            Applied::Upsert(key, record) => {
                acc.ships.insert(key, record);
            }
            Applied::Remove(key) => {
                acc.ships.remove(&key);
            }
        }
    }
    Ok(())
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

    fn collection() -> Collection {
        schema().collections[0].clone()
    }

    /// The vocabulary the measured capture carries, verbatim.
    fn symbols() -> Symbols {
        Symbols::from_vocabulary([
            "battle_cruiser",
            "battleship",
            "carrier",
            "destroyer",
            "heavy_cruiser",
            "light_cruiser",
            "submarine",
        ])
    }

    fn index_of(c: &Collection, id: u32) -> usize {
        c.value_fields()
            .position(|f| f.field_id == id)
            .expect("field is in the schema")
    }

    /// A raw row, built the way the collector packs one
    /// (`attach_log.rs:70138`-`:70150`).
    #[allow(clippy::too_many_arguments)]
    /// The clausewitz registry `CreateTaskForce` mints from. Spelled out here
    /// so a test row can name a legal space without importing the collector's
    /// constant.
    const OOB_TYPE_FLEET_ID_SPACE_FOR_TESTS: u32 = 61;
    /// The `CShip` registry, the second space a task force can be minted from.
    const CSHIP_ID_SPACE_FOR_TESTS: u32 = 51;

    /// The ordinary row builder. `task_force_id_space` is not an argument
    /// because the grammar leaves it no freedom: it is non-zero exactly when
    /// `FLAG_TASK_FORCE` is set, so every existing call site keeps its arity
    /// and its meaning. The tests that care WHICH registry a row names use
    /// [`raw_row_in_space`].
    #[allow(clippy::too_many_arguments)]
    fn raw_row(
        instance_id: u32,
        flags: u32,
        task_force_id: u32,
        category_enum: u32,
        strength_raw: u32,
        organisation_raw: u32,
        experience_raw: u32,
        sunk_convoys: u32,
        owner_tag_db_id: u32,
        definition: &str,
    ) -> Vec<u8> {
        let space = if flags & FLAG_TASK_FORCE != 0 {
            OOB_TYPE_FLEET_ID_SPACE_FOR_TESTS
        } else {
            0
        };
        raw_row_in_space(
            instance_id,
            flags,
            task_force_id,
            space,
            category_enum,
            strength_raw,
            organisation_raw,
            experience_raw,
            sunk_convoys,
            owner_tag_db_id,
            definition,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn raw_row_in_space(
        instance_id: u32,
        flags: u32,
        task_force_id: u32,
        task_force_id_space: u32,
        category_enum: u32,
        strength_raw: u32,
        organisation_raw: u32,
        experience_raw: u32,
        sunk_convoys: u32,
        owner_tag_db_id: u32,
        definition: &str,
    ) -> Vec<u8> {
        let mut row = Vec::new();
        for word in [
            instance_id,
            flags,
            task_force_id,
            category_enum,
            strength_raw,
            organisation_raw,
            experience_raw,
            sunk_convoys,
            owner_tag_db_id,
            task_force_id_space,
        ] {
            row.extend_from_slice(&word.to_le_bytes());
        }
        assert_eq!(row.len(), FIXED_ROW_LEN);
        row.extend_from_slice(&(definition.len() as u16).to_le_bytes());
        row.extend_from_slice(definition.as_bytes());
        row
    }

    const LIVE_FLAGS: u32 = FLAG_TASK_FORCE | FLAG_DEFINITION | FLAG_OWNER;

    fn full_row() -> Vec<u8> {
        raw_row(
            4575,
            LIVE_FLAGS,
            17938,
            3,
            4_600_000,
            920_000,
            1_000_000,
            126,
            368,
            "destroyer",
        )
    }

    fn tombstone_row(instance_id: u32) -> Vec<u8> {
        raw_row(instance_id, FLAG_DELETED, 0, 0, 0, 0, 0, 0, 0, "")
    }

    fn payload(rows: &[Vec<u8>], status_flags: u32, deleted_rows: u32) -> Vec<u8> {
        let row_count = rows.len() as u32;
        let mut out = Vec::new();
        out.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        out.extend_from_slice(&row_count.to_le_bytes());
        out.extend_from_slice(&row_count.to_le_bytes()); // changed_rows
        out.extend_from_slice(&status_flags.to_le_bytes());
        out.extend_from_slice(&row_count.to_le_bytes()); // observed_ship_rows
        out.extend_from_slice(&1u32.to_le_bytes()); // observed_task_force_rows
        out.extend_from_slice(&deleted_rows.to_le_bytes());
        out.extend_from_slice(&FIXED_SCALE_RAW.to_le_bytes());
        out.extend_from_slice(&PAYLOAD_VERSION.to_le_bytes());
        assert_eq!(out.len(), HEADER_LEN);
        for row in rows {
            out.extend_from_slice(row);
        }
        out
    }

    // --- Step 8.1 ------------------------------------------------------------

    #[test]
    fn the_schema_is_valid() {
        schema()
            .validate(grid().log2())
            .expect("d29 schema is well-formed");
    }

    // --- Step 8.5 ------------------------------------------------------------

    #[test]
    fn it_is_a_keyed_set_keyed_by_the_ship_instance() {
        let c = collection();
        assert_eq!(c.shape, Shape::KeyedSet, "first record replaces, later upsert");
        let keys = c.key_fields();
        assert_eq!(keys.len(), 1, "the fold keys rows by instance_id alone");
        assert_eq!(keys[0].field_id, field::INSTANCE_ID);
        assert_eq!(
            keys[0].kind,
            Kind::Unsigned,
            "a runtime object handle is not content: no manifest resolves it"
        );
        assert!(keys[0].kind.legal_as_key());
        assert!(
            !keys[0].optional && !keys[0].nullable && !keys[0].unknown_capable,
            "a key is concrete in every row"
        );
        assert_eq!(c.fields.len(), 10, "1 key + 9 value fields");
    }

    #[test]
    fn exactly_four_fields_are_gated_and_all_four_are_unknown_only() {
        // The schema declares exactly the states the mapper emits. `Absent` would
        // claim the concept does not apply to a ship; `Null` would claim we know
        // there is nothing. The collector clears each of these gates only when a
        // read failed — see the module docs.
        let c = collection();
        assert_eq!(
            c.optional_count(),
            0,
            "no field is ever inapplicable, so the presence bitmap is empty"
        );
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
            vec![
                field::TASK_FORCE_ID,
                // Two words, one gate: the reference is `(id_space, id)` since
                // the owner's 2026-08-27 ruling, and half of it is not a
                // reference.
                field::TASK_FORCE_ID_SPACE,
                field::DEFINITION,
                field::OWNER_TAG_DB_ID
            ],
            "exactly the flag-gated fields"
        );
    }

    #[test]
    fn no_field_is_derived() {
        // derived-field contract derived fields are clock-linear integers; nothing in d29 changes
        // deterministically with the clock, so `frame_tick` never enters a value.
        assert!(collection().fields.iter().all(|f| f.derived.is_none()));
    }

    // --- Step 8.2 ------------------------------------------------------------

    #[test]
    fn a_full_row_roundtrips_byte_for_byte() {
        let c = collection();
        let record = record_from_row(&c, &full_row(), 0, &symbols()).expect("maps");
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
    fn a_fully_gated_off_row_roundtrips_byte_for_byte() {
        // Every gate clear at once: the row shape a total read failure would
        // produce. Note the fold accepts it — it applies no "value without flag"
        // rule — so the mapper must too.
        let c = collection();
        let row = raw_row(9, 0, 0, 0, 0, 0, 0, 0, 0, "");
        let record = record_from_row(&c, &row, 0, &symbols()).expect("maps");
        let mut bytes = Vec::new();
        record.encode(&c, &mut bytes).expect("encodes");
        let (back, pos) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(pos, bytes.len());
        let mut reencoded = Vec::new();
        back.encode(&c, &mut reencoded).expect("re-encodes");
        assert_eq!(reencoded, bytes);
        for id in [
            field::TASK_FORCE_ID,
            field::TASK_FORCE_ID_SPACE,
            field::DEFINITION,
            field::OWNER_TAG_DB_ID,
        ] {
            assert_eq!(back.values[index_of(&c, id)], Value::Unknown, "field {id}");
        }
    }

    // --- Step 8.3: states through the mapper, from raw bytes ------------------

    #[test]
    fn the_mapper_emits_values_for_a_full_row() {
        let c = collection();
        let s = symbols();
        let r = record_from_row(&c, &full_row(), 0, &s).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::TASK_FORCE_ID)],
            Value::Unsigned(17938)
        );
        assert_eq!(
            r.values[index_of(&c, field::TASK_FORCE_ID_SPACE)],
            Value::Unsigned(OOB_TYPE_FLEET_ID_SPACE_FOR_TESTS as u64)
        );
        assert_eq!(
            r.values[index_of(&c, field::DEFINITION)],
            Value::ContentId(s.id("destroyer").expect("in the vocabulary"))
        );
        assert_eq!(r.values[index_of(&c, field::CATEGORY)], Value::Unsigned(3));
        assert_eq!(
            r.values[index_of(&c, field::STRENGTH)],
            Value::Continuous(46.0)
        );
        assert_eq!(
            r.values[index_of(&c, field::ORGANISATION)],
            Value::Continuous(9.2)
        );
        assert_eq!(
            r.values[index_of(&c, field::EXPERIENCE)],
            Value::Continuous(10.0)
        );
        assert_eq!(
            r.values[index_of(&c, field::SUNK_CONVOYS)],
            Value::Unsigned(126)
        );
        assert_eq!(
            r.values[index_of(&c, field::OWNER_TAG_DB_ID)],
            Value::ContentId(368)
        );
    }

    #[test]
    fn a_real_zero_is_a_value_not_an_empty_state() {
        // Every gate SET and every value zero: a freshly built ship with no
        // organisation, no experience and no kills. record-state contract keeps zero out of the
        // three empty states. `organisation_raw == 0` and `experience_raw == 0`
        // both occur in the measured capture.
        let c = collection();
        let row = raw_row(1, LIVE_FLAGS, 0, 0, 0, 0, 0, 0, 0, "submarine");
        let r = record_from_row(&c, &row, 0, &symbols()).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::TASK_FORCE_ID)],
            Value::Unsigned(0),
            "a zero task force id is a value, not an empty state"
        );
        assert_eq!(r.values[index_of(&c, field::CATEGORY)], Value::Unsigned(0));
        for id in [field::STRENGTH, field::ORGANISATION, field::EXPERIENCE] {
            assert_eq!(
                r.values[index_of(&c, id)],
                Value::Continuous(0.0),
                "channel {id} zero is a measured value"
            );
        }
        assert_eq!(
            r.values[index_of(&c, field::SUNK_CONVOYS)],
            Value::Unsigned(0)
        );
        assert_eq!(
            r.values[index_of(&c, field::OWNER_TAG_DB_ID)],
            Value::ContentId(0),
            "the gate is set, so the word is the value the frame carries"
        );
    }

    #[test]
    fn a_clear_gate_is_unknown_field_by_field() {
        // One gate clear at a time — the whole gate space, not one sampled case.
        let c = collection();
        let s = symbols();
        // `FLAG_TASK_FORCE` gates TWO fields, because the reference is two
        // words: a ship pointing at "490" with no registry does not point
        // anywhere, so the space is unknown exactly when the id is.
        let cases: [(u32, &[u32]); 3] = [
            (
                FLAG_TASK_FORCE,
                &[field::TASK_FORCE_ID, field::TASK_FORCE_ID_SPACE],
            ),
            (FLAG_DEFINITION, &[field::DEFINITION]),
            (FLAG_OWNER, &[field::OWNER_TAG_DB_ID]),
        ];
        for (cleared, gated_ids) in cases {
            let flags = LIVE_FLAGS & !cleared;
            let definition = if flags & FLAG_DEFINITION != 0 {
                "carrier"
            } else {
                ""
            };
            let row = raw_row(3, flags, 11, 10, 100_000, 200_000, 300_000, 4, 5, definition);
            let r = record_from_row(&c, &row, 0, &s)
                .unwrap_or_else(|e| panic!("maps with flags {flags:#x}: {e}"));
            for f in c.value_fields() {
                let expect_unknown = gated_ids.contains(&f.field_id);
                assert_eq!(
                    r.values[index_of(&c, f.field_id)] == Value::Unknown,
                    expect_unknown,
                    "field {} under flags {flags:#x}",
                    f.field_id
                );
            }
            // The ungated fields still carry their real values.
            assert_eq!(r.values[index_of(&c, field::CATEGORY)], Value::Unsigned(10));
            assert_eq!(
                r.values[index_of(&c, field::SUNK_CONVOYS)],
                Value::Unsigned(4)
            );
        }
    }

    #[test]
    fn the_unknown_state_survives_a_roundtrip() {
        let c = collection();
        let row = raw_row(3, FLAG_TASK_FORCE, 77, 2, 1, 2, 3, 0, 0, "");
        let record = record_from_row(&c, &row, 0, &symbols()).expect("maps");
        let mut bytes = Vec::new();
        record.encode(&c, &mut bytes).expect("encodes");
        let (back, _) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(
            back.values[index_of(&c, field::TASK_FORCE_ID)],
            Value::Unsigned(77)
        );
        for id in [field::DEFINITION, field::OWNER_TAG_DB_ID] {
            assert_eq!(back.values[index_of(&c, id)], Value::Unknown, "field {id}");
        }
    }

    #[test]
    fn a_tombstone_row_is_not_a_record() {
        // `world_fold.rs::fold_ships`: a delete row removes its key and nothing in it
        // is read as state. It still has a valid key.
        let c = collection();
        let row = tombstone_row(42);
        assert!(record_from_row(&c, &row, 0, &symbols()).is_err());
        assert_eq!(key_from_row(&row).expect("keys"), ship_key(42));
        assert!(parse_row(&row).expect("parses").is_deleted());
    }

    // --- Step 8.4: quantization actually saves bytes --------------------------

    #[test]
    fn fixed_five_beats_exact_and_loses_nothing() {
        // Both halves matter: `Fixed(5)` must be smaller than `Exact` AND recover
        // every raw word, or the saving would be bought with corruption.
        let c = collection();
        let mut exact = c.clone();
        for f in &mut exact.fields {
            if matches!(f.kind, Kind::Continuous) {
                f.quant = Quant::Exact;
            }
        }
        let s = symbols();
        let row = full_row();
        let mut fixed_bytes = Vec::new();
        record_from_row(&c, &row, 0, &s)
            .expect("maps")
            .encode(&c, &mut fixed_bytes)
            .expect("encodes");
        let mut exact_bytes = Vec::new();
        record_from_row(&exact, &row, 0, &s)
            .expect("maps")
            .encode(&exact, &mut exact_bytes)
            .expect("encodes");
        assert!(
            fixed_bytes.len() < exact_bytes.len(),
            "fixed(5) {} vs exact {}",
            fixed_bytes.len(),
            exact_bytes.len()
        );
    }

    #[test]
    fn fixed_five_is_lossless_across_the_whole_raw_range() {
        // The source is an int32 over a 100_000 denominator, so `fixed(5)` is its
        // own grid. This is the claim the choice rests on: every raw word must
        // survive `record -> encode -> decode` unchanged, including the extremes
        // and the values with no shorter decimal form. `1` is 1e-5 exactly, the
        // finest step the source can express.
        let c = collection();
        let s = symbols();
        let raws: [u32; 12] = [
            0,
            1,
            2,
            9,
            18_405,
            99_999,
            100_000,
            1_234_567,
            9_200_000,
            10_000_000,
            46_000_000,
            u32::MAX,
        ];
        for raw in raws {
            let row = raw_row(1, LIVE_FLAGS, 1, 3, raw, raw, raw, 0, 1, "destroyer");
            let record = record_from_row(&c, &row, 0, &s).expect("maps");
            let mut bytes = Vec::new();
            record.encode(&c, &mut bytes).expect("encodes");
            let (back, _) = Record::decode(&c, &bytes, 0).expect("decodes");
            for id in [field::STRENGTH, field::ORGANISATION, field::EXPERIENCE] {
                let Value::Continuous(v) = back.values[index_of(&c, id)] else {
                    panic!("field {id} is continuous");
                };
                assert_eq!(
                    (v * FIXED_SCALE).round() as u64,
                    raw as u64,
                    "raw {raw} must survive fixed(5) on field {id}"
                );
                assert_eq!(
                    back.values[index_of(&c, id)],
                    record.values[index_of(&c, id)],
                    "raw {raw} must decode to the value it encoded on field {id}"
                );
            }
        }
    }

    // --- Step 8.6: every field's offset, with scale as the special case -------

    #[test]
    fn every_field_reads_its_own_offset_at_its_own_scale() {
        // Distinct values in every word, so a transposed pair fails — and the
        // three scaled channels are checked against known fixed-point words,
        // which is the test that catches a 100000-vs-raw or 100000-vs-32768
        // mapping error.
        let c = collection();
        let s = symbols();
        let row = raw_row(
            0x1111_1111,
            LIVE_FLAGS,
            0x2222_2222,
            7,
            150_000,  // 1.5
            50_000,   // 0.5
            1,        // 0.00001
            0x5555_5555,
            0x6666_6666,
            "light_cruiser",
        );
        let r = record_from_row(&c, &row, 0, &s).expect("maps");
        assert_eq!(key_from_row(&row).expect("keys"), ship_key(0x1111_1111));
        assert_eq!(
            r.values[index_of(&c, field::TASK_FORCE_ID)],
            Value::Unsigned(0x2222_2222)
        );
        assert_eq!(r.values[index_of(&c, field::CATEGORY)], Value::Unsigned(7));
        assert_eq!(
            r.values[index_of(&c, field::STRENGTH)],
            Value::Continuous(1.5),
            "strength_raw 150000 / 100000"
        );
        assert_eq!(
            r.values[index_of(&c, field::ORGANISATION)],
            Value::Continuous(0.5),
            "organisation_raw 50000 / 100000"
        );
        assert_eq!(
            r.values[index_of(&c, field::EXPERIENCE)],
            Value::Continuous(1.0 / FIXED_SCALE),
            "experience_raw 1 / 100000 — the finest step the source expresses"
        );
        assert_eq!(
            r.values[index_of(&c, field::SUNK_CONVOYS)],
            Value::Unsigned(0x5555_5555),
            "sunk_convoys is a raw counter, never divided"
        );
        assert_eq!(
            r.values[index_of(&c, field::OWNER_TAG_DB_ID)],
            Value::ContentId(0x6666_6666)
        );
        assert_eq!(
            r.values[index_of(&c, field::DEFINITION)],
            Value::ContentId(s.id("light_cruiser").expect("in the vocabulary"))
        );
    }

    // --- Rejection surface ---------------------------------------------------

    #[test]
    fn the_mapper_rejects_every_row_the_fold_rejects() {
        let c = collection();
        let s = symbols();
        // Short of the fixed part plus the length prefix, including empty.
        for len in [0usize, 1, FIXED_ROW_LEN, FIXED_ROW_LEN + 1] {
            let row = vec![0u8; len];
            assert!(parse_row(&row).is_err(), "length {len} must be rejected");
            assert!(record_from_row(&c, &row, 0, &s).is_err(), "length {len}");
            assert!(key_from_row(&row).is_err(), "length {len}");
        }
        // An unknown flag bit (`:14593`) — every one of them, not one sample.
        for bit in 4..32u32 {
            let row = raw_row(1, LIVE_FLAGS | (1 << bit), 1, 1, 1, 1, 1, 1, 1, "carrier");
            assert!(
                record_from_row(&c, &row, 0, &s).is_err(),
                "flag bit {bit} is not in KNOWN_FLAGS and must be rejected"
            );
        }
        // `+36` is `task_force_id_space` now, not a must-be-zero reserve, and
        // it rides the `TASK_FORCE` gate in both directions.
        let mut spaceless = full_row();
        spaceless[36..40].copy_from_slice(&0u32.to_le_bytes());
        assert!(
            record_from_row(&c, &spaceless, 0, &s).is_err(),
            "a gated task-force reference must name its registry"
        );
        let mut unlinked = raw_row_in_space(
            4575,
            LIVE_FLAGS & !FLAG_TASK_FORCE,
            0,
            OOB_TYPE_FLEET_ID_SPACE_FOR_TESTS,
            3,
            4_600_000,
            920_000,
            1_000_000,
            126,
            368,
            "destroyer",
        );
        assert!(
            record_from_row(&c, &unlinked, 0, &s).is_err(),
            "a space with the gate clear is half a reference"
        );
        unlinked[36..40].copy_from_slice(&0u32.to_le_bytes());
        assert!(
            record_from_row(&c, &unlinked, 0, &s).is_ok(),
            "and with both words absent it maps"
        );
        // The definition flag with an empty string (`:14613`).
        let row = raw_row(1, LIVE_FLAGS, 1, 1, 1, 1, 1, 1, 1, "");
        assert!(record_from_row(&c, &row, 0, &s).is_err());
        // A definition string with no flag (`:14616`).
        let row = raw_row(1, LIVE_FLAGS & !FLAG_DEFINITION, 1, 1, 1, 1, 1, 1, 1, "carrier");
        assert!(record_from_row(&c, &row, 0, &s).is_err());
        // A declared string length that does not match the bytes carried.
        let mut short = full_row();
        short.pop();
        assert!(record_from_row(&c, &short, 0, &s).is_err());
        // Invalid UTF-8 in the definition (`read_string`, `:30263`).
        let mut bad_utf8 = raw_row(1, LIVE_FLAGS, 1, 1, 1, 1, 1, 1, 1, "carrier");
        let last = bad_utf8.len() - 1;
        bad_utf8[last] = 0xFF;
        assert!(record_from_row(&c, &bad_utf8, 0, &s).is_err());
    }

    #[test]
    fn the_mapper_accepts_every_row_the_fold_accepts() {
        // The mirror of the rule above, and the more dangerous direction: the
        // collector and the naval gate lane bound `instance_id`, `category_enum`,
        // `strength_raw`, `organisation_raw`, `experience_raw` and
        // `owner_tag_db_id`, but `fold_ships` bounds NONE of them. A mapper that
        // imported those producer-side bounds would refuse frames the shipping
        // recorder folds.
        let c = collection();
        let s = symbols();
        let row = raw_row(
            0,          // the fold has no zero-instance_id check
            LIVE_FLAGS,
            u32::MAX,   // task_force_id past MAX_OOB_ID
            u32::MAX,   // category_enum past COMPACT_SHIP_MAX_CATEGORY
            u32::MAX,   // strength_raw past COMPACT_SHIP_MAX_STRENGTH_RAW
            u32::MAX,
            u32::MAX,
            u32::MAX,
            u32::MAX,   // owner_tag_db_id past COMPACT_SHIP_MAX_OWNER_TAG_DB_ID
            "destroyer",
        );
        let record = record_from_row(&c, &row, 0, &s).expect("the fold folds this row");
        assert_eq!(key_from_row(&row).expect("keys"), ship_key(0));
        let mut bytes = Vec::new();
        record.encode(&c, &mut bytes).expect("encodes");
        let (back, _) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(back, record, "extreme words still round-trip");
        // A deleted row carrying live words: the fold ignores them entirely
        // (`:14629`-`:14632`), even though the gate lane rejects the shape.
        let live_delete = raw_row(7, FLAG_DELETED, 9, 9, 9, 9, 9, 9, 9, "");
        assert!(parse_row(&live_delete).expect("parses").is_deleted());
    }

    #[test]
    fn the_mapper_refuses_a_collection_it_cannot_fill() {
        // A caller handing over another domain's collection must not get a record
        // full of placeholders under this domain's field ids.
        let mut c = collection();
        c.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_row(&c, &full_row(), 0, &symbols()).is_err());
    }

    // --- Symbols -------------------------------------------------------------

    #[test]
    fn symbol_ids_do_not_depend_on_the_order_they_were_supplied_in() {
        // content-identity contract requires the assignment to be a function of the content set. Two
        // callers over the same vocabulary must mint the same ids, or no two
        // files are comparable.
        let a = Symbols::from_vocabulary(["destroyer", "carrier", "submarine"]);
        let b = Symbols::from_vocabulary(["submarine", "destroyer", "carrier", "destroyer"]);
        assert_eq!(a, b);
        assert_eq!(a.len(), 3, "duplicates collapse");
        for symbol in ["destroyer", "carrier", "submarine"] {
            let id = a.id(symbol).expect("resolves");
            assert_eq!(b.id(symbol), Some(id));
            assert_eq!(a.symbol(id), Some(symbol), "ids round-trip to their symbol");
        }
        assert_eq!(a.id("battleship"), None, "a symbol outside the content set");
        assert_eq!(a.symbol(99), None);
    }

    #[test]
    fn an_unresolvable_definition_is_refused_not_weakened_to_unknown() {
        // The string was captured, so "not captured" would be a false statement
        // about the capture, and dropping it would be data loss. The only honest
        // outcome is a diagnosis.
        let c = collection();
        let row = raw_row(1, LIVE_FLAGS, 1, 3, 1, 1, 1, 1, 1, "modded_super_dreadnought");
        assert!(record_from_row(&c, &row, 0, &symbols()).is_err());
        assert!(record_from_row(&c, &row, 0, &Symbols::empty()).is_err());
        // And with the symbol present it maps.
        let s = Symbols::from_vocabulary(["modded_super_dreadnought"]);
        assert!(record_from_row(&c, &row, 0, &s).is_ok());
    }

    #[test]
    fn every_symbol_id_of_a_realistic_vocabulary_is_a_one_byte_varint() {
        // Why the measurement does not depend on the eventual manifest-id assignment rule:
        // any assignment over a vocabulary this size encodes in one byte, so the
        // reported sizes are the same under any of them.
        let s = symbols();
        assert!(s.len() < 128);
        for index in 0..s.len() as u64 {
            assert!(s.symbol(index).is_some());
        }
    }

    #[test]
    fn definitions_in_payload_sees_exactly_the_rows_definitions() {
        let rows = vec![
            full_row(),
            raw_row(2, LIVE_FLAGS, 1, 2, 1, 1, 1, 0, 1, "submarine"),
            raw_row(3, FLAG_TASK_FORCE | FLAG_OWNER, 1, 2, 1, 1, 1, 0, 1, ""),
            tombstone_row(4),
        ];
        let found = definitions_in_payload(&payload(&rows, 0, 1)).expect("walks");
        assert_eq!(found, vec!["destroyer".to_string(), "submarine".to_string()]);
    }

    // --- split_payload -------------------------------------------------------

    #[test]
    fn split_payload_walks_variable_length_rows() {
        // Rows of three different lengths in one payload: a fixed-stride reader
        // mis-frames every row after the first, so this is the test that pins the
        // walk.
        let rows = vec![
            raw_row(1, LIVE_FLAGS, 1, 3, 1, 1, 1, 0, 1, "carrier"),
            raw_row(2, LIVE_FLAGS, 1, 2, 1, 1, 1, 0, 1, "battle_cruiser"),
            tombstone_row(3),
            raw_row(4, LIVE_FLAGS, 1, 5, 1, 1, 1, 0, 1, "light_cruiser"),
        ];
        let p = payload(&rows, 0, 1);
        let (header, walked) = split_payload(&p).expect("well-formed payload");
        assert_eq!(header.row_count, 4);
        assert_eq!(header.scale_raw, FIXED_SCALE_RAW);
        assert_eq!(walked.len(), 4);
        for (walked_row, built) in walked.iter().zip(&rows) {
            assert_eq!(*walked_row, &built[..], "row slices are the rows");
        }
        assert_eq!(
            walked[0].len(),
            FIXED_ROW_LEN + DEFINITION_LEN_PREFIX + "carrier".len()
        );
    }

    #[test]
    fn split_payload_enforces_the_folds_structural_rules() {
        let rows = vec![full_row()];
        let good = payload(&rows, 0, 0);
        assert!(split_payload(&good).is_ok());

        // Short of the header.
        for len in 0..HEADER_LEN {
            assert!(split_payload(&good[..len]).is_err(), "len {len}");
        }
        // Every truncation past the header: a row that runs off the end must be
        // diagnosed, never read from adjacent bytes.
        for len in HEADER_LEN..good.len() {
            assert!(split_payload(&good[..len]).is_err(), "truncated at {len}");
        }
        // Trailing bytes (`:14621`).
        let mut trailing = good.clone();
        trailing.push(0);
        assert!(split_payload(&trailing).is_err());
        // changed_rows > row_count (`:14557`).
        let mut bad = good.clone();
        bad[12..16].copy_from_slice(&2u32.to_le_bytes());
        assert!(split_payload(&bad).is_err());
        // deleted_rows > row_count (`:14564`).
        let mut bad = good.clone();
        bad[28..32].copy_from_slice(&2u32.to_le_bytes());
        assert!(split_payload(&bad).is_err());
        // row_count > observed_ship_rows + deleted_rows (`:14571`).
        let mut bad = good.clone();
        bad[20..24].copy_from_slice(&0u32.to_le_bytes());
        assert!(split_payload(&bad).is_err());
        // ...and the boundary itself is accepted: one observed row plus none
        // deleted is exactly enough for one row.
        assert!(split_payload(&good).is_ok());
        // A wrong scale_raw (`:14578`) — the frame declares its own denominator.
        for wrong in [0u32, 1, 32_768, 99_999, 1_000_000] {
            let mut bad = good.clone();
            bad[32..36].copy_from_slice(&wrong.to_le_bytes());
            assert!(split_payload(&bad).is_err(), "scale_raw {wrong}");
        }
        // A nonzero header reserved word (`:14579`).
        let mut bad = good.clone();
        bad[36..40].copy_from_slice(&1u32.to_le_bytes());
        assert!(split_payload(&bad).is_err());
        // A row_count whose walk would overflow must be diagnosed, not wrapped.
        let mut huge = good.clone();
        huge[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        huge[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        huge[20..24].copy_from_slice(&u32::MAX.to_le_bytes());
        huge[28..32].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(split_payload(&huge).is_err());
        // A zero-row payload is legal and carries no rows: 14 of the measured
        // capture's 903 records are exactly this.
        let empty = payload(&[], 0, 0);
        let (header, walked) = split_payload(&empty).expect("a rowless frame is well-formed");
        assert_eq!(header.row_count, 0);
        assert!(walked.is_empty());
    }

    #[test]
    fn peek_header_validates_the_header_without_reading_a_row() {
        // The fold applies every header check before it consults the status word
        // and walks rows only after (`:14549`-`:14589`), so a caller deciding
        // whether a record folds must be able to do exactly this. A payload whose
        // header is sound but whose ROWS are not must still peek cleanly.
        let mut rows = full_row();
        rows[4..8].copy_from_slice(&(LIVE_FLAGS | (1 << 31)).to_le_bytes()); // unknown flag
        let p = payload(&[rows], 0, 0);
        let header = peek_header(&p).expect("the header is sound");
        assert_eq!(header.row_count, 1);
        assert_eq!(header.status_flags, 0);
        assert_eq!(header.observed_task_force_rows, 1);
        assert!(
            split_payload(&p).is_ok(),
            "the walk is structural; the bad flag is a per-row semantic check"
        );
        assert!(parse_row(split_payload(&p).unwrap().1[0]).is_err());

        // A truncated row body: the header still peeks, the walk does not.
        let truncated = &p[..p.len() - 3];
        assert!(peek_header(truncated).is_ok());
        assert!(split_payload(truncated).is_err());

        // And every header rule still fails at peek.
        for (offset, patch) in [
            (12usize, 2u32),  // changed_rows > row_count
            (28, 2),          // deleted_rows > row_count
            (20, 0),          // observed_ship_rows too small
            (32, 32_768),     // wrong scale
            (36, 1),          // nonzero reserved
        ] {
            let mut bad = p.clone();
            bad[offset..offset + 4].copy_from_slice(&patch.to_le_bytes());
            assert!(peek_header(&bad).is_err(), "header rule at +{offset}");
        }
    }

    #[test]
    fn split_payload_never_panics_on_arbitrary_bytes() {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..4_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let len = (state as usize) % 300;
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }
            let _ = split_payload(&bytes);
            let _ = parse_row(&bytes);
            let _ = definitions_in_payload(&bytes);
        }
        // And corruption of a genuinely well-formed payload, byte by byte.
        let good = payload(&[full_row(), tombstone_row(2)], 0, 1);
        for i in 0..good.len() {
            let mut p = good.clone();
            p[i] ^= 0xFF;
            let _ = split_payload(&p);
            let _ = definitions_in_payload(&p);
        }
    }

    // --- The fold ------------------------------------------------------------

    #[test]
    fn the_first_record_replaces_and_later_records_upsert() {
        // `replace_or_sparse_after_first`, `world_fold.rs::replace_or_sparse_after_first`: `sparse` is false
        // exactly once. A later record leaves untouched keys alone.
        let schema = schema();
        let s = symbols();
        let mut acc = State::new();
        let first = payload(
            &[
                raw_row(1, LIVE_FLAGS, 10, 3, 100_000, 0, 0, 0, 1, "destroyer"),
                raw_row(2, LIVE_FLAGS, 10, 2, 200_000, 0, 0, 0, 1, "submarine"),
            ],
            0,
            0,
        );
        fold_payload(&schema, &mut acc, &first, false, 0, &s).expect("folds");
        assert_eq!(acc.len(), 2);

        // A sparse record touching only ship 1 leaves ship 2 in place.
        let second = payload(
            &[raw_row(1, LIVE_FLAGS, 11, 3, 300_000, 0, 0, 0, 1, "destroyer")],
            0,
            0,
        );
        fold_payload(&schema, &mut acc, &second, true, 0, &s).expect("folds");
        assert_eq!(acc.len(), 2, "the untouched ship survives a sparse record");
        let c = collection();
        let updated = acc.ships().get(&ship_key(1)).expect("ship 1");
        assert_eq!(
            updated.values[index_of(&c, field::TASK_FORCE_ID)],
            Value::Unsigned(11)
        );

        // A NON-sparse record would have dropped it — the first-record rule.
        let mut replaced = acc.clone();
        fold_payload(&schema, &mut replaced, &second, false, 0, &s).expect("folds");
        assert_eq!(replaced.len(), 1, "a replacing record clears first");
    }

    #[test]
    fn a_delete_row_removes_its_key_and_the_key_can_come_back() {
        // Both halves are in the measured capture: 22 tombstones, and 2 instance
        // ids that were deleted and later seen live again.
        let schema = schema();
        let s = symbols();
        let mut acc = State::new();
        let first = payload(
            &[raw_row(1, LIVE_FLAGS, 10, 3, 100_000, 0, 0, 0, 1, "destroyer")],
            0,
            0,
        );
        fold_payload(&schema, &mut acc, &first, false, 0, &s).expect("folds");
        assert_eq!(acc.len(), 1);

        fold_payload(&schema, &mut acc, &payload(&[tombstone_row(1)], 0, 1), true, 0, &s)
            .expect("folds");
        assert!(acc.is_empty(), "the tombstone removed the key");

        fold_payload(&schema, &mut acc, &first, true, 0, &s).expect("folds");
        assert_eq!(acc.len(), 1, "the same instance id can be re-inserted");
    }

    #[test]
    fn a_rejected_payload_leaves_the_accumulator_untouched() {
        // Atomicity: the fold is all-or-nothing per frame, so a bad row late in a
        // payload must not leave the earlier rows applied.
        let schema = schema();
        let s = symbols();
        let mut acc = State::new();
        let good = payload(
            &[raw_row(1, LIVE_FLAGS, 10, 3, 100_000, 0, 0, 0, 1, "destroyer")],
            0,
            0,
        );
        fold_payload(&schema, &mut acc, &good, false, 0, &s).expect("folds");
        let before = acc.clone();

        let mixed = payload(
            &[
                raw_row(2, LIVE_FLAGS, 10, 3, 1, 0, 0, 0, 1, "submarine"),
                // Unknown flag bit: fatal, and it is the SECOND row.
                raw_row(3, LIVE_FLAGS | (1 << 9), 10, 3, 1, 0, 0, 0, 1, "carrier"),
            ],
            0,
            0,
        );
        assert!(fold_payload(&schema, &mut acc, &mixed, true, 0, &s).is_err());
        assert_eq!(acc, before, "a rejected frame changes nothing");

        // Same for an unresolvable symbol in a later row.
        let unknown_symbol = payload(
            &[
                raw_row(4, LIVE_FLAGS, 10, 3, 1, 0, 0, 0, 1, "submarine"),
                raw_row(5, LIVE_FLAGS, 10, 3, 1, 0, 0, 0, 1, "not_in_the_content_set"),
            ],
            0,
            0,
        );
        assert!(fold_payload(&schema, &mut acc, &unknown_symbol, true, 0, &s).is_err());
        assert_eq!(acc, before);
    }

    #[test]
    fn repeated_ids_in_one_frame_apply_in_payload_order() {
        // `fold_ships` has no duplicate rejection (`:14628`-`:14654`): the last
        // row for an id wins, and a delete followed by an insert resurrects it.
        // Refusing such a frame would refuse one the recorder folds.
        let schema = schema();
        let s = symbols();
        let c = collection();
        let mut acc = State::new();
        let p = payload(
            &[
                raw_row(1, LIVE_FLAGS, 10, 3, 100_000, 0, 0, 0, 1, "destroyer"),
                tombstone_row(1),
                raw_row(1, LIVE_FLAGS, 12, 3, 500_000, 0, 0, 0, 1, "destroyer"),
            ],
            0,
            1,
        );
        fold_payload(&schema, &mut acc, &p, false, 0, &s).expect("folds");
        assert_eq!(acc.len(), 1);
        let ship = acc.ships().get(&ship_key(1)).expect("ship 1");
        assert_eq!(
            ship.values[index_of(&c, field::TASK_FORCE_ID)],
            Value::Unsigned(12),
            "the last row for the id wins"
        );

        // And the other order: insert then delete leaves nothing.
        let mut acc = State::new();
        let p = payload(
            &[
                raw_row(1, LIVE_FLAGS, 10, 3, 100_000, 0, 0, 0, 1, "destroyer"),
                tombstone_row(1),
            ],
            0,
            1,
        );
        fold_payload(&schema, &mut acc, &p, false, 0, &s).expect("folds");
        assert!(acc.is_empty());
    }

    #[test]
    fn the_fold_produces_a_state_the_differ_can_tombstone() {
        // The end-to-end shape the measurement depends on: two folded states
        // whose difference is one tombstone, built by `diff` rather than by hand,
        // so the encoder's "tombstone for an absent key" rule cannot be violated.
        let schema = schema();
        let s = symbols();
        let c = collection();
        let mut acc = State::new();
        fold_payload(
            &schema,
            &mut acc,
            &payload(
                &[
                    raw_row(1, LIVE_FLAGS, 10, 3, 1, 0, 0, 0, 1, "destroyer"),
                    raw_row(2, LIVE_FLAGS, 10, 2, 1, 0, 0, 0, 1, "submarine"),
                ],
                0,
                0,
            ),
            false,
            0,
            &s,
        )
        .expect("folds");
        let prior: KeyedState = acc.ships().clone();
        fold_payload(&schema, &mut acc, &payload(&[tombstone_row(2)], 0, 1), true, 0, &s)
            .expect("folds");
        let d = diff(&c, &prior, acc.ships());
        assert!(d.changed.is_empty());
        assert_eq!(d.tombstones, vec![ship_key(2)]);
    }

    #[test]
    fn fold_payload_refuses_a_schema_that_is_not_this_domains() {
        let mut schema = schema();
        schema.collections.push(schema.collections[0].clone());
        let mut acc = State::new();
        assert!(
            fold_payload(&schema, &mut acc, &payload(&[full_row()], 0, 0), false, 0, &symbols())
                .is_err()
        );
    }

    /// kills: dropping `TASK_FORCE_ID_SPACE` from the ship row, or reading it
    /// from anywhere but `+36`.
    ///
    /// **It does NOT kill ungating the field from `FLAG_TASK_FORCE`** -- every
    /// row it builds sets that flag, so a mapper that ignored the gate would
    /// answer the same. The gate is pinned instead by
    /// `a_clear_gate_is_unknown_field_by_field`, which clears each flag in turn
    /// and asserts that `FLAG_TASK_FORCE` makes BOTH words unknown, and by
    /// `the_mapper_rejects_every_row_the_fold_rejects`, which refuses a gated
    /// row with no space and an ungated row that carries one.
    ///
    /// **What production change makes this fail**: delete the
    /// `TASK_FORCE_ID_SPACE` arm from `record_from_row` and the schema field.
    /// Two ships in the same frame, one in each registry, then map to the same
    /// task-force reference -- which is exactly the mis-join the owner's
    /// 2026-08-27 ruling closes: d28 now publishes both `{51, 490}` and
    /// `{61, 490}`, so a ship saying only "490" would be handed to whichever
    /// one the consumer's map happened to hold.
    #[test]
    fn two_ships_in_two_registries_carry_two_distinguishable_references() {
        let c = collection();
        let s = symbols();
        let ship = |instance_id: u32, space: u32| {
            raw_row_in_space(
                instance_id,
                LIVE_FLAGS,
                490,
                space,
                3,
                100_000,
                0,
                0,
                0,
                1,
                "destroyer",
            )
        };
        let cship = record_from_row(&c, &ship(1, CSHIP_ID_SPACE_FOR_TESTS), 0, &s).expect("maps");
        let fleet =
            record_from_row(&c, &ship(2, OOB_TYPE_FLEET_ID_SPACE_FOR_TESTS), 0, &s).expect("maps");
        assert_eq!(
            cship.values[index_of(&c, field::TASK_FORCE_ID)],
            fleet.values[index_of(&c, field::TASK_FORCE_ID)],
            "the numbers are equal, which is the whole problem"
        );
        assert_ne!(
            cship.values[index_of(&c, field::TASK_FORCE_ID_SPACE)],
            fleet.values[index_of(&c, field::TASK_FORCE_ID_SPACE)],
            "and the references are not"
        );
        assert_eq!(
            cship.values[index_of(&c, field::TASK_FORCE_ID_SPACE)],
            Value::Unsigned(CSHIP_ID_SPACE_FOR_TESTS as u64)
        );
    }

    /// kills: dropping the payload-version check from `peek_header`.
    ///
    /// **What production change makes this fail**: restore the old
    /// `if u32_at(payload, 36) != 0 { reserved is nonzero }` rule. A v1 payload
    /// -- which wrote a hard zero there -- would then be ACCEPTED, and its ship
    /// rows would be read as v2 ones whose `+36` says the task-force reference
    /// carries no registry, so every linked ship would be refused one row at a
    /// time with a misleading message instead of the stream being refused once
    /// with the reason.
    #[test]
    fn a_v1_payload_is_refused_by_version() {
        let mut old = payload(&[full_row()], 0, 0);
        // v1's `reserved0`: a hard zero, enforced by v1's own fold.
        old[36..40].copy_from_slice(&0u32.to_le_bytes());
        let error = peek_header(&old).expect_err("v1 is refused");
        assert!(
            error.contains("version 0") && error.contains("id space"),
            "the refusal names the version and why: {error}"
        );
        // And an unknown future version is refused too, by the same word.
        let mut future = payload(&[full_row()], 0, 0);
        future[36..40].copy_from_slice(&7u32.to_le_bytes());
        let error = peek_header(&future).expect_err("v7 is refused");
        assert!(error.contains("version 7"), "{error}");
        // The current one is accepted, so the test is not vacuously refusing.
        peek_header(&payload(&[full_row()], 0, 0)).expect("v2 is legal");
    }
}
