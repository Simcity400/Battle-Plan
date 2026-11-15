use super::*;

#[test]
fn carrier_base_suffix_preserves_raw_host_bytes_and_dense_occurrences() {
    let previous = AirBaseRow {
        id: u64::MAX,
        host: u64::MAX - 1,
        capacity: i32::MIN,
        carrier: Recorded::Value(None),
        ..Default::default()
    };
    let inputs = AirBaseCarrierInputs {
        registered_countries: Some(vec![
            AirBaseCountryOccurrence {
                country: 99,
                slot: 1,
            },
            AirBaseCountryOccurrence {
                country: 7,
                slot: 2,
            },
            AirBaseCountryOccurrence {
                country: 99,
                slot: 1,
            },
        ]),
        host_name: Some(vec![0xff, 0, b'A']),
    };
    let after = AirBaseRow {
        carrier: Recorded::Value(Some(inputs)),
        ..previous.clone()
    };
    let delta = after.delta(Some(&previous)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 5);
    assert_eq!(
        AirBaseRow::apply_delta(&delta, Some(previous)).unwrap(),
        after
    );
    let mut bytes = Vec::new();
    after.encode_fields(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    assert_eq!(
        AirBaseRow::decode_fields(u64::MAX as u128, &mut input).unwrap(),
        after
    );
    input.finish().unwrap();

    let known_empty = AirBaseRow {
        carrier: Recorded::Value(Some(AirBaseCarrierInputs {
            registered_countries: Some(Vec::new()),
            host_name: Some(Vec::new()),
        })),
        ..after.clone()
    };
    let missing_name = AirBaseRow {
        carrier: Recorded::Value(Some(AirBaseCarrierInputs {
            registered_countries: Some(Vec::new()),
            host_name: None,
        })),
        ..after.clone()
    };
    assert_ne!(known_empty, missing_name);
    assert!(missing_name.delta(Some(&known_empty)).unwrap().is_some());
}

#[test]
fn carrier_registration_suffix_keeps_empty_pools_and_independent_signed_fuel() {
    let inputs = AirBaseRegistrationCarrierInputs {
        country: u32::MAX,
        fuel_demand_raw: None,
        fuel_allocated_raw: Some(i64::MIN),
        pools: Some(vec![
            CarrierAirPool {
                type_index: -1,
                subunit_index: 42,
                wing_start: 0,
                wing_count: 0,
                current_fuel_raw: Some(0),
                maximum_fuel_raw: Some(0),
            },
            CarrierAirPool {
                type_index: i32::MIN,
                subunit_index: u32::MAX,
                wing_start: 0,
                wing_count: 2,
                current_fuel_raw: None,
                maximum_fuel_raw: Some(i64::MAX),
            },
            CarrierAirPool {
                type_index: -1,
                subunit_index: 42,
                wing_start: 2,
                wing_count: 0,
                current_fuel_raw: Some(0),
                maximum_fuel_raw: Some(0),
            },
        ]),
    };
    let previous = AirBaseRegistrationRow {
        base: u64::MAX,
        slot: u32::MAX,
        capacity_raw: -100_000,
        wings: vec![u64::MAX - 1, u64::MAX - 1],
        carrier: Recorded::Value(None),
        transport_planes: Recorded::Value(Retired),
        fuel_demand_raw: Default::default(),
        fuel_allocated_raw: Default::default(),
    };
    let after = AirBaseRegistrationRow {
        carrier: Recorded::Value(Some(inputs.clone())),
        ..previous.clone()
    };
    let delta = after.delta(Some(&previous)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 2);
    assert_eq!(
        AirBaseRegistrationRow::apply_delta(&delta, Some(previous)).unwrap(),
        after
    );
    let key = (u64::MAX as u128) << 32 | u32::MAX as u128;
    let mut bytes = Vec::new();
    after.encode_fields(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    assert_eq!(
        AirBaseRegistrationRow::decode_fields(key, &mut input).unwrap(),
        after
    );
    input.finish().unwrap();
    let changed = AirBaseRegistrationRow {
        carrier: Recorded::Value(Some(AirBaseRegistrationCarrierInputs {
            fuel_demand_raw: Some(0),
            pools: None,
            ..inputs
        })),
        ..after.clone()
    };
    assert_ne!(changed, after);
    assert_eq!(changed.delta(Some(&after)).unwrap().unwrap().mask, 1 << 2);
}

#[test]
fn state_scalar_delta_does_not_repeat_unchanged_pool_metadata() {
    let previous = AirBaseRegistrationRow {
        base: 77,
        slot: 19,
        fuel_demand_raw: Recorded::Value(Some(-23)),
        fuel_allocated_raw: Recorded::Value(Some(11)),
        carrier: Recorded::Value(Some(AirBaseRegistrationCarrierInputs {
            pools: Some(vec![
                CarrierAirPool {
                    type_index: -7,
                    subunit_index: 9,
                    ..Default::default()
                };
                128
            ]),
            ..Default::default()
        })),
        ..Default::default()
    };
    let after = AirBaseRegistrationRow {
        fuel_allocated_raw: Recorded::Value(Some(i64::MIN)),
        ..previous.clone()
    };
    let delta = after.delta(Some(&previous)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 5);
    assert_eq!(
        AirBaseRegistrationRow::apply_delta(&delta, Some(previous)).unwrap(),
        after
    );
}
