use super::*;

fn ship() -> ShipRow {
    ShipRow {
        id: u64::MAX,
        name: Some(b"Cached\0\xfftail".to_vec()).into(),
        naming: Some(ShipNamingOperands {
            kind: i32::MIN,
            order: -7,
            ordered: 0xfe,
            programmatic: 0xab,
            override_name: b"\0hidden\xff".to_vec(),
            equipment: Some(u64::MAX),
        })
        .into(),
        ..Default::default()
    }
}
#[test]
fn ship_names_round_trip_exact_raw_packet_and_checkpoint() {
    for mut row in [
        ship(),
        ShipRow {
            id: 1,
            ..Default::default()
        },
    ] {
        for equipment in [Some(u64::MAX), None] {
            if let Recorded::Value(Some(naming)) = &mut row.naming {
                naming.equipment = equipment;
            }
            let mut bytes = Vec::new();
            encode_row(&row, 0, &mut bytes).unwrap();
            let mut input = Decoder::new(&bytes);
            assert_eq!(decode_row::<ShipRow>(0, &mut input).unwrap(), row);
            input.finish().unwrap();
            for cut in 0..bytes.len() {
                assert!(decode_row::<ShipRow>(0, &mut Decoder::new(&bytes[..cut])).is_err());
            }
            let rows = TableRows::Ships(vec![row.clone()]);
            bytes.clear();
            rows.encode_checkpoint_rows(&mut bytes).unwrap();
            let mut input = Decoder::new(&bytes);
            assert_eq!(
                TableId::Ships.decode_checkpoint_rows(&mut input).unwrap(),
                rows
            );
            input.finish().unwrap();
        }
    }
    for text in [Vec::new(), vec![0xff; 8193]] {
        let row = ShipRow {
            name: Some(text).into(),
            ..ship()
        };
        let mut bytes = Vec::new();
        encode_row(&row, 0, &mut bytes).unwrap();
        assert_eq!(
            decode_row::<ShipRow>(0, &mut Decoder::new(&bytes)).unwrap(),
            row
        );
    }
}
#[test]
fn ship_names_cache_operands_and_hull_delta_independently() {
    let before = ship();
    let mut cache = before.clone();
    cache.name = Some(b"Changed".to_vec()).into();
    let mut naming = before.clone();
    naming
        .naming
        .require_mut("naming")
        .unwrap()
        .as_mut()
        .unwrap()
        .order = i32::MIN;
    let mut hp = before.clone();
    hp.strength = -999;
    let refused = ShipRow {
        name: None.into(),
        naming: None.into(),
        ..before
    };
    for (after, mask) in [
        (cache, 1 << 9),
        (naming, 1 << 10),
        (hp, 1 << 1),
        (refused, (1 << 9) | (1 << 10)),
    ] {
        let delta = after.delta(Some(&before)).unwrap().unwrap();
        assert_eq!(delta.mask, mask);
        let mut wire = Vec::new();
        encode_delta(TableId::Ships, &delta, 0, &mut wire).unwrap();
        let delta = decode_delta(TableId::Ships, 0, &mut Decoder::new(&wire)).unwrap();
        assert_eq!(delta.apply(Some(before.clone())).unwrap(), after);
        assert_eq!(
            before
                .delta(Some(&after))
                .unwrap()
                .unwrap()
                .apply(Some(after))
                .unwrap(),
            before
        );
    }
    assert_eq!(ShipRow::NAMES[9..], ["name", "naming"]);
}
