use super::*;

#[test]
fn weather_suspension_none_is_distinct_from_zero_and_opaque_tags_are_strict() {
    let before = SimulationFeatureRow {
        dlc_mask: 0x1234,
        human_ai: 7,
        weather_suspend: crate::rows::Recorded::Value(None),
        ..Default::default()
    };
    let after = SimulationFeatureRow {
        weather_suspend: crate::rows::Recorded::Value(Some([0, 0])),
        ..before
    };
    let delta = after.delta(Some(&before)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 2);
    assert_eq!(
        SimulationFeatureRow::apply_delta(&delta, Some(before)).unwrap(),
        after
    );
    let mut bytes = Vec::new();
    after.encode_fields(&mut bytes).unwrap();
    assert_eq!(
        SimulationFeatureRow::decode_fields(0, &mut Decoder::new(&bytes)).unwrap(),
        after
    );
    let tag = bytes.len() - 4;
    bytes[tag] = 2;
    assert!(SimulationFeatureRow::decode_fields(0, &mut Decoder::new(&bytes)).is_err());
    bytes[tag] = 1;
    bytes.pop();
    assert!(SimulationFeatureRow::decode_fields(0, &mut Decoder::new(&bytes)).is_err());
}

#[test]
fn task_force_supply_suffix_keeps_signed_inputs_and_independent_origin_refusal() {
    let previous = TaskForceCounterInputRow {
        task_force: u64::MAX,
        repair_last_mission: i32::MIN,
        name: Some(vec![0xff, 0]).into(),
        supply: None.into(),
        ..Default::default()
    };
    let pack = UnitSupplyInputs {
        asked: i64::MIN,
        received: i64::MAX,
        from_state: -1_000,
        air_drops: 0,
        disruption: -7_000,
        division_demand: UnitSupplyDemand {
            count: i32::MIN,
            value: i64::MAX,
        },
        airplane_demand: UnitSupplyDemand {
            count: 0,
            value: i64::MIN,
        },
        ship_demand: UnitSupplyDemand {
            count: i32::MAX,
            value: -1_000,
        },
        origins: Some(vec![
            UnitSupplyOrigin {
                province: -3,
                requested: i64::MIN,
                received: 0,
            },
            UnitSupplyOrigin {
                province: -3,
                requested: 0,
                received: i64::MAX,
            },
        ]),
    };
    for supply in [
        None,
        Some(pack.clone()),
        Some(UnitSupplyInputs {
            origins: None,
            ..pack
        }),
    ] {
        let row = TaskForceCounterInputRow {
            supply: supply.into(),
            ..previous.clone()
        };
        let mut bytes = Vec::new();
        row.encode_fields(&mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(
            TaskForceCounterInputRow::decode_fields(row.key(), &mut input).unwrap(),
            row
        );
        input.finish().unwrap();
        if row != previous {
            for (old, new) in [(&previous, &row), (&row, &previous)] {
                let delta = new.delta(Some(old)).unwrap().unwrap();
                assert_eq!(delta.mask, 1 << 11);
                assert_eq!(delta.apply(Some(old.clone())).unwrap(), *new);
            }
        }
    }
}

#[test]
fn pride_name_suffix_preserves_empty_bytes_and_clears_without_changing_handle() {
    let previous = CountryCounterInputRow {
        forward_visibility: crate::rows::Recorded::Value(None),
        country: u32::MAX,
        pride_of_the_fleet: u64::MAX,
        pride_name: None.into(),
        ..Default::default()
    };
    for name in [Vec::new(), vec![0xff, 0, b'N']] {
        let row = CountryCounterInputRow {
            pride_name: Some(name).into(),
            ..previous.clone()
        };
        let mut bytes = Vec::new();
        row.encode_fields(&mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(
            CountryCounterInputRow::decode_fields(row.key(), &mut input).unwrap(),
            row
        );
        input.finish().unwrap();
        for (old, new) in [(&previous, &row), (&row, &previous)] {
            let delta = new.delta(Some(old)).unwrap().unwrap();
            assert_eq!(delta.mask, 1 << 15);
            assert_eq!(delta.apply(Some(old.clone())).unwrap(), *new);
        }
    }
}

fn older_counter_prefix<T: RowCodec + Fields + std::fmt::Debug + PartialEq>(
    table: TableId,
    index: usize,
    row: &T,
) -> T {
    let introduced = table
        .last_change()
        .appended
        .iter()
        .find(|field| field.index == index)
        .expect("counter suffix has a history entry")
        .introduced;
    let format = crate::format::FormatVersion::read(introduced - 1).unwrap();
    let fields = format.fields(table);
    let mut bytes = Vec::new();
    row.encode_fields(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    for kind in &T::SCHEMA[..fields] {
        kind.decode(&mut input).unwrap();
    }
    let prefix = &bytes[..bytes.len() - input.remaining()];
    let mut input = Decoder::new(prefix);
    input.format = format.version();
    input.recorded_fields = fields;
    let decoded = T::decode_fields(row.key(), &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(
        decoded.not_recorded_fields(),
        full_mask(T::SCHEMA.len()) ^ full_mask(fields)
    );
    assert_eq!(decoded.recorded_fields_bytes().unwrap(), prefix.len());
    assert!(decoded.encode_fields(&mut Vec::new()).is_err());

    let delta = row.delta(None).unwrap().unwrap();
    let mut input = Decoder::new(&delta.values);
    for kind in &T::SCHEMA[..fields] {
        kind.decode_change(&mut input).unwrap();
    }
    bytes.clear();
    table.encode_key(row.key(), 0, &mut bytes).unwrap();
    bytes.extend_from_slice(&full_mask(fields).to_le_bytes()[..fields.div_ceil(8)]);
    bytes.extend_from_slice(&delta.values[..delta.values.len() - input.remaining()]);
    let mut input = Decoder::new(&bytes);
    input.format = format.version();
    let delta = decode_delta(table, 0, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(delta.apply::<T>(None).unwrap(), decoded);
    decoded
}

#[test]
fn counter_supply_prefix_and_current_pride_delta_preserve_independent_operands() {
    let row = TaskForceCounterInputRow {
        task_force: u64::MAX,
        repair_last_mission: i32::MIN,
        received_supply_ratio: i64::MAX,
        supply: Some(UnitSupplyInputs {
            origins: Some(Vec::new()),
            ..Default::default()
        })
        .into(),
        ..Default::default()
    };
    let decoded = older_counter_prefix(TableId::TaskForceCounterInputs, 11, &row);
    assert_eq!(decoded.supply, Recorded::NotRecorded);
    assert!(
        decoded
            .supply
            .require("supply of task force counter inputs")
            .is_err()
    );
    assert_eq!(decoded.repair_last_mission, row.repair_last_mission);
    assert_eq!(decoded.received_supply_ratio, row.received_supply_ratio);
    let row = CountryCounterInputRow {
        forward_visibility: crate::rows::Recorded::Value(None),
        country: u32::MAX,
        pride_of_the_fleet: u64::MAX,
        first_division: 81,
        pride_name: Some(vec![0xff, 0]).into(),
        ..Default::default()
    };
    let before = CountryCounterInputRow {
        pride_name: None.into(),
        air_combat_modifiers_known: true.into(),
        air_defence_factor: Some(i64::MIN).into(),
        ..row.clone()
    };
    let current = CountryCounterInputRow {
        pride_name: row.pride_name,
        ..before.clone()
    };
    let delta = current.delta(Some(&before)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 15);
    assert_eq!(delta.apply(Some(before)).unwrap(), current);
    let mut bytes = Vec::new();
    current.encode_fields(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    assert_eq!(
        CountryCounterInputRow::decode_fields(current.key(), &mut input).unwrap(),
        current
    );
    input.finish().unwrap();
}

#[test]
fn older_ship_model_snapshots_and_deltas_preserve_nonempty_counter_prefix() {
    let row = ShipModelRow {
        ship: u64::MAX,
        variant: u64::MAX - 1,
        hull_sprite: vec![0xff, 0, b'H'],
        fallback_sprite: b"fallback".to_vec(),
        model_number: i32::MIN,
        formation_flags: u64::MAX,
        counter: Some(ShipCounterInputs {
            roster_ordinal: u32::MAX,
            definition_valid: 7,
            carrier_air_base: u64::MAX - 2,
            maximum_strength: i64::MAX,
            anti_air_base: i64::MIN,
            maximum_organisation: -31,
            positive_equipment_flags: u64::MAX - 3,
        }),
        subunit_index: Some(i32::MIN).into(),
        ..Default::default()
    };
    let decoded = older_counter_prefix(TableId::ShipModels, 6, &row);
    assert_eq!(decoded.subunit_index, Recorded::NotRecorded);
    assert_eq!(
        decoded.subunit_index.require("subunit index of ship models"),
        Err("subunit index of ship models not recorded".to_owned()),
    );
    assert_eq!(decoded.ship, row.ship);
    assert_eq!(decoded.variant, row.variant);
    assert_eq!(decoded.hull_sprite, row.hull_sprite);
    assert_eq!(decoded.fallback_sprite, row.fallback_sprite);
    assert_eq!(decoded.model_number, row.model_number);
    assert_eq!(decoded.formation_flags, row.formation_flags);
    assert_eq!(decoded.counter, row.counter);
}
