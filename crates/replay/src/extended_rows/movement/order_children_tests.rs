use super::*;

fn row() -> ArmyOrderPlanRow {
    let group = (7_u64 << 32) | engine::corders_group::REFERENCE_TYPE as u64;
    ArmyOrderPlanRow {
        owner_group: group,
        instance: 4,
        child_fronts: Recorded::Value(Some(OrderChildFrontInputs {
            area_province_count: i32::MAX,
            count: Some(2),
            group_child_count: Some(3),
            sections: Some(vec![
                OrderChildFront {
                    group,
                    segment_start: -1,
                    segment_end: -1,
                },
                OrderChildFront {
                    group,
                    segment_start: i32::MIN,
                    segment_end: i32::MAX,
                },
            ]),
        })),
        invasion_transports: Recorded::Value(Some(i32::MIN)),
        ..Default::default()
    }
}

#[test]
fn child_and_convoy_suffix_deltas_keep_independent_counts_and_signed_values() {
    let before = ArmyOrderPlanRow {
        owner_group: row().owner_group,
        instance: 4,
        ..Default::default()
    };
    let after = row();
    let delta = after.delta(Some(&before)).unwrap().unwrap();
    assert_eq!(delta.mask, (1 << 20) | (1 << 21));
    assert_eq!(
        ArmyOrderPlanRow::apply_delta(&delta, Some(before)).unwrap(),
        after
    );
    let mut bytes = Vec::new();
    after.encode_fields(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    assert_eq!(
        ArmyOrderPlanRow::decode_fields(after.key(), &mut input).unwrap(),
        after
    );
    input.finish().unwrap();
}

#[test]
fn published_plan_prefix_never_invents_child_or_convoy_operands() {
    let row = row();
    let version = crate::rows::compatibility::EFFECT_INPUTS_FORMAT;
    assert_eq!(
        FormatVersion::read(version)
            .unwrap()
            .fields(TableId::ArmyOrderPlans),
        20
    );
    let mut bytes = Vec::new();
    row.encode_fields(&mut bytes).unwrap();
    let expected = ArmyOrderPlanRow {
        child_fronts: Recorded::NotRecorded,
        invasion_transports: Recorded::NotRecorded,
        execution: Recorded::NotRecorded,
        ..row.clone()
    };
    let mut input = Decoder::new(&bytes);
    input.format = version;
    input.recorded_fields = 20;
    assert_eq!(
        ArmyOrderPlanRow::decode_fields(row.key(), &mut input).unwrap(),
        expected
    );
    let prefix_len = bytes.len() - input.remaining();
    let mut input = Decoder::new(&bytes[..prefix_len]);
    input.format = version;
    input.recorded_fields = 20;
    let historical = ArmyOrderPlanRow::decode_fields(row.key(), &mut input).unwrap();
    input.finish().unwrap();
    let prefix = FormatVersion::read(version)
        .unwrap()
        .fields(TableId::ArmyOrderPlans);
    assert_eq!(
        historical.not_recorded_fields(),
        TableId::ArmyOrderPlans.full_mask() & !((1 << prefix) - 1)
    );
    assert!(historical.encode_fields(&mut Vec::new()).is_err());
    assert!(historical.delta(None).is_err());
}

#[test]
fn zero_group_count_and_later_refusals_remain_distinct_from_empty_native_arrays() {
    for (group_count, sections) in [(Some(0), None), (None, None), (Some(-1), None)] {
        let mut row = row();
        let Recorded::Value(Some(inputs)) = &mut row.child_fronts else {
            unreachable!()
        };
        inputs.group_child_count = group_count;
        inputs.sections = sections;
        let mut bytes = Vec::new();
        row.encode_fields(&mut bytes).unwrap();
        assert_eq!(
            ArmyOrderPlanRow::decode_fields(row.key(), &mut Decoder::new(&bytes)).unwrap(),
            row
        );
    }
    let mut invalid = row();
    let Recorded::Value(Some(inputs)) = &mut invalid.child_fronts else {
        unreachable!()
    };
    inputs.group_child_count = Some(0);
    // Field serialization is unchecked; snapshots and deltas validate the row.
    assert!(encode_row(&invalid, 0, &mut Vec::new()).is_err());
    assert!(invalid.delta(None).is_err());
}

#[test]
fn child_prefix_roundtrips_both_raw_counts_and_rejects_extra_or_missing_entries() {
    let before = row();
    let mut after = before.clone();
    let Recorded::Value(Some(inputs)) = &mut after.child_fronts else {
        unreachable!()
    };
    inputs.count = Some(17);
    inputs.group_child_count = Some(1);
    inputs.sections.as_mut().unwrap().truncate(1);
    let delta = after.delta(Some(&before)).unwrap().unwrap();
    assert_eq!(
        ArmyOrderPlanRow::apply_delta(&delta, Some(before)).unwrap(),
        after
    );
    let mut bytes = Vec::new();
    after.encode_fields(&mut bytes).unwrap();
    assert_eq!(
        ArmyOrderPlanRow::decode_fields(after.key(), &mut Decoder::new(&bytes)).unwrap(),
        after
    );
    let Recorded::Value(Some(full)) = row().child_fronts else {
        unreachable!()
    };
    for sections in [vec![], full.sections.unwrap()] {
        let mut invalid = after.clone();
        let Recorded::Value(Some(inputs)) = &mut invalid.child_fronts else {
            unreachable!()
        };
        inputs.sections = Some(sections);
        assert!(encode_row(&invalid, 0, &mut Vec::new()).is_err());
        assert!(invalid.delta(Some(&after)).is_err());
    }
}
