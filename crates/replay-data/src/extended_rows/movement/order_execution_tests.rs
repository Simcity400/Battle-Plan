use super::*;

fn plan(inputs: Option<OrderExecutionInputs>) -> ArmyOrderPlanRow {
    ArmyOrderPlanRow {
        owner_group: (7_u64 << 32) | engine_reference_values::native_values::orders_group::REFERENCE_TYPE as u64,
        instance: u32::MAX,
        execution: Recorded::Value(inputs),
        ..Default::default()
    }
}

#[test]
fn execution_round_trip_preserves_raw_bytes_extreme_terms_and_progress_refusal() {
    for inputs in [
        None,
        Some(OrderExecutionInputs {
            route_is_ok: None,
            paradrop: None,
        }),
        Some(OrderExecutionInputs {
            route_is_ok: Some(0),
            paradrop: None,
        }),
        Some(OrderExecutionInputs {
            route_is_ok: Some(1),
            paradrop: Some(ParadropPreparationInputs {
                progress: 0,
                terms: Some(ParadropPreparationTerms {
                    relation_term: None,
                    leader_term: Some(0),
                    ..Default::default()
                }),
            }),
        }),
        Some(OrderExecutionInputs {
            route_is_ok: Some(u8::MAX),
            paradrop: Some(ParadropPreparationInputs {
                progress: i64::MIN,
                terms: None,
            }),
        }),
        Some(OrderExecutionInputs {
            route_is_ok: Some(2),
            paradrop: Some(ParadropPreparationInputs {
                progress: i64::MAX,
                terms: Some(ParadropPreparationTerms {
                    target: u32::MAX,
                    country_term: i64::MIN,
                    relation_term: Some(i64::MAX),
                    leader_term: None,
                    days_term: -1,
                }),
            }),
        }),
    ] {
        let row = plan(inputs);
        let mut bytes = Vec::new();
        row.encode_fields(&mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(
            ArmyOrderPlanRow::decode_fields(row.key(), &mut input).unwrap(),
            row
        );
        input.finish().unwrap();
    }
}

#[test]
fn execution_delta_retains_progress_when_only_terms_are_refused() {
    let before = plan(Some(OrderExecutionInputs {
        route_is_ok: Some(0xff),
        paradrop: Some(ParadropPreparationInputs {
            progress: -100_001,
            terms: Some(ParadropPreparationTerms {
                target: 0,
                country_term: 1,
                relation_term: Some(-1),
                leader_term: Some(i64::MIN),
                days_term: i64::MAX,
            }),
        }),
    }));
    let mut after = before.clone();
    let Recorded::Value(Some(inputs)) = &mut after.execution else {
        unreachable!()
    };
    inputs.paradrop.as_mut().unwrap().terms = None;
    let delta = after.delta(Some(&before)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 22);
    assert_eq!(
        ArmyOrderPlanRow::apply_delta(&delta, Some(before)).unwrap(),
        after
    );
}

#[test]
fn transport_sibling_refusals_and_signed_extremes_survive_snapshots_and_deltas() {
    let base = DivisionMovementRow {
        division: u64::MAX,
        ..Default::default()
    };
    for inputs in [
        None,
        Some(DivisionOrderTransportInputs::default()),
        Some(DivisionOrderTransportInputs {
            naval_weight: Some(i64::MIN),
            paradrop_eligible: None,
            paradrop_modifiers: None,
        }),
        Some(DivisionOrderTransportInputs {
            naval_weight: None,
            paradrop_eligible: Some(0xff),
            paradrop_modifiers: Some(ParadropTransportModifiers {
                country_term: i64::MAX,
                commander_term: Some(i64::MIN),
            }),
        }),
        Some(DivisionOrderTransportInputs {
            naval_weight: Some(-1),
            paradrop_eligible: Some(0),
            paradrop_modifiers: None,
        }),
        Some(DivisionOrderTransportInputs {
            naval_weight: Some(0),
            paradrop_eligible: Some(1),
            paradrop_modifiers: Some(ParadropTransportModifiers {
                country_term: 0,
                commander_term: None,
            }),
        }),
    ] {
        let row = DivisionMovementRow {
            order_transport: Recorded::Value(inputs),
            ..base.clone()
        };
        let mut bytes = Vec::new();
        row.encode_fields(&mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(
            DivisionMovementRow::decode_fields(row.key(), &mut input).unwrap(),
            row
        );
        input.finish().unwrap();
        if let Some(delta) = row.delta(Some(&base)).unwrap() {
            assert_eq!(delta.mask, 1 << 17);
            assert_eq!(
                DivisionMovementRow::apply_delta(&delta, Some(base.clone())).unwrap(),
                row
            );
        }
    }
}

#[test]
fn nonempty_published_prefix_does_not_invent_execution_or_transport() {
    let version = crate::rows::compatibility::EFFECT_INPUTS_FORMAT;
    let format = FormatVersion::read(version).unwrap();
    let row = plan(Some(OrderExecutionInputs {
        route_is_ok: Some(1),
        paradrop: None,
    }));
    let mut bytes = Vec::new();
    row.encode_fields(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    input.format = version;
    input.recorded_fields = format.fields(TableId::ArmyOrderPlans);
    let old = ArmyOrderPlanRow::decode_fields(row.key(), &mut input).unwrap();
    assert_eq!(old.instance, row.instance);
    assert_eq!(old.execution, Recorded::NotRecorded);
    let consumed = bytes.len() - input.remaining();
    let mut input = Decoder::new(&bytes[..consumed]);
    input.format = version;
    input.recorded_fields = format.fields(TableId::ArmyOrderPlans);
    assert_eq!(
        ArmyOrderPlanRow::decode_fields(row.key(), &mut input).unwrap(),
        old
    );
    input.finish().unwrap();
    assert!(old.delta(None).is_err());

    // DivisionMovement was relaid since: that recording refuses it by name.
    assert_eq!(
        format.table_refusal(TableId::DivisionMovement).unwrap(),
        format!(
            "DivisionMovement layout or meaning changed at format {}; this recording is format {version}",
            crate::rows::compatibility::STORED_ONCE_FORMAT
        )
    );
}
