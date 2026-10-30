use super::*;
use dashboard_base::prepared::{
    frame_changes::FrameChangeTracker,
    series::tests::{Fixture, assert_payloads_equal},
};
use dashboard_text::triggers::inputs;
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator, Start},
};
use replay_data::{
    rows::{
        CountryCollaborationRow, OccupationGroupLaw, OccupationSettingsRow, Recorded,
        StateAssignedManpowerRow, StateGarrisonRow, StateOccupationLawRow, TableRows, TableSchema,
    },
    scope::CaptureMode,
};
use std::collections::BTreeSet;

fn frame(hour: i32, read: bool) -> DecodedFrame {
    fixture(hour, read, 1503)
}

fn fixture(hour: i32, read: bool, countries: u32) -> DecodedFrame {
    let mut frame = dashboard_base::test_fixtures::source(1, hour, None);
    for (table, rows) in [
        (
            TableId::Countries,
            TableRows::Countries(
                (1..=countries)
                    .map(|id| CountryRow {
                        id,
                        canonical_slot: id,
                        tag: format!("country {id:04}").into_bytes(),
                        ..Default::default()
                    })
                    .collect(),
            ),
        ),
        (
            TableId::CountryLinks,
            TableRows::CountryLinks(
                (1..=countries)
                    .map(|id| CountryLinkRow {
                        id,
                        canonical_slot: id,
                    })
                    .collect(),
            ),
        ),
        (
            TableId::States,
            TableRows::States(
                (1..=2)
                    .map(|id| StateRow {
                        id,
                        owner: id,
                        controller: 1,
                        custom_name: format!("state {id}").into_bytes(),
                        ..Default::default()
                    })
                    .collect(),
            ),
        ),
        (
            TableId::StateResistance,
            TableRows::StateResistance(
                (1..=2)
                    .map(|state| StateResistanceRow {
                        state,
                        country: 2,
                        active: 1,
                        level: i64::from(state) * 3 - 5,
                        compliance: 17,
                        speed: -7,
                        compliance_speed: 11,
                        ..Default::default()
                    })
                    .collect(),
            ),
        ),
        (
            TableId::StateGarrisons,
            TableRows::StateGarrisons(
                (1..=2)
                    .map(|state| StateGarrisonRow {
                        state,
                        controller: 1,
                        occupied: 2,
                        ..Default::default()
                    })
                    .collect(),
            ),
        ),
        (
            TableId::StateOccupationLaws,
            TableRows::StateOccupationLaws(vec![StateOccupationLawRow {
                state: 1,
                controller: 1,
                occupied: 2,
                name: b"state_law".to_vec(),
            }]),
        ),
        (
            TableId::OccupationSettings,
            TableRows::OccupationSettings(vec![OccupationSettingsRow {
                country: 1,
                default_law_present: 1,
                law: b"default_law".to_vec(),
                group_laws: Recorded::Value(Some(vec![OccupationGroupLaw {
                    occupied: 2,
                    scope_country: Some(2),
                    law: None,
                }])),
                fallback_law: Recorded::Value(Some(b"fallback_law".to_vec())),
                ..Default::default()
            }]),
        ),
        (
            TableId::StateAssignedManpower,
            TableRows::StateAssignedManpower(Vec::new()),
        ),
        (
            TableId::CountryCollaboration,
            TableRows::CountryCollaboration(vec![CountryCollaborationRow {
                source: 2,
                target: 1,
                progress: -13,
            }]),
        ),
        (TableId::LexicalTokens, TableRows::LexicalTokens(Vec::new())),
    ] {
        frame.tables[table.index()] = TableView::Complete(rows).into();
    }
    if read {
        frame.metrics.table_ns[TableId::StateResistance.index()] = 1;
    }
    frame
}

fn partial(mut input: DecodedFrame, table: TableId, missing: u32) -> DecodedFrame {
    let TableView::Complete(rows) = &*input.tables[table.index()] else {
        panic!("complete fixture")
    };
    let mut rows = rows.clone();
    match &mut rows {
        TableRows::StateResistance(rows) => {
            rows.pop();
        }
        TableRows::StateGarrisons(rows) => {
            rows.pop();
        }
        TableRows::StateAssignedManpower(rows) => rows.clear(),
        TableRows::StateOccupationLaws(rows) => rows.clear(),
        TableRows::States(rows) => {
            rows.pop();
        }
        _ => panic!("partial fixture table"),
    }
    input.tables[table.index()] = TableView::Truncated { rows, missing }.into();
    input
}

fn unavailable(mut input: DecodedFrame, table: TableId, reason: &str) -> DecodedFrame {
    input.tables[table.index()] = TableView::Unavailable {
        reason: reason.into(),
    }
    .into();
    input
}

fn hours() -> Vec<DecodedFrame> {
    let mut inputs: Vec<_> = (100..120)
        .map(|hour| {
            frame(
                hour,
                ![101, 102, 103, 105, 109, 113, 116, 119].contains(&hour),
            )
        })
        .collect();
    if let TableView::Complete(TableRows::States(rows)) =
        &mut *inputs[1].tables[TableId::States.index()]
    {
        rows[0].owner = 3;
        rows[0].controller = 3;
        rows[0].custom_name = b"new name between reads".to_vec();
    }
    if let TableView::Complete(TableRows::StateOccupationLaws(rows)) =
        &mut *inputs[1].tables[TableId::StateOccupationLaws.index()]
    {
        rows[0].name = b"later_law".to_vec();
    }
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *inputs[2].tables[TableId::Countries.index()]
    {
        rows[0].id = 9001;
        rows[0].tag = b"new holder".to_vec();
        rows.sort_by_key(|row| row.id);
    }
    inputs[4] = partial(inputs[4].clone(), TableId::StateResistance, 1);
    inputs[5] = partial(inputs[5].clone(), TableId::States, 1);
    inputs[6] = partial(inputs[6].clone(), TableId::StateAssignedManpower, 7);
    inputs[7] = partial(inputs[7].clone(), TableId::StateAssignedManpower, 11);
    inputs[8] = unavailable(inputs[8].clone(), TableId::StateGarrisons, "first refusal");
    inputs[9] = unavailable(inputs[9].clone(), TableId::StateGarrisons, "second refusal");
    inputs[10] = unavailable(
        inputs[10].clone(),
        TableId::StateResistance,
        "resistance refused",
    );
    if let TableView::Complete(TableRows::States(rows)) =
        &mut *inputs[11].tables[TableId::States.index()]
    {
        rows.pop();
    }
    inputs[10] = unavailable(inputs[10].clone(), TableId::CountryLinks, "links refused");
    inputs[12] = partial(inputs[12].clone(), TableId::StateOccupationLaws, 1);
    inputs[13] = unavailable(
        inputs[13].clone(),
        TableId::CountryCollaboration,
        "relation refused",
    );
    if let TableView::Complete(TableRows::StateResistance(rows)) =
        &mut *inputs[14].tables[TableId::StateResistance.index()]
    {
        rows[0].level = i64::MAX;
        rows[1].level = 1;
        rows[1].country = u32::MAX;
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *inputs[15].tables[TableId::CountryLinks.index()]
    {
        rows.push(CountryLinkRow {
            id: 8000,
            canonical_slot: 1,
        });
    }
    if let TableView::Complete(TableRows::OccupationSettings(rows)) =
        &mut *inputs[15].tables[TableId::OccupationSettings.index()]
    {
        let mut alias = rows[0].clone();
        alias.country = 8000;
        rows.push(alias);
    }
    if let TableView::Complete(TableRows::StateResistance(rows)) =
        &mut *inputs[16].tables[TableId::StateResistance.index()]
    {
        rows.clear();
    }
    if let TableView::Complete(TableRows::StateAssignedManpower(rows)) =
        &mut *inputs[17].tables[TableId::StateAssignedManpower.index()]
    {
        rows.extend(
            [i32::MAX, 1]
                .into_iter()
                .enumerate()
                .map(|(ordinal, amount)| StateAssignedManpowerRow {
                    state: 1,
                    controller: 1,
                    occupied: 2,
                    ordinal: ordinal as u32,
                    amount,
                    ..Default::default()
                }),
        );
    }
    if let TableView::Complete(TableRows::CountryCollaboration(rows)) =
        &mut *inputs[18].tables[TableId::CountryCollaboration.index()]
    {
        rows[0].progress = 37;
    }
    inputs[18].metrics.table_ns[TableId::CountryCollaboration.index()] = 1;
    for index in [0, 4, 8, 15, 17] {
        inputs[index].metrics.table_ns[TableId::StateGarrisons.index()] = 1;
    }
    inputs
}

fn write<T: TableSchema>(walk: &mut Synthetic, input: &DecodedFrame) {
    let read = input.metrics.table_ns[T::ID.index()] != 0;
    let sample = Sample::FromReadBit;
    let input = match &*input.tables[T::ID.index()] {
        TableView::Complete(rows) => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: None,
            read,
            sample,
        },
        TableView::Truncated { rows, missing } => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: Some(*missing),
            read,
            sample,
        },
        TableView::Unavailable { reason } => Input::Unavailable {
            reason,
            read,
            sample,
        },
        TableView::NotRecorded { .. } => Input::NotRecorded { read, sample },
    };
    walk.table(T::ID, input).unwrap();
}

fn write_hour(walk: &mut Synthetic, input: &DecodedFrame) {
    write::<tables::Campaign>(walk, input);
    write::<tables::Countries>(walk, input);
    write::<tables::CountryLinks>(walk, input);
    write::<tables::States>(walk, input);
    write::<tables::StateResistance>(walk, input);
    write::<tables::StateGarrisons>(walk, input);
    write::<tables::StateOccupationLaws>(walk, input);
    write::<tables::OccupationSettings>(walk, input);
    write::<tables::StateAssignedManpower>(walk, input);
    write::<tables::CountryCollaboration>(walk, input);
    write::<tables::LexicalTokens>(walk, input);
    write::<tables::Provinces>(walk, input);
    write::<tables::ScriptFlags>(walk, input);
    write::<tables::CountryCounterInputs>(walk, input);
    for &table in dashboard_text::triggers::RECORDED_TABLES {
        if observed_tables(false).contains(&table)
            || [
                TableId::Provinces,
                TableId::ScriptFlags,
                TableId::CountryCounterInputs,
            ]
            .contains(&table)
        {
            continue;
        }
        let read = input.metrics.table_ns[table.index()] != 0;
        let sample = Sample::FromReadBit;
        let view = &*input.tables[table.index()];
        let input: Input<'_, CountryRow> = match view {
            TableView::Unavailable { reason } => Input::Unavailable {
                reason,
                read,
                sample,
            },
            TableView::NotRecorded { .. } => Input::NotRecorded { read, sample },
            _ => panic!("unhandled synthetic trigger table {table:?}"),
        };
        walk.table(table, input).unwrap();
    }
}

fn emit(out: &mut Vec<columnar::Hour>, ordered: &mut columnar::Emit) {
    for hour in std::mem::take(out) {
        ordered.observe(hour).unwrap();
    }
}

/// Where the first run's row blocks end and its projection restarts, by its
/// hours' ordinals: a restart need not start a block.
struct Placement {
    label: String,
    block: usize,
    ends: BTreeSet<usize>,
    restarts: BTreeSet<usize>,
    round_trip: bool,
}

impl Placement {
    fn ends(&self, ordinal: usize, hours: usize, first: bool) -> bool {
        ordinal + 1 == hours
            || ordinal % self.block == self.block - 1
            || first && self.ends.contains(&ordinal)
    }
}

fn placements(hours: usize) -> Vec<Placement> {
    let mut placements: Vec<_> = [
        None,
        Some(1),
        Some(2),
        Some(3),
        Some(6),
        Some(9),
        Some(14),
        Some(18),
    ]
    .into_iter()
    .map(|split: Option<usize>| Placement {
        label: format!("segments on row blocks, split={split:?}"),
        block: 3,
        ends: split
            .and_then(|split| split.checked_sub(1))
            .into_iter()
            .collect(),
        restarts: (1..hours)
            .filter(|&ordinal| ordinal % 3 == 0 || Some(ordinal) == split)
            .collect(),
        round_trip: split == Some(6),
    })
    .collect();
    placements.extend([
        Placement {
            label: "restarts inside row blocks".into(),
            block: 4,
            ends: BTreeSet::new(),
            restarts: [1, 6, 7, 13, 17].into(),
            round_trip: false,
        },
        Placement {
            label: "one walk through every row block".into(),
            block: 5,
            ends: BTreeSet::new(),
            restarts: BTreeSet::new(),
            round_trip: false,
        },
        Placement {
            label: "a restart at every hour".into(),
            block: 3,
            ends: BTreeSet::new(),
            restarts: (1..hours).collect(),
            round_trip: false,
        },
    ]);
    placements
}

/// The columnar walk and the production row path over `runs`, placed as
/// `placement` says, publish the same bytes at every row block's end.
fn check(runs: &[Vec<DecodedFrame>], catalogue: Option<Arc<Catalogue>>, placement: &Placement) {
    let format = replay_data::format::FormatVersion::CURRENT;
    let mut prototype = Prepared {
        catalogue: catalogue.clone(),
        bound: true,
        ..Default::default()
    };
    prototype.scan = scan::Scan::new(catalogue.as_deref()).bind(None);
    let mut actual = prototype.emit();
    let mut expected = prototype.fresh();
    let actual_store = Fixture::new();
    let expected_store = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut tracker = FrameChangeTracker::new();
    let tables = prototype.tables();
    let mut projector = prototype.projector(format, CaptureMode::Full).unwrap();
    let mut out: Vec<columnar::Hour> = Vec::new();
    assert!(projector.starts_at(Start::Run { continues: false }));
    assert!(projector.starts_at(Start::Run { continues: true }));
    for (run, inputs) in runs.iter().enumerate() {
        let mut walk = Synthetic::new(&tables, 1, inputs[0].hour).unwrap();
        let mut row_views =
            replay::columnar_frames::Views::new(&tables, format, CaptureMode::Full).unwrap();
        if run == 0 {
            projector
                .start(Start::Run { continues: false }, 1, inputs[0].hour)
                .unwrap();
        } else {
            projector = prototype.projector(format, CaptureMode::Full).unwrap();
            projector
                .start(Start::Run { continues: true }, 1, inputs[0].hour)
                .unwrap();
        }
        let mut first = inputs[0].hour;
        let mut opens = true;
        for (ordinal, input) in inputs.iter().enumerate() {
            let restarting = run == 0 && ordinal != 0 && placement.restarts.contains(&ordinal);
            if restarting {
                emit(&mut out, &mut actual);
                projector = prototype.projector(format, CaptureMode::Full).unwrap();
                assert!(projector.starts_at(Start::Restart(walk.state())));
                projector
                    .start(Start::Restart(walk.state()), 1, input.hour)
                    .unwrap();
            }
            let starts = ordinal == 0 || restarting;
            if opens {
                expected.begin_block();
                tracker.invalidate();
            }
            write_hour(&mut walk, input);
            walk.end(|state, manifest, _, _| {
                let (frame, raw) = row_views.advance_recorded(state, manifest, opens).unwrap();
                expected
                    .observe(
                        &frame,
                        format,
                        &flag,
                        &tracker.observe_updates(&frame, Arc::new(raw), format),
                    )
                    .unwrap();
                projector
                    .hour(
                        &segments::Hour {
                            branch: 1,
                            hour: frame.hour,
                            run: run as u32,
                            ordinal: ordinal as u32,
                            starts,
                            opens: starts,
                            state,
                            manifest,
                        },
                        &mut out,
                    )
                    .unwrap();
            })
            .unwrap();
            opens = placement.ends(ordinal, inputs.len(), run == 0);
            if !opens {
                continue;
            }
            emit(&mut out, &mut actual);
            let mut writer = actual_store.writer();
            actual
                .hold_block(1, first, input.hour)
                .unwrap()
                .write(&mut writer)
                .unwrap();
            actual.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
            let mut writer = expected_store.writer();
            expected
                .hold_block(1, first, input.hour)
                .unwrap()
                .write(&mut writer)
                .unwrap();
            expected.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
            if run == 0
                && input.hour == 100
                && inputs[0]
                    .complete::<tables::StateAssignedManpower>()
                    .is_ok_and(|rows| rows.is_empty())
            {
                let snapshot = expected.scan.snapshot();
                let row = snapshot.states.states[0].runs.last().unwrap().value.clone();
                for value in [row, actual.latest(1).unwrap()] {
                    assert_eq!(value.resistance_raw, -2);
                    assert_eq!(value.assigned_manpower, Some(0));
                    assert_eq!(value.garrison_strength_raw, Some(0));
                }
                if !prototype.reads_trigger_world() {
                    assert_eq!(
                        snapshot.trend.series["country 0001"]
                            .last()
                            .unwrap()
                            .resistance_raw,
                        "0"
                    );
                    assert_eq!(actual.resistance("country 0001"), Some(0));
                }
            }
            first = input.hour + 1;
            assert_payloads_equal(
                &actual_store.payloads(),
                &expected_store.payloads(),
                &format!(
                    "occupation {}, run={run}, hour={}",
                    placement.label, input.hour
                ),
            );
            if placement.round_trip {
                let mut restored =
                    Prepared::read(expected.saved_bytes().unwrap().as_slice()).unwrap();
                restored.catalogue = catalogue.clone();
                restored.bound = true;
                let scan = std::mem::take(&mut restored.scan).bind(restored.catalogue.as_deref());
                restored.scan = scan.bind(None);
                expected = restored;
            }
        }
    }
    assert!(!actual.keys("country 1503"));
    let sections: BTreeSet<_> = actual_store
        .payloads()
        .keys()
        .map(|(name, _)| name.clone())
        .collect();
    for section in [
        "occupied-states-1",
        "occupied-states-manifest-1",
        "occupation-trend-1-catalogue",
        "occupation-trend-values-1-0",
        "occupation-trend-values-1-1",
        "occupation-trend-ranks-1-0",
        "occupation-trend-ranks-1-1",
        "occupation-trend-facts-1",
    ] {
        assert!(
            sections.contains(section),
            "missing {section} with {}",
            placement.label
        );
    }
}

fn parity(runs: &[Vec<DecodedFrame>], catalogue: Option<Arc<Catalogue>>) {
    for placement in placements(runs[0].len()) {
        check(runs, catalogue.clone(), &placement);
    }
}

#[test]
fn whole_and_restarted_segments_publish_every_occupation_section() {
    for later in [120, 125] {
        parity(
            &[
                hours(),
                vec![
                    frame(later, false),
                    frame(later + 1, false),
                    frame(later + 2, true),
                ],
            ],
            None,
        );
    }
}

#[test]
fn equal_fresh_reads_date_facts_without_replacing_state_values() {
    let inputs: Vec<_> = (100..104).map(|hour| frame(hour, true)).collect();
    let mut prepared = Prepared::default();
    for input in &inputs {
        prepared
            .observe_frame(input, replay_data::format::FormatVersion::CURRENT, None)
            .unwrap();
    }
    let snapshot = prepared.scan.snapshot();
    assert_eq!(
        snapshot.states.states[0]
            .runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour))
            .collect::<Vec<_>>(),
        [(100, 103)]
    );
    assert_eq!(
        snapshot.trend.series["country 0001"]
            .iter()
            .map(|run| (run.first_hour, run.read_hour))
            .collect::<Vec<_>>(),
        [(100, 100), (101, 101), (102, 102), (103, 103)]
    );
    parity(&[inputs], None);
}

#[test]
fn recorded_mod_law_content_uses_the_same_scan_and_publication() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
        (
            "common/occupation_laws/mod.txt",
            "state_law={} later_law={} default_law={} fallback_law={}",
        ),
        (
            "common/defines/mod.lua",
            "NDefines.NInterface.GARRISON_STRENGTH_TO_SHOW_RED = 0.5\nNDefines.NResistance.GARRISON_STR_POW_MANPOWER = 1\nNDefines.NResistance.GARRISON_STR_POW_EQUIPMENT = 1\n",
        ),
    ]);
    parity(&[hours()], Some(Arc::new(catalogue)));
}

#[test]
fn missing_join_table_refuses_by_the_row_paths_name() {
    let format = replay_data::format::FormatVersion::CURRENT;
    let mut input = frame(100, true);
    input.tables[TableId::StateAssignedManpower.index()] = TableView::NotRecorded {
        mode: CaptureMode::Full,
    }
    .into();
    let mut walk = Synthetic::new(&observed_tables(false), 1, 100).unwrap();
    let mut projector = Prepared::default()
        .projector(format, CaptureMode::Full)
        .unwrap();
    let mut out: Vec<columnar::Hour> = Vec::new();
    write_hour(&mut walk, &input);
    walk.end(|state, manifest, frame, starts| {
        let expected = Prepared::default()
            .observe_frame(frame, format, None)
            .unwrap_err();
        let actual = projector
            .hour(
                &segments::Hour {
                    branch: 1,
                    hour: 100,
                    run: 0,
                    ordinal: 0,
                    starts,
                    opens: true,
                    state,
                    manifest,
                },
                &mut out,
            )
            .unwrap_err();
        assert_eq!(actual.to_string(), expected.to_string());
    })
    .unwrap();
}

fn triggered_catalogue() -> (Arc<Catalogue>, impl Sized) {
    let (catalogue, folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/mod.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
        ),
        (
            "common/occupation_laws/mod.txt",
            "default_law={visible={has_country_flag=controller_flag FROM={ROOT={has_country_flag=controller_flag}}} available={FROM={has_country_flag=occupied_flag}}} fallback_law={}",
        ),
    ]);
    catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    (Arc::new(catalogue), folder)
}

/// An hour whose two states' laws default to a law triggered by the
/// controllers' and the occupied country's flags, every flag set.
fn triggered(hour: i32, read: bool) -> DecodedFrame {
    use replay_data::rows::{CountryCounterInputRow, LexicalTokenRow, ScriptFlagRow};
    let mut frame = frame(hour, read);
    *frame.tables[TableId::StateOccupationLaws.index()] =
        TableView::Complete(TableRows::StateOccupationLaws(Vec::new()));
    if let TableView::Complete(TableRows::States(rows)) =
        &mut *frame.tables[TableId::States.index()]
    {
        rows[1].controller = 3;
    }
    *frame.tables[TableId::OccupationSettings.index()] =
        TableView::Complete(TableRows::OccupationSettings(
            [1, 3]
                .into_iter()
                .map(|country| OccupationSettingsRow {
                    country,
                    default_law_present: 1,
                    law: b"default_law".to_vec(),
                    group_laws: Recorded::Value(Some(vec![OccupationGroupLaw {
                        occupied: 2,
                        scope_country: Some(2),
                        law: None,
                    }])),
                    fallback_law: Recorded::Value(Some(b"fallback_law".to_vec())),
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 10,
                literal: b"controller_flag".to_vec(),
            },
            LexicalTokenRow {
                token: 20,
                literal: b"occupied_flag".to_vec(),
            },
        ]));
    *frame.tables[TableId::ScriptFlags.index()] = TableView::Complete(TableRows::ScriptFlags(
        [1, 2, 3, 4]
            .into_iter()
            .map(|country| ScriptFlagRow {
                scope_kind: 1,
                scope_id: country,
                token: if country == 2 { 20 } else { 10 },
                value: 1,
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(
            [1, 2, 3, 4]
                .into_iter()
                .map(|country| CountryCounterInputRow {
                    country,
                    owned_state_count: 1,
                    ..Default::default()
                })
                .collect(),
        ));
    frame
}

/// Clears the flag of ScriptFlags row `row`: of country `row + 1`.
fn clear_flag(frame: &mut DecodedFrame, row: usize) {
    if let TableView::Complete(TableRows::ScriptFlags(rows)) =
        &mut *frame.tables[TableId::ScriptFlags.index()]
    {
        rows[row].value = 0;
    }
}

#[test]
fn triggered_laws_follow_reached_countries_hold_supplier_and_recover_after_gaps() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let (catalogue, _folder) = triggered_catalogue();
    let mut hours: Vec<_> = (100..108)
        .map(|hour| triggered(hour, hour != 101))
        .collect();
    for index in 1..=4 {
        clear_flag(&mut hours[index], 0);
        if index >= 3 {
            clear_flag(&mut hours[index], 3);
        }
    }
    hours[4] = unavailable(hours[4].clone(), TableId::ScriptFlags, "flags refused");
    hours[5] = unavailable(hours[5].clone(), TableId::Countries, "identity refused");
    let format = replay_data::format::FormatVersion::CURRENT;
    let mut prototype = Prepared {
        catalogue: Some(Arc::clone(&catalogue)),
        bound: true,
        ..Default::default()
    };
    prototype.scan = scan::Scan::new(Some(&catalogue)).bind(None);
    let mut expected = prototype.fresh();
    let mut actual = prototype.emit();
    let mut projector = prototype.projector(format, CaptureMode::Full).unwrap();
    let mut out: Vec<columnar::Hour> = Vec::new();
    let mut walk = Synthetic::new(&prototype.tables(), 1, 100).unwrap();
    let mut row_views =
        replay::columnar_frames::Views::new(&prototype.tables(), format, CaptureMode::Full)
            .unwrap();
    let mut tracker = FrameChangeTracker::new();
    let flag = AtomicBool::new(false);
    // Preparation reads only the states a change reached: count them with check mode off.
    let work = inputs::reusing(|| {
        let mut work = Vec::new();
        for (ordinal, input) in hours.iter().enumerate() {
            write_hour(&mut walk, input);
            walk.end(|state, manifest, _, starts| {
                let (frame, raw) = row_views.advance_recorded(state, manifest, starts).unwrap();
                expected
                    .observe(
                        &frame,
                        format,
                        &flag,
                        &tracker.observe_updates(&frame, Arc::new(raw), format),
                    )
                    .unwrap();
                projector
                    .hour(
                        &segments::Hour {
                            branch: 1,
                            hour: input.hour,
                            run: 0,
                            ordinal: ordinal as u32,
                            starts,
                            opens: starts,
                            state,
                            manifest,
                        },
                        &mut out,
                    )
                    .unwrap();
            })
            .unwrap();
            for hour in std::mem::take(&mut out) {
                work.push(hour.evaluated_states());
                actual.observe(hour).unwrap();
            }
            let snapshot = expected.scan.snapshot();
            let row = |id| {
                snapshot
                    .states
                    .states
                    .iter()
                    .find(|state| state.state == id)
                    .unwrap()
                    .runs
                    .last()
                    .unwrap()
                    .value
                    .clone()
            };
            for (first, second) in [
                (row(1), row(2)),
                (actual.latest(1).unwrap(), actual.latest(2).unwrap()),
            ] {
                match input.hour {
                    100 | 101 | 106 | 107 => {
                        assert_eq!(first.law.as_deref(), Some("default_law"));
                        assert_eq!(second.law.as_deref(), Some("default_law"));
                    }
                    102 | 103 => {
                        assert_eq!(first.law.as_deref(), Some("fallback_law"));
                        assert_eq!(second.law.as_deref(), Some("default_law"));
                    }
                    104 => {
                        assert!(first.law.is_none());
                        assert!(
                            first
                                .law_missing
                                .as_ref()
                                .unwrap()
                                .contains("flags refused")
                        );
                    }
                    105 => {}
                    _ => unreachable!(),
                }
            }
            if input.hour == 105 {
                for reads in [snapshot.states.observed.as_slice(), actual.read_spans()] {
                    assert!(reads.iter().all(|read| read.last_hour < 105));
                }
            }
        }
        work
    });
    assert_eq!(work, [2, 0, 1, 0, 2, 0, 2, 0]);
    parity(&[hours], Some(catalogue));
}

#[test]
fn a_law_declared_on_too_few_reads_is_caught_on_both_paths() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let (catalogue, _folder) = triggered_catalogue();
    let mut hours: Vec<_> = (100..104).map(|hour| triggered(hour, true)).collect();
    for input in &mut hours[2..] {
        clear_flag(input, 0);
    }
    let format = replay_data::format::FormatVersion::CURRENT;
    let mut prototype = Prepared {
        catalogue: Some(Arc::clone(&catalogue)),
        bound: true,
        ..Default::default()
    };
    prototype.scan = scan::Scan::new(Some(&catalogue)).bind(None);
    let flag = AtomicBool::new(false);
    let row = || {
        let mut prepared = prototype.fresh();
        let mut tracker = FrameChangeTracker::new();
        for input in &hours {
            prepared
                .observe(input, format, &flag, &tracker.observe(input, format))
                .unwrap();
        }
    };
    let projected = || {
        let mut projector = prototype.projector(format, CaptureMode::Full).unwrap();
        let mut out: Vec<columnar::Hour> = Vec::new();
        let mut walk = Synthetic::new(&prototype.tables(), 1, 100).unwrap();
        for (ordinal, input) in hours.iter().enumerate() {
            write_hour(&mut walk, input);
            walk.end(|state, manifest, _, starts| {
                projector
                    .hour(
                        &segments::Hour {
                            branch: 1,
                            hour: input.hour,
                            run: 0,
                            ordinal: ordinal as u32,
                            starts,
                            opens: starts,
                            state,
                            manifest,
                        },
                        &mut out,
                    )
                    .unwrap();
            })
            .unwrap();
        }
    };
    for walked in [&row as &dyn Fn(), &projected] {
        assert_eq!(inputs::expecting_missed(walked).1, 0);
        // Declared on nothing, the law is kept once the controller loses its flag.
        let (_, missed) =
            inputs::expecting_missed(|| inputs::declaring(|_| Some(Some(Vec::new())), walked));
        assert!(missed > 0);
    }
}

#[test]
fn a_gap_ends_occupation_series_at_the_hour_before_it_on_both_paths() {
    let mut after = frame(102, true);
    *after.tables[TableId::StateResistance.index()] =
        TableView::Complete(TableRows::StateResistance(Vec::new()));
    let mut prepared = Prepared::default();
    for input in [frame(100, true), after.clone()] {
        prepared
            .observe_frame(
                &input,
                replay_data::format::FormatVersion::CURRENT,
                Some(
                    &FrameChangeTracker::new()
                        .observe(&input, replay_data::format::FormatVersion::CURRENT),
                ),
            )
            .unwrap();
    }
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    prepared
        .flush(&mut writer, 1, &AtomicBool::new(false))
        .unwrap();
    writer.commit().unwrap();
    let flag = AtomicBool::new(false);
    let mut reader = Reader::new(
        &fixture.store,
        &fixture.key,
        "occupation-trend-values-1-0".into(),
        &flag,
    )
    .unwrap();
    let actor = prepared.tags["country 0001"];
    let runs = reader.runs(actor, 100, 102).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!((runs[0].first_hour, runs[0].last_hour), (100, 100));
    assert!(reader.value_at(actor, 101).unwrap().is_none());
    assert!(reader.value_at(actor, 102).unwrap().is_none());
    parity(&[vec![frame(100, true)], vec![after]], None);
}

#[test]
fn restarts_off_the_row_blocks_publish_what_production_does_over_long_walks() {
    let walk: Vec<_> = (100..1200)
        .map(|hour| {
            let mut input = fixture(hour, hour % 24 == 0, 4);
            if let TableView::Complete(TableRows::StateResistance(rows)) =
                &mut *input.tables[TableId::StateResistance.index()]
            {
                for row in rows {
                    row.level += i64::from(hour / 24 % 5);
                }
            }
            if (450..520).contains(&hour)
                && let TableView::Complete(TableRows::States(rows)) =
                    &mut *input.tables[TableId::States.index()]
            {
                rows[0].owner = 3;
            }
            if (600..700).contains(&hour)
                && let TableView::Complete(TableRows::StateOccupationLaws(rows)) =
                    &mut *input.tables[TableId::StateOccupationLaws.index()]
            {
                rows[0].name = b"later_law".to_vec();
            }
            if let TableView::Complete(TableRows::CountryCollaboration(rows)) =
                &mut *input.tables[TableId::CountryCollaboration.index()]
            {
                rows[0].progress = i64::from(hour / 24);
            }
            if hour % 24 == 12 {
                input.metrics.table_ns[TableId::CountryCollaboration.index()] = 1;
            }
            if hour % 48 == 0 {
                input.metrics.table_ns[TableId::StateGarrisons.index()] = 1;
            }
            input
        })
        .collect();
    for restarts in [vec![37, 500, 1000], (97..1100).step_by(97).collect()] {
        check(
            std::slice::from_ref(&walk),
            None,
            &Placement {
                label: format!("720-hour row blocks, restarts at {restarts:?}"),
                block: 720,
                ends: BTreeSet::new(),
                restarts: restarts.into_iter().collect(),
                round_trip: false,
            },
        );
    }
}

#[test]
fn equipment_dictionary_and_partial_manpower_keep_absolute_strength_results() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    use replay_data::rows::{GarrisonEquipment, GarrisonEquipmentInputs, LexicalTokenRow};
    let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/mod.lua",
            "NDefines={NResistance={GARRISON_STR_POW_MANPOWER=2,GARRISON_STR_POW_EQUIPMENT=1},NInterface={GARRISON_STRENGTH_TO_SHOW_RED=0.75}}",
        ),
        (
            "common/units/equipment/mod.txt",
            "equipments={mod_weapon={is_archetype=yes build_cost_ic=2.00003}}",
        ),
    ]);
    let catalogue = Arc::new(catalogue);
    let mut inputs: Vec<_> = (100..105)
        .map(|hour| {
            let mut frame = frame(hour, true);
            *frame.tables[TableId::LexicalTokens.index()] =
                TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                    token: 700,
                    literal: if hour == 101 {
                        b"missing_mod_weapon".to_vec()
                    } else {
                        b"mod_weapon".to_vec()
                    },
                }]));
            if let TableView::Complete(TableRows::StateGarrisons(rows)) =
                &mut *frame.tables[TableId::StateGarrisons.index()]
            {
                rows[0].required_manpower = 100;
                rows[0].serialized_flag = u8::from(hour == 103);
                rows[0].equipment = Recorded::Value(GarrisonEquipmentInputs {
                    rows: vec![GarrisonEquipment {
                        equipment_token: 700,
                        archetype_token: 700,
                        archetype_active: 1,
                        required_raw: 100_000,
                        required_lookup_raw: 100_000,
                        allocated_raw: 50_000,
                        weight_raw: 200_003,
                    }],
                    failure: None,
                });
            }
            *frame.tables[TableId::StateAssignedManpower.index()] =
                TableView::Complete(TableRows::StateAssignedManpower(vec![
                    StateAssignedManpowerRow {
                        controller: 1,
                        occupied: 2,
                        state: 1,
                        amount: 100,
                        ..Default::default()
                    },
                ]));
            frame
        })
        .collect();
    inputs[3] = partial(inputs[3].clone(), TableId::StateAssignedManpower, 1);
    inputs[4] = unavailable(
        inputs[4].clone(),
        TableId::StateGarrisons,
        "garrisons refused",
    );
    let format = replay_data::format::FormatVersion::CURRENT;
    let mut prototype = Prepared {
        catalogue: Some(Arc::clone(&catalogue)),
        bound: true,
        ..Default::default()
    };
    prototype.scan = scan::Scan::new(Some(&catalogue)).bind(None);
    let mut row = prototype.fresh();
    let mut projected = prototype.emit();
    let mut projector = prototype.projector(format, CaptureMode::Full).unwrap();
    let mut out: Vec<columnar::Hour> = Vec::new();
    let mut walk = Synthetic::new(&prototype.tables(), 1, 100).unwrap();
    let mut tracker = FrameChangeTracker::new();
    for (ordinal, input) in inputs.iter().enumerate() {
        write_hour(&mut walk, input);
        walk.end(|state, manifest, frame, starts| {
            row.observe(
                frame,
                format,
                &AtomicBool::new(false),
                &tracker.observe(frame, format),
            )
            .unwrap();
            projector
                .hour(
                    &segments::Hour {
                        branch: 1,
                        hour: input.hour,
                        run: 0,
                        ordinal: ordinal as u32,
                        starts,
                        opens: starts,
                        state,
                        manifest,
                    },
                    &mut out,
                )
                .unwrap();
        })
        .unwrap();
        emit(&mut out, &mut projected);
        let snapshot = row.scan.snapshot();
        for value in [
            snapshot.states.states[0].runs.last().unwrap().value.clone(),
            projected.latest(1).unwrap(),
        ] {
            assert_eq!(
                value.garrison_strength_raw,
                [Some(49_999), None, Some(49_999), Some(0), None][ordinal]
            );
            if ordinal == 1 {
                assert!(
                    value
                        .garrison_missing
                        .as_ref()
                        .unwrap()
                        .contains("missing_mod_weapon")
                );
            }
            if ordinal == 3 {
                assert_eq!(value.assigned_manpower, None);
            }
        }
    }
    parity(&[inputs], Some(catalogue));
}
