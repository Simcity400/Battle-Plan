//! d23 `map_detail_live` — per-state resources and buildings, per-province
//! buildings.
//!
//! Derived from `world_fold.rs:33976`-`:34215`
//! (`fold_map_detail`), its payload constants at `:400`-`:403`, the shared row
// @world-fold-authority domain=23 functions=fold_map_detail lines=33976-34215 sha256=4522c62fc2061200ce47d2f28271b9bba54f4d0ddf71135a1aac19eeca322d3b closure=89f96e94ff996a4654e73c96322ee72656666f0c2568e51b2327c2532f632c32
//! helpers at `:29977`-`:30044`, and the collector read and packer that produce
//! the flags (`extractor/native/collector/src/attach_log.rs:58628`-`:58717`,
//! `:59312`-`:59444`, `:71729`-`:71850`). The fold and collector read are the
//! field-level basis.
//!
//! **Payload grammar** (`world_fold.rs::fold_map_detail`, packer `:71758`-`:71772`):
//! a 48-byte header followed by five fixed-stride arrays, in this order:
//!
//! | offset | width | header field |
//! |---|---|---|
//! | `+0`  | u64 | `elapsed_wall_ms` |
//! | `+8`  | u32 | `state_count` |
//! | `+12` | u32 | `changed_rows` |
//! | `+16` | u32 | `status_flags` |
//! | `+20` | u32 | `province_count` |
//! | `+24` | u32 | `observed_state_rows` |
//! | `+28` | u32 | `observed_province_rows` |
//! | `+32` | u32 | `resource_count` |
//! | `+36` | u32 | `state_building_count` |
//! | `+40` | u32 | `province_building_count` |
//! | `+44` | u32 | reserved, must be zero (`world_fold.rs::fold_map_detail`) |
//!
//! then `state_count` 28-byte state rows, `province_count` 20-byte province
//! rows, and three arrays of 12-byte entries — `resource_count`,
//! `state_building_count`, `province_building_count` — with an exact
//! total-length check (`world_fold.rs::fold_map_detail`).
//!
//! | row | offset | width | field | scale | gate |
//! |---|---|---|---|---|---|
//! | state    | `+0`  | u32 | `state_id`         | raw | none (key; zero rejected `world_fold.rs::fold_map_detail`) |
//! | state    | `+4`  | u32 | `flags`            | —   | `!0b11` rejected `world_fold.rs::fold_map_detail` |
//! | state    | `+8`  | u32 | `resource_first`   | —   | transport, `1<<0` |
//! | state    | `+12` | u32 | `resource_len`     | —   | transport, `1<<0` |
//! | state    | `+16` | u32 | `building_first`   | —   | transport, `1<<1` |
//! | state    | `+20` | u32 | `building_len`     | —   | transport, `1<<1` |
//! | state    | `+24` | u32 | reserved, zero `world_fold.rs::fold_map_detail` | — | — |
//! | province | `+0`  | u32 | `province_id`      | raw | none (key; zero rejected `world_fold.rs::fold_map_detail`) |
//! | province | `+4`  | u32 | `state_id`         | raw | `1<<0`, both directions `world_fold.rs::fold_map_detail` |
//! | province | `+8`  | u32 | `flags`            | —   | `!0b11` rejected `world_fold.rs::fold_map_detail` |
//! | province | `+12` | u32 | `building_first`   | —   | transport, `1<<1` |
//! | province | `+16` | u32 | `building_len`     | —   | transport, `1<<1` |
//! | entry    | `+0`  | u32 | `type_id`          | raw | zero rejected; resources also reject `u32::MAX` (`:22451`, `:30019`) |
//! | entry    | `+4`  | i32/u32 | `amount_raw` / `level` | **`/100000` for resources, RAW for levels** | — |
//! | entry    | `+8`  | u32 | reserved, zero `:22454`, `:30022` | — | — |
//!
//! **The scale trap.** One entry shape carries two different scales. A resource
//! entry's `amount_raw` is signed and folds as `amount_raw as f64 / FIXED_SCALE`
//! (`world_fold.rs::map_detail_resource_map`, `FIXED_SCALE = 100_000.0` at `:156`); a building entry's `level` is
//! unsigned and folds **raw**, `json!(entry.level)` (`:30042`), with no division
//! at all. Reading a level through `FIXED_SCALE` would be wrong by 100,000×, and
//! every round-trip test would still pass because the error is in the mapping.
//!
//! **Ranges are transport, not state.** `(first, len)` addresses a slice of a
//! shared entry array (`:22385`-`:22398`, `:22433`-`:22439`: `first + len <=
//! count`, and a clear flag forces an empty range). Two frames carrying the same
//! world state with the entry arrays laid out differently would differ in every
//! index, so the indices are not part of the state. The five arrays therefore
//! flatten into five keyed collections, the three entry arrays keyed by
//! `(parent_id, type_id)`.
//!
//! **`type_id` is a `ContentId`.** The wire carries a number; the human name
//! comes from a host-side registry at render time (`:30047`-`:30064`), so the
//! fold's string keys in its JSON are a rendering artefact and not payload. The
//! deciding question (content-identity contract, and the recipe's step 3) is whether a content manifest
//! must resolve the id for it to keep its meaning under another mod set. It must:
//! buildings are defined by `common/buildings` script and
//! `compact_contract::building_registry` says so in its own words — the table is
//! "loaded from the pinned vanilla `common/buildings` source set" and
//! "Unknown/mod-added ids remain numeric and therefore fail closed". Resource
//! ids are the same kind of thing (`common/resources`), and the fold's own
//! fallback for an unrecognised one is a numeric `tid_0x…` string (`world_fold.rs::map_detail_resource_name`).
//! Both are content, not engine discriminators, so both are `ContentId` — and
//! both are **key components**, which codec-measurement contract flags as the heavier manifest-retrofit
//! class. Every key in this domain is a `ContentId`; see the deliverable's open
//! questions.
//!
//! **Incremental, upsert, no deletes.** A record whose `status_flags` carry
//! `COMPACT_STATUS_DELTA_DATA` carries only the rows the collector saw change
//! (`attach_log.rs:58663`-`:58693`); a full-snapshot record clears everything
//! d23 owns first (`world_fold.rs::fold_map_detail`) and repopulates. Nothing in
//! the grammar deletes a state, a province, or an entry, so [`fold_payload`]
//! upserts and only a full snapshot removes. A delta before any baseline folds
//! nothing (`world_fold.rs::fold_map_detail`) and is the caller's to exclude.
//!
//! **The per-state province-building totals are not ported.**
//! `world_fold.rs::project_map_detail_province_building_totals` sums province
//! building levels into their state, which is a host-side derivation over data
//! this schema already carries in `province_buildings` + `provinces.state_id`.
//! Storing it would be storing a join. (It is a projection built at snapshot
//! time rather than a field the fold retains; the only thing the record itself
//! still owes is the state KEY, via
//! `world_fold.rs::ensure_map_detail_province_total_states`.)

use core::ops::Range;

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Quant, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 23;

pub const STATES: u32 = 1;
pub const PROVINCES: u32 = 2;
pub const STATE_RESOURCES: u32 = 3;
pub const STATE_BUILDINGS: u32 = 4;
pub const PROVINCE_BUILDINGS: u32 = 5;

/// Index of each collection in [`schema`]`().collections` and in the slices the
/// body encoders take. The order is the schema's and never changes.
pub const STATES_INDEX: usize = 0;
pub const PROVINCES_INDEX: usize = 1;
pub const STATE_RESOURCES_INDEX: usize = 2;
pub const STATE_BUILDINGS_INDEX: usize = 3;
pub const PROVINCE_BUILDINGS_INDEX: usize = 4;
pub const COLLECTION_COUNT: usize = 5;

/// Field ids are stable and append-only; schema-identity contract forbids reusing an id for a
/// *different* field. Ids are unique per collection, and an id that names the
/// same concept in two collections is deliberately the same id.
pub mod field {
    pub const STATE_ID: u32 = 1;
    /// On a `states` row: whether that state's resource list was captured.
    pub const RESOURCES_CAPTURED: u32 = 2;
    /// On a `states` or `provinces` row: whether that row's building list was
    /// captured. The same concept in both, so the same id.
    pub const BUILDINGS_CAPTURED: u32 = 3;
    pub const PROVINCE_ID: u32 = 4;
    /// The state a province belongs to; `Null` when it belongs to none.
    pub const PROVINCE_STATE_ID: u32 = 5;
    pub const RESOURCE_TYPE_ID: u32 = 6;
    pub const RESOURCE_AMOUNT: u32 = 7;
    pub const BUILDING_TYPE_ID: u32 = 8;
    pub const BUILDING_LEVEL: u32 = 9;
}

// --- Payload grammar, from `world_fold.rs:400`-`:403` and `:22320`-`:22336`. --

/// `MAP_DETAIL_HEADER_LEN`, `world_fold.rs::MAP_DETAIL_HEADER_LEN`.
pub const HEADER_LEN: usize = 48;
/// `MAP_DETAIL_STATE_ROW_LEN`, `world_fold.rs::MAP_DETAIL_STATE_ROW_LEN`.
pub const STATE_ROW_LEN: usize = 28;
/// `MAP_DETAIL_PROVINCE_ROW_LEN`, `world_fold.rs::MAP_DETAIL_PROVINCE_ROW_LEN`.
pub const PROVINCE_ROW_LEN: usize = 20;
/// `MAP_DETAIL_ENTRY_ROW_LEN`, `world_fold.rs::MAP_DETAIL_ENTRY_ROW_LEN`. One shape, three arrays.
pub const ENTRY_ROW_LEN: usize = 12;

/// State row `flags & 1` — the resource list is present (`world_fold.rs::fold_map_detail`).
pub const STATE_FLAG_RESOURCES: u32 = 1;
/// State row `flags & (1 << 1)` — the building list is present (`:22489`).
pub const STATE_FLAG_BUILDINGS: u32 = 1 << 1;
/// `world_fold.rs::fold_map_detail` — any other bit is a hard error.
pub const STATE_KNOWN_FLAGS: u32 = STATE_FLAG_RESOURCES | STATE_FLAG_BUILDINGS;

/// Province row `flags & 1` — the row carries a state (`world_fold.rs::fold_map_detail`).
pub const PROVINCE_FLAG_STATE: u32 = 1;
/// Province row `flags & (1 << 1)` — the building list is present (`:22503`).
pub const PROVINCE_FLAG_BUILDINGS: u32 = 1 << 1;
/// `world_fold.rs::fold_map_detail` — any other bit is a hard error.
pub const PROVINCE_KNOWN_FLAGS: u32 = PROVINCE_FLAG_STATE | PROVINCE_FLAG_BUILDINGS;

/// `FIXED_SCALE`, `world_fold.rs::FIXED_SCALE`. Applies to a **resource** entry's
/// `amount_raw` (`:30033`) and to nothing else in this domain — a building
/// entry's `level` is emitted raw (`:30042`).
pub const RESOURCE_AMOUNT_SCALE: f64 = 100_000.0;

pub fn schema() -> Schema {
    use field as f;

    // `states`: the state roster, plus whether each child list was captured.
    //
    // The two Booleans are the row's flag bits, and they are the only thing in a
    // state row that is not transport. They are load-bearing: a state whose
    // resource list was captured and is empty (flag set, `resource_len == 0` —
    // the common case, 1081 state rows against 718 resource entries in the
    // measured capture) is a different fact from a state whose resource list was
    // not captured at all (flag clear, which the collector produces when the
    // lane is scheduled without resources, `attach_log.rs:59327`-`:59334`).
    // Without these fields the absence of `(state_id, *)` rows would silently
    // assert "empty" about data that was never read — exactly the record-state contract fidelity
    // loss the four states exist to prevent.
    //
    // They are a real measured value in every row: `Bool`, never `Absent`,
    // `Null` or `Unknown`. A clear flag is not "we could not read it" — a failed
    // read aborts the whole d23 sample and badges the record
    // `COMPACT_STATUS_SOURCE_UNAVAILABLE` (`attach_log.rs:58647`-`:58657`),
    // which folds nothing at all (`world_fold.rs::fold_map_detail`).
    let states = Collection {
        collection_id: STATES,
        shape: Shape::KeyedSet,
        fields: vec![
            Field::new(f::STATE_ID, Kind::ContentId).key(0),
            Field::new(f::RESOURCES_CAPTURED, Kind::Boolean),
            Field::new(f::BUILDINGS_CAPTURED, Kind::Boolean),
        ],
    };

    // `provinces`: the province roster, its state, and the same capture flag.
    //
    // `state_id` is `nullable` and nothing else. The collector sets the flag
    // exactly when `state_id != 0` (`attach_log.rs:71816`), and `state_id` is
    // `0` exactly when the province's state pointer is null
    // (`attach_log.rs:59588`) — a sea or lake province genuinely belongs to no
    // state. That is record-state contract's null, "the relation genuinely points at nothing", word
    // for word. It is not `unknown`: every path that could not *read* the state
    // returns `Err` (`:59587`, `:59600`) and fails the whole sample. It is not
    // `absent`: every province row this schema carries can have a state.
    let provinces = Collection {
        collection_id: PROVINCES,
        shape: Shape::KeyedSet,
        fields: vec![
            Field::new(f::PROVINCE_ID, Kind::ContentId).key(0),
            Field::new(f::PROVINCE_STATE_ID, Kind::ContentId).nullable(),
            Field::new(f::BUILDINGS_CAPTURED, Kind::Boolean),
        ],
    };

    // The three flattened entry collections. `(parent_id, type_id)` is the whole
    // identity: the fold itself keys its JSON by the type's name (`:30028`,
    // `:30039`), so the entry's position in the shared array is not part of the
    // state.
    //
    // `amount` is `Quant::Exact`. derived-field contract makes exact the default and requires the
    // domain document to justify anything else; d23 has no domain document at
    // all, so no quantization is justified and none is declared. See the
    // deliverable's open questions for the empirical note.
    let state_resources = Collection {
        collection_id: STATE_RESOURCES,
        shape: Shape::KeyedSet,
        fields: vec![
            Field::new(f::STATE_ID, Kind::ContentId).key(0),
            Field::new(f::RESOURCE_TYPE_ID, Kind::ContentId).key(1),
            Field::new(f::RESOURCE_AMOUNT, Kind::Continuous).quant(Quant::Exact),
        ],
    };

    // `level` is `Unsigned`, not `Monotone` and not `Continuous`: the fold emits
    // the raw `u32` (`:30042`), and a building level falls as well as rises
    // (bombing, damage), so the monotone hint would be wrong about the data
    // rather than merely unhelpful.
    let state_buildings = Collection {
        collection_id: STATE_BUILDINGS,
        shape: Shape::KeyedSet,
        fields: vec![
            Field::new(f::STATE_ID, Kind::ContentId).key(0),
            Field::new(f::BUILDING_TYPE_ID, Kind::ContentId).key(1),
            Field::new(f::BUILDING_LEVEL, Kind::Unsigned),
        ],
    };
    let province_buildings = Collection {
        collection_id: PROVINCE_BUILDINGS,
        shape: Shape::KeyedSet,
        fields: vec![
            Field::new(f::PROVINCE_ID, Kind::ContentId).key(0),
            Field::new(f::BUILDING_TYPE_ID, Kind::ContentId).key(1),
            Field::new(f::BUILDING_LEVEL, Kind::Unsigned),
        ],
    };

    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![
            states,
            provinces,
            state_resources,
            state_buildings,
            province_buildings,
        ],
        event_part: None,
    }
}

// --- The payload header and its five tables ----------------------------------

/// The 48-byte payload header (`world_fold.rs::fold_map_detail`, packer
/// `attach_log.rs:71758`-`:71772`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub state_count: u32,
    /// How many rows the collector *saw* change. The fold never reads this
    /// field, and a full snapshot legitimately emits more rows than it counts
    /// changed (`attach_log.rs:58672`), so nothing here constrains it — adding a
    /// check the recorder does not make would reject streams the recorder
    /// accepts.
    pub changed_rows: u32,
    pub status_flags: u32,
    pub province_count: u32,
    pub observed_state_rows: u32,
    pub observed_province_rows: u32,
    pub resource_count: u32,
    pub state_building_count: u32,
    pub province_building_count: u32,
}

/// The five fixed-stride arrays of one payload, already bounded by the header's
/// exact total-length check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Tables<'a> {
    pub states: &'a [u8],
    pub provinces: &'a [u8],
    pub resources: &'a [u8],
    pub state_buildings: &'a [u8],
    pub province_buildings: &'a [u8],
}

/// Split a d23 DATA payload into its header and its five tables, applying the
/// fold's own structural checks (`world_fold.rs::fold_map_detail`).
///
/// This lives here rather than in each caller so the header grammar has exactly
/// one implementation: a measurement harness that re-derives it is a harness
/// that can drift from the recorder without anything failing.
pub fn split_payload(payload: &[u8]) -> core::result::Result<(PayloadHeader, Tables<'_>), String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "map_detail payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    let header = PayloadHeader {
        elapsed_wall_ms: u64::from_le_bytes(payload[0..8].try_into().expect("8 bytes")),
        state_count: u32_at(payload, 8),
        changed_rows: u32_at(payload, 12),
        status_flags: u32_at(payload, 16),
        province_count: u32_at(payload, 20),
        observed_state_rows: u32_at(payload, 24),
        observed_province_rows: u32_at(payload, 28),
        resource_count: u32_at(payload, 32),
        state_building_count: u32_at(payload, 36),
        province_building_count: u32_at(payload, 40),
    };
    if u32_at(payload, 44) != 0 {
        return Err("map_detail header reserved field is nonzero".into());
    }
    // Checked arithmetic throughout: the counts are input from a file on disk,
    // and the fold's `saturating_*` (`:22328`) turns an overflowing product into
    // a length mismatch rather than a wrapped, plausible one. Checked reports
    // the overflow as itself; both refuse the payload.
    let states_len = table_len(header.state_count, STATE_ROW_LEN, "state rows")?;
    let provinces_len = table_len(header.province_count, PROVINCE_ROW_LEN, "province rows")?;
    let resources_len = table_len(header.resource_count, ENTRY_ROW_LEN, "resource entries")?;
    let state_buildings_len = table_len(
        header.state_building_count,
        ENTRY_ROW_LEN,
        "state building entries",
    )?;
    let province_buildings_len = table_len(
        header.province_building_count,
        ENTRY_ROW_LEN,
        "province building entries",
    )?;
    let expected = [
        states_len,
        provinces_len,
        resources_len,
        state_buildings_len,
        province_buildings_len,
    ]
    .into_iter()
    .try_fold(HEADER_LEN, |acc, len| acc.checked_add(len))
    .ok_or_else(|| "map_detail payload length overflows".to_string())?;
    if payload.len() != expected {
        return Err(format!("map_detail len {} != {expected}", payload.len()));
    }

    let mut cursor = HEADER_LEN;
    let mut take = |len: usize| -> &[u8] {
        let slice = &payload[cursor..cursor + len];
        cursor += len;
        slice
    };
    let tables = Tables {
        states: take(states_len),
        provinces: take(provinces_len),
        resources: take(resources_len),
        state_buildings: take(state_buildings_len),
        province_buildings: take(province_buildings_len),
    };
    Ok((header, tables))
}

fn table_len(count: u32, stride: usize, label: &str) -> core::result::Result<usize, String> {
    (count as usize)
        .checked_mul(stride)
        .ok_or_else(|| format!("map_detail {label} count {count} overflows"))
}

// --- Row decoding ------------------------------------------------------------

/// A state row's own fields, before any cross-table range check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct StateRowFields {
    state_id: u32,
    flags: u32,
    resource_first: usize,
    resource_len: usize,
    building_first: usize,
    building_len: usize,
}

fn parse_state_row(row: &[u8]) -> core::result::Result<StateRowFields, String> {
    if row.len() != STATE_ROW_LEN {
        return Err(format!(
            "map_detail state row length {} != {STATE_ROW_LEN}",
            row.len()
        ));
    }
    let fields = StateRowFields {
        state_id: u32_at(row, 0),
        flags: u32_at(row, 4),
        resource_first: u32_at(row, 8) as usize,
        resource_len: u32_at(row, 12) as usize,
        building_first: u32_at(row, 16) as usize,
        building_len: u32_at(row, 20) as usize,
    };
    // `world_fold.rs::fold_map_detail`.
    if fields.state_id == 0 {
        return Err("map_detail state row has zero state_id".into());
    }
    // `world_fold.rs::fold_map_detail` — an unrecognised flag word means the row is not
    // this grammar, so nothing in it can be trusted.
    if fields.flags & !STATE_KNOWN_FLAGS != 0 {
        return Err(format!(
            "map_detail state row {} has unknown flags {:#x}",
            fields.state_id, fields.flags
        ));
    }
    // `world_fold.rs::fold_map_detail`.
    if u32_at(row, 24) != 0 {
        return Err(format!(
            "map_detail state row {} reserved field is nonzero",
            fields.state_id
        ));
    }
    Ok(fields)
}

/// A province row's own fields, before any cross-table range check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProvinceRowFields {
    province_id: u32,
    state_id: u32,
    flags: u32,
    building_first: usize,
    building_len: usize,
}

fn parse_province_row(row: &[u8]) -> core::result::Result<ProvinceRowFields, String> {
    if row.len() != PROVINCE_ROW_LEN {
        return Err(format!(
            "map_detail province row length {} != {PROVINCE_ROW_LEN}",
            row.len()
        ));
    }
    let fields = ProvinceRowFields {
        province_id: u32_at(row, 0),
        state_id: u32_at(row, 4),
        flags: u32_at(row, 8),
        building_first: u32_at(row, 12) as usize,
        building_len: u32_at(row, 16) as usize,
    };
    // `world_fold.rs::fold_map_detail`.
    if fields.province_id == 0 {
        return Err("map_detail province row has zero province_id".into());
    }
    // `world_fold.rs::fold_map_detail`.
    if fields.flags & !PROVINCE_KNOWN_FLAGS != 0 {
        return Err(format!(
            "map_detail province row {} has unknown flags {:#x}",
            fields.province_id, fields.flags
        ));
    }
    // `world_fold.rs::fold_map_detail`: the state flag and the state id must agree
    // in BOTH directions, so neither a flagged zero nor an unflagged state is a
    // legal row.
    if fields.flags & PROVINCE_FLAG_STATE != 0 && fields.state_id == 0 {
        return Err(format!(
            "map_detail province row {} has state flag without state",
            fields.province_id
        ));
    }
    if fields.flags & PROVINCE_FLAG_STATE == 0 && fields.state_id != 0 {
        return Err(format!(
            "map_detail province row {} has state without flag",
            fields.province_id
        ));
    }
    Ok(fields)
}

/// `validate_map_detail_range`, `world_fold.rs::validate_map_detail_range`: a flagged range must lie
/// inside its entry array, and a clear flag forces an empty range at zero.
fn checked_range(
    present: bool,
    first: usize,
    len: usize,
    total: usize,
    label: &str,
) -> core::result::Result<Option<Range<usize>>, String> {
    if !present {
        if first != 0 || len != 0 {
            return Err(format!("{label} range present without flag"));
        }
        return Ok(None);
    }
    let end = first
        .checked_add(len)
        .ok_or_else(|| format!("{label} range overflows"))?;
    if end > total {
        return Err(format!("{label} range exceeds entry table"));
    }
    Ok(Some(first..end))
}

/// The `(resources, buildings)` entry ranges a state row addresses, validated
/// against the header's entry counts (`world_fold.rs::fold_map_detail`).
///
/// The ranges are transport, never state: they say where this frame happens to
/// have put the entries, not anything about the world. They are public because
/// the harness, the tests and the fold step must all read them the same way.
#[allow(clippy::type_complexity)]
pub fn state_row_ranges(
    header: &PayloadHeader,
    row: &[u8],
) -> core::result::Result<(Option<Range<usize>>, Option<Range<usize>>), String> {
    let fields = parse_state_row(row)?;
    let resources = checked_range(
        fields.flags & STATE_FLAG_RESOURCES != 0,
        fields.resource_first,
        fields.resource_len,
        header.resource_count as usize,
        "map_detail state resource",
    )?;
    let buildings = checked_range(
        fields.flags & STATE_FLAG_BUILDINGS != 0,
        fields.building_first,
        fields.building_len,
        header.state_building_count as usize,
        "map_detail state building",
    )?;
    Ok((resources, buildings))
}

/// The building entry range a province row addresses
/// (`world_fold.rs::fold_map_detail`).
pub fn province_row_range(
    header: &PayloadHeader,
    row: &[u8],
) -> core::result::Result<Option<Range<usize>>, String> {
    let fields = parse_province_row(row)?;
    checked_range(
        fields.flags & PROVINCE_FLAG_BUILDINGS != 0,
        fields.building_first,
        fields.building_len,
        header.province_building_count as usize,
        "map_detail province building",
    )
}

/// The key of one state row: its `state_id` (`world_fold.rs::fold_map_detail`).
pub fn key_from_state_row(row: &[u8]) -> core::result::Result<Key, String> {
    Ok(key(&[parse_state_row(row)?.state_id as u64]))
}

/// The key of one province row: its `province_id` (`world_fold.rs::fold_map_detail`).
pub fn key_from_province_row(row: &[u8]) -> core::result::Result<Key, String> {
    Ok(key(&[parse_province_row(row)?.province_id as u64]))
}

/// The key of one flattened state-resource row: `(state_id, type_id)`.
pub fn key_from_state_resource_row(
    state_id: u32,
    entry: &[u8],
) -> core::result::Result<Key, String> {
    Ok(key(&[state_id as u64, resource_type_id(entry)? as u64]))
}

/// The key of one flattened state-building row: `(state_id, type_id)`.
pub fn key_from_state_building_row(
    state_id: u32,
    entry: &[u8],
) -> core::result::Result<Key, String> {
    Ok(key(&[state_id as u64, building_type_id(entry)? as u64]))
}

/// The key of one flattened province-building row: `(province_id, type_id)`.
pub fn key_from_province_building_row(
    province_id: u32,
    entry: &[u8],
) -> core::result::Result<Key, String> {
    Ok(key(&[province_id as u64, building_type_id(entry)? as u64]))
}

/// A resource entry's `type_id`, with the fold's checks (`world_fold.rs:22451`,
/// `:22454`). Resources reject `u32::MAX` as well as zero; building entries do
/// not (`world_fold.rs::read_map_detail_building_entry`). The asymmetry is the fold's, and it is mirrored rather than
/// unified — unifying it would reject frames the recorder accepts.
pub fn resource_type_id(entry: &[u8]) -> core::result::Result<u32, String> {
    check_entry_len(entry)?;
    let type_id = u32_at(entry, 0);
    if type_id == 0 || type_id == u32::MAX {
        return Err("map_detail resource entry has invalid type_id".into());
    }
    if u32_at(entry, 8) != 0 {
        return Err("map_detail resource entry reserved field is nonzero".into());
    }
    Ok(type_id)
}

/// A building entry's `type_id`, with the fold's checks
/// (`world_fold.rs::read_map_detail_building_entry`).
pub fn building_type_id(entry: &[u8]) -> core::result::Result<u32, String> {
    check_entry_len(entry)?;
    let type_id = u32_at(entry, 0);
    if type_id == 0 {
        return Err("map_detail building entry has zero type_id".into());
    }
    if u32_at(entry, 8) != 0 {
        return Err("map_detail building entry reserved field is nonzero".into());
    }
    Ok(type_id)
}

// --- The boundary mapping ----------------------------------------------------

/// One compact row of the `states` table, as the collector emits it today, onto
/// its stored Record. `row` is the raw 28-byte row slice (the caller has already
/// consumed the payload header via [`split_payload`]); d23 has exactly one state
/// row form, so length selects nothing and a wrong length is rejected.
/// `frame_tick` is derived-field contract's derived-field input (stored = v + c * (frame_tick >> u));
/// d23 declares no derived field, so it is taken and ignored — every domain's
/// mapper has the same shape.
///
/// This is the row's **own** record. On a delta record a clear flag means the
/// row says nothing about that list rather than that the list is gone, so
/// [`fold_payload`] merges this record with the accumulated one; see there.
pub fn record_from_state_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    let fields = parse_state_row(row)?;
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        values.push(match f.field_id {
            field::RESOURCES_CAPTURED => {
                Value::Bool(fields.flags & STATE_FLAG_RESOURCES != 0)
            }
            field::BUILDINGS_CAPTURED => {
                Value::Bool(fields.flags & STATE_FLAG_BUILDINGS != 0)
            }
            other => return Err(no_mapping("states", other)),
        });
    }
    Ok(Record::new(values))
}

/// One compact row of the `provinces` table onto its stored Record. See
/// [`record_from_state_row`] for the shared contract.
pub fn record_from_province_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    let fields = parse_province_row(row)?;
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        values.push(match f.field_id {
            // Null, not zero and not unknown: the flag is clear exactly when the
            // province's state pointer is null (`attach_log.rs:59588`), and the
            // relation genuinely points at nothing.
            field::PROVINCE_STATE_ID => {
                if fields.flags & PROVINCE_FLAG_STATE != 0 {
                    Value::ContentId(fields.state_id as u64)
                } else {
                    Value::Null
                }
            }
            field::BUILDINGS_CAPTURED => {
                Value::Bool(fields.flags & PROVINCE_FLAG_BUILDINGS != 0)
            }
            other => return Err(no_mapping("provinces", other)),
        });
    }
    Ok(Record::new(values))
}

/// One 12-byte resource entry onto its stored `state_resources` Record.
///
/// **`/100000`** (`world_fold.rs::map_detail_resource_map`), and the word is signed.
pub fn record_from_state_resource_row(
    collection: &Collection,
    entry: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    let _ = resource_type_id(entry)?;
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        values.push(match f.field_id {
            field::RESOURCE_AMOUNT => {
                Value::Continuous(i32_at(entry, 4) as f64 / RESOURCE_AMOUNT_SCALE)
            }
            other => return Err(no_mapping("state_resources", other)),
        });
    }
    Ok(Record::new(values))
}

/// One 12-byte building entry onto its stored `state_buildings` Record.
///
/// **Raw, never scaled** (`world_fold.rs::map_detail_building_map`). Shares the entry shape with
/// [`record_from_state_resource_row`] and not its scale.
pub fn record_from_state_building_row(
    collection: &Collection,
    entry: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    building_level_record(collection, entry, frame_tick, "state_buildings")
}

/// One 12-byte building entry onto its stored `province_buildings` Record. The
/// same shape, the same scale (none) and the same fold helper
/// (`world_fold.rs::map_detail_building_map`) as [`record_from_state_building_row`]; it is a
/// separate entry point because it is a separate collection.
pub fn record_from_province_building_row(
    collection: &Collection,
    entry: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    building_level_record(collection, entry, frame_tick, "province_buildings")
}

fn building_level_record(
    collection: &Collection,
    entry: &[u8],
    frame_tick: u64,
    label: &str,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    let _ = building_type_id(entry)?;
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        values.push(match f.field_id {
            field::BUILDING_LEVEL => Value::Unsigned(u32_at(entry, 4) as u64),
            other => return Err(no_mapping(label, other)),
        });
    }
    Ok(Record::new(values))
}

/// A collection this mapper does not know how to fill. Filling it with a
/// placeholder would publish invented data under a field id that means something
/// else.
fn no_mapping(collection: &str, field_id: u32) -> String {
    format!("map_detail {collection} has no mapping for field id {field_id}")
}

// --- The domain fold ---------------------------------------------------------

/// The accumulated state of all five collections, in [`schema`] order.
///
/// d23 is incremental, so a single record is not a state: reconstructing an hour
/// means folding every record since the last full snapshot. This lives beside
/// the mapper for the same reason [`split_payload`] does — the measurement
/// harness, the tests and any future caller must share one implementation of the
/// accumulation rules, because a re-derived fold drifts from the recorder
/// without anything failing.
#[derive(Clone, Debug, PartialEq)]
pub struct MapDetailState {
    collections: Vec<KeyedState>,
}

/// **Written out rather than derived.** A derived `Default` builds an *empty*
/// collection list, and `MapDetailState::default()` is public and idiomatic —
/// every accessor and the fold itself index the five collections, so that value
/// is a panic waiting for its first caller. `Default` is `new`.
impl Default for MapDetailState {
    fn default() -> Self {
        Self::new()
    }
}

impl MapDetailState {
    pub fn new() -> Self {
        Self {
            collections: vec![KeyedState::new(); COLLECTION_COUNT],
        }
    }

    /// The five collections in schema order — the shape `encode_baseline`,
    /// `encode_delta` and `decode_delta` take.
    pub fn as_slice(&self) -> &[KeyedState] {
        &self.collections
    }

    pub fn get(&self, index: usize) -> &KeyedState {
        &self.collections[index]
    }

    /// `world_fold.rs::fold_map_detail`: a full snapshot drops everything d23 owns
    /// before repopulating.
    pub fn clear(&mut self) {
        for state in &mut self.collections {
            state.clear();
        }
    }

    pub fn rows(&self) -> usize {
        self.collections.iter().map(KeyedState::len).sum()
    }
}

/// Fold one d23 DATA payload into `acc`, exactly as `fold_map_detail` folds it.
///
/// `delta_update` is `status_flags & COMPACT_STATUS_DELTA_DATA != 0`
/// (`world_fold.rs::fold_map_detail`). The caller supplies it rather than this function
/// reading the bit itself, because that constant belongs to `compact_contract`
/// and d23 is not admitted to the production-codec dependency surface; copying
/// the literal here is exactly the drift the shared-contract rule forbids. A
/// **failure-status** record folds nothing at all
/// (`world_fold.rs::fold_map_detail`) and a delta before any baseline folds nothing
/// (`world_fold.rs::fold_map_detail`); both are the caller's to exclude before calling this.
///
/// **Atomic.** Every row and every entry is decoded and checked before anything
/// is written, so a payload this function rejects leaves `acc` exactly as it
/// found it — the frame-level all-or-nothing the fold has by returning `Err`
/// before it mutates.
///
/// **Every entry is checked, referenced or not.** `fold_map_detail` decodes all
/// three entry arrays in full (`world_fold.rs::fold_map_detail`) before it looks at a single
/// range, so an entry with a zero `type_id` or a nonzero reserved word makes the
/// frame invalid even if no row addresses it. Checking only the addressed
/// entries would accept frames the recorder refuses.
///
/// **Repeated ids fold in payload order, last one winning.** The fold has no
/// duplicate rejection: a second row for one `state_id` overwrites the first in
/// its JSON map (`:22480`), and a repeated `type_id` inside one range overwrites
/// in `map_detail_resource_map` / `map_detail_building_map` (`:30028`, `:30039`).
/// Refusing such a frame would refuse one the shipping recorder folds, so this
/// applies the rows sequentially and reproduces the same final state.
///
/// **Upsert, never delete.** Only a full snapshot removes rows, by clearing
/// first. A delta row whose flag is clear says nothing about that child list, so
/// the accumulated list and the accumulated capture flag are both retained —
/// `fold_map_detail` does exactly this, by not inserting (`:22481`, `:22489`,
/// `:22499`, `:22503`).
pub fn fold_payload(
    schema: &Schema,
    acc: &mut MapDetailState,
    payload: &[u8],
    delta_update: bool,
    frame_tick: u64,
) -> core::result::Result<(), String> {
    if schema.collections.len() != COLLECTION_COUNT {
        return Err(format!(
            "map_detail schema has {} collections, expected {COLLECTION_COUNT}",
            schema.collections.len()
        ));
    }
    // An accumulator that is not this domain's shape is a caller error, and
    // every line below indexes the five collections. Diagnosed, never a panic.
    if acc.as_slice().len() != COLLECTION_COUNT {
        return Err(format!(
            "map_detail accumulator has {} collections, expected {COLLECTION_COUNT}",
            acc.as_slice().len()
        ));
    }
    let (header, tables) = split_payload(payload)?;

    // ---- Decode and check everything first. ----
    struct DecodedState {
        key: Key,
        state_id: u32,
        record: Record,
        resources: Option<Range<usize>>,
        buildings: Option<Range<usize>>,
    }
    struct DecodedProvince {
        key: Key,
        record: Record,
        buildings: Option<Range<usize>>,
    }

    let states_c = &schema.collections[STATES_INDEX];
    let provinces_c = &schema.collections[PROVINCES_INDEX];
    let resources_c = &schema.collections[STATE_RESOURCES_INDEX];
    let state_buildings_c = &schema.collections[STATE_BUILDINGS_INDEX];
    let province_buildings_c = &schema.collections[PROVINCE_BUILDINGS_INDEX];

    // The three entry arrays, decoded in full — `world_fold.rs::fold_map_detail`
    // reads and validates every declared entry before any range is applied.
    let resources = decode_entries(
        tables.resources,
        resources_c,
        frame_tick,
        EntryKind::Resource,
    )?;
    let state_buildings = decode_entries(
        tables.state_buildings,
        state_buildings_c,
        frame_tick,
        EntryKind::Building,
    )?;
    let province_buildings = decode_entries(
        tables.province_buildings,
        province_buildings_c,
        frame_tick,
        EntryKind::Building,
    )?;

    let mut decoded_states = Vec::with_capacity(header.state_count as usize);
    for row in tables.states.chunks_exact(STATE_ROW_LEN) {
        let fields = parse_state_row(row)?;
        let (resources, buildings) = state_row_ranges(&header, row)?;
        decoded_states.push(DecodedState {
            key: key_from_state_row(row)?,
            state_id: fields.state_id,
            record: record_from_state_row(states_c, row, frame_tick)?,
            resources,
            buildings,
        });
    }

    let mut decoded_provinces = Vec::with_capacity(header.province_count as usize);
    for row in tables.provinces.chunks_exact(PROVINCE_ROW_LEN) {
        decoded_provinces.push(DecodedProvince {
            key: key_from_province_row(row)?,
            record: record_from_province_row(provinces_c, row, frame_tick)?,
            buildings: province_row_range(&header, row)?,
        });
    }

    // ---- Nothing below can fail, so `acc` is never left half-written. ----
    if !delta_update {
        acc.clear();
    }

    let resources_captured = value_index(states_c, field::RESOURCES_CAPTURED)?;
    let state_buildings_captured = value_index(states_c, field::BUILDINGS_CAPTURED)?;
    let province_state_id = value_index(provinces_c, field::PROVINCE_STATE_ID)?;
    let province_buildings_captured = value_index(provinces_c, field::BUILDINGS_CAPTURED)?;

    // Rows are applied in payload order, so a repeated id resolves the way the
    // fold's map resolves it: the last row wins, field by field.
    for decoded in decoded_states {
        let mut record = decoded.record;
        let prior = acc.collections[STATES_INDEX].get(&decoded.key);
        if let Some(range) = decoded.resources {
            let entries = children_of(&resources, range, decoded.state_id)?;
            replace_children(
                &mut acc.collections[STATE_RESOURCES_INDEX],
                decoded.state_id,
                entries,
            );
        } else if let Some(prior) = prior {
            // The row says nothing about resources, so the accumulated answer
            // stands — including "we have captured them", which a later frame
            // must not silently downgrade to "we have not".
            record.values[resources_captured] = prior.values[resources_captured];
        }
        let prior = acc.collections[STATES_INDEX].get(&decoded.key);
        if let Some(range) = decoded.buildings {
            let entries = children_of(&state_buildings, range, decoded.state_id)?;
            replace_children(
                &mut acc.collections[STATE_BUILDINGS_INDEX],
                decoded.state_id,
                entries,
            );
        } else if let Some(prior) = prior {
            record.values[state_buildings_captured] = prior.values[state_buildings_captured];
        }
        acc.collections[STATES_INDEX].insert(decoded.key, record);
    }

    for decoded in decoded_provinces {
        let mut record = decoded.record;
        let province_id = decoded.key.first() as u32;
        let prior = acc.collections[PROVINCES_INDEX].get(&decoded.key);
        // `world_fold.rs::fold_map_detail`: a clear state flag does not insert, so whatever
        // the accumulator already holds for this province survives.
        if record.values[province_state_id].is_null() {
            if let Some(prior) = prior {
                record.values[province_state_id] = prior.values[province_state_id];
            }
        }
        if let Some(range) = decoded.buildings {
            let entries = children_of(&province_buildings, range, province_id)?;
            replace_children(
                &mut acc.collections[PROVINCE_BUILDINGS_INDEX],
                province_id,
                entries,
            );
        } else if let Some(prior) = prior {
            record.values[province_buildings_captured] = prior.values[province_buildings_captured];
        }
        acc.collections[PROVINCES_INDEX].insert(decoded.key, record);
    }

    Ok(())
}

enum EntryKind {
    Resource,
    Building,
}

/// One whole entry array, decoded and checked entry by entry into
/// `(type_id, record)` pairs. The array's length is the header's count times the
/// stride, so this decodes exactly the entries the header declares — the fold's
/// own `for _ in 0..count` loops (`world_fold.rs::fold_map_detail`).
fn decode_entries(
    array: &[u8],
    collection: &Collection,
    frame_tick: u64,
    kind: EntryKind,
) -> core::result::Result<Vec<(u32, Record)>, String> {
    let mut out = Vec::with_capacity(array.len() / ENTRY_ROW_LEN);
    for entry in array.chunks_exact(ENTRY_ROW_LEN) {
        out.push(match kind {
            EntryKind::Resource => (
                resource_type_id(entry)?,
                record_from_state_resource_row(collection, entry, frame_tick)?,
            ),
            EntryKind::Building => (
                building_type_id(entry)?,
                building_level_record(collection, entry, frame_tick, "buildings")?,
            ),
        });
    }
    Ok(out)
}

/// One parent's slice of a decoded entry array, keyed by `(parent_id, type_id)`.
///
/// The range was checked against the header count and the array was decoded from
/// exactly that many entries, so the lookup cannot miss; it is a `?` rather than
/// an index so a future caller pairing a range with the wrong array gets an error
/// instead of a panic.
fn children_of(
    entries: &[(u32, Record)],
    range: Range<usize>,
    parent_id: u32,
) -> core::result::Result<Vec<(Key, Record)>, String> {
    entries
        .get(range.clone())
        .ok_or_else(|| {
            format!(
                "map_detail range {}..{} is outside its {} entry table",
                range.start,
                range.end,
                entries.len()
            )
        })
        .map(|slice| {
            slice
                .iter()
                .map(|(type_id, record)| (key(&[parent_id as u64, *type_id as u64]), record.clone()))
                .collect()
        })
}

/// Replace every `(parent_id, *)` row with `entries`. The parent's list is
/// carried whole by the row that owns it, so a row that carries the list carries
/// all of it. `entries` is applied in payload order, so a `type_id` repeated
/// inside one range resolves the way the fold's map resolves it: last wins.
fn replace_children(state: &mut KeyedState, parent_id: u32, entries: Vec<(Key, Record)>) {
    let low = Key(vec![parent_id as i128, 0]);
    let high = Key(vec![parent_id as i128 + 1, 0]);
    let stale: Vec<Key> = state.range(low..high).map(|(k, _)| k.clone()).collect();
    for key in stale {
        state.remove(&key);
    }
    for (key, record) in entries {
        state.insert(key, record);
    }
}

fn value_index(collection: &Collection, field_id: u32) -> core::result::Result<usize, String> {
    collection
        .value_fields()
        .position(|f| f.field_id == field_id)
        .ok_or_else(|| {
            format!(
                "map_detail collection {} is missing field id {field_id}",
                collection.collection_id
            )
        })
}

fn check_entry_len(entry: &[u8]) -> core::result::Result<(), String> {
    if entry.len() != ENTRY_ROW_LEN {
        return Err(format!(
            "map_detail entry length {} != {ENTRY_ROW_LEN}",
            entry.len()
        ));
    }
    Ok(())
}

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
    use crate::body::{apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta};
    use crate::TickGrid;

    fn grid() -> TickGrid {
        TickGrid::new(3).expect("3 is a legal grid")
    }

    fn collection(index: usize) -> Collection {
        schema().collections[index].clone()
    }

    fn index_of(c: &Collection, id: u32) -> usize {
        c.value_fields()
            .position(|f| f.field_id == id)
            .expect("field is in the schema")
    }

    // --- raw payload builders, packing exactly what the collector packs ------
    // (`attach_log.rs:71758`-`:71850`).

    #[derive(Clone, Default)]
    struct RawState {
        state_id: u32,
        resources: Option<Vec<(u32, i32)>>,
        buildings: Option<Vec<(u32, u32)>>,
    }

    #[derive(Clone, Default)]
    struct RawProvince {
        province_id: u32,
        state_id: u32,
        buildings: Option<Vec<(u32, u32)>>,
    }

    fn raw_payload(status_flags: u32, states: &[RawState], provinces: &[RawProvince]) -> Vec<u8> {
        let resource_count: usize = states
            .iter()
            .map(|s| s.resources.as_ref().map_or(0, Vec::len))
            .sum();
        let state_building_count: usize = states
            .iter()
            .map(|s| s.buildings.as_ref().map_or(0, Vec::len))
            .sum();
        let province_building_count: usize = provinces
            .iter()
            .map(|p| p.buildings.as_ref().map_or(0, Vec::len))
            .sum();
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        payload.extend_from_slice(&(states.len() as u32).to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes()); // changed_rows
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&(provinces.len() as u32).to_le_bytes());
        payload.extend_from_slice(&(states.len() as u32).to_le_bytes()); // observed
        payload.extend_from_slice(&(provinces.len() as u32).to_le_bytes()); // observed
        payload.extend_from_slice(&(resource_count as u32).to_le_bytes());
        payload.extend_from_slice(&(state_building_count as u32).to_le_bytes());
        payload.extend_from_slice(&(province_building_count as u32).to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes()); // reserved

        let mut resource_cursor = 0u32;
        let mut state_building_cursor = 0u32;
        for s in states {
            let resource_len = s.resources.as_ref().map_or(0, Vec::len) as u32;
            let building_len = s.buildings.as_ref().map_or(0, Vec::len) as u32;
            let mut flags = 0u32;
            if s.resources.is_some() {
                flags |= STATE_FLAG_RESOURCES;
            }
            if s.buildings.is_some() {
                flags |= STATE_FLAG_BUILDINGS;
            }
            payload.extend_from_slice(&s.state_id.to_le_bytes());
            payload.extend_from_slice(&flags.to_le_bytes());
            // A clear flag forces `first == len == 0` (`world_fold.rs::validate_map_detail_range`).
            // The shipping packer writes the running cursor unconditionally
            // (`attach_log.rs:71798`), which would be a rejected row for a
            // flagless state after a flagged one — a latent inconsistency that
            // cannot fire today because `include_resources` and
            // `include_buildings` are per-frame, not per-row. These fixtures
            // pack what the fold accepts.
            let first = if s.resources.is_some() { resource_cursor } else { 0 };
            payload.extend_from_slice(&first.to_le_bytes());
            payload.extend_from_slice(&resource_len.to_le_bytes());
            let first = if s.buildings.is_some() { state_building_cursor } else { 0 };
            payload.extend_from_slice(&first.to_le_bytes());
            payload.extend_from_slice(&building_len.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            resource_cursor += resource_len;
            state_building_cursor += building_len;
        }
        let mut province_building_cursor = 0u32;
        for p in provinces {
            let building_len = p.buildings.as_ref().map_or(0, Vec::len) as u32;
            let mut flags = 0u32;
            if p.state_id != 0 {
                flags |= PROVINCE_FLAG_STATE;
            }
            if p.buildings.is_some() {
                flags |= PROVINCE_FLAG_BUILDINGS;
            }
            payload.extend_from_slice(&p.province_id.to_le_bytes());
            payload.extend_from_slice(&p.state_id.to_le_bytes());
            payload.extend_from_slice(&flags.to_le_bytes());
            let first = if p.buildings.is_some() { province_building_cursor } else { 0 };
            payload.extend_from_slice(&first.to_le_bytes());
            payload.extend_from_slice(&building_len.to_le_bytes());
            province_building_cursor += building_len;
        }
        for s in states {
            for (type_id, amount) in s.resources.iter().flatten() {
                payload.extend_from_slice(&type_id.to_le_bytes());
                payload.extend_from_slice(&amount.to_le_bytes());
                payload.extend_from_slice(&0u32.to_le_bytes());
            }
        }
        for s in states {
            for (type_id, level) in s.buildings.iter().flatten() {
                payload.extend_from_slice(&type_id.to_le_bytes());
                payload.extend_from_slice(&level.to_le_bytes());
                payload.extend_from_slice(&0u32.to_le_bytes());
            }
        }
        for p in provinces {
            for (type_id, level) in p.buildings.iter().flatten() {
                payload.extend_from_slice(&type_id.to_le_bytes());
                payload.extend_from_slice(&level.to_le_bytes());
                payload.extend_from_slice(&0u32.to_le_bytes());
            }
        }
        payload
    }

    /// A state row's raw 28 bytes on their own, for the row-level mapper tests.
    fn state_row(state_id: u32, flags: u32, ranges: [u32; 4], reserved: u32) -> Vec<u8> {
        let mut row = Vec::with_capacity(STATE_ROW_LEN);
        row.extend_from_slice(&state_id.to_le_bytes());
        row.extend_from_slice(&flags.to_le_bytes());
        for v in ranges {
            row.extend_from_slice(&v.to_le_bytes());
        }
        row.extend_from_slice(&reserved.to_le_bytes());
        assert_eq!(row.len(), STATE_ROW_LEN);
        row
    }

    fn province_row(province_id: u32, state_id: u32, flags: u32, range: [u32; 2]) -> Vec<u8> {
        let mut row = Vec::with_capacity(PROVINCE_ROW_LEN);
        row.extend_from_slice(&province_id.to_le_bytes());
        row.extend_from_slice(&state_id.to_le_bytes());
        row.extend_from_slice(&flags.to_le_bytes());
        for v in range {
            row.extend_from_slice(&v.to_le_bytes());
        }
        assert_eq!(row.len(), PROVINCE_ROW_LEN);
        row
    }

    fn entry(type_id: u32, value: i32, reserved: u32) -> Vec<u8> {
        let mut e = Vec::with_capacity(ENTRY_ROW_LEN);
        e.extend_from_slice(&type_id.to_le_bytes());
        e.extend_from_slice(&value.to_le_bytes());
        e.extend_from_slice(&reserved.to_le_bytes());
        assert_eq!(e.len(), ENTRY_ROW_LEN);
        e
    }

    /// The payload the measured capture's first record has in miniature: two
    /// states, one with resources and buildings and one with captured-but-empty
    /// lists, and two provinces, one in a state and one in none.
    fn full_payload() -> Vec<u8> {
        raw_payload(
            0,
            &[
                RawState {
                    state_id: 7,
                    // 0x3E03 steel = 4.0, 0x4E37 oil = -1.5.
                    resources: Some(vec![(0x3E03, 400_000), (0x4E37, -150_000)]),
                    buildings: Some(vec![(0x2FAB, 3), (0x4E22, 0)]),
                },
                RawState {
                    state_id: 9,
                    resources: Some(Vec::new()),
                    buildings: Some(Vec::new()),
                },
            ],
            &[
                RawProvince {
                    province_id: 11,
                    state_id: 7,
                    buildings: Some(vec![(0x4CC1, 5)]),
                },
                RawProvince {
                    province_id: 12,
                    state_id: 0,
                    buildings: Some(Vec::new()),
                },
            ],
        )
    }

    fn folded(payload: &[u8], delta: bool) -> MapDetailState {
        let schema = schema();
        let mut acc = MapDetailState::new();
        fold_payload(&schema, &mut acc, payload, delta, 0).expect("folds");
        acc
    }

    // --- step 8.1: the schema validates ------------------------------------

    #[test]
    fn the_schema_is_valid() {
        schema()
            .validate(grid().log2())
            .expect("d23 schema is well-formed");
    }

    // --- step 8.5: shape and keys ------------------------------------------

    #[test]
    fn every_collection_is_a_keyed_set_with_content_keys() {
        let s = schema();
        assert_eq!(s.collections.len(), COLLECTION_COUNT);
        let expected: [(u32, &[u32]); 5] = [
            (STATES, &[field::STATE_ID]),
            (PROVINCES, &[field::PROVINCE_ID]),
            (
                STATE_RESOURCES,
                &[field::STATE_ID, field::RESOURCE_TYPE_ID],
            ),
            (
                STATE_BUILDINGS,
                &[field::STATE_ID, field::BUILDING_TYPE_ID],
            ),
            (
                PROVINCE_BUILDINGS,
                &[field::PROVINCE_ID, field::BUILDING_TYPE_ID],
            ),
        ];
        for (i, (collection_id, key_ids)) in expected.into_iter().enumerate() {
            let c = &s.collections[i];
            assert_eq!(c.collection_id, collection_id, "collection {i}");
            assert_eq!(c.shape, Shape::KeyedSet, "only a keyed set has deltas");
            let keys = c.key_fields();
            assert_eq!(
                keys.iter().map(|f| f.field_id).collect::<Vec<_>>(),
                key_ids.to_vec(),
                "collection {collection_id} key"
            );
            for k in &keys {
                assert_eq!(
                    k.kind,
                    Kind::ContentId,
                    "every d23 key component is content-defined (content-identity contract)"
                );
                assert!(k.kind.legal_as_key());
            }
        }
    }

    #[test]
    fn the_declared_states_are_exactly_one_nullable_field() {
        // Step 2's "declare exactly what the mapper can emit". d23 declares no
        // optional and no unknown-capable field anywhere: nothing in the payload
        // is inapplicable to its own row, and nothing survives a failed read
        // (a failed read badges the whole record and folds nothing,
        // `attach_log.rs:58647`, `world_fold.rs::fold_map_detail`). The one empty state
        // that exists is `provinces.state_id` = Null.
        let s = schema();
        let mut nullable = Vec::new();
        for c in &s.collections {
            assert_eq!(c.optional_count(), 0, "collection {}", c.collection_id);
            for f in c.value_fields() {
                assert!(
                    !f.unknown_capable,
                    "collection {} field {}",
                    c.collection_id, f.field_id
                );
                if f.nullable {
                    nullable.push((c.collection_id, f.field_id));
                }
            }
        }
        assert_eq!(nullable, vec![(PROVINCES, field::PROVINCE_STATE_ID)]);
    }

    #[test]
    fn no_field_is_derived() {
        // derived-field contract derived fields are clock-linear integers; nothing in d23 changes
        // deterministically with the clock, so `frame_tick` never enters a value.
        for c in &schema().collections {
            assert!(c.fields.iter().all(|f| f.derived.is_none()));
        }
    }

    // --- step 8.4: quantization --------------------------------------------

    #[test]
    fn the_one_continuous_field_is_exact() {
        // Step 4's "quantization saves bytes" test does not apply: d23 declares
        // no quantized field. derived-field contract makes `exact` the default and requires the
        // domain evidence to justify anything else; none justifies rounding a
        // resource amount. Building levels are integers and are not continuous
        // at all, so no quantization question arises for them.
        let s = schema();
        let continuous: Vec<(u32, u32)> = s
            .collections
            .iter()
            .flat_map(|c| {
                c.value_fields()
                    .filter(|f| matches!(f.kind, Kind::Continuous))
                    .map(move |f| (c.collection_id, f.field_id))
            })
            .collect();
        assert_eq!(continuous, vec![(STATE_RESOURCES, field::RESOURCE_AMOUNT)]);
        for c in &s.collections {
            for f in c.value_fields() {
                if matches!(f.kind, Kind::Continuous) {
                    assert_eq!(f.quant, Quant::Exact);
                } else {
                    assert!(!f.quant.is_quantized());
                }
            }
        }
    }

    // --- step 8.2: byte-identical round trip --------------------------------

    #[test]
    fn a_full_payload_roundtrips_byte_for_byte() {
        let schema = schema();
        let acc = folded(&full_payload(), false);
        let mut bytes = Vec::new();
        encode_baseline(&schema, acc.as_slice(), &mut bytes).expect("baseline encodes");
        let (back, consumed) = decode_baseline(&schema, &bytes, 0).expect("decodes");
        assert_eq!(consumed, bytes.len(), "consumes exactly its own bytes");
        assert_eq!(back, acc.as_slice().to_vec(), "decode reproduces the state");
        let mut reencoded = Vec::new();
        encode_baseline(&schema, &back, &mut reencoded).expect("re-encodes");
        assert_eq!(reencoded, bytes, "decode/encode is byte-identical");
    }

    #[test]
    fn every_record_roundtrips_byte_for_byte() {
        // The per-record form of the same gate, so a defect in one collection's
        // record layout cannot hide behind another collection's bytes.
        let schema = schema();
        let acc = folded(&full_payload(), false);
        for (index, c) in schema.collections.iter().enumerate() {
            let state = acc.get(index);
            assert!(!state.is_empty(), "collection {} has rows", c.collection_id);
            for record in state.values() {
                let mut bytes = Vec::new();
                record.encode(c, &mut bytes).expect("encodes");
                let (back, pos) = Record::decode(c, &bytes, 0).expect("decodes");
                assert_eq!(pos, bytes.len());
                assert_eq!(&back, record);
                let mut reencoded = Vec::new();
                back.encode(c, &mut reencoded).expect("re-encodes");
                assert_eq!(reencoded, bytes, "collection {}", c.collection_id);
            }
        }
    }

    // --- step 8.3: states through the mapper, from raw bytes ----------------

    #[test]
    fn the_state_mapper_reports_the_rows_own_capture_flags() {
        let c = collection(STATES_INDEX);
        for (flags, resources, buildings) in [
            (0u32, false, false),
            (STATE_FLAG_RESOURCES, true, false),
            (STATE_FLAG_BUILDINGS, false, true),
            (STATE_KNOWN_FLAGS, true, true),
        ] {
            let row = state_row(7, flags, [0; 4], 0);
            let r = record_from_state_row(&c, &row, 0).expect("maps");
            assert_eq!(
                r.values[index_of(&c, field::RESOURCES_CAPTURED)],
                Value::Bool(resources),
                "flags {flags:#x}"
            );
            assert_eq!(
                r.values[index_of(&c, field::BUILDINGS_CAPTURED)],
                Value::Bool(buildings),
                "flags {flags:#x}"
            );
        }
    }

    #[test]
    fn a_province_without_a_state_maps_to_null_not_zero() {
        // The one empty state this domain has, from raw bytes. Null is not
        // Unsigned(0), not Absent and not Unknown, and the distinction is what
        // separates "this province is in no state" from "state number zero".
        let c = collection(PROVINCES_INDEX);
        let stateless = province_row(12, 0, PROVINCE_FLAG_BUILDINGS, [0, 0]);
        let r = record_from_province_row(&c, &stateless, 0).expect("maps");
        assert_eq!(r.values[index_of(&c, field::PROVINCE_STATE_ID)], Value::Null);
        assert!(r.values[index_of(&c, field::PROVINCE_STATE_ID)].is_null());
        assert_ne!(
            r.values[index_of(&c, field::PROVINCE_STATE_ID)],
            Value::ContentId(0)
        );

        let owned = province_row(11, 7, PROVINCE_FLAG_STATE | PROVINCE_FLAG_BUILDINGS, [0, 0]);
        let r = record_from_province_row(&c, &owned, 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::PROVINCE_STATE_ID)],
            Value::ContentId(7)
        );
    }

    #[test]
    fn a_null_state_survives_a_roundtrip() {
        let c = collection(PROVINCES_INDEX);
        let row = province_row(12, 0, PROVINCE_FLAG_BUILDINGS, [0, 0]);
        let record = record_from_province_row(&c, &row, 0).expect("maps");
        let mut bytes = Vec::new();
        record.encode(&c, &mut bytes).expect("encodes");
        let (back, pos) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(pos, bytes.len());
        assert_eq!(
            back.values[index_of(&c, field::PROVINCE_STATE_ID)],
            Value::Null
        );
    }

    #[test]
    fn a_real_zero_is_a_value_not_an_empty_state() {
        // A building at level 0 and a resource amount of exactly 0 are measured
        // values. Level 0 entries occur 19 distinct levels' worth in the real
        // capture, so this is not a hypothetical.
        let c = collection(STATE_BUILDINGS_INDEX);
        let r = record_from_state_building_row(&c, &entry(0x4E22, 0, 0), 0).expect("maps");
        assert_eq!(r.values[index_of(&c, field::BUILDING_LEVEL)], Value::Unsigned(0));
        assert!(r.values[index_of(&c, field::BUILDING_LEVEL)].carries_value());

        let c = collection(STATE_RESOURCES_INDEX);
        let r = record_from_state_resource_row(&c, &entry(0x3E03, 0, 0), 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::RESOURCE_AMOUNT)],
            Value::Continuous(0.0)
        );

        // And a state whose lists were captured and are empty is not a state
        // whose lists were never captured: both flags set, no child rows.
        let acc = folded(&full_payload(), false);
        let states = collection(STATES_INDEX);
        let empty = acc.get(STATES_INDEX).get(&key(&[9])).expect("state 9");
        assert_eq!(
            empty.values[index_of(&states, field::RESOURCES_CAPTURED)],
            Value::Bool(true)
        );
        assert!(acc
            .get(STATE_RESOURCES_INDEX)
            .keys()
            .all(|k| k.first() != 9));
    }

    // --- step 8.6: every scale factor ---------------------------------------

    #[test]
    fn resource_amounts_are_scaled_and_building_levels_are_not() {
        // The test that catches the domain's scale trap: one entry shape, two
        // scales. A level read through FIXED_SCALE would be wrong by 100,000x
        // and every round-trip test would still pass.
        let resources = collection(STATE_RESOURCES_INDEX);
        let amount = index_of(&resources, field::RESOURCE_AMOUNT);
        for (raw, expected) in [
            (100_000i32, 1.0f64),
            (0, 0.0),
            (400_000, 4.0),
            (-150_000, -1.5),
            (1, 1.0 / 100_000.0),
            (-1, -1.0 / 100_000.0),
            (2_100_000, 21.0),
            (i32::MAX, i32::MAX as f64 / 100_000.0),
            (i32::MIN, i32::MIN as f64 / 100_000.0),
        ] {
            let r = record_from_state_resource_row(&resources, &entry(0x3E03, raw, 0), 0)
                .expect("maps");
            assert_eq!(
                r.values[amount],
                Value::Continuous(expected),
                "raw {raw} must read {expected} (scale {RESOURCE_AMOUNT_SCALE})"
            );
        }

        for (index, map) in [
            (
                STATE_BUILDINGS_INDEX,
                record_from_state_building_row
                    as fn(&Collection, &[u8], u64) -> core::result::Result<Record, String>,
            ),
            (PROVINCE_BUILDINGS_INDEX, record_from_province_building_row),
        ] {
            let c = collection(index);
            let level = index_of(&c, field::BUILDING_LEVEL);
            for raw in [0u32, 1, 5, 19, 100_000, u32::MAX] {
                let r = map(&c, &entry(0x4CC1, raw as i32, 0), 0).expect("maps");
                assert_eq!(
                    r.values[level],
                    Value::Unsigned(raw as u64),
                    "a building level is emitted raw (world_fold.rs:29946)"
                );
            }
        }
    }

    // --- the payload splitter -----------------------------------------------

    #[test]
    fn split_payload_enforces_the_folds_structural_rules() {
        let payload = full_payload();
        let (header, tables) = split_payload(&payload).expect("well-formed");
        assert_eq!(header.state_count, 2);
        assert_eq!(header.province_count, 2);
        assert_eq!(header.resource_count, 2);
        assert_eq!(header.state_building_count, 2);
        assert_eq!(header.province_building_count, 1);
        assert_eq!(tables.states.len(), 2 * STATE_ROW_LEN);
        assert_eq!(tables.provinces.len(), 2 * PROVINCE_ROW_LEN);
        assert_eq!(tables.resources.len(), 2 * ENTRY_ROW_LEN);
        assert_eq!(tables.state_buildings.len(), 2 * ENTRY_ROW_LEN);
        assert_eq!(tables.province_buildings.len(), ENTRY_ROW_LEN);

        // Short of the header, at every length.
        for len in 0..HEADER_LEN {
            assert!(split_payload(&payload[..len]).is_err(), "len {len}");
        }
        // A length that does not match the counts, in both directions.
        let mut short = payload.clone();
        short.pop();
        assert!(split_payload(&short).is_err());
        let mut long = payload.clone();
        long.push(0);
        assert!(split_payload(&long).is_err());
        // A nonzero header reserved word (`world_fold.rs::fold_map_detail`).
        let mut reserved = payload.clone();
        reserved[44..48].copy_from_slice(&1u32.to_le_bytes());
        assert!(split_payload(&reserved).is_err());
        // A count whose byte length overflows `usize` must be rejected, not
        // wrapped into a plausible small length. Every one of the five counts.
        for offset in [8usize, 20, 32, 36, 40] {
            let mut huge = payload.clone();
            huge[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            assert!(split_payload(&huge).is_err(), "count at +{offset}");
        }
        // An empty payload — 48 bytes, five empty tables — is the shape 858 of
        // the measured capture's 903 records have, and it is well-formed.
        let empty = raw_payload(0, &[], &[]);
        assert_eq!(empty.len(), HEADER_LEN);
        let (header, tables) = split_payload(&empty).expect("an empty frame is well-formed");
        assert_eq!(header.state_count, 0);
        assert!(tables.states.is_empty());
    }

    #[test]
    fn split_payload_does_not_constrain_changed_rows() {
        // The fold never reads `changed_rows` (it is absent from
        // `fold_map_detail` entirely), and a full snapshot emits rows it did not
        // count as changed. A check here would refuse frames the recorder folds.
        let mut payload = full_payload();
        payload[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        let (header, _) = split_payload(&payload).expect("accepted");
        assert_eq!(header.changed_rows, u32::MAX);
    }

    // --- the row rejections the fold makes ----------------------------------

    #[test]
    fn the_state_mapper_rejects_every_row_the_fold_rejects() {
        let c = collection(STATES_INDEX);
        for len in [0usize, 1, STATE_ROW_LEN - 1, STATE_ROW_LEN + 1, 48] {
            let row = vec![0u8; len];
            assert!(record_from_state_row(&c, &row, 0).is_err(), "length {len}");
            assert!(key_from_state_row(&row).is_err(), "length {len}");
        }
        // Zero state_id (`world_fold.rs::fold_map_detail`).
        assert!(record_from_state_row(&c, &state_row(0, STATE_KNOWN_FLAGS, [0; 4], 0), 0).is_err());
        // Every unknown flag bit (`:22374`), not one sample.
        for bit in 2..32u32 {
            let row = state_row(7, STATE_KNOWN_FLAGS | (1 << bit), [0; 4], 0);
            assert!(
                record_from_state_row(&c, &row, 0).is_err(),
                "state flag bit {bit}"
            );
        }
        // A nonzero row reserved word (`:22380`).
        assert!(record_from_state_row(&c, &state_row(7, 0, [0; 4], 1), 0).is_err());
    }

    #[test]
    fn the_province_mapper_rejects_every_row_the_fold_rejects() {
        let c = collection(PROVINCES_INDEX);
        for len in [0usize, 1, PROVINCE_ROW_LEN - 1, PROVINCE_ROW_LEN + 1, 28] {
            let row = vec![0u8; len];
            assert!(record_from_province_row(&c, &row, 0).is_err(), "length {len}");
            assert!(key_from_province_row(&row).is_err(), "length {len}");
        }
        // Zero province_id (`world_fold.rs::fold_map_detail`).
        assert!(record_from_province_row(&c, &province_row(0, 0, 0, [0, 0]), 0).is_err());
        for bit in 2..32u32 {
            let row = province_row(11, 7, PROVINCE_KNOWN_FLAGS | (1 << bit), [0, 0]);
            assert!(
                record_from_province_row(&c, &row, 0).is_err(),
                "province flag bit {bit}"
            );
        }
        // The state flag and the state id must agree BOTH ways
        // (`:22423`-`:22432`).
        assert!(
            record_from_province_row(&c, &province_row(11, 0, PROVINCE_FLAG_STATE, [0, 0]), 0)
                .is_err(),
            "state flag without a state"
        );
        assert!(
            record_from_province_row(&c, &province_row(11, 7, 0, [0, 0]), 0).is_err(),
            "a state without its flag"
        );
    }

    #[test]
    fn the_entry_mappers_reject_every_entry_the_fold_rejects() {
        let resources = collection(STATE_RESOURCES_INDEX);
        let buildings = collection(STATE_BUILDINGS_INDEX);
        for len in [0usize, 1, ENTRY_ROW_LEN - 1, ENTRY_ROW_LEN + 1] {
            let e = vec![0u8; len];
            assert!(record_from_state_resource_row(&resources, &e, 0).is_err());
            assert!(record_from_state_building_row(&buildings, &e, 0).is_err());
            assert!(record_from_province_building_row(&buildings, &e, 0).is_err());
        }
        // Zero type_id, both kinds (`:22451`, `:30019`).
        assert!(record_from_state_resource_row(&resources, &entry(0, 1, 0), 0).is_err());
        assert!(record_from_state_building_row(&buildings, &entry(0, 1, 0), 0).is_err());
        // `u32::MAX` is rejected for a RESOURCE and accepted for a BUILDING —
        // the fold's own asymmetry (`:22451` vs `:30019`), mirrored exactly.
        assert!(record_from_state_resource_row(&resources, &entry(u32::MAX, 1, 0), 0).is_err());
        assert!(record_from_state_building_row(&buildings, &entry(u32::MAX, 1, 0), 0).is_ok());
        // A nonzero entry reserved word (`:22454`, `:30022`).
        assert!(record_from_state_resource_row(&resources, &entry(0x3E03, 1, 1), 0).is_err());
        assert!(record_from_state_building_row(&buildings, &entry(0x4E22, 1, 1), 0).is_err());
    }

    #[test]
    fn a_range_outside_its_entry_table_is_rejected() {
        // `validate_map_detail_range` (`world_fold.rs::validate_map_detail_range`), both halves: a
        // flagged range must fit, and a clear flag forces an empty range.
        let payload = full_payload();
        let (header, _) = split_payload(&payload).expect("well-formed");
        assert_eq!(header.resource_count, 2);
        // first + len past the table.
        let row = state_row(7, STATE_FLAG_RESOURCES, [2, 1, 0, 0], 0);
        assert!(state_row_ranges(&header, &row).is_err());
        let row = state_row(7, STATE_FLAG_RESOURCES, [0, 3, 0, 0], 0);
        assert!(state_row_ranges(&header, &row).is_err());
        // Exactly at the end is legal, and so is an empty range at the end.
        let row = state_row(7, STATE_FLAG_RESOURCES, [1, 1, 0, 0], 0);
        assert_eq!(
            state_row_ranges(&header, &row).expect("fits").0,
            Some(1..2)
        );
        let row = state_row(7, STATE_FLAG_RESOURCES, [2, 0, 0, 0], 0);
        assert_eq!(
            state_row_ranges(&header, &row).expect("fits").0,
            Some(2..2)
        );
        // A clear flag with a non-empty or non-zero range.
        for ranges in [[1u32, 0, 0, 0], [0, 1, 0, 0]] {
            let row = state_row(7, 0, ranges, 0);
            assert!(state_row_ranges(&header, &row).is_err(), "{ranges:?}");
        }
        for range in [[1u32, 0], [0, 1]] {
            let row = province_row(11, 7, PROVINCE_FLAG_STATE, range);
            assert!(province_row_range(&header, &row).is_err(), "{range:?}");
        }
        // A range whose end overflows `usize` is refused, not wrapped.
        let row = state_row(7, STATE_FLAG_RESOURCES, [u32::MAX, u32::MAX, 0, 0], 0);
        assert!(state_row_ranges(&header, &row).is_err());
    }

    #[test]
    fn the_mappers_refuse_a_collection_they_cannot_fill() {
        let mut c = collection(STATES_INDEX);
        c.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_state_row(&c, &state_row(7, 0, [0; 4], 0), 0).is_err());

        let mut c = collection(PROVINCES_INDEX);
        c.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_province_row(&c, &province_row(11, 0, 0, [0, 0]), 0).is_err());

        let mut c = collection(STATE_RESOURCES_INDEX);
        c.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_state_resource_row(&c, &entry(0x3E03, 1, 0), 0).is_err());

        let mut c = collection(STATE_BUILDINGS_INDEX);
        c.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_state_building_row(&c, &entry(0x4E22, 1, 0), 0).is_err());
        assert!(record_from_province_building_row(&c, &entry(0x4E22, 1, 0), 0).is_err());
    }

    // --- the keys -----------------------------------------------------------

    #[test]
    fn the_keys_are_the_ids_and_the_type_ids() {
        assert_eq!(
            key_from_state_row(&state_row(7, 0, [0; 4], 0)).expect("keys"),
            key(&[7])
        );
        assert_eq!(
            key_from_province_row(&province_row(11, 7, PROVINCE_FLAG_STATE, [0, 0])).expect("keys"),
            key(&[11])
        );
        assert_eq!(
            key_from_state_resource_row(7, &entry(0x3E03, 0, 0)).expect("keys"),
            key(&[7, 0x3E03])
        );
        assert_eq!(
            key_from_state_building_row(7, &entry(0x4E22, 0, 0)).expect("keys"),
            key(&[7, 0x4E22])
        );
        assert_eq!(
            key_from_province_building_row(11, &entry(0x4CC1, 0, 0)).expect("keys"),
            key(&[11, 0x4CC1])
        );
        // An entry the fold rejects has no key either.
        assert!(key_from_state_resource_row(7, &entry(0, 0, 0)).is_err());
        assert!(key_from_state_building_row(7, &[]).is_err());
    }

    // --- the fold -----------------------------------------------------------

    #[test]
    fn a_full_snapshot_flattens_every_array_into_its_collection() {
        let acc = folded(&full_payload(), false);
        let states = collection(STATES_INDEX);
        let provinces = collection(PROVINCES_INDEX);
        let resources = collection(STATE_RESOURCES_INDEX);
        let state_buildings = collection(STATE_BUILDINGS_INDEX);
        let province_buildings = collection(PROVINCE_BUILDINGS_INDEX);

        assert_eq!(acc.get(STATES_INDEX).len(), 2);
        assert_eq!(acc.get(PROVINCES_INDEX).len(), 2);
        assert_eq!(acc.get(STATE_RESOURCES_INDEX).len(), 2);
        assert_eq!(acc.get(STATE_BUILDINGS_INDEX).len(), 2);
        assert_eq!(acc.get(PROVINCE_BUILDINGS_INDEX).len(), 1);
        assert_eq!(acc.rows(), 9);

        // The entries landed under their own parent, not under the array index.
        assert_eq!(
            acc.get(STATE_RESOURCES_INDEX)[&key(&[7, 0x3E03])].values
                [index_of(&resources, field::RESOURCE_AMOUNT)],
            Value::Continuous(4.0)
        );
        assert_eq!(
            acc.get(STATE_RESOURCES_INDEX)[&key(&[7, 0x4E37])].values
                [index_of(&resources, field::RESOURCE_AMOUNT)],
            Value::Continuous(-1.5)
        );
        assert_eq!(
            acc.get(STATE_BUILDINGS_INDEX)[&key(&[7, 0x2FAB])].values
                [index_of(&state_buildings, field::BUILDING_LEVEL)],
            Value::Unsigned(3)
        );
        assert_eq!(
            acc.get(PROVINCE_BUILDINGS_INDEX)[&key(&[11, 0x4CC1])].values
                [index_of(&province_buildings, field::BUILDING_LEVEL)],
            Value::Unsigned(5)
        );
        assert_eq!(
            acc.get(PROVINCES_INDEX)[&key(&[11])].values
                [index_of(&provinces, field::PROVINCE_STATE_ID)],
            Value::ContentId(7)
        );
        assert_eq!(
            acc.get(PROVINCES_INDEX)[&key(&[12])].values
                [index_of(&provinces, field::PROVINCE_STATE_ID)],
            Value::Null
        );
        assert_eq!(
            acc.get(STATES_INDEX)[&key(&[7])].values[index_of(&states, field::BUILDINGS_CAPTURED)],
            Value::Bool(true)
        );
    }

    #[test]
    fn a_full_snapshot_drops_everything_the_domain_owns() {
        // `world_fold.rs::fold_map_detail`. A state, a province and every entry
        // that the new snapshot does not mention must be gone, not merged.
        let schema = schema();
        let mut acc = MapDetailState::new();
        fold_payload(&schema, &mut acc, &full_payload(), false, 0).expect("folds");
        let second = raw_payload(
            0,
            &[RawState {
                state_id: 99,
                resources: Some(vec![(0x4E25, 300_000)]),
                buildings: None,
            }],
            &[],
        );
        fold_payload(&schema, &mut acc, &second, false, 0).expect("folds");
        assert_eq!(acc.get(STATES_INDEX).len(), 1);
        assert!(acc.get(STATES_INDEX).contains_key(&key(&[99])));
        assert!(acc.get(PROVINCES_INDEX).is_empty());
        assert!(acc.get(PROVINCE_BUILDINGS_INDEX).is_empty());
        assert!(acc.get(STATE_BUILDINGS_INDEX).is_empty());
        assert_eq!(acc.get(STATE_RESOURCES_INDEX).len(), 1);
        assert!(acc
            .get(STATE_RESOURCES_INDEX)
            .contains_key(&key(&[99, 0x4E25])));
    }

    #[test]
    fn a_delta_upserts_and_never_deletes_a_row_it_does_not_mention() {
        let schema = schema();
        let mut acc = MapDetailState::new();
        fold_payload(&schema, &mut acc, &full_payload(), false, 0).expect("folds");
        let before = acc.clone();
        // A delta touching only state 7's buildings.
        let delta = raw_payload(
            1 << 10,
            &[RawState {
                state_id: 7,
                resources: None,
                buildings: Some(vec![(0x2FAB, 4)]),
            }],
            &[],
        );
        fold_payload(&schema, &mut acc, &delta, true, 0).expect("folds");

        // State 9, both provinces and the province building are untouched.
        assert_eq!(acc.get(STATES_INDEX).len(), 2);
        assert_eq!(acc.get(PROVINCES_INDEX), before.get(PROVINCES_INDEX));
        assert_eq!(
            acc.get(PROVINCE_BUILDINGS_INDEX),
            before.get(PROVINCE_BUILDINGS_INDEX)
        );
        // State 7's resources are untouched: the row said nothing about them.
        assert_eq!(
            acc.get(STATE_RESOURCES_INDEX),
            before.get(STATE_RESOURCES_INDEX)
        );
        // Its building list was carried whole, so the entry the delta did not
        // repeat is gone and the one it did carry has the new level.
        let state_buildings = collection(STATE_BUILDINGS_INDEX);
        assert_eq!(acc.get(STATE_BUILDINGS_INDEX).len(), 1);
        assert_eq!(
            acc.get(STATE_BUILDINGS_INDEX)[&key(&[7, 0x2FAB])].values
                [index_of(&state_buildings, field::BUILDING_LEVEL)],
            Value::Unsigned(4)
        );
    }

    #[test]
    fn a_delta_row_with_a_clear_flag_retains_the_accumulated_capture_flag() {
        // `world_fold.rs::fold_map_detail`: a clear flag does not insert, so the
        // accumulated answer stands. Downgrading `captured` to false here would
        // assert "never read" about data that was read an hour ago.
        let schema = schema();
        let mut acc = MapDetailState::new();
        fold_payload(&schema, &mut acc, &full_payload(), false, 0).expect("folds");
        let states = collection(STATES_INDEX);
        let delta = raw_payload(
            1 << 10,
            &[RawState {
                state_id: 7,
                resources: None,
                buildings: None,
            }],
            &[RawProvince {
                province_id: 11,
                state_id: 7,
                buildings: None,
            }],
        );
        fold_payload(&schema, &mut acc, &delta, true, 0).expect("folds");
        let row = &acc.get(STATES_INDEX)[&key(&[7])];
        assert_eq!(
            row.values[index_of(&states, field::RESOURCES_CAPTURED)],
            Value::Bool(true)
        );
        assert_eq!(
            row.values[index_of(&states, field::BUILDINGS_CAPTURED)],
            Value::Bool(true)
        );
        let provinces = collection(PROVINCES_INDEX);
        let row = &acc.get(PROVINCES_INDEX)[&key(&[11])];
        assert_eq!(
            row.values[index_of(&provinces, field::BUILDINGS_CAPTURED)],
            Value::Bool(true)
        );
        assert_eq!(
            row.values[index_of(&provinces, field::PROVINCE_STATE_ID)],
            Value::ContentId(7)
        );

        // A state the accumulator has never seen has nothing to retain: a clear
        // flag on a new row is `false`, not a borrowed `true`.
        let delta = raw_payload(
            1 << 10,
            &[RawState {
                state_id: 42,
                resources: None,
                buildings: None,
            }],
            &[],
        );
        fold_payload(&schema, &mut acc, &delta, true, 0).expect("folds");
        let row = &acc.get(STATES_INDEX)[&key(&[42])];
        assert_eq!(
            row.values[index_of(&states, field::RESOURCES_CAPTURED)],
            Value::Bool(false)
        );
        assert_eq!(
            row.values[index_of(&states, field::BUILDINGS_CAPTURED)],
            Value::Bool(false)
        );
    }

    #[test]
    fn a_rejected_payload_leaves_the_accumulator_untouched() {
        let schema = schema();
        let mut acc = MapDetailState::new();
        fold_payload(&schema, &mut acc, &full_payload(), false, 0).expect("folds");
        let before = acc.clone();
        // A frame whose SECOND state row is invalid: a fold that applied rows as
        // it read them would have written the first one already.
        let mut payload = raw_payload(
            0,
            &[
                RawState {
                    state_id: 5,
                    resources: Some(vec![(0x3E03, 100_000)]),
                    buildings: None,
                },
                RawState {
                    state_id: 6,
                    resources: None,
                    buildings: None,
                },
            ],
            &[],
        );
        // Zero the second row's state_id (`world_fold.rs::fold_map_detail`).
        let second = HEADER_LEN + STATE_ROW_LEN;
        payload[second..second + 4].copy_from_slice(&0u32.to_le_bytes());
        assert!(fold_payload(&schema, &mut acc, &payload, false, 0).is_err());
        assert_eq!(acc, before, "a refused frame changes nothing");
    }

    #[test]
    fn a_repeated_id_in_one_frame_folds_in_payload_order() {
        // `fold_map_detail` has NO duplicate rejection: a second row for one
        // `state_id` overwrites the first in its JSON map (`:22480`), and a
        // repeated `type_id` inside one range overwrites in
        // `map_detail_resource_map` (`:30028`). Refusing such a frame would
        // refuse one the shipping recorder folds, so this reproduces it.
        let schema = schema();
        let resources = collection(STATE_RESOURCES_INDEX);
        let amount = index_of(&resources, field::RESOURCE_AMOUNT);
        let states = collection(STATES_INDEX);

        // Two rows for one state: the later row's list is the state's list.
        let mut acc = MapDetailState::new();
        let payload = raw_payload(
            0,
            &[
                RawState {
                    state_id: 7,
                    resources: Some(vec![(0x3E03, 100_000)]),
                    buildings: None,
                },
                RawState {
                    state_id: 7,
                    resources: Some(vec![(0x4E25, 200_000)]),
                    buildings: None,
                },
            ],
            &[],
        );
        fold_payload(&schema, &mut acc, &payload, false, 0).expect("the recorder folds this");
        assert_eq!(acc.get(STATES_INDEX).len(), 1);
        let keys: Vec<Key> = acc.get(STATE_RESOURCES_INDEX).keys().cloned().collect();
        assert_eq!(keys, vec![key(&[7, 0x4E25])], "the later row's list wins");

        // A second row whose flag is CLEAR does not insert, so the first row's
        // list survives — exactly what the fold does by not inserting.
        let mut acc = MapDetailState::new();
        let payload = raw_payload(
            0,
            &[
                RawState {
                    state_id: 7,
                    resources: Some(vec![(0x3E03, 100_000)]),
                    buildings: None,
                },
                RawState {
                    state_id: 7,
                    resources: None,
                    buildings: None,
                },
            ],
            &[],
        );
        fold_payload(&schema, &mut acc, &payload, false, 0).expect("folds");
        assert_eq!(
            acc.get(STATE_RESOURCES_INDEX).keys().cloned().collect::<Vec<_>>(),
            vec![key(&[7, 0x3E03])]
        );
        assert_eq!(
            acc.get(STATES_INDEX)[&key(&[7])].values[index_of(&states, field::RESOURCES_CAPTURED)],
            Value::Bool(true)
        );

        // Two rows for one province: the later state and building list win.
        let mut acc = MapDetailState::new();
        let payload = raw_payload(
            0,
            &[],
            &[
                RawProvince { province_id: 11, state_id: 5, buildings: Some(vec![(0x4CC1, 1)]) },
                RawProvince { province_id: 11, state_id: 6, buildings: Some(vec![(0x4CC1, 2)]) },
            ],
        );
        fold_payload(&schema, &mut acc, &payload, false, 0).expect("folds");
        let provinces = collection(PROVINCES_INDEX);
        assert_eq!(acc.get(PROVINCES_INDEX).len(), 1);
        assert_eq!(
            acc.get(PROVINCES_INDEX)[&key(&[11])].values
                [index_of(&provinces, field::PROVINCE_STATE_ID)],
            Value::ContentId(6)
        );
        let province_buildings = collection(PROVINCE_BUILDINGS_INDEX);
        assert_eq!(
            acc.get(PROVINCE_BUILDINGS_INDEX)[&key(&[11, 0x4CC1])].values
                [index_of(&province_buildings, field::BUILDING_LEVEL)],
            Value::Unsigned(2)
        );

        // A type repeated inside ONE parent's range: last wins, as the fold's
        // map does.
        let mut acc = MapDetailState::new();
        let payload = raw_payload(
            0,
            &[RawState {
                state_id: 7,
                resources: Some(vec![(0x3E03, 100_000), (0x3E03, 300_000)]),
                buildings: None,
            }],
            &[],
        );
        fold_payload(&schema, &mut acc, &payload, false, 0).expect("the recorder folds this");
        assert_eq!(acc.get(STATE_RESOURCES_INDEX).len(), 1);
        assert_eq!(
            acc.get(STATE_RESOURCES_INDEX)[&key(&[7, 0x3E03])].values[amount],
            Value::Continuous(3.0),
            "the later entry in the range wins"
        );
    }

    #[test]
    fn an_entry_no_range_references_is_still_checked() {
        // `fold_map_detail` decodes all three entry arrays in full
        // (`:22443`-`:22466`) before it applies a single range, so an entry with
        // a zero `type_id` or a nonzero reserved word invalidates the frame even
        // when no row addresses it. Checking only the addressed entries would
        // accept frames the recorder refuses.
        let schema = schema();
        // One state addressing entry 0 of each array; entry 1 of each array is
        // present in the header's count but referenced by nobody.
        let build = |patch: &dyn Fn(&mut Vec<u8>, usize)| -> Vec<u8> {
            let mut payload = raw_payload(
                0,
                &[
                    RawState {
                        state_id: 7,
                        resources: Some(vec![(0x3E03, 1), (0x4E25, 2)]),
                        buildings: Some(vec![(0x2FAB, 1), (0x4E22, 2)]),
                    },
                ],
                &[RawProvince {
                    province_id: 11,
                    state_id: 7,
                    buildings: Some(vec![(0x4CC1, 1), (0x31F6, 2)]),
                }],
            );
            // Shorten every range to its first entry, leaving the second
            // unreferenced but still declared by the header counts.
            let s = HEADER_LEN;
            payload[s + 12..s + 16].copy_from_slice(&1u32.to_le_bytes()); // resource_len
            payload[s + 20..s + 24].copy_from_slice(&1u32.to_le_bytes()); // building_len
            let p = HEADER_LEN + STATE_ROW_LEN;
            payload[p + 16..p + 20].copy_from_slice(&1u32.to_le_bytes()); // building_len
            let entries_at = HEADER_LEN + STATE_ROW_LEN + PROVINCE_ROW_LEN;
            patch(&mut payload, entries_at);
            payload
        };
        // Unpatched, the frame is well-formed and folds only the referenced
        // entries — the flattening must not pick up the unreferenced ones.
        let mut acc = MapDetailState::new();
        fold_payload(&schema, &mut acc, &build(&|_, _| {}), false, 0).expect("folds");
        assert_eq!(acc.get(STATE_RESOURCES_INDEX).len(), 1);
        assert_eq!(acc.get(STATE_BUILDINGS_INDEX).len(), 1);
        assert_eq!(acc.get(PROVINCE_BUILDINGS_INDEX).len(), 1);
        let before = acc.clone();

        // The second entry of each of the three arrays, invalidated two ways.
        // Array bases: resources at +0, state buildings at +2 entries, province
        // buildings at +4 entries; the unreferenced one is the second of each.
        for unreferenced in [1usize, 3, 5] {
            for (offset, word) in [(0usize, 0u32), (8, 1)] {
                let payload = build(&|p: &mut Vec<u8>, base: usize| {
                    let at = base + unreferenced * ENTRY_ROW_LEN + offset;
                    p[at..at + 4].copy_from_slice(&word.to_le_bytes());
                });
                let mut acc = before.clone();
                assert!(
                    fold_payload(&schema, &mut acc, &payload, false, 0).is_err(),
                    "unreferenced entry {unreferenced} patched at +{offset} must invalidate \
                     the frame"
                );
                assert_eq!(acc, before, "and must leave the accumulator alone");
            }
        }
    }

    #[test]
    fn the_default_accumulator_is_a_usable_one() {
        // `MapDetailState::default()` is public and idiomatic; a derived Default
        // would build an empty collection list, and every accessor and the fold
        // itself index the five collections.
        assert_eq!(MapDetailState::default(), MapDetailState::new());
        assert_eq!(MapDetailState::default().as_slice().len(), COLLECTION_COUNT);
        let schema = schema();
        let mut acc = MapDetailState::default();
        fold_payload(&schema, &mut acc, &full_payload(), false, 0).expect("folds");
        assert_eq!(acc.rows(), 9);
        // And an accumulator of the wrong shape is diagnosed, never indexed.
        let mut wrong = MapDetailState::new();
        wrong.collections.pop();
        assert!(fold_payload(&schema, &mut wrong, &full_payload(), false, 0).is_err());
    }

    #[test]
    fn overlapping_ranges_land_under_their_own_parents() {
        // Only `first + len <= total` is checked (`world_fold.rs::validate_map_detail_range`), so two
        // states may legally address the same entries. Flattening must key them
        // by parent, not collapse them.
        let schema = schema();
        let mut acc = MapDetailState::new();
        let mut payload = raw_payload(
            0,
            &[
                RawState {
                    state_id: 7,
                    resources: Some(vec![(0x3E03, 100_000)]),
                    buildings: None,
                },
                RawState {
                    state_id: 8,
                    resources: Some(Vec::new()),
                    buildings: None,
                },
            ],
            &[],
        );
        // Point state 8 at state 7's single entry.
        let second = HEADER_LEN + STATE_ROW_LEN;
        payload[second + 8..second + 12].copy_from_slice(&0u32.to_le_bytes());
        payload[second + 12..second + 16].copy_from_slice(&1u32.to_le_bytes());
        fold_payload(&schema, &mut acc, &payload, false, 0).expect("folds");
        assert_eq!(acc.get(STATE_RESOURCES_INDEX).len(), 2);
        assert!(acc
            .get(STATE_RESOURCES_INDEX)
            .contains_key(&key(&[7, 0x3E03])));
        assert!(acc
            .get(STATE_RESOURCES_INDEX)
            .contains_key(&key(&[8, 0x3E03])));
    }

    #[test]
    fn replacing_one_parents_children_leaves_its_neighbours_alone() {
        // The range scan that clears a parent's rows must stop at that parent:
        // an off-by-one in the bound would delete the next state's entries.
        let schema = schema();
        let mut acc = MapDetailState::new();
        let payload = raw_payload(
            0,
            &[
                RawState { state_id: 6, resources: Some(vec![(0x3E03, 1)]), buildings: None },
                RawState { state_id: 7, resources: Some(vec![(0x3E03, 2)]), buildings: None },
                RawState { state_id: 8, resources: Some(vec![(0x3E03, 3)]), buildings: None },
            ],
            &[],
        );
        fold_payload(&schema, &mut acc, &payload, false, 0).expect("folds");
        let delta = raw_payload(
            1 << 10,
            &[RawState {
                state_id: 7,
                resources: Some(vec![(0x4E25, 9)]),
                buildings: None,
            }],
            &[],
        );
        fold_payload(&schema, &mut acc, &delta, true, 0).expect("folds");
        let keys: Vec<Key> = acc.get(STATE_RESOURCES_INDEX).keys().cloned().collect();
        assert_eq!(
            keys,
            vec![
                key(&[6, 0x3E03]),
                key(&[7, 0x4E25]),
                key(&[8, 0x3E03])
            ],
            "only state 7's rows were replaced"
        );
    }

    #[test]
    fn fold_payload_refuses_a_schema_that_is_not_this_domains() {
        let mut wrong = schema();
        wrong.collections.pop();
        let mut acc = MapDetailState::new();
        assert!(fold_payload(&wrong, &mut acc, &full_payload(), false, 0).is_err());
    }

    // --- the delta grammar, end to end --------------------------------------

    #[test]
    fn a_folded_hour_pair_encodes_as_a_delta_and_replays_exactly() {
        // The unit-scale form of the measurement bin's mandatory replay: encode
        // a baseline, then a delta, decode both against the reader's own
        // reconstructed prior, and require stored equality with the fold.
        let schema = schema();
        let mut acc = MapDetailState::new();
        fold_payload(&schema, &mut acc, &full_payload(), false, 0).expect("folds");
        let prior = acc.clone();

        let mut baseline = Vec::new();
        encode_baseline(&schema, prior.as_slice(), &mut baseline).expect("encodes");
        let (mut reader, consumed) = decode_baseline(&schema, &baseline, 0).expect("decodes");
        assert_eq!(consumed, baseline.len());

        let update = raw_payload(
            1 << 10,
            &[RawState {
                state_id: 7,
                resources: Some(vec![(0x3E03, 500_000)]),
                buildings: None,
            }],
            &[RawProvince {
                province_id: 12,
                state_id: 9,
                buildings: Some(vec![(0x4CC1, 2)]),
            }],
        );
        fold_payload(&schema, &mut acc, &update, true, 0).expect("folds");

        let deltas: Vec<Option<_>> = (0..COLLECTION_COUNT)
            .map(|i| {
                let d = diff(&schema.collections[i], prior.get(i), acc.get(i));
                (!d.is_empty()).then_some(d)
            })
            .collect();
        assert!(deltas[STATE_RESOURCES_INDEX].is_some(), "the amount moved");
        assert!(deltas[PROVINCES_INDEX].is_some(), "province 12 gained a state");
        assert!(
            deltas[STATE_BUILDINGS_INDEX].is_none(),
            "nothing touched the state buildings"
        );

        let mut bytes = Vec::new();
        encode_delta(&schema, prior.as_slice(), &deltas, &mut bytes).expect("encodes");
        let (decoded, consumed) = decode_delta(&schema, &reader, &bytes, 0).expect("decodes");
        assert_eq!(consumed, bytes.len());
        for (i, d) in decoded.iter().enumerate() {
            if let Some(d) = d {
                apply(&mut reader[i], d);
            }
        }
        for i in 0..COLLECTION_COUNT {
            assert_eq!(reader[i].len(), acc.get(i).len(), "collection {i} row count");
            assert!(
                diff(&schema.collections[i], &reader[i], acc.get(i)).is_empty(),
                "collection {i} is not stored-equal to the fold"
            );
        }
        // A province that gained a state is a Null -> ContentId transition, and
        // it has to survive the update path, not only the baseline path.
        let provinces = collection(PROVINCES_INDEX);
        assert_eq!(
            reader[PROVINCES_INDEX][&key(&[12])].values
                [index_of(&provinces, field::PROVINCE_STATE_ID)],
            Value::ContentId(9)
        );
    }

    #[test]
    fn fold_payload_never_panics_on_arbitrary_bytes() {
        // Total over hostile input: every public entry point reads bytes that
        // came off a disk, so they must diagnose rather than unwind.
        let schema = schema();
        let mut state = 0x2545_F491_4F6C_DD1Du64;
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
            let mut acc = MapDetailState::new();
            let _ = fold_payload(&schema, &mut acc, &bytes, state & 1 == 0, 0);
            let _ = split_payload(&bytes);
        }
        // And corruption of a genuinely well-formed payload, byte by byte.
        let good = full_payload();
        for i in 0..good.len() {
            let mut payload = good.clone();
            payload[i] ^= 0xFF;
            let mut acc = MapDetailState::new();
            let _ = fold_payload(&schema, &mut acc, &payload, false, 0);
        }
    }
}
