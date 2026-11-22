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
    bytes[9..17].copy_from_slice(&i64_bytes(i64::MAX / 10_000));
    let row = decode_row::<SteppedRow>(0, &mut Decoder::new(&bytes)).unwrap();
    assert_eq!(row.level, i64::MAX / 10_000 * 10_000);
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
    let mut negative = ShipRow {
        experience: -1_999,
        ..captured.clone()
    };
    negative.round_to_display();
    assert_eq!(negative.experience, -1_000);
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
    assert_eq!(grace(3_600_000, Some(72)), grace(3_615_000, Some(72)));
    assert_ne!(
        DivisionStatusRow::SUPPLY_GRACE_STEP.round(3_600_000),
        DivisionStatusRow::SUPPLY_GRACE_STEP.round(3_615_000)
    );
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

#[test]
fn supply_ratio_and_grace_bounds_match_exact_preimages_and_wrapping() {
    assert_eq!(DivisionStatusRow::supply_ratio_bounds(Some(35_000), Some(35_499)), Some([35_499, 35_499]));
    assert_eq!(DivisionStatusRow::supply_ratio_bounds(None, Some(35_499)), Some([35_499, 35_499]));
    assert_eq!(DivisionStatusRow::supply_ratio_bounds(Some(35_000), None), Some([35_000, 35_999]));
    assert_eq!(DivisionStatusRow::supply_ratio_bounds(Some(0), None), Some([-999, 999]));
    assert_eq!(DivisionStatusRow::supply_ratio_bounds(Some(-1_000), None), Some([-1_999, -1_000]));
    assert_eq!(DivisionStatusRow::supply_ratio_bounds(Some(35_200), None), Some([35_200, 35_200]));
    assert_eq!(DivisionStatusRow::supply_ratio_bounds(None, None), None);
    for sentinel in [i64::MIN, i64::MIN + 1, i64::MAX] {
        assert_eq!(DivisionStatusRow::supply_ratio_bounds(Some(sentinel), None), Some([sentinel, sentinel]));
        assert_eq!(DivisionStatusRow::supply_grace_bounds(sentinel, None, 72), [sentinel, sentinel]);
    }
    let max_step = (i64::MAX - 1) / 1_000 * 1_000;
    assert_eq!(DivisionStatusRow::supply_ratio_bounds(Some(max_step), None), Some([max_step, i64::MAX - 1]));
    let min_step = (i64::MIN + 2) / 1_000 * 1_000;
    assert_eq!(DivisionStatusRow::supply_ratio_bounds(Some(min_step), None), Some([i64::MIN + 2, min_step]));
    assert_eq!(DivisionStatusRow::supply_grace_bounds(0, None, 72), [-999, 71_999]);
    assert_eq!(DivisionStatusRow::supply_grace_bounds(3_600_000, None, 72), [3_600_000, 3_671_999]);
    assert_eq!(DivisionStatusRow::supply_grace_bounds(3_600_000, Some(3_635_999), 72), [3_635_999, 3_635_999]);
    assert_eq!(DivisionStatusRow::supply_grace_bounds(100_000, None, 72), [100_000, 100_999]);
    assert_eq!(DivisionStatusRow::supply_grace_bounds(0, None, 0), [-999, 999]);
    assert_eq!(DivisionStatusRow::supply_grace_bounds(0, None, -72), [-999, 999]);
    assert_eq!(
        DivisionStatusRow::effective_supply_bounds([0, 0], 7_200_000, [3_600_000, 3_671_999], 72),
        [50_000, 50_999]
    );
    assert_eq!(
        DivisionStatusRow::effective_supply_bounds([0, 0], 0, [3_600_000, 3_671_999], 72),
        [0, 0]
    );
    let limit = i64::MAX / 100_000;
    let boundary_step = limit / 1_000 * 1_000;
    let boundary_bounds = DivisionStatusRow::supply_grace_bounds(boundary_step, None, 4);
    assert!(boundary_bounds[0] <= limit && boundary_bounds[1] >= limit + 1);
    assert_eq!(
        DivisionStatusRow::effective_supply_bounds([0, 0], 100_000, boundary_bounds, 4),
        [0, 100_000]
    );
}
