//! Step-8 tests for [`crate::d13`].
//!
//! Every state case goes through the **public mapper on raw row bytes**, never a
//! hand-built `Vec<Value>`: a hand-built record would pass with a wrong mapper,
//! which is the defect these tests exist to catch.

use super::*;
use crate::body::{apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta};
use crate::TickGrid;

fn grid() -> TickGrid {
    TickGrid::new(3).expect("3 is a legal grid")
}

fn countries() -> Collection {
    schema().collections[COUNTRIES_INDEX].clone()
}

fn active() -> Collection {
    schema().collections[ACTIVE_RESEARCH_INDEX].clone()
}

fn index_of(c: &Collection, id: u32) -> usize {
    c.value_fields()
        .position(|f| f.field_id == id)
        .expect("field is in the schema")
}

// --- Raw-byte builders, packed the way `attach_log.rs` packs them ------------

/// A 16-byte country row (`attach_log.rs:68858`-`:68871`).
fn country_row(tag: u32, flags: u32, completed: u32, slots: u32) -> Vec<u8> {
    let mut row = Vec::with_capacity(COUNTRY_ROW_LEN);
    row.extend_from_slice(&tag.to_le_bytes());
    row.extend_from_slice(&flags.to_le_bytes());
    row.extend_from_slice(&completed.to_le_bytes());
    row.extend_from_slice(&slots.to_le_bytes());
    assert_eq!(row.len(), COUNTRY_ROW_LEN);
    row
}

/// The row the collector packs for a country it could read completely.
fn full_country_row() -> Vec<u8> {
    country_row(
        7,
        COUNTRY_FLAG_COMPLETED_TECH_COUNT
            | COUNTRY_FLAG_RESEARCH_SLOT_COUNT
            | COUNTRY_FLAG_ACTIVE_RESEARCH,
        42,
        4,
    )
}

#[derive(Clone, Copy, Default)]
struct ActiveSpec {
    tag: u32,
    slot_index: u32,
    tech_handle_raw: u32,
    tech_def_index_raw: u32,
    progress_raw: u32,
    live_progress_raw: Option<i64>,
    base_required_points_raw: Option<i64>,
    tech_name: Option<&'static str>,
    quartet: Option<(i64, i64, i64, i64)>,
    a29: Option<A29Spec>,
}

#[derive(Clone, Copy)]
struct A29Spec {
    authority: u32,
    branch: u32,
    callback_sequence: u64,
    callback_thread_id: u32,
    producer_game_hour_raw: i32,
    callback_qpc_ticks: u64,
    qpc_frequency_hz: u64,
    source_record_sha256: [u8; 32],
    ui_action_gate_return_rva: u32,
    candidate_count: u32,
    selected_ordinal: u32,
    selected_game_hour_raw: i32,
    selected_candidate_id: u64,
    selected_qpc_ticks: u64,
    selection_cutoff_qpc_ticks: u64,
    candidate_set_sha256: [u8; 32],
    selected_present_receipt_sha256: [u8; 32],
}

const FRAME_HOUR: i32 = 60_832_407;

impl A29Spec {
    /// A block that satisfies every clause this crate can evaluate, at
    /// [`FRAME_HOUR`], for branch `2` (the reduced form, whose quartet shape rule
    /// is `reduction > 0`).
    fn valid() -> Self {
        Self {
            authority: A29_AUTHORITY,
            branch: 2,
            callback_sequence: 9,
            callback_thread_id: 1234,
            producer_game_hour_raw: FRAME_HOUR,
            callback_qpc_ticks: 500,
            qpc_frequency_hz: 10_000_000,
            source_record_sha256: [0xAB; 32],
            ui_action_gate_return_rva: A29_UI_YEAR_HELPER_RETURN_RVA,
            candidate_count: 3,
            selected_ordinal: 1,
            selected_game_hour_raw: FRAME_HOUR,
            selected_candidate_id: 77,
            selected_qpc_ticks: 600,
            selection_cutoff_qpc_ticks: 700,
            candidate_set_sha256: [0xCD; 32],
            selected_present_receipt_sha256: [0xEF; 32],
        }
    }

    fn bytes(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(A29_PROVENANCE_LEN);
        b.extend_from_slice(&self.authority.to_le_bytes());
        b.extend_from_slice(&self.branch.to_le_bytes());
        b.extend_from_slice(&self.callback_sequence.to_le_bytes());
        b.extend_from_slice(&self.callback_thread_id.to_le_bytes());
        b.extend_from_slice(&self.producer_game_hour_raw.to_le_bytes());
        b.extend_from_slice(&self.callback_qpc_ticks.to_le_bytes());
        b.extend_from_slice(&self.qpc_frequency_hz.to_le_bytes());
        b.extend_from_slice(&self.source_record_sha256);
        b.extend_from_slice(&self.ui_action_gate_return_rva.to_le_bytes());
        b.extend_from_slice(&self.candidate_count.to_le_bytes());
        b.extend_from_slice(&self.selected_ordinal.to_le_bytes());
        b.extend_from_slice(&self.selected_game_hour_raw.to_le_bytes());
        b.extend_from_slice(&self.selected_candidate_id.to_le_bytes());
        b.extend_from_slice(&self.selected_qpc_ticks.to_le_bytes());
        b.extend_from_slice(&self.selection_cutoff_qpc_ticks.to_le_bytes());
        b.extend_from_slice(&self.candidate_set_sha256);
        b.extend_from_slice(&self.selected_present_receipt_sha256);
        assert_eq!(b.len(), A29_PROVENANCE_LEN);
        b
    }
}

/// One active-research row, packed exactly as `attach_log.rs:68906`-`:68957`
/// packs it — including the `u16`-prefixed string and the tail order.
fn active_row(spec: &ActiveSpec) -> Vec<u8> {
    let mut flags = 0u32;
    if spec.live_progress_raw.is_some() {
        flags |= ACTIVE_FLAG_LIVE_PROGRESS;
    }
    if spec.base_required_points_raw.is_some() {
        flags |= ACTIVE_FLAG_BASE_REQUIRED_POINTS;
    }
    if spec.tech_name.is_some() {
        flags |= ACTIVE_FLAG_TECH_NAME;
    }
    if spec.quartet.is_some() {
        flags |= ACTIVE_FLAG_AHEAD_QUARTET;
    }
    if spec.a29.is_some() {
        flags |= ACTIVE_FLAG_A29_PROVENANCE;
    }
    let mut row = Vec::new();
    row.extend_from_slice(&spec.tag.to_le_bytes());
    row.extend_from_slice(&spec.slot_index.to_le_bytes());
    row.extend_from_slice(&spec.tech_handle_raw.to_le_bytes());
    row.extend_from_slice(&spec.tech_def_index_raw.to_le_bytes());
    row.extend_from_slice(&spec.progress_raw.to_le_bytes());
    row.extend_from_slice(&flags.to_le_bytes());
    row.extend_from_slice(&spec.live_progress_raw.unwrap_or(0).to_le_bytes());
    row.extend_from_slice(&spec.base_required_points_raw.unwrap_or(0).to_le_bytes());
    assert_eq!(row.len(), ACTIVE_ROW_FIXED_LEN);
    if let Some(name) = spec.tech_name {
        row.extend_from_slice(&(name.len() as u16).to_le_bytes());
        row.extend_from_slice(name.as_bytes());
    }
    if let Some((a, b, c, d)) = spec.quartet {
        for v in [a, b, c, d] {
            row.extend_from_slice(&v.to_le_bytes());
        }
    }
    if let Some(a29) = spec.a29 {
        row.extend_from_slice(&a29.bytes());
    }
    row
}

/// The row shape the reference capture actually carries: flags `0b11`, no tails.
fn measured_active_spec() -> ActiveSpec {
    ActiveSpec {
        tag: 7,
        slot_index: 2,
        tech_handle_raw: 0x1234,
        tech_def_index_raw: 0x1234,
        progress_raw: 305_000,
        live_progress_raw: Some(32_925_000),
        base_required_points_raw: Some(100_000),
        tech_name: None,
        quartet: None,
        a29: None,
    }
}

/// Every tail present, so every gate is set.
fn full_active_spec() -> ActiveSpec {
    ActiveSpec {
        tech_name: Some("engineers4"),
        // branch 2 needs `reduction > 0`; the other three are unconstrained.
        quartet: Some((88_000, 500_000, 25_000, 375_000)),
        a29: Some(A29Spec::valid()),
        ..measured_active_spec()
    }
}

/// A whole DATA payload from country rows and active rows.
fn payload(country_rows: &[Vec<u8>], active_rows: &[Vec<u8>]) -> Vec<u8> {
    payload_with(country_rows, active_rows, 0, |_| {})
}

fn payload_with(
    country_rows: &[Vec<u8>],
    active_rows: &[Vec<u8>],
    status_flags: u32,
    patch: impl Fn(&mut Vec<u8>),
) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
    p.extend_from_slice(&(country_rows.len() as u32).to_le_bytes());
    p.extend_from_slice(&(country_rows.len() as u32).to_le_bytes()); // changed_rows
    p.extend_from_slice(&status_flags.to_le_bytes());
    p.extend_from_slice(&TECH_PROGRESS_SCALE_RAW.to_le_bytes());
    p.extend_from_slice(&TECH_LIVE_SCALE_RAW.to_le_bytes());
    p.extend_from_slice(&(active_rows.len() as u32).to_le_bytes());
    assert_eq!(p.len(), HEADER_LEN);
    for row in country_rows {
        p.extend_from_slice(row);
    }
    for row in active_rows {
        p.extend_from_slice(row);
    }
    patch(&mut p);
    p
}

// --- 1. Schema validity ------------------------------------------------------

#[test]
fn the_schema_is_valid() {
    schema()
        .validate(grid().log2())
        .expect("d13 schema is well-formed");
    assert_eq!(schema().collections.len(), COLLECTION_COUNT);
}

// --- 5. Shape and key --------------------------------------------------------

#[test]
fn both_collections_are_keyed_sets_with_the_folds_own_identities() {
    let c = countries();
    assert_eq!(c.shape, Shape::KeyedSet, "baseline replaces, deltas upsert");
    let keys = c.key_fields();
    assert_eq!(keys.len(), 1, "a country row is keyed by its tag alone");
    assert_eq!(keys[0].field_id, field::TAG_DB_ID);
    assert_eq!(
        keys[0].kind,
        Kind::ContentId,
        "a country tag is content-defined (§5), so a manifest must resolve it"
    );
    assert!(keys[0].kind.legal_as_key());
    assert_eq!(c.fields.len(), 4, "1 key + 2 counts + 1 capture flag");

    let a = active();
    assert_eq!(a.shape, Shape::KeyedSet);
    let keys = a.key_fields();
    assert_eq!(
        keys.len(),
        2,
        "the fold keys an active row by (tag, slot_index) (`:13326`-`:13327`)"
    );
    assert_eq!(keys[0].field_id, field::TAG_DB_ID);
    assert_eq!(keys[0].kind, Kind::ContentId);
    assert_eq!(keys[1].field_id, field::SLOT_INDEX);
    assert_eq!(
        keys[1].kind,
        Kind::Unsigned,
        "a research slot ordinal is an engine position, not mod content"
    );
    assert!(keys.iter().all(|f| f.kind.legal_as_key()));
    assert_eq!(a.fields.len(), 13, "2 key + 11 value fields");
}

#[test]
fn the_keys_are_read_from_the_rows_the_fold_reads_them_from() {
    assert_eq!(
        key_from_country_row(&full_country_row()).expect("keys"),
        key(&[7])
    );
    // `fold_tech` validates nothing about a country tag, not even zero.
    assert_eq!(
        key_from_country_row(&country_row(0, 0, u32::MAX, u32::MAX)).expect("zero tag is legal"),
        key(&[0])
    );
    assert!(key_from_country_row(&[]).is_err());
    assert!(key_from_country_row(&vec![0u8; COUNTRY_ROW_LEN + 1]).is_err());

    let row = active_row(&measured_active_spec());
    assert_eq!(key_from_active_row(&row).expect("keys"), key(&[7, 2]));
    assert!(key_from_active_row(&row[..ACTIVE_ROW_FIXED_LEN - 1]).is_err());
}

// --- 2. Byte-identical round trips ------------------------------------------

fn assert_roundtrips(c: &Collection, record: &Record) {
    let mut bytes = Vec::new();
    record.encode(c, &mut bytes).expect("encodes");
    let (back, pos) = Record::decode(c, &bytes, 0).expect("decodes");
    assert_eq!(pos, bytes.len(), "consumes exactly its own bytes");
    assert_eq!(&back, record, "decode reproduces the record");
    let mut reencoded = Vec::new();
    back.encode(c, &mut reencoded).expect("re-encodes");
    assert_eq!(reencoded, bytes, "decode/encode is byte-identical");
}

#[test]
fn a_full_row_roundtrips_byte_for_byte() {
    let c = countries();
    assert_roundtrips(
        &c,
        &record_from_country_row(&c, &full_country_row(), 0).expect("maps"),
    );
    let a = active();
    let row = active_row(&full_active_spec());
    assert_roundtrips(
        &a,
        &record_from_active_row_at_hour(&a, &row, Some(FRAME_HOUR)).expect("maps"),
    );
    // And the shape the reference capture actually carries.
    let row = active_row(&measured_active_spec());
    assert_roundtrips(&a, &record_from_active_row(&a, &row, 0).expect("maps"));
}

#[test]
fn a_fully_gated_off_row_roundtrips_byte_for_byte() {
    let c = countries();
    assert_roundtrips(
        &c,
        &record_from_country_row(&c, &country_row(7, 0, u32::MAX, u32::MAX), 0).expect("maps"),
    );
    let a = active();
    let row = active_row(&ActiveSpec {
        live_progress_raw: None,
        base_required_points_raw: None,
        ..measured_active_spec()
    });
    assert_roundtrips(&a, &record_from_active_row(&a, &row, 0).expect("maps"));
}

// --- 3. States through the mapper -------------------------------------------

#[test]
fn the_schema_declares_only_unknown_and_the_mapper_emits_only_unknown() {
    // Nothing here is inapplicable to a row this schema carries, and nothing is
    // a relation that can point at nothing, so no field is `optional` or
    // `nullable`. Every empty state this domain has is §3's "applicable, not
    // captured".
    for c in [countries(), active()] {
        assert_eq!(c.optional_count(), 0, "no field is ever inapplicable");
        assert!(
            c.value_fields().all(|f| !f.nullable),
            "no field is a relation that can point at nothing"
        );
    }
    let unknown: Vec<u32> = countries()
        .value_fields()
        .filter(|f| f.unknown_capable)
        .map(|f| f.field_id)
        .collect();
    assert_eq!(
        unknown,
        vec![field::COMPLETED_TECH_COUNT, field::RESEARCH_SLOTS],
        "the capture flag itself is always readable, so it is never unknown"
    );
    let unknown: Vec<u32> = active()
        .value_fields()
        .filter(|f| f.unknown_capable)
        .map(|f| f.field_id)
        .collect();
    assert_eq!(
        unknown,
        vec![
            field::LIVE_PROGRESS_RAW,
            field::BASE_REQUIRED_POINTS_RAW,
            field::AHEAD_OF_TIME_YEARS_RAW,
            field::AHEAD_OF_TIME_PENALTY_RAW,
            field::AHEAD_OF_TIME_REDUCTION_YEARS_RAW,
            field::AHEAD_OF_TIME_REDUCED_PENALTY_RAW,
            field::AHEAD_OF_TIME_BRANCH,
        ]
    );
}

#[test]
fn a_real_zero_is_a_value_not_an_empty_state() {
    // Every flag set, every value zero: a country that has researched nothing and
    // a slot that has just started. §3 keeps zero out of the three empty states.
    let c = countries();
    let r = record_from_country_row(
        &c,
        &country_row(
            1,
            COUNTRY_FLAG_COMPLETED_TECH_COUNT
                | COUNTRY_FLAG_RESEARCH_SLOT_COUNT
                | COUNTRY_FLAG_ACTIVE_RESEARCH,
            0,
            0,
        ),
        0,
    )
    .expect("maps");
    assert_eq!(
        r.values[index_of(&c, field::COMPLETED_TECH_COUNT)],
        Value::Unsigned(0)
    );
    assert_eq!(
        r.values[index_of(&c, field::RESEARCH_SLOTS)],
        Value::Unsigned(0)
    );

    let a = active();
    let row = active_row(&ActiveSpec {
        progress_raw: 0,
        live_progress_raw: Some(0),
        base_required_points_raw: Some(0),
        quartet: Some((0, 0, 0, 0)),
        a29: Some(A29Spec {
            // Branch 0 is "no display": the all-zero quartet is its exact shape,
            // and it is a *value*, which is why a clear gate can never be null.
            branch: 0,
            ..A29Spec::valid()
        }),
        ..measured_active_spec()
    });
    let r = record_from_active_row_at_hour(&a, &row, Some(FRAME_HOUR)).expect("maps");
    assert_eq!(r.values[index_of(&a, field::PROGRESS_RAW)], Value::Unsigned(0));
    assert_eq!(
        r.values[index_of(&a, field::LIVE_PROGRESS_RAW)],
        Value::Signed(0)
    );
    assert_eq!(
        r.values[index_of(&a, field::BASE_REQUIRED_POINTS_RAW)],
        Value::Signed(0)
    );
    for id in [
        field::AHEAD_OF_TIME_YEARS_RAW,
        field::AHEAD_OF_TIME_PENALTY_RAW,
        field::AHEAD_OF_TIME_REDUCTION_YEARS_RAW,
        field::AHEAD_OF_TIME_REDUCED_PENALTY_RAW,
    ] {
        assert_eq!(r.values[index_of(&a, id)], Value::Signed(0), "field {id}");
    }
    assert_eq!(
        r.values[index_of(&a, field::AHEAD_OF_TIME_BRANCH)],
        Value::Enum(0),
        "branch 0 is a measured code, not an empty state"
    );
}

#[test]
fn a_clear_country_gate_is_unknown_field_by_field() {
    // The whole gate space, one bit at a time — not one sampled case.
    let c = countries();
    let all = COUNTRY_FLAG_COMPLETED_TECH_COUNT
        | COUNTRY_FLAG_RESEARCH_SLOT_COUNT
        | COUNTRY_FLAG_ACTIVE_RESEARCH;
    for flags in 0..=all {
        let completed = if flags & COUNTRY_FLAG_COMPLETED_TECH_COUNT != 0 {
            42
        } else {
            COUNTRY_COUNT_ABSENT_SENTINEL
        };
        let slots = if flags & COUNTRY_FLAG_RESEARCH_SLOT_COUNT != 0 {
            4
        } else {
            COUNTRY_COUNT_ABSENT_SENTINEL
        };
        let r = record_from_country_row(&c, &country_row(7, flags, completed, slots), 0)
            .unwrap_or_else(|e| panic!("maps with flags {flags:#x}: {e}"));
        assert_eq!(
            r.values[index_of(&c, field::COMPLETED_TECH_COUNT)],
            if flags & COUNTRY_FLAG_COMPLETED_TECH_COUNT != 0 {
                Value::Unsigned(42)
            } else {
                Value::Unknown
            },
            "completed_tech_count under flags {flags:#x}"
        );
        assert_eq!(
            r.values[index_of(&c, field::RESEARCH_SLOTS)],
            if flags & COUNTRY_FLAG_RESEARCH_SLOT_COUNT != 0 {
                Value::Unsigned(4)
            } else {
                Value::Unknown
            },
            "research_slots under flags {flags:#x}"
        );
        assert_eq!(
            r.values[index_of(&c, field::ACTIVE_RESEARCH_CAPTURED)],
            Value::Bool(flags & COUNTRY_FLAG_ACTIVE_RESEARCH != 0),
            "active_research_captured under flags {flags:#x}"
        );
    }
}

#[test]
fn a_clear_active_gate_is_unknown_field_by_field() {
    let a = active();
    // The three independent gates of an active row: live progress, base points,
    // and the quartet-plus-provenance pair the fold ties together.
    for live in [None, Some(1_000_i64)] {
        for base in [None, Some(100_000_i64)] {
            for ahead in [false, true] {
                let spec = ActiveSpec {
                    live_progress_raw: live,
                    base_required_points_raw: base,
                    quartet: ahead.then_some((88_000, 500_000, 25_000, 375_000)),
                    a29: ahead.then(A29Spec::valid),
                    ..measured_active_spec()
                };
                let row = active_row(&spec);
                let r = record_from_active_row_at_hour(&a, &row, Some(FRAME_HOUR))
                    .unwrap_or_else(|e| panic!("maps ({live:?}, {base:?}, {ahead}): {e}"));
                assert_eq!(
                    r.values[index_of(&a, field::LIVE_PROGRESS_RAW)],
                    live.map_or(Value::Unknown, Value::Signed)
                );
                assert_eq!(
                    r.values[index_of(&a, field::BASE_REQUIRED_POINTS_RAW)],
                    base.map_or(Value::Unknown, Value::Signed)
                );
                for (id, expected) in [
                    (field::AHEAD_OF_TIME_YEARS_RAW, 88_000_i64),
                    (field::AHEAD_OF_TIME_PENALTY_RAW, 500_000),
                    (field::AHEAD_OF_TIME_REDUCTION_YEARS_RAW, 25_000),
                    (field::AHEAD_OF_TIME_REDUCED_PENALTY_RAW, 375_000),
                ] {
                    assert_eq!(
                        r.values[index_of(&a, id)],
                        if ahead {
                            Value::Signed(expected)
                        } else {
                            Value::Unknown
                        },
                        "field {id} with ahead={ahead}"
                    );
                }
                assert_eq!(
                    r.values[index_of(&a, field::AHEAD_OF_TIME_BRANCH)],
                    if ahead { Value::Enum(2) } else { Value::Unknown }
                );
                // Never Absent and never Null anywhere in this domain.
                assert!(r.values.iter().all(|v| v.is_present() && !v.is_null()));
            }
        }
    }
}

#[test]
fn the_unknown_state_survives_a_roundtrip() {
    let c = countries();
    let record = record_from_country_row(
        &c,
        &country_row(3, COUNTRY_FLAG_COMPLETED_TECH_COUNT, 11, COUNTRY_COUNT_ABSENT_SENTINEL),
        0,
    )
    .expect("maps");
    let mut bytes = Vec::new();
    record.encode(&c, &mut bytes).expect("encodes");
    let (back, _) = Record::decode(&c, &bytes, 0).expect("decodes");
    assert_eq!(
        back.values[index_of(&c, field::COMPLETED_TECH_COUNT)],
        Value::Unsigned(11)
    );
    assert_eq!(
        back.values[index_of(&c, field::RESEARCH_SLOTS)],
        Value::Unknown
    );

    let a = active();
    let row = active_row(&ActiveSpec {
        live_progress_raw: None,
        ..measured_active_spec()
    });
    let record = record_from_active_row(&a, &row, 0).expect("maps");
    let mut bytes = Vec::new();
    record.encode(&a, &mut bytes).expect("encodes");
    let (back, _) = Record::decode(&a, &bytes, 0).expect("decodes");
    assert_eq!(
        back.values[index_of(&a, field::LIVE_PROGRESS_RAW)],
        Value::Unknown
    );
    for id in [
        field::AHEAD_OF_TIME_YEARS_RAW,
        field::AHEAD_OF_TIME_BRANCH,
    ] {
        assert_eq!(back.values[index_of(&a, id)], Value::Unknown, "field {id}");
    }
}

// --- 4. Quantization ---------------------------------------------------------

#[test]
fn no_field_is_continuous_so_the_quantization_test_does_not_apply() {
    // Step 4's "quantization saves bytes" test is recorded as inapplicable, with
    // its reason: d13 has no continuous field at all. `fold_tech` is the one fold
    // that emits every scaled quantity in BOTH forms — `progress_raw` beside
    // `progress_units` (`world_fold.rs:13343`-`:13347`), and likewise for
    // `live_progress` (`:13349`-`:13353`), `base_required_points`
    // (`:13358`-`:13365`) and the quartet (`:13401`-`:13426`) — so the raw
    // integer is the fold's own first-class state and the `_units` companion is
    // its rendering at a scale the fold publishes in the same object
    // (`"ahead_of_time_fixed_scale": 100_000`, `:13400`). Carrying the raws is
    // both the fold-faithful port and the only lossless one: the six
    // `TECH_LIVE_SCALE` fields are `i64` and the fold accepts every `i64`, which
    // no `f64` form can carry past 2^53.
    for c in [countries(), active()] {
        assert!(
            c.value_fields().all(|f| !matches!(f.kind, Kind::Continuous)),
            "collection {} has a continuous field",
            c.collection_id
        );
        assert!(c.value_fields().all(|f| !f.quant.is_quantized()));
    }
}

#[test]
fn no_field_is_derived() {
    // §6 derived fields are clock-linear integers. `progress_raw` rises at a rate
    // the clock does not fix and resets when a slot takes a new technology, so it
    // is not `v + c * (frame_tick >> u)`. The tick still enters this domain — via
    // the A29 same-hour gate, not via a transform.
    for c in [countries(), active()] {
        assert!(c.fields.iter().all(|f| f.derived.is_none()));
    }
}

// --- 6. Every field's offset, and the two scales ----------------------------

#[test]
fn every_country_field_comes_from_its_own_offset() {
    let c = countries();
    // Distinct words at every offset, so a transposition fails.
    let r = record_from_country_row(
        &c,
        &country_row(
            0xDEAD_BEEF,
            COUNTRY_FLAG_COMPLETED_TECH_COUNT | COUNTRY_FLAG_RESEARCH_SLOT_COUNT,
            1_111,
            2_222,
        ),
        0,
    )
    .expect("maps");
    assert_eq!(
        r.values[index_of(&c, field::COMPLETED_TECH_COUNT)],
        Value::Unsigned(1_111),
        "completed_tech_count is the word at +8 (`world_fold.rs:13315`)"
    );
    assert_eq!(
        r.values[index_of(&c, field::RESEARCH_SLOTS)],
        Value::Unsigned(2_222),
        "research_slots is the word at +12 (`:13316`)"
    );
    assert_eq!(
        r.values[index_of(&c, field::ACTIVE_RESEARCH_CAPTURED)],
        Value::Bool(false)
    );
    // The tag at +0 is the key and is not in the body.
    assert_eq!(
        key_from_country_row(&country_row(0xDEAD_BEEF, 0, u32::MAX, u32::MAX)).expect("keys"),
        key(&[0xDEAD_BEEF])
    );
}

#[test]
fn every_active_field_comes_from_its_own_offset_and_carries_its_own_scale() {
    // This is the test that catches the 1000-vs-100000 class. Every value is a
    // distinct word, so a transposed pair fails; and the module's two scale
    // constants are checked by reproducing the fold's OWN `_units` expressions
    // from the stored raws, so attaching the wrong divisor downstream fails here
    // rather than 100x-wrong in a replay.
    let a = active();
    let spec = ActiveSpec {
        tag: 11,
        slot_index: 3,
        tech_handle_raw: 0x0101,
        tech_def_index_raw: 0x0202,
        progress_raw: 1_500,
        live_progress_raw: Some(250_000),
        base_required_points_raw: Some(700_000),
        tech_name: Some("x"),
        quartet: Some((88_000, 500_000, 25_000, 375_000)),
        a29: Some(A29Spec::valid()),
    };
    let row = active_row(&spec);
    let r = record_from_active_row_at_hour(&a, &row, Some(FRAME_HOUR)).expect("maps");

    assert_eq!(
        r.values[index_of(&a, field::TECH_HANDLE_RAW)],
        Value::ContentId(0x0101),
        "tech_handle_raw is the word at +8 (`world_fold.rs:13328`)"
    );
    assert_eq!(
        r.values[index_of(&a, field::TECH_DEF_INDEX_RAW)],
        Value::ContentId(0x0202),
        "tech_def_index_raw is the word at +12 (`:13329`) and is a separate field"
    );
    assert_eq!(
        r.values[index_of(&a, field::PROGRESS_RAW)],
        Value::Unsigned(1_500),
        "progress_raw is the word at +16 (`:13330`), raw"
    );
    assert_eq!(
        r.values[index_of(&a, field::LIVE_PROGRESS_RAW)],
        Value::Signed(250_000),
        "live_progress_raw is the i64 at +24 (`:13335`), raw"
    );
    assert_eq!(
        r.values[index_of(&a, field::BASE_REQUIRED_POINTS_RAW)],
        Value::Signed(700_000),
        "base_required_points_raw is the i64 at +32 (`:13336`), raw"
    );
    assert_eq!(
        r.values[index_of(&a, field::TECH_NAME_CAPTURED)],
        Value::Bool(true)
    );
    // The quartet, in the fold's own order (`:13378`-`:13381`).
    for (id, expected) in [
        (field::AHEAD_OF_TIME_YEARS_RAW, 88_000_i64),
        (field::AHEAD_OF_TIME_PENALTY_RAW, 500_000),
        (field::AHEAD_OF_TIME_REDUCTION_YEARS_RAW, 25_000),
        (field::AHEAD_OF_TIME_REDUCED_PENALTY_RAW, 375_000),
    ] {
        assert_eq!(r.values[index_of(&a, id)], Value::Signed(expected), "field {id}");
    }
    assert_eq!(
        r.values[index_of(&a, field::AHEAD_OF_TIME_BRANCH)],
        Value::Enum(2),
        "branch is the word at A29 +4 (`:13434`)"
    );
    assert_eq!(key_from_active_row(&row).expect("keys"), key(&[11, 3]));

    // The two scales, applied to the stored raws exactly as the fold applies
    // them. `progress_raw` divides by 1000 (`:13346`); the other six divide by
    // 100000 (`:13352`, `:13364`, `:13413`-`:13425`). Reading either through the
    // other is wrong by 100x.
    let Value::Unsigned(progress) = r.values[index_of(&a, field::PROGRESS_RAW)] else {
        panic!("progress_raw is unsigned")
    };
    assert_eq!(progress as f64 / TECH_PROGRESS_SCALE, 1.5);
    assert_ne!(progress as f64 / TECH_LIVE_SCALE, 1.5, "the 100x error class");
    let Value::Signed(live) = r.values[index_of(&a, field::LIVE_PROGRESS_RAW)] else {
        panic!("live_progress_raw is signed")
    };
    assert_eq!(live as f64 / TECH_LIVE_SCALE, 2.5);
    assert_ne!(live as f64 / TECH_PROGRESS_SCALE, 2.5, "the 100x error class");
    let Value::Signed(base) = r.values[index_of(&a, field::BASE_REQUIRED_POINTS_RAW)] else {
        panic!("base_required_points_raw is signed")
    };
    assert_eq!(base as f64 / TECH_LIVE_SCALE, 7.0);
    let Value::Signed(years) = r.values[index_of(&a, field::AHEAD_OF_TIME_YEARS_RAW)] else {
        panic!("quartet is signed")
    };
    assert_eq!(years as f64 / TECH_LIVE_SCALE, 0.88, "the dossier's 0.88 years");
    // And the header words the packer writes are the same two constants
    // (`attach_log.rs:68843`-`:68844`).
    assert_eq!(TECH_PROGRESS_SCALE_RAW as f64, TECH_PROGRESS_SCALE);
    assert_eq!(TECH_LIVE_SCALE_RAW as f64, TECH_LIVE_SCALE);
    assert_eq!(TECH_LIVE_SCALE / TECH_PROGRESS_SCALE, 100.0);
}

#[test]
fn a_signed_live_progress_survives_the_whole_i64_range() {
    // The reason this port carries raw integers rather than a Continuous form:
    // `fold_tech` accepts every `i64` (`:13335`), and above 2^53 no `f64` can
    // carry one back. `Value::Signed` does.
    let a = active();
    for raw in [i64::MIN, -1, 0, 1, i64::MAX, (1i64 << 53) + 1] {
        let row = active_row(&ActiveSpec {
            live_progress_raw: Some(raw),
            ..measured_active_spec()
        });
        let r = record_from_active_row(&a, &row, 0).expect("maps");
        assert_eq!(
            r.values[index_of(&a, field::LIVE_PROGRESS_RAW)],
            Value::Signed(raw)
        );
        assert_roundtrips(&a, &r);
    }
}

// --- Rejection: every row the fold rejects ----------------------------------

#[test]
fn the_country_mapper_rejects_what_it_must() {
    let c = countries();
    for len in [0usize, 1, COUNTRY_ROW_LEN - 1, COUNTRY_ROW_LEN + 1, 40] {
        assert!(
            record_from_country_row(&c, &vec![0u8; len], 0).is_err(),
            "length {len} must be rejected, not read"
        );
    }
    // A clear capture flag over a word that is not the collector's sentinel is a
    // contradiction between this domain's two authorities: the collector packs
    // `u32::MAX` there (`attach_log.rs:68863`, `:68869`) and the fold would
    // publish the word as a count. Refused rather than resolved silently.
    for word in [0u32, 1, 2_000, u32::MAX - 1] {
        assert!(
            record_from_country_row(&c, &country_row(7, 0, word, u32::MAX), 0).is_err(),
            "completed_tech_count {word} without its flag"
        );
        assert!(
            record_from_country_row(&c, &country_row(7, 0, u32::MAX, word), 0).is_err(),
            "research_slot_count {word} without its flag"
        );
    }
    // The sentinel itself under a SET flag is a real (if absurd) value: the fold
    // reads the word unconditionally, so refusing it would refuse a payload the
    // recorder folds.
    let r = record_from_country_row(
        &c,
        &country_row(7, COUNTRY_FLAG_COMPLETED_TECH_COUNT, u32::MAX, u32::MAX),
        0,
    )
    .expect("a flagged sentinel is a value");
    assert_eq!(
        r.values[index_of(&c, field::COMPLETED_TECH_COUNT)],
        Value::Unsigned(u32::MAX as u64)
    );
    // Country flag bits above 1<<2 are ignored, not rejected: `fold_tech` never
    // reads this word at all, so refusing an unfamiliar bit would refuse a
    // payload the recorder folds.
    for bit in 3..32u32 {
        let flags = COUNTRY_FLAG_COMPLETED_TECH_COUNT
            | COUNTRY_FLAG_RESEARCH_SLOT_COUNT
            | COUNTRY_FLAG_ACTIVE_RESEARCH
            | (1 << bit);
        assert!(record_from_country_row(&c, &country_row(7, flags, 1, 1), 0).is_ok());
    }
}

#[test]
fn the_active_mapper_rejects_every_row_the_fold_rejects() {
    let a = active();
    let base = measured_active_spec();

    // Short rows, including the empty slice.
    for len in [0usize, 1, ACTIVE_ROW_FIXED_LEN - 1] {
        assert!(record_from_active_row(&a, &vec![0u8; len], 0).is_err(), "length {len}");
    }
    // A slice longer than the row's own grammar consumes is a mis-split table,
    // not a row with padding.
    let mut padded = active_row(&base);
    padded.push(0);
    assert!(record_from_active_row(&a, &padded, 0).is_err());

    // Every unknown flag bit (`world_fold.rs:13332`), not just one sample.
    for bit in 5..32u32 {
        let mut row = active_row(&base);
        let flags = u32::from_le_bytes([row[20], row[21], row[22], row[23]]) | (1 << bit);
        row[20..24].copy_from_slice(&flags.to_le_bytes());
        assert!(record_from_active_row(&a, &row, 0).is_err(), "flag bit {bit}");
        assert!(active_row_len(&row).is_err(), "flag bit {bit} at length time");
    }

    // A gated-off value carrying a non-zero word (`:13354`, `:13366`).
    for (offset, label) in [(24usize, "live_progress"), (32, "base_required_points")] {
        let mut row = active_row(&ActiveSpec {
            live_progress_raw: None,
            base_required_points_raw: None,
            ..base
        });
        row[offset..offset + 8].copy_from_slice(&1i64.to_le_bytes());
        assert!(record_from_active_row(&a, &row, 0).is_err(), "{label} without flag");
    }

    // An empty tech_name (`:13371`) and invalid UTF-8 (`:30160`).
    let mut row = active_row(&base);
    row[20..24].copy_from_slice(&(0b111u32).to_le_bytes());
    row.extend_from_slice(&0u16.to_le_bytes());
    assert!(record_from_active_row(&a, &row, 0).is_err(), "empty tech_name");
    let mut row = active_row(&base);
    row[20..24].copy_from_slice(&(0b111u32).to_le_bytes());
    row.extend_from_slice(&1u16.to_le_bytes());
    row.push(0xFF);
    assert!(record_from_active_row(&a, &row, 0).is_err(), "invalid utf-8 tech_name");

    // Quartet range, each of the four independently, on both sides (`:13395`).
    for index in 0..4usize {
        for bad in [-1i64, AHEAD_RAW_MAX + 1] {
            let mut q = [1_000i64, 1_000, 1_000, 1_000];
            q[index] = bad;
            // Branch 2 needs reduction > 0; use branch 0 shape only when legal.
            let row = active_row(&ActiveSpec {
                quartet: Some((q[0], q[1], q[2], q[3])),
                a29: Some(A29Spec::valid()),
                ..base
            });
            assert!(
                record_from_active_row_at_hour(&a, &row, Some(FRAME_HOUR)).is_err(),
                "quartet[{index}] = {bad}"
            );
        }
        // And the bound itself is accepted, so it is not off by one.
        let mut q = [0i64, 0, 1, 0];
        q[index] = AHEAD_RAW_MAX;
        if index == 2 {
            q[2] = AHEAD_RAW_MAX;
        }
        let row = active_row(&ActiveSpec {
            quartet: Some((q[0], q[1], q[2], q[3])),
            a29: Some(A29Spec { branch: 2, ..A29Spec::valid() }),
            ..base
        });
        assert!(
            record_from_active_row_at_hour(&a, &row, Some(FRAME_HOUR)).is_ok(),
            "quartet[{index}] = {AHEAD_RAW_MAX} is in range"
        );
    }

    // The two flag-pair rules (`:13429`, `:13561`), both directions.
    let mut row = active_row(&ActiveSpec {
        quartet: Some((0, 0, 1, 0)),
        a29: Some(A29Spec::valid()),
        ..base
    });
    let with_a29_only = (u32::from_le_bytes([row[20], row[21], row[22], row[23]])
        & !ACTIVE_FLAG_AHEAD_QUARTET)
        .to_le_bytes();
    row[20..24].copy_from_slice(&with_a29_only);
    assert!(
        record_from_active_row_at_hour(&a, &row, Some(FRAME_HOUR)).is_err(),
        "A29 provenance without the quartet"
    );
    let row = active_row(&ActiveSpec {
        quartet: Some((0, 0, 1, 0)),
        a29: None,
        ..base
    });
    assert!(
        record_from_active_row_at_hour(&a, &row, Some(FRAME_HOUR)).is_err(),
        "quartet without A29 provenance"
    );
}

#[test]
fn every_checkable_a29_clause_is_enforced() {
    // Each of the eighteen clauses of `world_fold.rs:13491`-`:13521` that this
    // crate can evaluate, broken one at a time from a block that otherwise
    // passes. The two digest clauses (`:13517`, `:13520`) are not here and cannot
    // be: their expected values need `recorder_host`. See the module docs.
    let a = active();
    let good_quartet = (88_000i64, 500_000, 25_000, 375_000);
    let build = |spec: A29Spec, quartet: (i64, i64, i64, i64)| {
        active_row(&ActiveSpec {
            quartet: Some(quartet),
            a29: Some(spec),
            ..measured_active_spec()
        })
    };
    assert!(
        record_from_active_row_at_hour(&a, &build(A29Spec::valid(), good_quartet), Some(FRAME_HOUR))
            .is_ok(),
        "the reference block must pass, or the negatives prove nothing"
    );

    let broken: Vec<(&str, A29Spec, (i64, i64, i64, i64))> = vec![
        ("authority", A29Spec { authority: 2, ..A29Spec::valid() }, good_quartet),
        ("authority", A29Spec { authority: 0, ..A29Spec::valid() }, good_quartet),
        ("branch", A29Spec { branch: 3, ..A29Spec::valid() }, good_quartet),
        ("branch", A29Spec { branch: u32::MAX, ..A29Spec::valid() }, good_quartet),
        // Branch 0 requires years/penalty/reduced_penalty all zero.
        ("branch 0 shape", A29Spec { branch: 0, ..A29Spec::valid() }, (1, 0, 0, 0)),
        ("branch 0 shape", A29Spec { branch: 0, ..A29Spec::valid() }, (0, 1, 0, 0)),
        ("branch 0 shape", A29Spec { branch: 0, ..A29Spec::valid() }, (0, 0, 0, 1)),
        // Branch 1 requires reduction == 0 and reduced_penalty == penalty.
        ("branch 1 shape", A29Spec { branch: 1, ..A29Spec::valid() }, (1, 5, 1, 5)),
        ("branch 1 shape", A29Spec { branch: 1, ..A29Spec::valid() }, (1, 5, 0, 6)),
        // Branch 2 requires reduction > 0.
        ("branch 2 shape", A29Spec::valid(), (1, 5, 0, 4)),
        ("callback_sequence", A29Spec { callback_sequence: 0, ..A29Spec::valid() }, good_quartet),
        ("callback_thread_id", A29Spec { callback_thread_id: 0, ..A29Spec::valid() }, good_quartet),
        (
            "producer_game_hour_raw",
            A29Spec { producer_game_hour_raw: FRAME_HOUR + 1, selected_game_hour_raw: FRAME_HOUR + 1, ..A29Spec::valid() },
            good_quartet,
        ),
        ("callback_qpc_ticks", A29Spec { callback_qpc_ticks: 0, ..A29Spec::valid() }, good_quartet),
        ("qpc_frequency_hz", A29Spec { qpc_frequency_hz: 0, ..A29Spec::valid() }, good_quartet),
        (
            "ui_action_gate_return_rva",
            A29Spec { ui_action_gate_return_rva: A29_UI_YEAR_HELPER_RETURN_RVA + 1, ..A29Spec::valid() },
            good_quartet,
        ),
        ("candidate_count", A29Spec { candidate_count: 0, selected_ordinal: 0, ..A29Spec::valid() }, good_quartet),
        ("selected_ordinal", A29Spec { selected_ordinal: 3, ..A29Spec::valid() }, good_quartet),
        (
            "selected_game_hour_raw",
            A29Spec { selected_game_hour_raw: FRAME_HOUR - 1, ..A29Spec::valid() },
            good_quartet,
        ),
        (
            "callback_qpc_ticks ordering",
            A29Spec { callback_qpc_ticks: 601, ..A29Spec::valid() },
            good_quartet,
        ),
        (
            "selected_qpc_ticks",
            A29Spec { callback_qpc_ticks: 0, selected_qpc_ticks: 0, ..A29Spec::valid() },
            good_quartet,
        ),
        (
            "selection_cutoff_qpc_ticks",
            A29Spec { selection_cutoff_qpc_ticks: 599, ..A29Spec::valid() },
            good_quartet,
        ),
        ("candidate_set_sha256", A29Spec { candidate_set_sha256: [0; 32], ..A29Spec::valid() }, good_quartet),
        (
            "selected_present_receipt_sha256",
            A29Spec { selected_present_receipt_sha256: [0; 32], ..A29Spec::valid() },
            good_quartet,
        ),
        ("source_record_sha256", A29Spec { source_record_sha256: [0; 32], ..A29Spec::valid() }, good_quartet),
    ];
    for (label, spec, quartet) in broken {
        assert!(
            record_from_active_row_at_hour(&a, &build(spec, quartet), Some(FRAME_HOUR)).is_err(),
            "clause `{label}` must be enforced"
        );
    }
}

#[test]
fn the_mappers_refuse_a_collection_they_cannot_fill() {
    // A caller handing over another domain's collection must not get a record
    // full of placeholders under this domain's field ids.
    let mut c = countries();
    c.fields.push(Field::new(999, Kind::Unsigned));
    assert!(record_from_country_row(&c, &full_country_row(), 0).is_err());
    let mut a = active();
    a.fields.push(Field::new(999, Kind::Unsigned));
    assert!(record_from_active_row(&a, &active_row(&measured_active_spec()), 0).is_err());
}

// --- split_payload and the row walk -----------------------------------------

#[test]
fn split_payload_enforces_the_folds_structural_rules() {
    let p = payload(
        &[full_country_row()],
        &[active_row(&measured_active_spec())],
    );
    let (header, tables) = split_payload(&p).expect("well-formed payload");
    assert_eq!(header.country_count, 1);
    assert_eq!(header.active_count, 1);
    assert_eq!(header.progress_scale_raw, TECH_PROGRESS_SCALE_RAW);
    assert_eq!(header.live_scale_raw, TECH_LIVE_SCALE_RAW);
    assert_eq!(tables.countries.len(), COUNTRY_ROW_LEN);
    assert_eq!(tables.active.len(), ACTIVE_ROW_FIXED_LEN);

    // Short of the header.
    for len in 0..HEADER_LEN {
        assert!(split_payload(&p[..len]).is_err(), "len {len}");
    }
    // Short of the fold's minimum (`world_fold.rs:13308`).
    for len in HEADER_LEN..p.len() {
        assert!(split_payload(&p[..len]).is_err(), "truncated at {len}");
    }
    // Counts whose byte length overflows `usize` must be rejected, not wrapped
    // into a plausible small length.
    for offset in [8usize, 28] {
        let mut huge = p.clone();
        huge[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(split_payload(&huge).is_err(), "count at +{offset}");
    }
    // The fold never reads `changed_rows`, `status_flags` or the two scale words,
    // so neither does this: checking them would reject payloads the recorder
    // folds. They are carried, not enforced.
    for (offset, value) in [(12usize, 999u32), (16, 0xFFFF), (20, 7), (24, 7)] {
        let mut patched = p.clone();
        patched[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        let (header, _) = split_payload(&patched).expect("still folds");
        let seen = match offset {
            12 => header.changed_rows,
            16 => header.status_flags,
            20 => header.progress_scale_raw,
            _ => header.live_scale_raw,
        };
        assert_eq!(seen, value, "header word at +{offset} is carried verbatim");
    }
}

#[test]
fn active_row_spans_walks_variable_tails_and_enforces_exact_consumption() {
    let rows = vec![
        active_row(&measured_active_spec()),
        active_row(&full_active_spec()),
        active_row(&ActiveSpec {
            slot_index: 4,
            tech_name: Some("a much longer technology name"),
            ..measured_active_spec()
        }),
    ];
    let table: Vec<u8> = rows.iter().flatten().copied().collect();
    let spans = active_row_spans(&table, 3).expect("walks");
    assert_eq!(spans.len(), 3);
    for (span, row) in spans.iter().zip(&rows) {
        assert_eq!(&table[span.clone()], &row[..]);
    }
    assert_eq!(
        rows[1].len(),
        ACTIVE_ROW_FIXED_LEN + 2 + "engineers4".len() + AHEAD_QUARTET_LEN + A29_PROVENANCE_LEN,
        "a full row is 40 + string + 32 + 176"
    );

    // Trailing bytes (`world_fold.rs:13571`) and a short count both fail.
    assert!(active_row_spans(&table, 2).is_err(), "unconsumed rows");
    assert!(active_row_spans(&table, 4).is_err(), "a row that is not there");
    let mut trailing = table.clone();
    trailing.push(0);
    assert!(active_row_spans(&trailing, 3).is_err());
    // Every truncation is refused rather than read as a shorter row.
    for len in 0..table.len() {
        assert!(active_row_spans(&table[..len], 3).is_err(), "truncated at {len}");
    }
    // An empty table with a zero count is legal and yields nothing.
    assert_eq!(active_row_spans(&[], 0).expect("empty"), Vec::new());
}

/// Lane JO's XUI09 witness tail, as the producer appends it after the last
/// active row. `dictionary`/`measured`/`refused`/`vector` counts of zero give
/// the smallest legal tail; the counts drive the walk, so the shapes below
/// exercise every arm of it.
fn xui09_tail(dictionary: &[&str], measured: u32, refused: u32, vectors: &[(u32, u32, u32)]) -> Vec<u8> {
    let mut tail = Vec::new();
    tail.extend_from_slice(&XUI09_TAIL_MAGIC.to_le_bytes());
    tail.extend_from_slice(&0u32.to_le_bytes()); // flags
    tail.extend_from_slice(&(dictionary.len() as u32).to_le_bytes());
    tail.extend_from_slice(&measured.to_le_bytes());
    tail.extend_from_slice(&refused.to_le_bytes());
    tail.extend_from_slice(&(vectors.len() as u32).to_le_bytes());
    assert_eq!(tail.len(), XUI09_TAIL_HEADER_LEN);
    for id in dictionary {
        tail.extend_from_slice(&0u32.to_le_bytes()); // definition index
        tail.extend_from_slice(&(id.len() as u16).to_le_bytes());
        tail.extend_from_slice(id.as_bytes());
    }
    tail.extend(std::iter::repeat(0u8).take(measured as usize * 4 + refused as usize * 8));
    for (country_flags, state_count, completed_count) in vectors {
        tail.extend_from_slice(&0u32.to_le_bytes()); // tag_db_id
        tail.extend_from_slice(&country_flags.to_le_bytes());
        tail.extend_from_slice(&state_count.to_le_bytes());
        tail.extend_from_slice(&completed_count.to_le_bytes());
        let state_bytes = if country_flags & XUI09_COUNTRY_FLAG_DENSE != 0 { 1 } else { 5 };
        tail.extend(std::iter::repeat(0u8).take(
            *state_count as usize * state_bytes + *completed_count as usize * 4,
        ));
    }
    tail
}

/// kills: this port not knowing about the XUI09 witness tail — which is what it
/// did until now, and which made it refuse every payload a shipping recorder
/// writes, `active_count == 0` included.
///
/// This is the same defect that cost the t53 capture all 59 of its D13 records
/// (`runs/jt_a29_values/LIVE_D13_EXCLUSION.md`): a decoder that has not been
/// taught the tail reports it as trailing bytes. The canonical validator was
/// the third D13 decoder with this gap; this port was the fourth.
#[test]
fn the_active_table_splitter_measures_the_xui09_witness_tail() {
    let rows = vec![
        active_row(&ActiveSpec {
            slot_index: 4,
            ..measured_active_spec()
        }),
        active_row(&full_active_spec()),
    ];
    let table: Vec<u8> = rows.iter().flatten().copied().collect();

    for tail in [
        xui09_tail(&[], 0, 0, &[]),
        xui09_tail(&["infantry_weapons", "fighter1"], 2, 1, &[(0, 5, 2)]),
        xui09_tail(&["fighter1"], 1, 0, &[(XUI09_COUNTRY_FLAG_DENSE, 7, 3), (0, 2, 0)]),
    ] {
        assert_eq!(xui09_tail_len(&tail).expect("walks"), tail.len());

        let mut with_tail = table.clone();
        with_tail.extend_from_slice(&tail);
        let spans = active_row_spans(&with_tail, 2).expect("the tail is measured, not refused");
        assert_eq!(spans.len(), 2);
        for (span, row) in spans.iter().zip(&rows) {
            assert_eq!(&with_tail[span.clone()], &row[..]);
        }

        // A payload with no active rows at all still ends with the tail, and
        // that shape is the one an unchanged hour actually emits.
        assert_eq!(
            active_row_spans(&tail, 0).expect("a tail with no rows before it"),
            Vec::new()
        );

        // And a whole payload folds, which is the entry point the benchmark and
        // the provenance scan use.
        let s = schema();
        let payload = payload_with(&[full_country_row()], &rows, 0, |p| {
            p.extend_from_slice(&tail);
        });
        folded(&s, &payload).expect("a payload carrying the tail folds");
    }

    // Measuring is not trusting: bytes that are not a tail are still refused.
    let mut wrong_magic = xui09_tail(&[], 0, 0, &[]);
    wrong_magic[0] ^= 0xff;
    assert!(xui09_tail_len(&wrong_magic).is_err(), "only XU09 is a tail");
    let truncated = xui09_tail(&["fighter1"], 0, 0, &[(0, 3, 0)]);
    for len in 1..truncated.len() {
        assert!(
            xui09_tail_len(&truncated[..len]).is_err(),
            "a tail truncated at {len} must be refused, not measured short"
        );
    }
    let mut trailing = table.clone();
    trailing.extend_from_slice(&xui09_tail(&[], 0, 0, &[]));
    trailing.push(0);
    assert!(
        active_row_spans(&trailing, 2).is_err(),
        "bytes after the tail are still trailing bytes"
    );
}

// --- The frame boundary ------------------------------------------------------

/// The whole state after folding one payload into a fresh accumulator.
fn folded(s: &Schema, p: &[u8]) -> core::result::Result<TechState, String> {
    let mut acc = TechState::new();
    fold_payload_at_hour(s, &mut acc, p, Some(FRAME_HOUR))?;
    Ok(acc)
}

#[test]
fn fold_payload_reproduces_the_folds_two_collections() {
    let s = schema();
    let p = payload(
        &[full_country_row(), country_row(9, 0b011, 7, 2)],
        &[
            active_row(&measured_active_spec()),
            active_row(&ActiveSpec {
                tag: 9,
                slot_index: 0,
                ..measured_active_spec()
            }),
        ],
    );
    let state = folded(&s, &p).expect("folds");
    assert_eq!(state.get(COUNTRIES_INDEX).len(), 2);
    assert_eq!(state.get(ACTIVE_RESEARCH_INDEX).len(), 2);
    assert_eq!(state.rows(), 4);
    assert!(state.get(ACTIVE_RESEARCH_INDEX).contains_key(&key(&[7, 2])));
    assert!(state.get(ACTIVE_RESEARCH_INDEX).contains_key(&key(&[9, 0])));
    assert_eq!(state.as_slice().len(), COLLECTION_COUNT);
    assert_eq!(TechState::default(), TechState::new());
}

#[test]
fn a_country_the_payload_omits_keeps_its_research_state() {
    // `fold_tech` publishes with `for (tag, obj) in per_tag` (`:13577`), so it
    // touches only the tags the payload carries; a country the collector could
    // not read this hour (`attach_log.rs:68824`, `:80504`) simply is not in the
    // payload and keeps what it had. Modelling each payload as the whole state
    // would tombstone it and lose research the recorder still holds.
    let s = schema();
    let c = countries();
    let mut acc = TechState::new();
    fold_payload_at_hour(
        &s,
        &mut acc,
        &payload(
            &[country_row(7, 0b111, 41, 4), country_row(9, 0b111, 7, 2)],
            &[
                active_row(&measured_active_spec()),
                active_row(&ActiveSpec { tag: 9, slot_index: 0, ..measured_active_spec() }),
            ],
        ),
        Some(FRAME_HOUR),
    )
    .expect("folds");
    assert_eq!(acc.get(COUNTRIES_INDEX).len(), 2);
    assert_eq!(acc.get(ACTIVE_RESEARCH_INDEX).len(), 2);

    // Hour 2 carries only country 7. Country 9 must survive whole — its record
    // and its active row.
    fold_payload_at_hour(
        &s,
        &mut acc,
        &payload(
            &[country_row(7, 0b111, 42, 4)],
            &[active_row(&measured_active_spec())],
        ),
        Some(FRAME_HOUR + 1),
    )
    .expect("folds");
    assert_eq!(acc.get(COUNTRIES_INDEX).len(), 2, "country 9 is retained");
    assert_eq!(
        acc.get(COUNTRIES_INDEX)[&key(&[9])].values[index_of(&c, field::COMPLETED_TECH_COUNT)],
        Value::Unsigned(7),
        "country 9 keeps the value it had"
    );
    assert_eq!(
        acc.get(COUNTRIES_INDEX)[&key(&[7])].values[index_of(&c, field::COMPLETED_TECH_COUNT)],
        Value::Unsigned(42),
        "country 7 is updated"
    );
    assert!(acc.get(ACTIVE_RESEARCH_INDEX).contains_key(&key(&[9, 0])));
    assert_eq!(acc.get(ACTIVE_RESEARCH_INDEX).len(), 2);
}

#[test]
fn a_carried_country_has_its_whole_slot_set_replaced() {
    // The other half of the same rule: `per_tag` gives every carried country a
    // fresh empty `in_progress_live` (`:13321`) before any active row attaches,
    // so a slot the new payload does not repeat is gone — and only for that
    // country.
    let s = schema();
    let mut acc = TechState::new();
    fold_payload_at_hour(
        &s,
        &mut acc,
        &payload(
            &[full_country_row(), country_row(9, 0b111, 7, 2)],
            &[
                active_row(&measured_active_spec()),
                active_row(&ActiveSpec { slot_index: 3, ..measured_active_spec() }),
                active_row(&ActiveSpec { tag: 9, slot_index: 0, ..measured_active_spec() }),
            ],
        ),
        Some(FRAME_HOUR),
    )
    .expect("folds");
    assert_eq!(acc.get(ACTIVE_RESEARCH_INDEX).len(), 3);

    // Country 7 comes back with one slot; its slot 2 must go, country 9's must
    // stay even though country 9 is not in this payload at all.
    fold_payload_at_hour(
        &s,
        &mut acc,
        &payload(
            &[full_country_row()],
            &[active_row(&ActiveSpec { slot_index: 3, ..measured_active_spec() })],
        ),
        Some(FRAME_HOUR + 1),
    )
    .expect("folds");
    let active_state = acc.get(ACTIVE_RESEARCH_INDEX);
    assert_eq!(active_state.len(), 2);
    assert!(!active_state.contains_key(&key(&[7, 2])), "slot 2 was dropped");
    assert!(active_state.contains_key(&key(&[7, 3])));
    assert!(active_state.contains_key(&key(&[9, 0])), "another country is untouched");

    // A carried country with no active rows at all clears its slots and only
    // its slots.
    fold_payload_at_hour(
        &s,
        &mut acc,
        &payload(&[full_country_row()], &[]),
        Some(FRAME_HOUR + 2),
    )
    .expect("folds");
    assert_eq!(acc.get(ACTIVE_RESEARCH_INDEX).len(), 1);
    assert!(acc.get(ACTIVE_RESEARCH_INDEX).contains_key(&key(&[9, 0])));
}

#[test]
fn an_active_row_referencing_an_unknown_tag_is_a_hard_error() {
    // `world_fold.rs:13564`-`:13566`. It cannot be checked one row at a time,
    // which is why the frame boundary lives in this module. It is the frame's own
    // country rows that matter, not the accumulator's: `per_tag` is rebuilt from
    // the payload every time (`:13312`).
    let s = schema();
    let p = payload(
        &[full_country_row()],
        &[active_row(&ActiveSpec {
            tag: 999,
            ..measured_active_spec()
        })],
    );
    assert!(folded(&s, &p).is_err());
    // Even with tag 999 already in the accumulator from an earlier hour.
    let mut acc = TechState::new();
    fold_payload_at_hour(
        &s,
        &mut acc,
        &payload(&[country_row(999, 0b111, 1, 1)], &[]),
        Some(FRAME_HOUR),
    )
    .expect("folds");
    assert!(fold_payload_at_hour(&s, &mut acc, &p, Some(FRAME_HOUR + 1)).is_err());
}

#[test]
fn a_repeated_country_tag_folds_last_wins_and_a_repeated_slot_is_refused() {
    let s = schema();
    // `per_tag.insert` (`:13322`) overwrites, and the active rows attach to the
    // survivor, so a keyed set reproduces the fold exactly.
    let p = payload(
        &[country_row(7, 0b111, 1, 1), country_row(7, 0b111, 2, 2)],
        &[active_row(&measured_active_spec())],
    );
    let state = folded(&s, &p).expect("folds");
    let countries_c = countries();
    assert_eq!(state.get(COUNTRIES_INDEX).len(), 1);
    let row = &state.get(COUNTRIES_INDEX)[&key(&[7])];
    assert_eq!(
        row.values[index_of(&countries_c, field::COMPLETED_TECH_COUNT)],
        Value::Unsigned(2),
        "the last country row wins"
    );
    assert_eq!(state.get(ACTIVE_RESEARCH_INDEX).len(), 1);

    // A repeated (tag, slot_index) is refused: the fold `push`es both rows onto
    // its array (`:13567`) and a keyed set can hold only one, so measuring it
    // would measure a state nobody holds.
    let p = payload(
        &[full_country_row()],
        &[
            active_row(&measured_active_spec()),
            active_row(&ActiveSpec {
                progress_raw: 1,
                ..measured_active_spec()
            }),
        ],
    );
    assert!(folded(&s, &p).is_err());
}

#[test]
fn fold_payload_is_atomic_and_never_panics() {
    let s = schema();
    let good = payload(&[full_country_row()], &[active_row(&full_active_spec())]);
    assert!(folded(&s, &good).is_ok());
    // A rejected payload leaves the accumulator exactly as it was.
    let mut acc = TechState::new();
    fold_payload_at_hour(
        &s,
        &mut acc,
        &payload(&[full_country_row()], &[active_row(&measured_active_spec())]),
        Some(FRAME_HOUR),
    )
    .expect("folds");
    let before = acc.clone();
    let bad = payload(
        &[full_country_row()],
        &[active_row(&ActiveSpec { tag: 999, ..measured_active_spec() })],
    );
    assert!(fold_payload_at_hour(&s, &mut acc, &bad, Some(FRAME_HOUR + 1)).is_err());
    assert_eq!(acc, before, "a rejected payload writes nothing");

    // Every truncation and every single-byte corruption returns a diagnosis.
    for len in 0..good.len() {
        let _ = folded(&s, &good[..len]);
    }
    for i in 0..good.len() {
        let mut p = good.clone();
        p[i] ^= 0xFF;
        let _ = folded(&s, &p);
    }
    // A schema that is not this domain's is a diagnosed caller error.
    let mut wrong = s.clone();
    wrong.collections.pop();
    let mut acc = TechState::new();
    assert!(fold_payload_at_hour(&wrong, &mut acc, &good, Some(FRAME_HOUR)).is_err());
}

// --- The two clocks, and the A29 acceptance policy --------------------------

#[test]
fn a_frame_tick_is_never_used_as_the_folds_game_hour() {
    // A tick index and `game_hour_raw` share a rate and not an origin:
    // `hour_index = raw_date - 43_799_999` (`compact_contract/src/calendar.rs:9`).
    // The reference capture's raw hour 60_832_407 is tick 17_032_408. So the
    // fixed-signature entries must not evaluate the A29 same-hour gate at all —
    // and must not silently skip it either.
    let s = schema();
    let a = active();
    let row = active_row(&full_active_spec());
    for tick in [
        0u64,
        1,
        FRAME_HOUR as u64,
        (FRAME_HOUR as i64 - 43_799_999) as u64,
        u64::MAX,
    ] {
        assert!(
            record_from_active_row(&a, &row, tick).is_err(),
            "tick {tick} must refuse an A29-bearing row rather than guess a game hour"
        );
    }
    let p = payload(&[full_country_row()], &[row.clone()]);
    let mut acc = TechState::new();
    assert!(fold_payload(&s, &mut acc, &p, FRAME_HOUR as u64).is_err());
    assert_eq!(acc, TechState::new(), "a refused payload writes nothing");

    // A row with no provenance block ignores the tick entirely, at every value,
    // because d13 declares no derived field.
    let plain = active_row(&measured_active_spec());
    let first = record_from_active_row(&a, &plain, 0).expect("maps");
    for tick in [0u64, 1, FRAME_HOUR as u64, u64::MAX] {
        assert_eq!(
            record_from_active_row(&a, &plain, tick).expect("maps"),
            first,
            "tick {tick} changes nothing"
        );
    }
    let plain_payload = payload(&[full_country_row()], &[plain]);
    let mut acc = TechState::new();
    assert!(fold_payload(&s, &mut acc, &plain_payload, u64::MAX).is_ok());
}

#[test]
fn the_game_hour_entry_evaluates_the_same_hour_gate() {
    // `world_fold.rs:13505`, against the fold's own `game_hour_raw`.
    let a = active();
    let row = active_row(&full_active_spec());
    assert!(record_from_active_row_at_hour(&a, &row, Some(FRAME_HOUR)).is_ok());
    for wrong in [0i32, -1, FRAME_HOUR - 1, FRAME_HOUR + 1, i32::MAX] {
        assert!(
            record_from_active_row_at_hour(&a, &row, Some(wrong)).is_err(),
            "game hour {wrong} must fail the same-hour gate"
        );
    }
    // The fold's own `None` path (`world_fold.rs:4632`) skips the clause.
    assert!(record_from_active_row_at_hour(&a, &row, None).is_ok());
}

#[test]
fn the_a29_provenance_predicates_see_the_whole_flag_space() {
    // The public helpers a caller uses to implement the fail-closed acceptance
    // policy the module docs describe.
    let plain = active_row(&measured_active_spec());
    let named = active_row(&ActiveSpec { tech_name: Some("x"), ..measured_active_spec() });
    let full = active_row(&full_active_spec());
    assert!(!active_row_carries_a29_provenance(&plain).expect("reads"));
    assert!(!active_row_carries_a29_provenance(&named).expect("reads"));
    assert!(active_row_carries_a29_provenance(&full).expect("reads"));
    assert!(active_row_carries_a29_provenance(&plain[..ACTIVE_ROW_FIXED_LEN - 1]).is_err());

    assert!(!payload_carries_a29_provenance(&payload(&[full_country_row()], &[])).expect("reads"));
    assert!(!payload_carries_a29_provenance(&payload(
        &[full_country_row()],
        &[plain.clone(), named.clone()]
    ))
    .expect("reads"));
    // Found wherever it sits in the table, not only first.
    for rows in [
        vec![full.clone()],
        vec![plain.clone(), full.clone()],
        vec![plain, named, full],
    ] {
        assert!(payload_carries_a29_provenance(&payload(&[full_country_row()], &rows))
            .expect("reads"));
    }
    // A payload it cannot walk is an error, never a `false`.
    assert!(payload_carries_a29_provenance(&[]).is_err());
}

// --- The delta grammar, end to end ------------------------------------------

#[test]
fn a_frame_to_frame_delta_replays_to_the_folds_own_state() {
    let s = schema();
    let mut acc = TechState::new();
    fold_payload_at_hour(
        &s,
        &mut acc,
        &payload(
            &[full_country_row(), country_row(9, 0b111, 7, 2)],
            &[
                active_row(&measured_active_spec()),
                active_row(&ActiveSpec { tag: 9, slot_index: 0, ..measured_active_spec() }),
            ],
        ),
        Some(FRAME_HOUR),
    )
    .expect("folds");
    let hour0 = acc.clone();

    // Hour 1: one country's count moves, one slot finishes (its row leaves), and
    // one country stops being readable.
    fold_payload_at_hour(
        &s,
        &mut acc,
        &payload(
            &[
                country_row(7, 0b111, 43, 4),
                country_row(9, COUNTRY_FLAG_ACTIVE_RESEARCH, u32::MAX, u32::MAX),
            ],
            &[active_row(&ActiveSpec {
                progress_raw: 306_000,
                ..measured_active_spec()
            })],
        ),
        Some(FRAME_HOUR + 1),
    )
    .expect("folds");
    let hour1 = acc.clone();

    let mut baseline = Vec::new();
    encode_baseline(&s, hour0.as_slice(), &mut baseline).expect("baseline encodes");
    let (mut reader, consumed) = decode_baseline(&s, &baseline, 0).expect("baseline decodes");
    assert_eq!(consumed, baseline.len());
    assert_eq!(reader.as_slice(), hour0.as_slice());

    let deltas: Vec<_> = s
        .collections
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let d = diff(c, &reader[i], hour1.get(i));
            if d.is_empty() {
                None
            } else {
                Some(d)
            }
        })
        .collect();
    assert!(deltas[ACTIVE_RESEARCH_INDEX]
        .as_ref()
        .is_some_and(|d| d.tombstones.len() == 1));
    let mut bytes = Vec::new();
    encode_delta(&s, &reader, &deltas, &mut bytes).expect("delta encodes");
    let (decoded, consumed) = decode_delta(&s, &reader, &bytes, 0).expect("delta decodes");
    assert_eq!(consumed, bytes.len());
    for (i, d) in decoded.iter().enumerate() {
        if let Some(d) = d {
            apply(&mut reader[i], d);
        }
    }
    assert_eq!(reader.as_slice(), hour1.as_slice(), "reader replays the fold");
    // And the unknown that arrived in hour 1 survived the delta path.
    let c = countries();
    assert_eq!(
        reader[COUNTRIES_INDEX][&key(&[9])].values[index_of(&c, field::COMPLETED_TECH_COUNT)],
        Value::Unknown
    );
}


// ---------------------------------------------------------------------------
// Lane JT — the DERIVED ahead-of-time authority (`1<<5`).
// ---------------------------------------------------------------------------

/// An active row carrying only the fixed head plus whatever tails `flags` asks
/// for, with every tail filled with zeroes. Only the LENGTHS matter here.
fn active_row_with_tails(flags: u32) -> Vec<u8> {
    let mut row = vec![0u8; ACTIVE_ROW_FIXED_LEN - 16];
    row[20..24].copy_from_slice(&flags.to_le_bytes());
    row.extend_from_slice(&0i64.to_le_bytes());
    row.extend_from_slice(&0i64.to_le_bytes());
    assert_eq!(row.len(), ACTIVE_ROW_FIXED_LEN);
    if flags & ACTIVE_FLAG_AHEAD_QUARTET != 0 {
        row.extend_from_slice(&[0u8; AHEAD_QUARTET_LEN]);
    }
    if flags & ACTIVE_FLAG_A29_PROVENANCE != 0 {
        row.extend_from_slice(&[0u8; A29_PROVENANCE_LEN]);
    }
    if flags & ACTIVE_FLAG_AHEAD_DERIVED != 0 {
        row.extend_from_slice(&[0u8; A29_DERIVED_AUTHORITY_LEN]);
    }
    row
}

/// kills: any relaxation that lets an ahead-of-time quartet onto the wire
/// without an authority, and any reintroduction of the old rule that the ONLY
/// admissible authority is the UI-observed producer receipt.
///
/// The quartet is four plausible non-negative numbers. Unauthenticated, the
/// replay consumer cannot tell a derived value from an observed one from a
/// corrupted one, so exactly one of the two authority tails must accompany it —
/// never neither, and never both.
#[test]
fn an_ahead_quartet_needs_exactly_one_a29_authority() {
    let quartet_only = active_row_with_tails(ACTIVE_FLAG_AHEAD_QUARTET);
    let error = active_row_len(&quartet_only)
        .expect_err("a quartet with no authority is unauthenticated");
    assert!(error.contains("lacks an A29 producer authority"), "{error}");

    // The derived authority admits it, and the row length accounts for the tail
    // so a stream of them cannot desynchronise this port.
    let derived = active_row_with_tails(ACTIVE_FLAG_AHEAD_QUARTET | ACTIVE_FLAG_AHEAD_DERIVED);
    assert_eq!(
        active_row_len(&derived).expect("derived authority admits the quartet"),
        ACTIVE_ROW_FIXED_LEN + AHEAD_QUARTET_LEN + A29_DERIVED_AUTHORITY_LEN
    );

    // The observed receipt still admits it, unchanged.
    let observed = active_row_with_tails(ACTIVE_FLAG_AHEAD_QUARTET | ACTIVE_FLAG_A29_PROVENANCE);
    assert_eq!(
        active_row_len(&observed).expect("observed receipt admits the quartet"),
        ACTIVE_ROW_FIXED_LEN + AHEAD_QUARTET_LEN + A29_PROVENANCE_LEN
    );

    // Both at once is a contradiction: the row would claim the values were
    // simultaneously witnessed through the UI and derived without it.
    let both = active_row_with_tails(
        ACTIVE_FLAG_AHEAD_QUARTET | ACTIVE_FLAG_A29_PROVENANCE | ACTIVE_FLAG_AHEAD_DERIVED,
    );
    let error = active_row_len(&both).expect_err("two authorities disagree");
    assert!(
        error.contains("both observed and derived"),
        "{error}"
    );

    // And an authority with nothing to authenticate is refused from either side.
    let derived_alone = active_row_with_tails(ACTIVE_FLAG_AHEAD_DERIVED);
    let error =
        active_row_len(&derived_alone).expect_err("an authority needs its quartet");
    assert!(
        error.contains("derived authority without ahead quartet"),
        "{error}"
    );
}
