//! d20 `divisions_live` — the reference domain schema.
//!
//! Derived from `world_fold.rs:23311`-`:23864`
// @world-fold-authority domain=20 functions=fold_divisions,fold_division_document lines=23311-23864 sha256=d2fe41cff1381f2b35b9aedcb651f7fed9bb2a34574d3c76d31f759e4800353a closure=51c17feccd9a8245c57a17e07ed3356c7185b2765b870c75347295321bdfd4af
//! (row decode and object build) and `extractor/documents/domains/d20_divisions_live.md`.
//! The rationale for every quantization choice is in
//! `extractor/native/runs/d20_reference/07_schema_v1.md`.
//!
//! **v1 excludes the four hook-derived counter fields.** Their predicate depends
//! on live engine helper calls, so they cost a sentinel-gated hook, and
//! instrumentation volume is the measured cause of the injection crashes. The
//! read-only path is proved first.

use crate::schema::{Collection, Field, Kind, Quant, Schema, Shape};

pub const DOMAIN: u32 = 20;
pub const DIVISIONS: u32 = 1;

/// Field ids are stable and append-only; §11 forbids reuse for a different field.
pub mod field {
    pub const DIV_ID: u32 = 1;
    pub const DIV_TYPE: u32 = 2;
    pub const OWNER: u32 = 3;
    pub const LOCATION_PROVINCE: u32 = 4;
    pub const TEMPLATE_ID: u32 = 5;
    pub const ORGANISATION: u32 = 6;
    pub const MAX_ORGANISATION: u32 = 7;
    pub const STRENGTH: u32 = 8;
    pub const MOVING: u32 = 9;
    pub const MOVE_TARGET_PROVINCE: u32 = 10;
    pub const MOVE_PROGRESS: u32 = 11;
    pub const IN_COMBAT: u32 = 12;
    pub const EXPERIENCE: u32 = 13;
    pub const CURRENT_SUPPLY: u32 = 14;
    pub const MAX_SUPPLY: u32 = 15;
    pub const SUPPLY_GAIN: u32 = 16;
    pub const DISRUPTED_SUPPLY: u32 = 17;
    pub const DIG_IN: u32 = 18;
    pub const DIG_IN_CAP: u32 = 19;
    pub const STRATEGIC_REDEPLOYMENT: u32 = 20;
    pub const CURRENT_MANPOWER: u32 = 21;
    pub const CUMULATIVE_LOSSES: u32 = 22;
    pub const FUEL: u32 = 23;
    pub const ARMY_ID: u32 = 24;
    pub const ORDER_INSTANCE_ID: u32 = 25;
    pub const ORDER_LINK: u32 = 26;
}

/// Declared precision for every continuous field except `move_progress`.
///
/// Passes the outcome test: the low-supply counter law selects frames at 0.20,
/// 0.35 and 0.50, two decimal places clear of the boundary. Passes the appearance
/// test: these set bar lengths and badge thresholds read once per frame, not
/// interpolated between samples.
pub const BAR_PRECISION: Quant = Quant::Fixed(2);

/// `order_link`: army / army group / unresolved.
pub const ORDER_LINK_CARDINALITY: u32 = 3;

pub fn schema() -> Schema {
    use field as f;
    let mut fields = vec![
        // Key: (div_id, div_type), both unsigned, per `world_fold.rs::fold_divisions`.
        Field::new(f::DIV_ID, Kind::Unsigned).key(0),
        Field::new(f::DIV_TYPE, Kind::Unsigned).key(1),
        // Position and identity.
        Field::new(f::OWNER, Kind::ContentId).optional().nullable(),
        Field::new(f::LOCATION_PROVINCE, Kind::ContentId),
        Field::new(f::TEMPLATE_ID, Kind::Unsigned).optional().nullable(),
    ];

    // Bar-drawn continuous values: quantized.
    for id in [f::ORGANISATION, f::STRENGTH] {
        fields.push(Field::new(id, Kind::Continuous).quant(BAR_PRECISION));
    }
    // The A15 extension fields. A legacy 104-byte row does not carry them
    // (`world_fold.rs::fold_divisions`), and the schema does support them, so their
    // state on such a row is **unknown**, not absent — §3 reserves absent for
    // "not applicable, or unsupported by this schema".
    fields.push(
        Field::new(f::MAX_ORGANISATION, Kind::Continuous)
            .optional()
            .nullable()
            .unknown_capable()
            .quant(BAR_PRECISION),
    );

    fields.push(Field::new(f::MOVING, Kind::Boolean));
    fields.push(
        Field::new(f::MOVE_TARGET_PROVINCE, Kind::ContentId)
            .optional()
            .nullable(),
    );
    // The one field the appearance test forbids quantizing: the viewer
    // interpolates a unit's position between provinces from it.
    fields.push(
        Field::new(f::MOVE_PROGRESS, Kind::Continuous)
            .optional()
            .nullable()
            .quant(Quant::Exact),
    );
    fields.push(Field::new(f::IN_COMBAT, Kind::Boolean));

    for id in [
        f::EXPERIENCE,
        f::CURRENT_SUPPLY,
        f::MAX_SUPPLY,
        f::SUPPLY_GAIN,
        f::FUEL,
    ] {
        fields.push(
            Field::new(id, Kind::Continuous)
                .optional()
                .nullable()
                .quant(BAR_PRECISION),
        );
    }
    // Extension fields, unknown rather than absent on a legacy row.
    for id in [f::DIG_IN, f::DIG_IN_CAP] {
        fields.push(
            Field::new(id, Kind::Continuous)
                .optional()
                .nullable()
                .unknown_capable()
                .quant(BAR_PRECISION),
        );
    }

    // Both are `has_`-gated in the fold, so both can be null, and neither is
    // required. `disrupted_supply` is NOT a boolean: `world_fold.rs` folds it as
    // `disrupted_supply_raw as f64 / FIXED_SCALE` with the comment "Null when the
    // field was absent". `strategic_redeployment` folds as `raw != 0`, so it is.
    fields.push(
        Field::new(f::DISRUPTED_SUPPLY, Kind::Continuous)
            .optional()
            .nullable()
            .quant(BAR_PRECISION),
    );
    fields.push(
        Field::new(f::STRATEGIC_REDEPLOYMENT, Kind::Boolean)
            .optional()
            .nullable(),
    );
    fields.push(Field::new(f::CURRENT_MANPOWER, Kind::Unsigned).optional().nullable());
    // Aggregate of per-division counters; transfers can decrease it, so §4's
    // "monotone is a hint, never a validity constraint" applies.
    fields.push(Field::new(f::CUMULATIVE_LOSSES, Kind::Monotone).optional().nullable());
    fields.push(Field::new(f::ARMY_ID, Kind::Unsigned).optional().nullable());
    fields.push(Field::new(f::ORDER_INSTANCE_ID, Kind::Unsigned).optional().nullable());
    fields.push(
        Field::new(f::ORDER_LINK, Kind::Enum)
            .optional()
            .nullable()
            .enum_card(ORDER_LINK_CARDINALITY),
    );

    Schema {
        domain: DOMAIN,
        cadence_log2: 0,
        collections: vec![Collection {
            collection_id: DIVISIONS,
            shape: Shape::KeyedSet,
            fields,
        }],
        event_part: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::Record;
    use crate::value::Value;
    use crate::TickGrid;

    fn grid() -> TickGrid {
        TickGrid::new(3).expect("3 is a legal grid")
    }

    #[test]
    fn the_schema_is_valid() {
        schema().validate(grid().log2()).expect("d20 schema is well-formed");
    }

    #[test]
    fn it_is_a_keyed_set_with_a_composite_key() {
        let s = schema();
        let c = &s.collections[0];
        assert_eq!(c.shape, Shape::KeyedSet, "baseline replaces, deltas upsert");
        let keys = c.key_fields();
        assert_eq!(keys.len(), 2, "world_fold builds the uid as div_id:div_type");
        assert_eq!(keys[0].field_id, field::DIV_ID);
        assert_eq!(keys[1].field_id, field::DIV_TYPE);
        for k in &keys {
            assert!(k.kind.legal_as_key(), "both key components are unsigned");
        }
    }

    #[test]
    fn move_progress_is_exact_and_the_bars_are_not() {
        let s = schema();
        let c = &s.collections[0];
        let find = |id: u32| c.fields.iter().find(|f| f.field_id == id).unwrap();
        assert_eq!(
            find(field::MOVE_PROGRESS).quant,
            Quant::Exact,
            "the viewer interpolates position from it, so the appearance test forbids rounding"
        );
        for id in [field::ORGANISATION, field::STRENGTH] {
            assert_eq!(
                find(id).quant,
                BAR_PRECISION,
                "bar lengths are drawn, not interpolated, so they may be quantized"
            );
        }
    }

    #[test]
    fn v1_carries_no_hook_derived_field() {
        // The four counter_* fields are excluded until the read-only path is
        // proved; if one reappears it must come with a justified hook.
        let s = schema();
        assert_eq!(
            s.collections[0].fields.len(),
            26,
            "2 key + 24 sampleable non-key fields"
        );
    }

    #[test]
    fn a_full_row_roundtrips_byte_for_byte() {
        let s = schema();
        let c = &s.collections[0];
        let values: Vec<Value> = c
            .value_fields()
            .map(|f| match f.kind {
                Kind::Boolean => Value::Bool(true),
                Kind::Enum => Value::Enum(1),
                Kind::ContentId => Value::ContentId(42),
                Kind::Monotone => Value::Unsigned(7),
                Kind::Continuous => Value::Continuous(0.5),
                _ => Value::Unsigned(3),
            })
            .collect();
        let record = Record::new(values.clone());
        let mut bytes = Vec::new();
        record.encode(c, &mut bytes).expect("encodes");
        let (back, pos) = Record::decode(c, &bytes, 0).expect("decodes");
        assert_eq!(pos, bytes.len(), "consumes exactly its own bytes");

        let mut reencoded = Vec::new();
        back.encode(c, &mut reencoded).expect("re-encodes");
        assert_eq!(reencoded, bytes, "decode/encode is byte-identical");
    }

    #[test]
    fn the_four_states_survive_a_roundtrip_on_a_real_row() {
        let s = schema();
        let c = &s.collections[0];
        // owner absent, template null, max_organisation unknown, experience zero.
        let values: Vec<Value> = c
            .value_fields()
            .map(|f| match f.field_id {
                field::OWNER => Value::Absent,
                field::TEMPLATE_ID => Value::Null,
                field::MAX_ORGANISATION => Value::Unknown,
                field::EXPERIENCE => Value::Continuous(0.0),
                _ => match f.kind {
                    Kind::Boolean => Value::Bool(false),
                    Kind::Enum => Value::Enum(0),
                    Kind::ContentId => Value::ContentId(1),
                    Kind::Monotone => Value::Unsigned(0),
                    Kind::Continuous => Value::Continuous(1.0),
                    _ => Value::Unsigned(0),
                },
            })
            .collect();

        let mut bytes = Vec::new();
        Record::new(values).encode(c, &mut bytes).expect("encodes");
        let (back, _) = Record::decode(c, &bytes, 0).expect("decodes");

        let index = |id: u32| c.value_fields().position(|f| f.field_id == id).unwrap();
        assert_eq!(back.values[index(field::OWNER)], Value::Absent);
        assert_eq!(back.values[index(field::TEMPLATE_ID)], Value::Null);
        assert_eq!(back.values[index(field::MAX_ORGANISATION)], Value::Unknown);
        assert_eq!(
            back.values[index(field::EXPERIENCE)],
            Value::Continuous(0.0),
            "zero is not null, absent or unknown"
        );
    }

    #[test]
    fn only_the_extension_fields_are_unknown_capable() {
        // Only the three A15 extension fields are unknown-capable, so the unknown
        // bitmap is one byte rather than zero — and stays one byte however many
        // fields the schema grows, because it is sized by that count alone.
        let s = schema();
        let c = &s.collections[0];
        // The A15 extension fields are unknown-capable because a legacy row does
        // not carry them; everything else is not, so most rows spend zero bytes
        // on that bitmap. This is the mechanism that makes four states free.
        let unknown_capable: Vec<u32> = c
            .value_fields()
            .filter(|f| f.unknown_capable)
            .map(|f| f.field_id)
            .collect();
        assert_eq!(
            unknown_capable,
            vec![field::MAX_ORGANISATION, field::DIG_IN, field::DIG_IN_CAP]
        );
    }

    #[test]
    fn quantizing_the_bars_actually_saves_bytes() {
        let s = schema();
        let c = &s.collections[0];
        let row: Vec<Value> = c
            .value_fields()
            .map(|f| match f.kind {
                Kind::Boolean => Value::Bool(true),
                Kind::Enum => Value::Enum(2),
                Kind::ContentId => Value::ContentId(1000),
                Kind::Monotone => Value::Unsigned(500),
                Kind::Continuous => Value::Continuous(0.75),
                _ => Value::Unsigned(1000),
            })
            .collect();
        let mut quantized = Vec::new();
        Record::new(row.clone()).encode(c, &mut quantized).expect("encodes");

        // Same row against a schema where every continuous field is exact.
        let mut exact_schema = s.clone();
        for f in exact_schema.collections[0].fields.iter_mut() {
            if matches!(f.kind, Kind::Continuous) {
                f.quant = Quant::Exact;
            }
        }
        let mut exact = Vec::new();
        Record::new(row)
            .encode(&exact_schema.collections[0], &mut exact)
            .expect("encodes");

        assert!(
            quantized.len() < exact.len(),
            "quantized {} bytes vs exact {} bytes",
            quantized.len(),
            exact.len()
        );
    }
}
