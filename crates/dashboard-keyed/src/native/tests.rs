use super::testing::{assert_published_as_walked, operate, publish_hours, resuming, write};
use crate::tests::{Changing, Mines, frame};
use dashboard_series::{
    Blob, Budget, Cache, Evidence, Identity, KeySpec, Knownness, Observation, Publication, RunMode,
    Snapshot, Span, Writer,
    family::Directory,
    memory::{Memory, publish},
};
use replay::{columnar_frames::synthetic::Synthetic, container::TableFields};
use replay_data::{
    format::FormatVersion,
    rows::{
        AirWingRaidReference, AirWingRow, CountryRow, Field, NavalRegionalMineRow, Recorded,
        RowCodec, RowsVisit, TableId, TableRows, tables,
    },
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, TableView, not_selected};

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct SelectedCountries;

impl crate::Observer for SelectedCountries {
    const NAME: &'static str = "test-selected-countries";
    const TABLES: &'static [TableId] = &[TableId::Countries];
    const READ: TableId = TableId::Countries;

    type Content = ();

    fn content(
        _source: &dashboard_base::Source,
        _options: Option<&dashboard_core::config::ContentOptions>,
        _check: &mut dyn FnMut() -> std::io::Result<()>,
    ) -> std::io::Result<Self::Content> {
        Ok(())
    }

    fn fields() -> std::io::Result<Vec<TableFields>> {
        Ok(vec![TableFields::named(
            TableId::Countries,
            &["canonical_slot", "tag"],
        )?])
    }

    fn observe(
        &mut self,
        _: FormatVersion,
        _: &DecodedFrame,
        _: &mut crate::Values,
    ) -> std::io::Result<bool> {
        unreachable!()
    }
}

fn assert_same_live_and_restored(view: &TableView, held: TableView, fields: u64, expected: bool) {
    let table = TableId::Countries;
    let mut emit = super::Emit::<SelectedCountries>::new(FormatVersion::CURRENT, 1).unwrap();
    assert_eq!(emit.fields[0], fields);
    emit.frame.tables[table.index()] = held.clone().into();
    assert_eq!(
        super::same(view, &emit.frame.tables[table.index()], fields),
        expected
    );
    let mut saved = Vec::new();
    emit.save(&mut saved).unwrap();
    let restored = super::Emit::<SelectedCountries>::load(
        FormatVersion::CURRENT,
        1,
        &Directory::default(),
        &saved,
    )
    .unwrap();
    let comparison =
        super::restore_comparison(&restored.restored[0].as_ref().unwrap().bytes, fields).unwrap();
    match &held {
        TableView::Complete(_) => assert!(super::same(&held, &comparison, fields)),
        _ => assert_eq!(comparison, held),
    }
    assert_eq!(super::same(view, &comparison, fields), expected);
    assert!(restored.frame.tables[table.index()].is_not_selected());
    let mut resaved = Vec::new();
    restored.save(&mut resaved).unwrap();
    assert_eq!(resaved, saved);
}

#[test]
fn restated_views_compare_as_main_before_and_after_save_and_load() {
    let fields = 0b11;
    let row = CountryRow {
        id: 11,
        canonical_slot: 7,
        tag: b"GER".to_vec(),
        political_power_raw: Some(5),
        border_land_doctrine_level: Recorded::NotRecorded,
        ..Default::default()
    };
    let held = TableView::Complete(TableRows::Countries(vec![row.clone()]));
    let changed = CountryRow {
        political_power_raw: Some(9),
        border_land_doctrine_level: Recorded::Value(Some(4)),
        ..row.clone()
    };
    assert_same_live_and_restored(
        &TableView::Complete(TableRows::Countries(vec![changed.clone()])),
        held.clone(),
        fields,
        true,
    );
    assert_same_live_and_restored(
        &TableView::Complete(TableRows::Countries(vec![CountryRow {
            canonical_slot: 8,
            ..changed.clone()
        }])),
        held.clone(),
        fields,
        false,
    );
    assert_same_live_and_restored(
        &TableView::Truncated {
            rows: TableRows::Countries(vec![changed]),
            missing: 1,
        },
        TableView::Truncated {
            rows: TableRows::Countries(vec![row.clone()]),
            missing: 1,
        },
        fields,
        false,
    );
    for rows in [
        vec![],
        vec![
            row.clone(),
            CountryRow {
                id: 12,
                ..row.clone()
            },
        ],
        vec![CountryRow {
            id: 12,
            ..row.clone()
        }],
        vec![CountryRow {
            tag: b"REN".to_vec(),
            ..row
        }],
    ] {
        assert_same_live_and_restored(
            &TableView::Complete(TableRows::Countries(rows)),
            held.clone(),
            fields,
            false,
        );
    }
    assert_same_live_and_restored(
        &TableView::Unavailable {
            reason: "second".into(),
        },
        TableView::Unavailable {
            reason: "first".into(),
        },
        fields,
        false,
    );
    assert_same_live_and_restored(
        &TableView::Unavailable {
            reason: "first".into(),
        },
        TableView::Unavailable {
            reason: "first".into(),
        },
        fields,
        true,
    );
    for mode in CaptureMode::ALL {
        assert_same_live_and_restored(
            &TableView::NotRecorded { mode },
            TableView::NotRecorded { mode },
            fields,
            true,
        );
    }
    assert_same_live_and_restored(&not_selected(), (*not_selected()).clone(), fields, false);
}

#[test]
fn saved_rows_rebuild_every_views_path_tables_default_fields() {
    struct RecordedDefaults;

    impl RowsVisit for RecordedDefaults {
        type Output = ();

        fn rows<R: RowCodec>(self, _: &[R]) {
            let row = R::default();
            row.check_key_fields().unwrap();
            row.visit_wire(&mut |_, field| {
                assert!(!matches!(field, Field::NotRecorded));
                Ok(())
            })
            .unwrap();
        }
    }

    macro_rules! rows {
        ($($table:ident),+ $(,)?) => {
            [$(TableRows::$table(vec![Default::default()])),+]
        };
    }
    for rows in rows![
        Countries,
        CountryLinks,
        NavalRegionalMines,
        NavalSupremacy,
        FactionMembers,
        Subjects,
        WarRelations,
        AirCountryRelations,
        RelationTuples,
        CountryCounterInputs,
        ReservedCountryNaming,
        AirWings,
        AirRegionalMissions,
        AirStatistics,
        CountryOccupationReserveManpower,
        OccupationSettings,
        IntelNetworkSummaries,
    ] {
        rows.visit(RecordedDefaults);
        let full = rows.table().full_mask();
        for fields in std::iter::once(0)
            .chain((0..rows.table().schema().len()).map(|index| full & !(1 << index)))
            .chain([full])
        {
            for view in [
                TableView::Complete(rows.clone()),
                TableView::Truncated {
                    rows: rows.clone(),
                    missing: 1,
                },
            ] {
                let mut saved = Vec::new();
                super::comparison(&view, fields, &mut saved).unwrap();
                let restored = super::restore_comparison(&saved, fields).unwrap();
                assert_eq!(restored, view);
                assert!(super::same(&view, &restored, fields));
            }
        }
    }
}

#[test]
fn mine_amount_rows_require_the_selected_amount_when_restored() {
    let row = NavalRegionalMineRow {
        country: 11,
        kind: 1,
        region: 3,
        amount: 5,
    };
    let view = TableView::Complete(TableRows::NavalRegionalMines(vec![row]));
    let mut saved = Vec::new();
    super::comparison(&view, 0, &mut saved).unwrap();
    let error = super::restore_comparison(&saved, 0).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(
        error
            .to_string()
            .contains("invalid sparse regional mine row")
    );
    saved.clear();
    super::comparison(&view, 1, &mut saved).unwrap();
    assert_eq!(super::restore_comparison(&saved, 1).unwrap(), view);
}

#[test]
fn complete_comparison_saves_only_selected_fields() {
    use replay_data::codec::put_varint;

    let rows = vec![
        CountryRow {
            id: 11,
            canonical_slot: 7,
            tag: b"GER".to_vec(),
            political_power_raw: Some(987_654),
            command_power_raw: 123_456,
            convoys_raw: Some(111_111),
            border_land_doctrine_level: Recorded::NotRecorded,
            ..Default::default()
        },
        CountryRow {
            id: 9001,
            canonical_slot: 8,
            tag: b"LONGER".to_vec(),
            political_power_raw: Some(222_222),
            stability_raw: 333_333,
            manpower_residual: -4,
            border_land_doctrine_level: Recorded::Value(Some(4)),
            ..Default::default()
        },
    ];
    let fields = 0b11 | (1 << 9);
    let mut selected = Vec::new();
    put_varint(&mut selected, rows.len() as u128);
    for row in &rows {
        put_varint(&mut selected, row.key());
        selected.push(1);
        Field::U32(row.canonical_slot)
            .encode(&mut selected)
            .unwrap();
        selected.push(1);
        Field::Bytes(&row.tag).encode(&mut selected).unwrap();
        match row.border_land_doctrine_level {
            Recorded::NotRecorded => selected.push(0),
            Recorded::Value(value) => {
                selected.push(1);
                Field::OptionalI32(value).encode(&mut selected).unwrap();
            }
        }
    }
    let view = TableView::Complete(TableRows::Countries(rows.clone()));
    let mut saved = Vec::new();
    super::comparison(&view, fields, &mut saved).unwrap();
    assert_eq!(saved.len(), 1 + 2 + 8 + selected.len());
    assert_eq!(&saved[3..11], &fields.to_le_bytes());
    assert_eq!(&saved[11..], &selected);
    let restored = super::restore_comparison(&saved, fields).unwrap();
    let TableView::Complete(TableRows::Countries(restored_rows)) = &restored else {
        unreachable!();
    };
    for row in restored_rows {
        assert_eq!(row.political_power_raw, None);
        assert_eq!(row.command_power_raw, 0);
        assert_eq!(row.stability_raw, 0);
        assert_eq!(row.convoys_raw, None);
        assert_eq!(row.manpower_residual, 0);
    }
    assert!(super::same(&view, &restored, fields));
    let mut changed = rows;
    changed[0].political_power_raw = None;
    changed[0].command_power_raw = -999;
    changed[1].manpower_residual = 888;
    let mut resaved = Vec::new();
    super::comparison(
        &TableView::Complete(TableRows::Countries(changed)),
        fields,
        &mut resaved,
    )
    .unwrap();
    assert_eq!(resaved, saved);
}

#[test]
fn load_refuses_a_changed_comparison_field_selection() {
    let mut emit = super::Emit::<SelectedCountries>::new(FormatVersion::CURRENT, 1).unwrap();
    emit.frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow::default()])).into();
    emit.fields[0] |= 1 << 2;
    let mut saved = Vec::new();
    emit.save(&mut saved).unwrap();
    let Err(error) = super::Emit::<SelectedCountries>::load(
        FormatVersion::CURRENT,
        1,
        &Directory::default(),
        &saved,
    ) else {
        panic!("a changed selection must refuse the saved comparison");
    };
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(
        error.to_string(),
        "Keyed comparison field selection changed"
    );
}

#[test]
fn populated_nested_fields_resume_with_the_same_observations_and_runs() {
    #[derive(Default, serde::Serialize, serde::Deserialize)]
    struct Wings {
        observed: u32,
    }

    impl crate::Observer for Wings {
        const NAME: &'static str = "test-nested-wings";
        const TABLES: &'static [TableId] = &[TableId::AirWings];
        const READ: TableId = TableId::AirWings;

        type Content = ();

        fn content(
            _source: &dashboard_base::Source,
            _options: Option<&dashboard_core::config::ContentOptions>,
            _check: &mut dyn FnMut() -> std::io::Result<()>,
        ) -> std::io::Result<Self::Content> {
            Ok(())
        }

        fn fields() -> std::io::Result<Vec<TableFields>> {
            Ok(vec![TableFields::all(TableId::AirWings)])
        }

        fn observe(
            &mut self,
            _: FormatVersion,
            frame: &DecodedFrame,
            values: &mut crate::Values,
        ) -> std::io::Result<bool> {
            let Ok(rows) = frame.complete::<tables::AirWings>() else {
                return Ok(false);
            };
            self.observed += 1;
            for row in rows {
                let Recorded::Value(Some(name)) = &row.name else {
                    return Ok(false);
                };
                let Recorded::Value(Some(raid)) = row.raid_reference else {
                    return Ok(false);
                };
                values.push(
                    format!("{}|{}", row.id, String::from_utf8_lossy(name)),
                    i128::from(raid.raw) + i128::from(raid.resolves),
                );
            }
            Ok(true)
        }
    }

    let mut rows = vec![
        AirWingRow {
            id: 41,
            owner: 11,
            planes: 17,
            name: Recorded::Value(Some(b"First wing".to_vec())),
            raid_reference: Recorded::Value(Some(AirWingRaidReference {
                raw: 9001,
                resolves: true,
            })),
            ..Default::default()
        },
        AirWingRow {
            id: 53,
            owner: 21,
            planes: 29,
            name: Recorded::Value(Some(b"Second wing".to_vec())),
            raid_reference: Recorded::Value(Some(AirWingRaidReference {
                raw: 8002,
                resolves: false,
            })),
            ..Default::default()
        },
    ];
    let mut frames: Vec<_> = (100..=102)
        .map(|hour| {
            let mut frame = frame(hour, Some(&[]));
            frame.tables[TableId::AirWings.index()] =
                TableView::Complete(TableRows::AirWings(rows.clone())).into();
            frame.metrics.table_ns[TableId::AirWings.index()] = 1;
            frame
        })
        .collect();
    rows[0].name = Recorded::Value(Some(b"Renamed wing".to_vec()));
    rows[1].raid_reference = Recorded::Value(Some(AirWingRaidReference {
        raw: 8002,
        resolves: true,
    }));
    frames[2].tables[TableId::AirWings.index()] =
        TableView::Complete(TableRows::AirWings(rows)).into();
    let publish = |resume| {
        super::testing::publish_bound::<Wings>(
            operate::<Wings>(&frames, &[1, 2], write::<tables::AirWings>),
            frames[0].branch,
            &[100, 101, 102],
            resume,
            None,
        )
    };
    for ((uninterrupted, resumed), observed) in
        publish(None).iter().zip(publish(Some(100))).zip([1, 1, 2])
    {
        let (uninterrupted, expected) = uninterrupted.read::<Wings>(100, 102, None);
        let (resumed, actual) = resumed.read::<Wings>(100, 102, None);
        assert_eq!(uninterrupted.observer.observed, observed);
        assert_eq!(resumed.observer.observed, observed);
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap(),
        );
    }
}

fn mines(walk: &mut Synthetic, frame: &DecodedFrame) {
    write::<tables::Countries>(walk, frame);
    write::<tables::CountryLinks>(walk, frame);
    write::<tables::NavalRegionalMines>(walk, frame);
}

/// From 100: values moving, a name appearing and ending, the mines unavailable for two hours,
/// an empty read, a gap after 112, and two owners' raw references linking to one slot.
fn scenario() -> Vec<DecodedFrame> {
    let mut frames = vec![
        frame(100, Some(&[(11, 3, 5)])),
        frame(101, Some(&[(11, 3, 5), (9001, 4, 6)])),
        frame(102, Some(&[(11, 3, 7), (21, 4, 2)])),
        frame(103, Some(&[(11, 3, 7), (21, 4, 2)])),
        frame(104, None),
        frame(105, None),
        frame(106, Some(&[(9001, 3, 7)])),
        frame(107, Some(&[(9001, 3, 7), (21, 9, 4)])),
        frame(108, Some(&[(9001, 3, 7), (21, 9, 4)])),
        frame(109, Some(&[])),
        frame(110, Some(&[(21, 4, 3)])),
        frame(111, Some(&[(21, 4, 3)])),
        frame(112, Some(&[(21, 4, 3)])),
        frame(116, Some(&[(21, 4, 3)])),
        frame(117, Some(&[(21, 4, 8), (11, 3, 1)])),
        frame(118, Some(&[(21, 4, 8), (11, 3, 1)])),
    ];
    for frame in &mut frames {
        frame.metrics.table_ns[replay_data::rows::TableId::NavalRegionalMines.index()] = 1;
    }
    frames
}

/// The native publication holds the runs, known hours, reads and names the walk's scan holds
/// of the same hours, whatever segments start, wherever the publisher cuts, after a resume, and
/// whether an observer gives hours whole or as their changes.
#[test]
fn a_native_publication_holds_what_the_walks_scan_holds() {
    let frames = scenario();
    for (starts, cuts, resume) in [
        (&[][..], &[118][..], None),
        (&[3, 9][..], &[103, 109, 118][..], Some(109)),
        (&[5, 6, 7, 14][..], &[100, 105, 112, 116, 118][..], Some(105)),
        (&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15][..], &[107, 118][..], Some(107)),
    ] {
        let label = format!("starts {starts:?}, cuts {cuts:?}, resume {resume:?}");
        let (native, walked) =
            assert_published_as_walked::<Mines>(&frames, mines, starts, cuts, resume, &label);
        assert!(native.observer.observed <= walked.observer.observed, "{label}");
        assert_published_as_walked::<Changing<Mines>>(
            &frames,
            mines,
            starts,
            cuts,
            resume,
            &format!("{label}, as changes"),
        );
    }
}

/// Each key's changes over any ascending hours, read from its runs once, are those one lookup of
/// the key at each hour gives: inside runs, across gaps and unavailable hours, before the
/// publication's first hour and after its last.
#[test]
fn changes_read_from_runs_are_those_each_hours_lookup_gives() {
    let frames = scenario();
    let hours = operate::<Mines>(&frames, &resuming(&frames, &[3, 9], Some(109)), mines);
    let published = publish_hours::<Mines>(hours, frames[0].branch, &[103, 109, 118], Some(109));
    for hours in [
        (100..=118).collect::<Vec<i32>>(),
        vec![100],
        vec![118],
        vec![95, 100, 113, 118, 130],
        vec![101, 104, 105, 106, 112, 116],
        vec![100, 103],
        vec![112, 113, 116],
        vec![112, 116],
        (90..=140).step_by(3).collect(),
        Vec::new(),
    ] {
        let [read, looked] = published.changes::<Mines>(&hours);
        assert_eq!(read, looked, "hours {hours:?}");
    }

    let budget = Budget::new(16 << 20);
    let cache = Cache::new(&budget);
    let identity = Identity {
        source: [1; 32],
        content: [2; 32],
        branch: [3; 32],
        schema: super::SCHEMA,
    };
    let mut writer = Writer::new(&budget, identity, 90).unwrap();
    let mut handles = Vec::new();
    for actor in 0..2 {
        let key = writer
            .intern(
                KeySpec {
                    schema: super::SCHEMA,
                    actor,
                    measure: 0,
                    dimensions: Blob::EMPTY,
                },
                Blob::EMPTY,
                RunMode::Exact,
                0,
            )
            .unwrap();
        writer
            .evidence(
                key,
                Evidence {
                    span: Span::new(90, 140).unwrap(),
                    known: Knownness::Known,
                    supplying_read: None,
                    holder: None,
                    identity_first: None,
                },
                None,
            )
            .unwrap();
        handles.push(key);
    }
    for (first, last, value) in [(100, 101, 5), (102, 103, 5), (105, 105, 8), (107, 108, 5)] {
        writer
            .run(
                handles[0],
                Span::new(first, last).unwrap(),
                value,
                true,
                false,
            )
            .unwrap();
    }
    let mut pages = Memory::default();
    let root = publish(
        &mut writer,
        &mut pages,
        &cache,
        Publication {
            generation: 1,
            segment: 0,
            accepted: Span::new(90, 140).unwrap(),
            reached: 140,
            refusal: None,
        },
        Blob::EMPTY,
    )
    .unwrap();
    let mut snapshot = Snapshot::open(pages, &cache, root, identity).unwrap();
    let keys = [0, 1, 2];
    for hours in [
        vec![99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109],
        vec![100, 103, 104, 107, 108, 110],
        vec![100, 107],
        vec![101, 102],
    ] {
        let read = super::changes(&mut snapshot, &handles, &keys, &hours).unwrap();
        let looked: Vec<Vec<_>> = keys
            .iter()
            .map(|&key| {
                let mut held = None;
                let mut changes = Vec::new();
                for (at, &hour) in hours.iter().enumerate() {
                    let now = match snapshot.value_at(key, hour).unwrap() {
                        Observation::Value { run, .. } => Some(run.value),
                        _ => None,
                    };
                    if now != held {
                        changes.push((at, now));
                    }
                    held = now;
                }
                changes
            })
            .collect();
        assert_eq!(read, looked, "runs with equal neighbours, hours {hours:?}");
    }
    for hours in [&[100, 100][..], &[102, 101][..]] {
        assert!(super::changes(&mut snapshot, &handles, &keys, hours).is_err());
    }
}
