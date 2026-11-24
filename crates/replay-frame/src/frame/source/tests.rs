use super::*;
use replay_data::rows::{
    CharacterPortrait, CharacterPortraitRow, CharacterRow, CountryCommandPowerRow, DivisionRow,
    DivisionStatusRow, LexicalTokenRow, ProvinceRow, Recorded, UnitLeaderRow, tables,
};

fn frame() -> DecodedFrame {
    DecodedFrame {
        branch: 3,
        hour: 41,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
        }),
    }
}

fn old_lexical(frame: &DecodedFrame) -> &[LexicalTokenRow] {
    match &*frame.tables[TableId::LexicalTokens.index()] {
        TableView::Complete(TableRows::LexicalTokens(rows))
        | TableView::Truncated {
            rows: TableRows::LexicalTokens(rows),
            ..
        } => rows,
        _ => &[],
    }
}

#[test]
fn decoded_sources_keep_native_order_duplicates_and_search_ties() {
    let mut frame = frame();
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
        [
            (7, b"first".as_slice()),
            (7, b"last".as_slice()),
            (9, b"mod_key".as_slice()),
        ]
        .into_iter()
        .map(|(token, text)| LexicalTokenRow {
            token,
            literal: text.to_vec(),
        })
        .collect(),
    ));
    let source: &dyn RecordedSource = &frame;
    let before = old_lexical(&frame);
    let after = source.lexical().unwrap();
    assert!(after.iter().eq(before.iter()));
    assert_eq!(
        after.binary_search_by_key(&7, |row| row.token),
        before.binary_search_by_key(&7, |row| row.token)
    );
    assert_eq!(after.partition_point(|row| row.token == 7), 2);
    assert_eq!(
        after
            .slice(1..3)
            .iter()
            .map(|row| row.literal.as_slice())
            .collect::<Vec<_>>(),
        [b"last".as_slice(), b"mod_key".as_slice()]
    );
    let raw: &dyn DecodedSource = &frame;
    assert!(std::ptr::eq(
        raw.complete::<tables::LexicalTokens>().unwrap(),
        before
    ));
}

#[test]
fn decoded_evidence_keeps_gaps_layout_and_exact_refusal_bytes() {
    let lexical = TableRows::LexicalTokens(vec![LexicalTokenRow {
        token: 7,
        literal: b"mod_key".to_vec(),
    }]);
    for view in [
        TableView::Complete(lexical.clone()),
        TableView::Truncated {
            rows: lexical,
            missing: 3,
        },
        TableView::Unavailable {
            reason: "synthetic read failure".into(),
        },
        TableView::NotRecorded {
            mode: CaptureMode::Map,
        },
        TableView::Complete(TableRows::Countries(Vec::new())),
        TableView::Truncated {
            rows: TableRows::Countries(Vec::new()),
            missing: 4,
        },
    ] {
        let mut frame = frame();
        *frame.tables[TableId::LexicalTokens.index()] = view;
        let source: &dyn RecordedSource = &frame;
        assert!(source.lexical_rows().iter().eq(old_lexical(&frame).iter()));
        match (frame.complete::<tables::LexicalTokens>(), source.lexical()) {
            (Ok(before), Ok(after)) => assert!(after.iter().eq(before.iter())),
            (Err(before), Err(after)) => {
                assert_eq!(after.kind(), before.kind());
                assert_eq!(after.to_string().as_bytes(), before.to_string().as_bytes());
            }
            _ => panic!("decoded lexical admission changed"),
        }
    }
}

#[test]
fn equal_values_keep_independent_acquisition_dates_and_shared_identity() {
    let mut first = frame();
    first.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 7,
            literal: b"mod_key".to_vec(),
        }]))
        .into();
    first.tables[TableId::LexicalTokens.index()] = first.tables[TableId::LexicalTokens.index()]
        .clone()
        .with_sampled_at(Some(39));
    let mut later = first.clone();
    later.hour = 45;
    later.tables[TableId::LexicalTokens.index()] = later.tables[TableId::LexicalTokens.index()]
        .clone()
        .with_sampled_at(Some(44));
    let a: &dyn RecordedSource = &first;
    let b: &dyn RecordedSource = &later;
    assert_eq!(
        (
            a.branch(),
            a.hour(),
            a.sampled_at(TableId::LexicalTokens.index())
        ),
        (3, 41, Some(39))
    );
    assert_eq!(
        (
            b.branch(),
            b.hour(),
            b.sampled_at(TableId::LexicalTokens.index())
        ),
        (3, 45, Some(44))
    );
    assert!(
        a.identity(TableId::LexicalTokens.index())
            .unwrap()
            .same(&b.identity(TableId::LexicalTokens.index()).unwrap())
    );
    assert!(SharedView::ptr_eq(
        &(&first as &dyn DecodedSource).shared(TableId::LexicalTokens.index()),
        &first.tables[TableId::LexicalTokens.index()]
    ));
    assert!(
        a.shared_lexical()
            .holds(&later.tables[TableId::LexicalTokens.index()])
    );
    let equal = TableView::Complete(TableRows::LexicalTokens(old_lexical(&first).to_vec())).into();
    assert!(a.shared_lexical().holds(&equal));
}

#[test]
fn typed_groups_borrow_the_decoded_names_portraits_units_and_commands() {
    let mut frame = frame();
    *frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![CharacterRow {
            id: 7,
            token: b"Renamed mod character".to_vec(),
            ..Default::default()
        }]));
    *frame.tables[TableId::CharacterPortraits.index()] =
        TableView::Complete(TableRows::CharacterPortraits(vec![CharacterPortraitRow {
            character: 7,
            portraits: vec![CharacterPortrait {
                category: 2,
                size: 1,
                is_set: 1,
                kind: 1,
                identity: b"gfx/mod/portrait.dds".to_vec(),
            }],
        }]));
    *frame.tables[TableId::Divisions.index()] =
        TableView::Complete(TableRows::Divisions(vec![DivisionRow {
            id: 17,
            roster_owner: 10,
            logical_owner: 20,
            expeditionary_owner: 30,
            province: 4,
            template: 19,
            ..Default::default()
        }]));
    *frame.tables[TableId::DivisionStatus.index()] =
        TableView::Complete(TableRows::DivisionStatus(vec![DivisionStatusRow {
            division: 17,
            name: Recorded::Value(Some(b"Mod brigade".to_vec())),
            ..Default::default()
        }]));
    *frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![UnitLeaderRow {
            id: 8,
            reference_type: 2,
            character_id: 7,
            owner: 10,
            inherited_text_b: b"Mod leader".to_vec(),
            ..Default::default()
        }]));
    *frame.tables[TableId::Provinces.index()] = TableView::Truncated {
        rows: TableRows::Provinces(vec![ProvinceRow {
            id: 4,
            owner: 10,
            controller: 20,
            victory_points: -3,
            ..Default::default()
        }]),
        missing: 2,
    };
    *frame.tables[TableId::CountryCommandPower.index()] =
        TableView::Complete(TableRows::CountryCommandPower(vec![
            CountryCommandPowerRow {
                canonical_slot: 1,
                allocated_command_power_raw: -125_000,
                ..Default::default()
            },
        ]));
    let raw: &dyn DecodedSource = &frame;
    let source: &dyn RecordedSource = raw;
    assert!(std::ptr::eq(
        raw.characters().unwrap().get(0).unwrap(),
        &raw const frame.complete::<tables::Characters>().unwrap()[0]
    ));
    assert_eq!(raw.characters().unwrap()[0].token, b"Renamed mod character");
    assert!(
        raw.portraits().unwrap().iter().eq(frame
            .complete::<tables::CharacterPortraits>()
            .unwrap()
            .iter())
    );
    assert_eq!(
        raw.portraits().unwrap()[0].portraits[0].identity,
        b"gfx/mod/portrait.dds"
    );
    let unit = source.divisions().unwrap().next().unwrap();
    assert_eq!(
        (
            unit.id,
            unit.roster_owner,
            unit.logical_owner,
            unit.expeditionary_owner,
            unit.province,
            unit.template
        ),
        (17, 10, 20, 30, 4, 19)
    );
    assert_eq!(
        source.division_names().unwrap().iter().next().unwrap().name,
        &Recorded::Value(Some(b"Mod brigade".to_vec()))
    );
    let leader = source.leader((8u64 << 32) | 2).unwrap().unwrap();
    assert_eq!(
        (
            leader.id,
            leader.character_id,
            leader.owner,
            leader.inherited_text_b.as_slice()
        ),
        (8, 7, 10, b"Mod leader".as_slice())
    );
    let province = source.province_rows().get(0).unwrap();
    assert_eq!(
        (
            province.id,
            province.owner,
            province.controller,
            province.victory_points
        ),
        (4, 10, 20, -3)
    );
    assert_eq!(
        source.status(TableId::Provinces),
        OperandStatus::Truncated(2)
    );
    assert_eq!(
        source.provinces().err().unwrap().to_string(),
        "hour 41 Provinces is truncated (2 missing rows)"
    );
    assert_eq!(
        source.command_allocations().unwrap().get(1),
        Some(CommandAllocation {
            canonical_slot: 1,
            allocated_command_power_raw: -125_000
        })
    );
    assert_eq!(source.command_allocations().unwrap().get(2), None);
}
