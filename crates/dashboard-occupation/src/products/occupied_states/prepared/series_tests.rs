//! The publication ([`columnar::emit`]) held to the row scan, its parity oracle, on the same
//! hours as the native recorder writes them: every window, the manifest, the occupiers' facts,
//! means and ranks, and the trend's warnings, across segment starts and restarts, cuts, a gap
//! and resumes; and the cases the projection and the row scan meet alike.
use super::*;
use dashboard_base::prepared::frame_changes::FrameChangeTracker;
use dashboard_series::{
    Blob, Budget, Cache, Publication, Snapshot, Span, Writer,
    family::Directory,
    memory::{Memory, publish},
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

/// The row scan over the same hours, in a consumer scope of its own as preparation's was.
struct Oracle {
    scan: scan::Scan<'static>,
    catalogue: Option<Arc<Catalogue>>,
    own: catalogue::OwnRefusals,
}

impl Oracle {
    fn new(catalogue: Option<Arc<Catalogue>>) -> Self {
        let mut scan = scan::Scan::new(catalogue.as_deref()).bind(None);
        scan.format = replay_data::format::FormatVersion::CURRENT;
        Self {
            scan,
            catalogue,
            own: catalogue::OwnRefusals::default(),
        }
    }

    fn push(&mut self, frame: &DecodedFrame) -> io::Result<()> {
        let held = self.catalogue.clone();
        let mut scan = std::mem::take(&mut self.scan).bind(held.as_deref());
        let pushed = catalogue::own_refusals(&mut self.own, || scan.push(frame, frame.hour, None));
        self.scan = scan.bind(None);
        pushed
    }

    fn snapshot(&mut self) -> OccupationHistory {
        let held = self.catalogue.clone();
        let scan = std::mem::take(&mut self.scan).bind(held.as_deref());
        let snapshot = catalogue::own_refusals(&mut self.own, || scan.snapshot());
        self.scan = scan.bind(None);
        snapshot
    }
}

fn binding(catalogue: &Option<Arc<Catalogue>>) -> Binding {
    Binding::bound(
        catalogue.clone(),
        scan::Scan::new(catalogue.as_deref()).bind(None),
    )
}

fn identity() -> dashboard_series::Identity {
    dashboard_series::Identity {
        source: [7; 32],
        content: [8; 32],
        branch: [9; 32],
        schema: SCHEMA,
    }
}

/// A publication of the hours of `runs`, cut where `placement` ends its blocks and its
/// projection restarted where it says, with the row scan of the same hours.
struct Built {
    pages: Memory,
    root: u64,
    /// The hours each cut closed, as the windows of their blocks.
    windows: Vec<HourSpan>,
    oracle: Oracle,
}

fn build(
    runs: &[Vec<DecodedFrame>],
    catalogue: Option<Arc<Catalogue>>,
    placement: &Placement,
) -> Built {
    let format = replay_data::format::FormatVersion::CURRENT;
    let mut bound = binding(&catalogue);
    let budget = Budget::new(256 << 20);
    let cache = Cache::new(&budget);
    let mut pages = Memory::default();
    let first = runs[0][0].hour;
    let mut writer = Writer::new(&budget, identity(), first).unwrap();
    let mut emit = bound.emit();
    let mut oracle = Oracle::new(catalogue.clone());
    let mut projector = bound.projector(format, CaptureMode::Full).unwrap();
    let mut out: Vec<columnar::Hour> = Vec::new();
    let (mut root, mut generation, mut windows, mut from) = (0, 0, Vec::new(), first);
    for (run, inputs) in runs.iter().enumerate() {
        let mut walk = Synthetic::new(bound.tables(), 1, inputs[0].hour).unwrap();
        let mut world_held: Option<dashboard_world::World> = None;
        if run > 0 {
            projector = bound.projector(format, CaptureMode::Full).unwrap();
        }
        assert!(projector.starts_at(Start::Run { continues: run > 0 }));
        projector
            .start(Start::Run { continues: run > 0 }, 1, inputs[0].hour)
            .unwrap();
        for (ordinal, input) in inputs.iter().enumerate() {
            let restarting = run == 0 && ordinal != 0 && placement.restarts.contains(&ordinal);
            if restarting {
                projector = bound.projector(format, CaptureMode::Full).unwrap();
                assert!(projector.starts_at(Start::Restart(walk.state())));
                projector
                    .start(Start::Restart(walk.state()), 1, input.hour)
                    .unwrap();
            }
            let starts = ordinal == 0 || restarting;
            write_hour(&mut walk, input);
            walk.end(|state, manifest, frame, walk_starts| {
                let world = dashboard_world::World::at(
                    &mut world_held,
                    walk_starts,
                    &columnar::needs(),
                    (format, CaptureMode::Full),
                    state,
                    manifest,
                )
                .unwrap();
                oracle.push(frame).unwrap();
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
                            world,
                        },
                        &mut out,
                    )
                    .unwrap();
            })
            .unwrap();
            for hour in out.drain(..) {
                emit.hour(&mut writer, hour).unwrap();
            }
            if !placement.ends(ordinal, inputs.len(), run == 0) {
                continue;
            }
            emit.cut(&mut writer, input.hour).unwrap();
            generation += 1;
            root = publish(
                &mut writer,
                &mut pages,
                &cache,
                Publication {
                    generation,
                    segment: generation - 1,
                    accepted: Span::new(first, input.hour).unwrap(),
                    reached: input.hour,
                    refusal: None,
                },
                Blob::EMPTY,
            )
            .unwrap();
            windows.push(HourSpan {
                first_hour: from,
                last_hour: input.hour,
            });
            from = input.hour + 1;
            if placement.round_trip {
                let mut saved = Vec::new();
                emit.save(&mut saved).unwrap();
                writer = Writer::resume(&mut pages, &cache, &budget, root, identity()).unwrap();
                let mut snapshot =
                    Snapshot::open(pages.clone(), &cache, root, identity()).unwrap();
                let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
                // A later run binds its content anew, its texts named afresh.
                bound = binding(&catalogue);
                emit = bound.resume(&directory, &saved).unwrap();
            }
        }
    }
    Built {
        pages,
        root,
        windows,
        oracle,
    }
}

fn raw(run: &OccupationTrendRun, column: usize) -> i128 {
    let raw = if column == 0 {
        &run.resistance_raw
    } else {
        &run.compliance_raw
    };
    raw.parse().unwrap()
}

/// A mean's canonical runs as its key holds them, from an occupier's runs: a supplying read, a
/// segment start, an hour without a run before and a new value each start one.
fn means(runs: &[OccupationTrendRun], column: usize) -> Vec<(i32, i32, bool, bool, i128)> {
    let mut out: Vec<(i32, i32, bool, bool, i128)> = Vec::new();
    for run in runs {
        let value = raw(run, column);
        let follows = out
            .last()
            .is_some_and(|last| last.1.checked_add(1) == Some(run.first_hour));
        let starts = run.starts_segment || !follows;
        let supplying = run.read_hour == run.first_hour;
        match out.last_mut() {
            Some(last) if !starts && !supplying && last.4 == value => last.1 = run.last_hour,
            _ => out.push((run.first_hour, run.last_hour, starts, supplying, value)),
        }
    }
    out
}

/// `key`'s canonical runs over `first..=last`, each whole.
fn canonical<P: dashboard_series::PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    first: i32,
    last: i32,
) -> Vec<(i32, i32, bool, bool, i128)> {
    let mut out = Vec::new();
    let mut hour = first;
    while hour <= last {
        match read::run_at(snapshot, key, hour).unwrap() {
            Some(run) => {
                out.push((
                    run.span.first,
                    run.span.last,
                    run.break_before,
                    run.supplying_boundary,
                    run.value,
                ));
                hour = run.span.last + 1;
            }
            None => hour += 1,
        }
    }
    out
}

/// The publication `built` holds what its row scan reads: each window, the manifest's laws and
/// warnings, the trend's warnings, and every occupier's runs, means and ranks.
fn check_built(built: &mut Built, label: &str) {
    let budget = Budget::new(256 << 20);
    let cache = Cache::new(&budget);
    let mut snapshot =
        Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let history = built.oracle.snapshot();
    for (index, &interval) in built.windows.iter().enumerate() {
        let actual = read::window(&mut snapshot, &directory, "1", index, interval).unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(window_reply(&history.states, "1", index, interval)).unwrap(),
            "{label}: window {index}"
        );
    }
    let listing = read::listing(&directory).unwrap().unwrap_or_default();
    assert_eq!(
        serde_json::to_value(&listing.laws).unwrap(),
        serde_json::to_value(&history.states.laws).unwrap(),
        "{label}: laws"
    );
    assert_eq!(listing.warnings, history.states.warnings, "{label}: warnings");
    assert_eq!(
        read::trend_warnings(&directory).unwrap(),
        built.oracle.scan.trend.warnings(),
        "{label}: trend warnings"
    );
    let tags = read::occupiers(&directory);
    let series = &history.trend.series;
    assert_eq!(
        tags.keys().collect::<BTreeSet<_>>(),
        series
            .iter()
            .filter(|(_, runs)| !runs.is_empty())
            .map(|(tag, _)| tag)
            .collect::<BTreeSet<_>>(),
        "{label}: occupiers"
    );
    let accepted = snapshot.publication().accepted;
    let mut rebuilt = History::default();
    for (tag, &actor) in &tags {
        rebuilt.series.insert(
            tag.clone(),
            read::fact_runs(&mut snapshot, &directory, actor, accepted.first, accepted.last)
                .unwrap(),
        );
        if let Some(segments) = history.trend.segments.get(tag) {
            rebuilt.segments.insert(tag.clone(), segments.clone());
        }
        for column in 0..2 {
            let key = read::key(&directory, actor, MEANS[column]).unwrap();
            let held = canonical(&mut snapshot, key, accepted.first, accepted.last);
            assert_eq!(held, means(&series[tag], column), "{label}: {tag} means {column}");
        }
    }
    let flag = AtomicBool::new(false);
    let params = OccupationTrendParams {
        branch_id: "1".into(),
        tags: Vec::new(),
        top: None,
        collaboration: true,
    };
    let reply = |history: &History| {
        serde_json::to_value(
            occupation_trend::reply("1", history, &params, Vec::new(), &flag).unwrap(),
        )
        .unwrap()
    };
    assert_eq!(reply(&rebuilt), reply(&history.trend), "{label}: occupier runs");
    for column in 0..2 {
        let positions = directory.dimensions(columnar::emit::FAMILY, POSITIONS[column]);
        for hour in accepted.first..=accepted.last {
            let mut ranked: Vec<(i128, &String)> = series
                .iter()
                .filter_map(|(tag, runs)| {
                    let run = runs
                        .iter()
                        .find(|run| run.first_hour <= hour && hour <= run.last_hour)?;
                    Some((raw(run, column), tag))
                })
                .collect();
            ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
            assert!(ranked.len() <= positions.len(), "{label}: positions at {hour}");
            for (position, &(_, key)) in positions.iter().enumerate() {
                let held = read::run_at(&mut snapshot, key, hour)
                    .unwrap()
                    .map(|run| run.value);
                let tag = ranked.get(position).map(|&(_, tag)| i128::from(tags[tag]));
                assert_eq!(held, tag, "{label}: position {position} by {column} at {hour}");
            }
            for (position, (_, tag)) in ranked.iter().enumerate() {
                let key = read::key(&directory, tags[*tag], RANKS[column]).unwrap();
                assert_eq!(
                    read::run_at(&mut snapshot, key, hour)
                        .unwrap()
                        .map(|run| run.value),
                    Some(position as i128 + 1),
                    "{label}: {tag}'s rank by {column} at {hour}"
                );
            }
        }
    }
}

fn check(runs: &[Vec<DecodedFrame>], catalogue: Option<Arc<Catalogue>>, placement: &Placement) {
    let mut built = build(runs, catalogue, placement);
    check_built(&mut built, &placement.label);
}

fn parity(runs: &[Vec<DecodedFrame>], catalogue: Option<Arc<Catalogue>>) {
    for placement in placements(runs[0].len()) {
        check(runs, catalogue.clone(), &placement);
    }
}

/// Feeds `hours` to the row scan and to one projection and its emit, publishing nothing; after
/// each hour, `each` sees the scan, the emit and how many states the projection read.
fn walk_both(
    hours: &[DecodedFrame],
    catalogue: Option<Arc<Catalogue>>,
    mut each: impl FnMut(&DecodedFrame, &mut Oracle, &columnar::Emit, usize),
) {
    let format = replay_data::format::FormatVersion::CURRENT;
    let bound = binding(&catalogue);
    let budget = Budget::new(64 << 20);
    let mut writer = Writer::new(&budget, identity(), hours[0].hour).unwrap();
    let mut emit = bound.emit();
    let mut oracle = Oracle::new(catalogue.clone());
    let mut projector = bound.projector(format, CaptureMode::Full).unwrap();
    let mut out: Vec<columnar::Hour> = Vec::new();
    let mut walk = Synthetic::new(bound.tables(), 1, hours[0].hour).unwrap();
    let mut world_held: Option<dashboard_world::World> = None;
    for (ordinal, input) in hours.iter().enumerate() {
        write_hour(&mut walk, input);
        walk.end(|state, manifest, frame, starts| {
            let world = dashboard_world::World::at(
                &mut world_held,
                starts,
                &columnar::needs(),
                (format, CaptureMode::Full),
                state,
                manifest,
            )
            .unwrap();
            oracle.push(frame).unwrap();
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
                        world,
                    },
                    &mut out,
                )
                .unwrap();
        })
        .unwrap();
        let evaluated = out.iter().map(columnar::Hour::evaluated_states).sum();
        for hour in out.drain(..) {
            emit.hour(&mut writer, hour).unwrap();
        }
        each(input, &mut oracle, &emit, evaluated);
    }
}

#[test]
fn whole_and_restarted_segments_publish_what_the_row_scan_reads() {
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
    let mut oracle = Oracle::new(None);
    for input in &inputs {
        oracle.push(input).unwrap();
    }
    let snapshot = oracle.snapshot();
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
    let mut world_held: Option<dashboard_world::World> = None;
    let mut projector = binding(&None)
        .projector(format, CaptureMode::Full)
        .unwrap();
    let mut out: Vec<columnar::Hour> = Vec::new();
    write_hour(&mut walk, &input);
    walk.end(|state, manifest, frame, starts| {
        let world = dashboard_world::World::at(
            &mut world_held,
            starts,
            &columnar::needs(),
            (format, CaptureMode::Full),
            state,
            manifest,
        )
        .unwrap();
        let expected = Oracle::new(None).push(frame).unwrap_err();
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
                    world,
                },
                &mut out,
            )
            .unwrap_err();
        assert_eq!(actual.to_string(), expected.to_string());
    })
    .unwrap();
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
    // Preparation reads only the states a change reached: count them with check mode off.
    let work = inputs::reusing(|| {
        let mut work = Vec::new();
        walk_both(
            &hours,
            Some(Arc::clone(&catalogue)),
            |input, oracle, emit, evaluated| {
                work.push(evaluated);
                let snapshot = oracle.snapshot();
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
                let mut pairs = vec![(row(1), row(2))];
                if input.hour != 105 {
                    pairs.push((emit.latest(1).unwrap(), emit.latest(2).unwrap()));
                }
                for (first, second) in pairs {
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
                    assert!(snapshot.states.observed.iter().all(|read| read.last_hour < 105));
                    // The hour cannot be read: no state is occupied through it.
                    assert!(emit.latest(1).is_none() && emit.latest(2).is_none());
                }
            },
        );
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
    let row = || {
        let format = replay_data::format::FormatVersion::CURRENT;
        let mut scan = scan::Scan::new(Some(&catalogue));
        scan.format = format;
        let mut tracker = FrameChangeTracker::new();
        for input in &hours {
            let changes = tracker.observe(input, format);
            scan.push(input, input.hour, Some(&changes)).unwrap();
        }
    };
    let projected = || walk_both(&hours, Some(Arc::clone(&catalogue)), |_, _, _, _| {});
    for walked in [&row as &dyn Fn(), &projected] {
        assert_eq!(inputs::expecting_missed(walked).1, 0);
        // Declared on nothing, the law is kept once the controller loses its flag.
        let (_, missed) =
            inputs::expecting_missed(|| inputs::declaring(|_| Some(Some(Vec::new())), walked));
        assert!(missed > 0);
    }
}

#[test]
fn a_gap_ends_occupation_series_at_the_hour_before_it() {
    let mut after = frame(102, true);
    *after.tables[TableId::StateResistance.index()] =
        TableView::Complete(TableRows::StateResistance(Vec::new()));
    let runs = [vec![frame(100, true)], vec![after]];
    for placement in placements(1) {
        let mut built = build(&runs, None, &placement);
        let budget = Budget::new(64 << 20);
        let cache = Cache::new(&budget);
        let mut snapshot =
            Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
        let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
        let actor = read::occupiers(&directory)["country 0001"];
        let key = read::key(&directory, actor, MEANS[0]).unwrap();
        let held = engine::runs(&mut snapshot, key, 100, 102).unwrap();
        assert_eq!(held.len(), 1);
        assert_eq!((held[0].span.first, held[0].span.last), (100, 100));
        assert!(read::run_at(&mut snapshot, key, 101).unwrap().is_none());
        assert!(read::run_at(&mut snapshot, key, 102).unwrap().is_none());
        drop(snapshot);
        check_built(&mut built, &placement.label);
    }
}

#[test]
fn restarts_off_the_row_blocks_publish_what_the_row_scan_reads_over_long_walks() {
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
    for (restarts, round_trip) in [
        (vec![37, 500, 1000], false),
        ((97..1100).step_by(97).collect(), false),
        (vec![720], true),
    ] {
        check(
            std::slice::from_ref(&walk),
            None,
            &Placement {
                label: format!("720-hour blocks, restarts at {restarts:?}, resumed {round_trip}"),
                block: 720,
                ends: BTreeSet::new(),
                restarts: restarts.into_iter().collect(),
                round_trip,
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
    let mut ordinal = 0;
    walk_both(&inputs, Some(Arc::clone(&catalogue)), |_, oracle, emit, _| {
        let snapshot = oracle.snapshot();
        for value in [
            snapshot.states.states[0].runs.last().unwrap().value.clone(),
            emit.latest(1).unwrap(),
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
        ordinal += 1;
    });
    parity(&[inputs], Some(catalogue));
}

#[test]
fn a_cut_after_every_hour_publishes_what_the_row_scan_reads() {
    check(
        &[hours()],
        None,
        &Placement {
            label: "a cut after every hour".into(),
            block: 1,
            ends: BTreeSet::new(),
            restarts: BTreeSet::new(),
            round_trip: false,
        },
    );
}
