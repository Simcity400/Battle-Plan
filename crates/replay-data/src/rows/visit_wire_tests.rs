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

/// The recorded fields `visit_wire` hands over, encoded one after another.
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
    // Stepped experience goes in its step's units, as the row path writes it.
    assert_eq!(bytes, expected);
    assert_eq!(unrecorded, skipped);
    assert_eq!(skipped, 1 << 8);
    assert_eq!(order, (0..ShipRow::SCHEMA.len()).collect::<Vec<_>>());
    // A shared row visits what its row does.
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

/// The wire fields of a country, as `CountryRow`'s schema orders them.
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

/// A row is read from the wire only at a key its table's width holds.
#[test]
fn rows_are_read_from_the_wire_only_at_keys_their_table_holds() {
    assert!(CountryRow::from_wire(1 << 32, &mut country_wire).is_err());
    assert_eq!(CountryRow::from_wire(7, &mut country_wire).unwrap().id, 7);
}

/// An unrecorded wire field reads as the field's own absence, which only a field that can
/// be unrecorded holds.
#[test]
fn unrecorded_wire_fields_read_as_unrecorded() {
    assert_eq!(
        from_wire_field::<Recorded<Option<i32>>>(Field::NotRecorded, None).unwrap(),
        Recorded::NotRecorded
    );
    assert!(from_wire_field::<i64>(Field::NotRecorded, None).is_err());
}

/// A patch sets the fields its mask names, from their wire form, and no other.
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
