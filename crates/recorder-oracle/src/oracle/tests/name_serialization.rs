use super::*;

#[test]
fn state_names_use_stored_length_and_first_nul_without_collapsing_presence() {
    for (stored, saved, differences) in [
        (b"A\0tail".as_slice(), Some(b"A".as_slice()), 0),
        (b"\0tail".as_slice(), Some(b"".as_slice()), 0),
        (b"".as_slice(), None, 0),
        (b"".as_slice(), Some(b"".as_slice()), 1),
        (b"\0tail".as_slice(), None, 1),
        (b"A\0tail".as_slice(), Some(b"B".as_slice()), 1),
    ] {
        let (mut frame, mut save, membership, _) = fixture();
        let TableView::Complete(TableRows::States(rows)) =
            &mut *frame.tables[TableId::States.index()]
        else {
            unreachable!()
        };
        rows[0].custom_name = stored.to_vec();
        save.states.get_mut(&1).unwrap().name = saved.map(<[u8]>::to_vec);
        let context = country_context(&frame).unwrap();
        let mut report = Report::default();
        compare_states(&frame, &save, &membership, &context.links, &mut report).unwrap();
        assert_eq!(
            report.differences.len(),
            differences,
            "{stored:?} {saved:?}: {:?}",
            report.differences
        );
        assert_eq!(
            report
                .limitations
                .values()
                .flatten()
                .filter(|field| field.ends_with(".custom_name"))
                .count(),
            usize::from(stored.contains(&0)),
        );
    }
}

#[test]
fn province_names_keep_emitted_empty_sparse_rows_and_first_nul_prefixes() {
    for stored in [b"G\0tail".as_slice(), b"\0tail".as_slice()] {
        let (mut frame, mut save, _, _) = fixture();
        frame.hour = 1;
        *frame.tables[TableId::ProvinceNames.index()] =
            TableView::Complete(TableRows::ProvinceNames(vec![
                replay_data::rows::ProvinceNameRow {
                    province: 2,
                    name: stored.to_vec(),
                },
            ]));
        save.provinces.get_mut(&2).unwrap().name =
            Some(content_reader_parser::parser::c_string(stored).to_vec());
        let mut report = Report::default();
        compare_province_names(&frame, &save, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(
            report
                .limitations
                .values()
                .flatten()
                .any(|field| field == "province 2.name override")
        );
        save.provinces.get_mut(&2).unwrap().name = Some(b"different".to_vec());
        let mut report = Report::default();
        compare_province_names(&frame, &save, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        save.provinces.get_mut(&2).unwrap().name = Some(Vec::new());
        *frame.tables[TableId::ProvinceNames.index()] =
            TableView::Complete(TableRows::ProvinceNames(Vec::new()));
        let mut report = Report::default();
        compare_province_names(&frame, &save, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
    }
}

#[test]
fn faction_names_compare_the_native_first_nul_emission() {
    for (stored, saved, differences) in [
        (b"Literal\0tail".as_slice(), Some(b"Literal".as_slice()), 0),
        (b"\0tail".as_slice(), Some(b"".as_slice()), 0),
        (b"".as_slice(), None, 0),
        (b"".as_slice(), Some(b"".as_slice()), 1),
        (b"\0tail".as_slice(), None, 1),
        (b"Literal\0tail".as_slice(), Some(b"Other".as_slice()), 1),
    ] {
        let (mut frame, mut save, membership, equipment) = relations_fixture();
        let TableView::Complete(TableRows::Factions(rows)) =
            &mut *frame.tables[TableId::Factions.index()]
        else {
            unreachable!()
        };
        rows[1].name = stored.to_vec();
        save.factions.get_mut(&rows[1].id).unwrap().name = saved.map(<[u8]>::to_vec);
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        assert_eq!(
            report.differences.len(),
            differences,
            "{:?}",
            report.differences
        );
        assert_eq!(
            report
                .limitations
                .values()
                .flatten()
                .filter(|field| field.ends_with(".literal_name"))
                .count(),
            usize::from(stored.contains(&0))
        );
    }
}

#[test]
fn air_wing_names_compare_first_nul_and_require_emitted_empty_presence() {
    let (mut frame, mut saved, _, _) = fixture();
    let wing = (1_u64 << 32) | 54;
    saved.air_wings.insert(
        wing,
        save::AirWing {
            government_in_exile_tag: None,
            manpower: Some(40),
            reinforcement_setting: Some(0),
            id: wing,
            name: Some(b"A".to_vec()),
            owner: b"GER".to_vec(),
            country_override: b"---".to_vec(),
            air_base: 0,
            planes: 0,
            mission_mask: 0,
            mission_region: None,
            mission_active: None,
            stop_training_at_max_xp: false,
            region_change_penalty: None,
            transfer_progress: None,
            transferring_to: None,
            pool_ordinal: 0,
        },
    );
    saved.air.wings.push(save::air::Wing {
        id: wing,
        pool_type: b"fixture_plane".to_vec(),
        name: Some(b"A".to_vec()),
        accidents: None,
        experience: Decimal::parse(b"0").unwrap(),
        equipment: save::air::EquipmentPool {
            entries: Vec::new(),
            allow_zero: false,
        },
        ace: None,
        raid: None,
    });
    for (stored, expected, differences) in [
        (
            Recorded::Value(Some(b"A\0tail".to_vec())),
            Some(b"A".to_vec()),
            0,
        ),
        (
            Recorded::Value(Some(b"\0tail".to_vec())),
            Some(Vec::new()),
            0,
        ),
        (Recorded::Value(Some(Vec::new())), Some(Vec::new()), 0),
        (Recorded::Value(Some(Vec::new())), None, 1),
        (Recorded::Value(None), Some(Vec::new()), 1),
        (
            Recorded::Value(Some(b"A\0tail".to_vec())),
            Some(b"B".to_vec()),
            1,
        ),
    ] {
        saved.air_wings.get_mut(&wing).unwrap().name = expected;
        *frame.tables[TableId::AirWings.index()] =
            TableView::Complete(TableRows::AirWings(vec![replay_data::rows::AirWingRow {
                acquired_manpower: Some(40).into(),
                exile_country: Some(0).into(),
                replacement_setting: replay_data::rows::Recorded::Value(Some(0)),
                id: wing,
                name: stored,
                owner: 101,
                ..Default::default()
            }]));
        let countries = frame.complete::<tables::Countries>().unwrap();
        let links = countries.iter().map(|row| (row.id, row)).collect();
        let mut report = Report::default();
        compare_air_wings(&frame, &saved, &links, &mut report).unwrap();
        assert_eq!(
            report.differences.len(),
            differences,
            "{:?}",
            report.differences
        );
    }
}
