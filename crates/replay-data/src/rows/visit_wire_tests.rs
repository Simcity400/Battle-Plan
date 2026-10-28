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
