//! Display steps through the row and nested value codecs ([`crate::display`]).
use super::*;
use crate::display::{DISPLAY_PRECISION_FORMAT, Step};
use crate::rows::test_rows::{Gauge, SteppedRow, TENTH};


#[test]
fn naval_condition_operands_keep_raw_digits_through_capture_snapshot_and_delta() {
    let row = NavalCombatShipRow {
        combat: 7,
        battle_slot: 1,
        organisation: 1_485_001,
        base_organisation: 3_000_001,
        escape_progress: 49_499,
        ..Default::default()
    };
    let mut rounded = row.clone();
    rounded.round_to_display();
    assert_eq!(rounded, row);
    let mut bytes = Vec::new();
    encode_row(&row, 0, &mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    assert_eq!(
        decode_row::<NavalCombatShipRow>(0, &mut input).unwrap(),
        row
    );
    input.finish().unwrap();
    let next = NavalCombatShipRow {
        organisation: 1_485_002,
        escape_progress: 49_501,
        ..row.clone()
    };
    let delta = next.delta(Some(&row)).unwrap().unwrap();
    assert_eq!(delta.apply(Some(row)).unwrap(), next);
}


fn captured() -> SteppedRow {
    SteppedRow {
        id: 3,
        exact: 123_457,
        level: 129_999,
        optional: Some(-129_999),
        recorded: Recorded::Value(150_000),
        pair: [10_001, -19_999],
        gauges: vec![Gauge {
            id: 1,
            level: 55_555,
        }],
    }
}

/// [`captured`] as the game displays it: tenths truncated toward zero, the
/// whole column rounded half away from zero, the unstepped column exact.
fn displayed() -> SteppedRow {
    SteppedRow {
        id: 3,
        exact: 123_457,
        level: 120_000,
        optional: Some(-120_000),
        recorded: Recorded::Value(200_000),
        pair: [10_000, -10_000],
        gauges: vec![Gauge {
            id: 1,
            level: 50_000,
        }],
    }
}

fn i64_bytes(value: i64) -> [u8; 8] {
    value.to_le_bytes()
}

#[test]
fn stepped_columns_store_whole_steps_and_decode_native_units() {
    let mut bytes = Vec::new();
    encode_row(&captured(), 0, &mut bytes).unwrap();
    // The key, then each field: the unstepped column exact, every stepped
    // one (top-level and nested) as its value divided by the step.
    let mut expected = vec![3];
    expected.extend(i64_bytes(123_457));
    expected.extend(i64_bytes(12));
    expected.push(1);
    expected.extend(i64_bytes(-12));
    expected.extend(i64_bytes(2));
    expected.extend(16_u32.to_le_bytes());
    expected.extend(i64_bytes(1));
    expected.extend(i64_bytes(-1));
    expected.extend(16_u32.to_le_bytes());
    expected.extend(1_u32.to_le_bytes());
    expected.extend(1_u32.to_le_bytes());
    expected.extend(i64_bytes(5));
    assert_eq!(bytes, expected);
    let mut input = Decoder::new(&bytes);
    assert_eq!(
        decode_row::<SteppedRow>(0, &mut input).unwrap(),
        displayed()
    );
    input.finish().unwrap();
}

#[test]
fn capture_rounding_matches_the_encoder_and_is_idempotent() {
    let mut row = captured();
    assert!(row.has_display_steps());
    row.round_to_display();
    assert_eq!(row, displayed());
    row.round_to_display();
    assert_eq!(row, displayed());
    let mut encoded = Vec::new();
    encode_row(&captured(), 0, &mut encoded).unwrap();
    let mut rounded = Vec::new();
    encode_row(&displayed(), 0, &mut rounded).unwrap();
    assert_eq!(encoded, rounded);
    assert!(!CountryLinkRow::default().has_display_steps());
}

#[test]
fn deltas_carry_whole_steps_and_drift_below_a_step_is_no_change() {
    let previous = displayed();
    let next = SteppedRow {
        level: 150_000,
        ..displayed()
    };
    let delta = next.delta(Some(&previous)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 1);
    // Three tenths: zigzag 6, one byte instead of three for 30000.
    assert_eq!(delta.values, [6]);
    assert_eq!(delta.apply(Some(previous.clone())).unwrap(), next);
    let mut streamed = Vec::new();
    assert!(
        next.encode_change(Some(&previous), 0, &mut streamed)
            .unwrap()
    );
    let mut input = Decoder::new(&streamed);
    let restored = decode_delta_typed::<SteppedRow>(&mut input);
    input.finish().unwrap();
    assert_eq!(restored.apply(Some(previous.clone())).unwrap(), next);

    // Capture rounds a value that moved only below its step back onto the
    // baseline, so the row writes nothing.
    let mut drifted = SteppedRow {
        level: 129_998,
        optional: Some(-120_001),
        gauges: vec![Gauge {
            id: 1,
            level: 59_999,
        }],
        ..displayed()
    };
    drifted.round_to_display();
    assert_eq!(drifted, previous);
    assert!(
        !drifted
            .encode_change(Some(&previous), 0, &mut Vec::new())
            .unwrap()
    );
}

/// A delta row read back from streamed bytes without a TableId.
fn decode_delta_typed<T: RowCodec + Fields>(input: &mut Decoder<'_>) -> DeltaRow {
    let key = input.varint().unwrap();
    let fields = T::SCHEMA.len();
    let width = fields.div_ceil(8);
    let mut mask_bytes = [0; 8];
    mask_bytes[..width].copy_from_slice(input.take(width).unwrap());
    let mask = u64::from_le_bytes(mask_bytes);
    let mut values = Vec::new();
    for (index, kind) in T::SCHEMA.iter().enumerate() {
        if mask & (1 << index) != 0 {
            kind.decode_change(input)
                .unwrap()
                .encode(&mut values)
                .unwrap();
        }
    }
    DeltaRow {
        format: crate::FORMAT_VERSION,
        key,
        mask,
        recorded_fields: fields,
        values,
    }
}

#[test]
fn recordings_before_display_steps_decode_their_columns_exact() {
    let mut bytes = Vec::new();
    encode_row(&captured(), 0, &mut bytes).unwrap();
    // The same bytes in an older format are exact values, never multiplied.
    let mut input = Decoder::new(&bytes);
    input.format = DISPLAY_PRECISION_FORMAT - 1;
    let older = decode_row::<SteppedRow>(0, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(
        older,
        SteppedRow {
            id: 3,
            exact: 123_457,
            level: 12,
            optional: Some(-12),
            recorded: Recorded::Value(2),
            pair: [1, -1],
            gauges: vec![Gauge { id: 1, level: 5 }],
        }
    );
    // An older delta changes the exact value by its exact difference.
    let delta = DeltaRow {
        format: DISPLAY_PRECISION_FORMAT - 1,
        key: 3,
        mask: 1 << 1,
        recorded_fields: SteppedRow::SCHEMA.len(),
        values: vec![6],
    };
    assert_eq!(
        delta.apply(Some(older.clone())).unwrap(),
        SteppedRow { level: 15, ..older }
    );
}

#[test]
fn step_units_beyond_the_native_range_are_malformed_and_sentinels_stay() {
    let mut bytes = vec![0];
    bytes.extend(i64_bytes(0));
    bytes.extend(i64_bytes(i64::MAX - 1));
    bytes.push(0);
    bytes.extend(i64_bytes(0));
    bytes.extend(16_u32.to_le_bytes());
    bytes.extend([0; 16]);
    bytes.extend(4_u32.to_le_bytes());
    bytes.extend(0_u32.to_le_bytes());
    assert!(decode_row::<SteppedRow>(0, &mut Decoder::new(&bytes)).is_err());
    // The same column within range decodes.
    bytes[9..17].copy_from_slice(&i64_bytes(i64::MAX / 10_000));
    let row = decode_row::<SteppedRow>(0, &mut Decoder::new(&bytes)).unwrap();
    assert_eq!(row.level, i64::MAX / 10_000 * 10_000);
    // A sentinel word is stored and read back as itself.
    let sentinel = SteppedRow {
        level: i64::MIN,
        pair: [i64::MAX, 129_999],
        ..displayed()
    };
    let mut encoded = Vec::new();
    encode_row(&sentinel, 0, &mut encoded).unwrap();
    let decoded = decode_row::<SteppedRow>(0, &mut Decoder::new(&encoded)).unwrap();
    assert_eq!(
        decoded,
        SteppedRow {
            pair: [i64::MAX, 120_000],
            ..sentinel
        }
    );
}

#[test]
fn element_steps_leave_the_other_elements_exact() {
    const SOME: Step = TENTH.only(&[1]);
    let mut pair = [129_999_i64, 129_999];
    Value::round_to(&mut pair, SOME);
    assert_eq!(pair, [129_999, 120_000]);
    let units = Value::to_units(&[129_999_i64, 129_999], SOME).unwrap();
    assert_eq!(units, [129_999, 12]);
    assert_eq!(Value::from_units(units, SOME).unwrap(), [129_999, 120_000]);
}

#[test]
fn tables_round_only_their_stepped_fields() {
    let mut rows = TableRows::CountryLinks(vec![CountryLinkRow {
        id: 4,
        canonical_slot: 129_999,
    }]);
    let before = rows.clone();
    rows.round_to_display();
    assert_eq!(rows, before);
    assert_eq!(TableId::CountryLinks.display_step(0), None);
}

/// Ships records each ship's experience again, at a hundredth of a point
/// truncated toward zero: a sample keeps the value the game's level and
/// progress read, and experience that moved only below the step is no change.
#[test]
fn ship_experience_is_recorded_at_a_hundredth() {
    assert_eq!(
        TableId::Ships.display_step(3),
        Some(ShipRow::EXPERIENCE_STEP)
    );
    let captured = ShipRow {
        name: Default::default(),
        naming: Default::default(),
        acquired_manpower: Default::default(),
        required_manpower: Default::default(),

        id: 5,
        task_force: 9,
        strength: 123_456,
        organisation: 654_321,
        // 30.00999 experience: the game's level-2 threshold (0.3 of 100)
        // reached, 0.009 below the next hundredth.
        experience: 3_000_999,
        category: 3,
        sunk_convoys: 2,
        class_id: 7,
    };
    let mut bytes = Vec::new();
    encode_row(&captured, 0, &mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    let decoded = decode_row::<ShipRow>(0, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(decoded.experience, 3_000_000);
    assert_eq!(
        decoded,
        ShipRow {
            experience: 3_000_000,
            ..captured.clone()
        }
    );
    // Negative experience truncates toward zero, as the formatter does.
    let mut negative = ShipRow {
        experience: -1_999,
        ..captured.clone()
    };
    negative.round_to_display();
    assert_eq!(negative.experience, -1_000);
    // A training award of 0.3 is thirty steps; a drift below a hundredth
    // writes nothing.
    let next = ShipRow {
        experience: 3_030_000,
        ..decoded.clone()
    };
    let delta = next.delta(Some(&decoded)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 3);
    assert_eq!(delta.values, [60]);
    let mut drifted = ShipRow {
        experience: 3_000_999,
        ..decoded.clone()
    };
    drifted.round_to_display();
    assert!(drifted.delta(Some(&decoded)).unwrap().is_none());
}

#[test]
fn remaining_grace_keeps_the_whole_percent_the_game_shows() {
    let grace = DivisionStatusRow::displayed_grace;
    // The counter hover's supply-grace share and its whole percent, for vanilla's and the
    // installed mods' NMilitary.SUPPLY_GRACE hours and an odd one.
    for hours in [72, 84, 96, 240, 336, 7] {
        let percent = i64::from(hours) * 1_000;
        let share = |remaining: i64| remaining * 100_000 / (i64::from(hours) * 100_000);
        for remaining in [
            0,
            1,
            percent - 1,
            percent,
            percent + 1,
            37 * percent + 999,
            100 * percent,
            150 * percent + 5,
            i64::MAX / 100_000,
        ] {
            let stored = grace(remaining, Some(hours));
            assert_eq!(stored % DivisionStatusRow::SUPPLY_GRACE_STEP.raw(), 0);
            assert_eq!(share(stored) / 1_000, share(remaining) / 1_000);
            assert_eq!(share(stored) % 1_000, 0, "{hours} {remaining}");
        }
    }
    // An hour's regain (at most 0.15) inside a percent stores no change;
    // 0.01 hour would have.
    assert_eq!(grace(3_600_000, Some(72)), grace(3_615_000, Some(72)));
    assert_ne!(
        DivisionStatusRow::SUPPLY_GRACE_STEP.round(3_600_000),
        DivisionStatusRow::SUPPLY_GRACE_STEP.round(3_615_000)
    );
    // Without a positive define, below zero or where the share's product
    // would wrap, the value keeps 0.01 hour; a sentinel stays.
    assert_eq!(grace(123_456, None), 123_000);
    assert_eq!(grace(123_456, Some(0)), 123_000);
    assert_eq!(grace(123_456, Some(-72)), 123_000);
    assert_eq!(grace(-123_456, Some(72)), -123_000);
    let wrapping = i64::MAX / 100_000 + 1;
    assert_eq!(grace(wrapping, Some(72)), wrapping / 1_000 * 1_000);
    for sentinel in [i64::MIN, i64::MIN + 1, i64::MAX] {
        assert_eq!(grace(sentinel, Some(72)), sentinel);
    }
    assert_eq!(crate::rows::compatibility::GRACE_PERCENT_FORMAT, 102);
}
