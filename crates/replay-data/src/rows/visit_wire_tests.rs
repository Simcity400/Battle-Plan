use super::*;

fn ship() -> ShipRow {
    ShipRow {
        id: 7,
        task_force: 3,
        strength: -5,
        organisation: 9,
        experience: 123_457,
        category: 2,
        sunk_convoys: 1,
        class_id: -4,
        acquired_manpower: Some(11).into(),
        required_manpower: Recorded::NotRecorded,
        name: Some(b"Ark\0\xff".to_vec()).into(),
        naming: Some(ShipNamingOperands {
            kind: 1,
            order: 2,
            ordered: 1,
            override_name: b"x".to_vec(),
            programmatic: 0,
            equipment: None,
        })
        .into(),
    }
}

fn visited(row: &impl RowCodec) -> (Vec<u8>, u64, Vec<usize>) {
    let mut bytes = Vec::new();
    let mut skipped = 0;
    let mut order = Vec::new();
    row.visit_wire(&mut |index, field| {
        order.push(index);
        match field {
            Field::NotRecorded => {
                skipped |= 1 << index;
                Ok(())
            }
            field => field.encode(&mut bytes),
        }
    })
    .unwrap();
    (bytes, skipped, order)
}

#[test]
fn visited_wire_fields_are_the_fields_the_row_encodes() {
    let ship = ship();
    let mut expected = Vec::new();
    let skipped = ship.encode_recorded_fields(&mut expected).unwrap();
    let (bytes, unrecorded, order) = visited(&ship);
    assert_eq!(bytes, expected);
    assert_eq!(unrecorded, skipped);
    assert_eq!(skipped, 1 << 8);
    assert_eq!(order, (0..ShipRow::SCHEMA.len()).collect::<Vec<_>>());
    let shared = std::sync::Arc::new(DivisionHistoryRow {
        division: 4,
        name: b"First".to_vec(),
        provinces: vec![3, 1],
        medal_token: Some(-2),
        ..Default::default()
    });
    let mut expected = Vec::new();
    shared.as_ref().encode_recorded_fields(&mut expected).unwrap();
    assert_eq!(visited(&shared).0, expected);
}

fn country_wire(index: usize) -> io::Result<Field<'static>> {
    Ok(match index {
        1 => Field::Bytes(b"GER"),
        2 | 6 => Field::OptionalI64(None),
        3..=5 => Field::I64(0),
        7 => Field::U8(0),
        8 => Field::I32(0),
        9 => Field::OptionalI32(None),
        _ => Field::U32(0),
    })
}

#[test]
fn rows_are_read_from_the_wire_only_at_keys_their_table_holds() {
    assert!(CountryRow::from_wire(1 << 32, &mut country_wire).is_err());
    assert_eq!(CountryRow::from_wire(7, &mut country_wire).unwrap().id, 7);
}

#[test]
fn unrecorded_wire_fields_read_as_unrecorded() {
    assert_eq!(
        from_wire_field::<Recorded<Option<i32>>>(Field::NotRecorded, None).unwrap(),
        Recorded::NotRecorded
    );
    assert!(from_wire_field::<i64>(Field::NotRecorded, None).is_err());
}

#[test]
fn derived_rows_hold_their_unrecorded_cells_as_not_recorded() {
    fn holds<T: RowCodec + Fields>(table: &str) {
        let fields = T::SCHEMA.len();
        let value = |index: usize| match T::SCHEMA[index] {
            Kind::I32 => Field::I32(-1_000 - index as i32),
            _ => Field::I64(-1_000 - index as i64),
        };
        let plain = |skip: Option<usize>| {
            let mut bytes = Vec::new();
            for index in (0..fields).filter(|&index| Some(index) != skip) {
                value(index).encode(&mut bytes).unwrap();
            }
            bytes
        };
        for absent in 0..fields {
            let row = T::from_wire(5, &mut |index| Ok(if index == absent { Field::NotRecorded } else { value(index) }))
                .unwrap_or_else(|error| panic!("{table} without field {absent}: {error}"));
            assert_eq!(row.not_recorded_fields(), 1 << absent, "{table} without field {absent}");
            let (bytes, skipped, _) = visited(&row);
            assert_eq!((bytes, skipped), (plain(Some(absent)), 1 << absent), "{table} without field {absent}");
        }
        let whole = T::from_wire(5, &mut |index| Ok(value(index))).unwrap();
        assert_eq!(whole.not_recorded_fields(), 0, "{table}");
        let mut written = Vec::new();
        assert_eq!(whole.encode_recorded_fields(&mut written).unwrap(), 0, "{table}");
        assert_eq!(written, plain(None), "{table}'s recorded row");
    }
    holds::<ArmyTotalRow>("ArmyTotals");
    holds::<TerritoryTotalRow>("TerritoryTotals");
    holds::<CountryStabilityRow>("CountryStability");
    holds::<CountryExperienceDailyRow>("CountryExperienceDaily");
    holds::<StabilityPartRow>("StabilityParts");
}

#[test]
fn patches_set_only_the_fields_their_mask_names() {
    let mut row = CountryRow::from_wire(7, &mut country_wire).unwrap();
    let mut read = Vec::new();
    assert!(
        row.patch_from_wire(1 << 1 | 1 << 4, &mut |index| {
            read.push(index);
            Ok(match index {
                1 => Field::Bytes(b"ITA"),
                _ => Field::I64(25),
            })
        })
        .unwrap()
    );
    assert_eq!(read, [1, 4]);
    let mut expected = CountryRow::from_wire(7, &mut country_wire).unwrap();
    expected.tag = b"ITA".to_vec();
    expected.stability_raw = 25;
    assert_eq!(row, expected);
}
