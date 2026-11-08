use super::fixtures::{every, recording};
use super::*;
use crate::native_columns::{RowCells, TableInput, table_hour};
use replay_columnar::{
    manifest::{self, Section},
    state::Cell,
    writer::{Dictionary as Strings, Sample, TableWriter},
};
use replay_data::rows::{CountryRow, DivisionRow, RowCodec, WorldTensionRow};
use std::sync::atomic::{AtomicU64, Ordering};

const READ: Sample = Sample::FromReadBit;
const TABLES: [TableId; 3] = [
    TableId::Countries,
    TableId::Divisions,
    TableId::WorldTension,
];

/// One table's hour as its producer hands it.
enum Input<'a> {
    Countries(TableInput<'a, CountryRow>),
    Divisions(TableInput<'a, DivisionRow>),
    Tension(TableInput<'a, WorldTensionRow>),
}

/// The native writer's hours of three tables, applied by a walker and seen through [`Views`].
struct Harness {
    state: State,
    writers: Vec<TableWriter>,
    strings: Strings,
    cells: RowCells,
    views: Views,
    /// The same tables noted by slot every hour and built only when a test asks.
    slotted: Views,
    hour: i32,
}

impl Harness {
    fn new() -> Self {
        let schema = schema().unwrap();
        Self {
            writers: TABLES
                .iter()
                .map(|table| TableWriter::new(&schema[table.index()]).unwrap())
                .collect(),
            state: State::new(schema).unwrap(),
            strings: Strings::new(),
            cells: RowCells::default(),
            views: Views::new(&TABLES, FormatVersion::CURRENT, CaptureMode::Full).unwrap(),
            slotted: Views::new(&TABLES, FormatVersion::CURRENT, CaptureMode::Full)
                .unwrap()
                .by_slot(),
            hour: 0,
        }
    }

    fn hour(&mut self, inputs: Vec<Input<'_>>) -> DecodedFrame {
        self.hour += 1;
        let mut statuses = vec![Status::NotRecorded; TABLE_COUNT];
        let mut read = vec![0_u64; TABLE_COUNT.div_ceil(64)];
        let mut samples = Vec::new();
        let mut sections: Vec<(Section, Vec<u8>)> = Vec::new();
        for input in inputs {
            let (table, written) = match input {
                Input::Countries(input) => (
                    TableId::Countries,
                    table_hour(
                        &mut self.writers[0],
                        &mut self.strings,
                        &mut self.cells,
                        input,
                    ),
                ),
                Input::Divisions(input) => (
                    TableId::Divisions,
                    table_hour(
                        &mut self.writers[1],
                        &mut self.strings,
                        &mut self.cells,
                        input,
                    ),
                ),
                Input::Tension(input) => (
                    TableId::WorldTension,
                    table_hour(
                        &mut self.writers[2],
                        &mut self.strings,
                        &mut self.cells,
                        input,
                    ),
                ),
            };
            let written = written.unwrap();
            let index = table.index();
            statuses[index] = written.evidence.status;
            if written.evidence.read {
                read[index / 64] |= 1 << (index % 64);
            }
            match written.evidence.sample {
                Sample::FromReadBit => {}
                Sample::Reset => samples.push((index, None)),
                Sample::Carried(hour) => samples.push((index, Some(hour))),
            }
            if let Some(prepared) = &written.prepared
                && let Some(bytes) = &prepared.section
            {
                sections.push((
                    Section {
                        table: index,
                        bytes: bytes.len() as u64,
                        columns: prepared.columns.clone(),
                        ops: prepared.ops,
                    },
                    bytes.clone(),
                ));
            }
            let at = TABLES.iter().position(|held| *held == table).unwrap();
            self.writers[at]
                .commit(written.prepared, written.evidence, self.hour)
                .unwrap();
        }
        sections.sort_by_key(|(section, _)| section.table);
        let appends = self.strings.take_appended();
        let manifest = Manifest {
            branch: 1,
            hour: self.hour,
            flags: if samples.is_empty() {
                0
            } else {
                manifest::SAMPLES
            },
            total_ns: 1_000 + self.hour as u64,
            pool_bytes: 7,
            dropped_frames: 0,
            appends: appends.iter().map(|append| &append[..]).collect(),
            statuses,
            read,
            samples,
            sections: sections
                .iter()
                .map(|(section, _)| section.clone())
                .collect(),
        };
        self.state
            .apply(&manifest, |at| Ok(&sections[at].1[..]))
            .unwrap();
        self.slotted
            .note(&self.state, &manifest, self.hour == 1)
            .unwrap();
        self.views
            .advance(&self.state, &manifest, self.hour == 1)
            .unwrap()
    }

    /// The slot-noted views of the walk's hour, built from every hour noted since their last.
    fn slotted(&mut self) -> DecodedFrame {
        self.slotted
            .build(&self.state, (1, self.hour), Metrics::default(), None)
            .unwrap()
    }
}

fn country(id: u32, stability: i64) -> CountryRow {
    CountryRow {
        id,
        canonical_slot: 1_000 + id,
        tag: format!("C{id:03}").into_bytes(),
        political_power_raw: (id % 2 == 0).then_some(i64::from(id) * 10),
        stability_raw: stability,
        ..Default::default()
    }
}

fn division(id: u64, hit_points: i64) -> DivisionRow {
    let mut row = DivisionRow {
        id,
        roster_owner: 1,
        logical_owner: 1,
        expeditionary_owner: 1,
        province: 7,
        template: 3,
        hit_points,
        organisation: 1_234_567,
        ..Default::default()
    };
    // The wire holds stepped fields at their step, as a reconstruction reads them back.
    row.round_to_display();
    row
}

fn complete<R: RowCodec>(rows: Vec<R>, wrap: impl FnOnce(Vec<R>) -> TableRows) -> TableView {
    TableView::Complete(wrap(rows))
}

/// Each hour's views are the rows written, as a reconstruction of the row recording holds them:
/// changed, inserted and removed rows, a truncated read, an unavailable table recorded again,
/// stepped fields, text and optional cells; a table an hour leaves alone keeps its allocation
/// and its sample hour follows its reads.
#[test]
fn walked_hours_are_the_views_their_rows_make() {
    let mut walk = Harness::new();
    let first_countries = [country(1, 10), country(2, 20), country(3, 30)];
    let first_divisions = [division(10, 1_500_000), division(11, 2_000_000)];
    let first = walk.hour(vec![
        Input::Countries(TableInput::Snapshot {
            rows: &first_countries,
            partial: None,
            read: true,
            sample: READ,
        }),
        Input::Divisions(TableInput::Snapshot {
            rows: &first_divisions,
            partial: None,
            read: true,
            sample: READ,
        }),
        Input::Tension(TableInput::Snapshot {
            rows: &[WorldTensionRow {
                total_raw: 5,
                source_count: 2,
                country_aggregate_count: 3,
            }],
            partial: None,
            read: true,
            sample: READ,
        }),
    ]);
    assert_eq!(
        first.tables[TableId::Countries.index()],
        complete(first_countries.to_vec(), TableRows::Countries)
    );
    assert_eq!(
        first.tables[TableId::Divisions.index()],
        complete(first_divisions.to_vec(), TableRows::Divisions)
    );
    assert_eq!(
        first.tables[TableId::Countries.index()].sampled_at(),
        Some(1)
    );
    assert_eq!(first.metrics.total_ns, 1_001);
    assert_eq!(first.metrics.pool_bytes, 7);
    assert_eq!(
        first.metrics.table_ns[TableId::Countries.index()],
        READ_TIME_NOT_RECORDED
    );
    assert_eq!(first.metrics.table_ns[TableId::Clock.index()], 0);

    // Hour 2: a stability change, an insert and a removal; divisions left alone, unread;
    // world tension unavailable.
    let second = walk.hour(vec![
        Input::Countries(TableInput::Changes {
            rows: &[country(2, 21), country(4, 40)],
            removed: &[3],
            read: true,
            sample: READ,
        }),
        Input::Divisions(TableInput::Same {
            read: false,
            sample: READ,
        }),
        Input::Tension(TableInput::Unavailable {
            reason: "tension walk refused",
            read: false,
            sample: READ,
        }),
    ]);
    assert_eq!(
        second.tables[TableId::Countries.index()],
        complete(
            vec![country(1, 10), country(2, 21), country(4, 40)],
            TableRows::Countries
        )
    );
    assert!(SharedView::ptr_eq(
        &first.tables[TableId::Divisions.index()],
        &second.tables[TableId::Divisions.index()]
    ));
    assert_eq!(
        second.tables[TableId::Divisions.index()].sampled_at(),
        Some(1)
    );
    assert_eq!(
        second.tables[TableId::WorldTension.index()],
        TableView::Unavailable {
            reason: "tension walk refused".into()
        }
    );

    // Hour 3: countries read again unchanged; a division's hit points change; tension
    // recorded again.
    let third = walk.hour(vec![
        Input::Countries(TableInput::Same {
            read: true,
            sample: READ,
        }),
        Input::Divisions(TableInput::Changes {
            rows: &[division(10, 750_000)],
            removed: &[],
            read: true,
            sample: READ,
        }),
        Input::Tension(TableInput::Snapshot {
            rows: &[WorldTensionRow {
                total_raw: 7,
                source_count: 2,
                country_aggregate_count: 3,
            }],
            partial: None,
            read: true,
            sample: READ,
        }),
    ]);
    assert!(SharedView::ptr_eq(
        &second.tables[TableId::Countries.index()],
        &third.tables[TableId::Countries.index()]
    ));
    assert_eq!(
        third.tables[TableId::Countries.index()].sampled_at(),
        Some(3)
    );
    assert_eq!(
        third.tables[TableId::Divisions.index()],
        complete(
            vec![division(10, 750_000), division(11, 2_000_000)],
            TableRows::Divisions
        )
    );
    assert_eq!(
        third.tables[TableId::WorldTension.index()],
        complete(
            vec![WorldTensionRow {
                total_raw: 7,
                source_count: 2,
                country_aggregate_count: 3,
            }],
            TableRows::WorldTension
        )
    );

    // Hour 4: countries read in part, two rows missing; hour 5: whole again with a new row.
    let fourth = walk.hour(vec![
        Input::Countries(TableInput::Snapshot {
            rows: &[country(1, 11), country(4, 40)],
            partial: Some(2),
            read: true,
            sample: READ,
        }),
        Input::Divisions(TableInput::Same {
            read: false,
            sample: READ,
        }),
        Input::Tension(TableInput::Same {
            read: false,
            sample: READ,
        }),
    ]);
    assert_eq!(
        fourth.tables[TableId::Countries.index()],
        TableView::Truncated {
            rows: TableRows::Countries(vec![country(1, 11), country(4, 40)]),
            missing: 2,
        }
    );
    let all = [
        country(1, 11),
        country(2, 22),
        country(4, 40),
        country(5, 50),
    ];
    let fifth = walk.hour(vec![
        Input::Countries(TableInput::Snapshot {
            rows: &all,
            partial: None,
            read: true,
            sample: READ,
        }),
        Input::Divisions(TableInput::Same {
            read: false,
            sample: READ,
        }),
        Input::Tension(TableInput::Same {
            read: false,
            sample: READ,
        }),
    ]);
    assert_eq!(
        fifth.tables[TableId::Countries.index()],
        complete(all.to_vec(), TableRows::Countries)
    );
    assert_eq!(
        fifth.tables[TableId::Countries.index()].sampled_at(),
        Some(5)
    );
    // Unselected tables are the one view every unselected table shares.
    assert!(SharedView::ptr_eq(
        &fifth.tables[TableId::Clock.index()],
        &not_selected()
    ));
}

/// Views noted by slot and built only now and then hold, at each build, what views built every
/// hour hold: rows changed, deleted, inserted, deleted and inserted again under the same key or
/// at a freed slot, a truncated read between builds; a view no row changed since keeps its
/// allocation, sections or not.
#[test]
fn views_noted_by_slot_and_built_rarely_hold_the_hourly_views() {
    let mut walk = Harness::new();
    let same = || {
        Input::Divisions(TableInput::Same {
            read: true,
            sample: READ,
        })
    };
    let first = walk.hour(vec![
        Input::Countries(TableInput::Snapshot {
            rows: &[country(1, 10), country(2, 20), country(3, 30)],
            partial: None,
            read: true,
            sample: READ,
        }),
        Input::Divisions(TableInput::Snapshot {
            rows: &[division(10, 1_500_000), division(11, 2_000_000)],
            partial: None,
            read: true,
            sample: READ,
        }),
    ]);
    let built = walk.slotted();
    for table in TABLES {
        assert_eq!(built.tables[table.index()], first.tables[table.index()]);
    }
    // Hours 2 to 4, built at 4: a change, an insert and a removal; the inserted row removed and
    // the removed one back; a division changed twice and one removed.
    walk.hour(vec![
        Input::Countries(TableInput::Changes {
            rows: &[country(2, 21), country(4, 40)],
            removed: &[3],
            read: true,
            sample: READ,
        }),
        Input::Divisions(TableInput::Changes {
            rows: &[division(10, 1_000_000)],
            removed: &[],
            read: true,
            sample: READ,
        }),
    ]);
    walk.hour(vec![
        Input::Countries(TableInput::Changes {
            rows: &[country(3, 33)],
            removed: &[4],
            read: true,
            sample: READ,
        }),
        Input::Divisions(TableInput::Changes {
            rows: &[division(10, 750_000)],
            removed: &[11],
            read: true,
            sample: READ,
        }),
    ]);
    let fourth = walk.hour(vec![
        Input::Countries(TableInput::Changes {
            rows: &[country(1, 12)],
            removed: &[],
            read: true,
            sample: READ,
        }),
        same(),
    ]);
    let built = walk.slotted();
    for table in TABLES {
        assert_eq!(built.tables[table.index()], fourth.tables[table.index()]);
    }
    // Hour 5 changes nothing the views hold: both keep their allocations.
    walk.hour(vec![
        Input::Countries(TableInput::Same {
            read: true,
            sample: READ,
        }),
        same(),
    ]);
    let carried = walk.slotted();
    for table in [TableId::Countries, TableId::Divisions] {
        assert!(SharedView::ptr_eq(
            &carried.tables[table.index()],
            &built.tables[table.index()]
        ));
    }
    drop((built, carried));
    // Hours 6 to 8, built at 8: a row removed and inserted again, new rows at freed slots; then
    // a truncated read and a whole one.
    walk.hour(vec![
        Input::Countries(TableInput::Changes {
            rows: &[country(5, 50)],
            removed: &[1, 2],
            read: true,
            sample: READ,
        }),
        Input::Divisions(TableInput::Changes {
            rows: &[division(12, 900_000), division(13, 800_000)],
            removed: &[10],
            read: true,
            sample: READ,
        }),
    ]);
    let seventh = walk.hour(vec![
        Input::Countries(TableInput::Changes {
            rows: &[country(1, 15), country(6, 60)],
            removed: &[5],
            read: true,
            sample: READ,
        }),
        Input::Divisions(TableInput::Changes {
            rows: &[division(10, 600_000), division(13, 700_000)],
            removed: &[12],
            read: true,
            sample: READ,
        }),
    ]);
    let built = walk.slotted();
    for table in TABLES {
        assert_eq!(built.tables[table.index()], seventh.tables[table.index()]);
    }
    drop(built);
    walk.hour(vec![
        Input::Countries(TableInput::Snapshot {
            rows: &[country(1, 16)],
            partial: Some(2),
            read: true,
            sample: READ,
        }),
        same(),
    ]);
    let ninth = walk.hour(vec![
        Input::Countries(TableInput::Snapshot {
            rows: &[country(1, 17), country(3, 34), country(7, 70)],
            partial: None,
            read: true,
            sample: READ,
        }),
        Input::Divisions(TableInput::Changes {
            rows: &[division(11, 500_000)],
            removed: &[],
            read: true,
            sample: READ,
        }),
    ]);
    let built = walk.slotted();
    for table in TABLES {
        assert_eq!(built.tables[table.index()], ninth.tables[table.index()]);
    }
}

/// A changed slot's mask names every column its runs wrote, and a deleted slot's key is kept.
#[test]
fn changes_name_each_changed_rows_columns_and_each_deleted_key() {
    let mut walk = Harness::new();
    walk.hour(vec![Input::Countries(TableInput::Snapshot {
        rows: &[country(1, 10), country(2, 20), country(3, 30)],
        partial: None,
        read: true,
        sample: READ,
    })]);
    let mut changed = country(2, 20);
    changed.tag = b"NEW".to_vec();
    changed.stability_raw = 25;
    walk.hour(vec![Input::Countries(TableInput::Changes {
        rows: &[changed],
        removed: &[3],
        read: true,
        sample: READ,
    })]);
    let changes = walk.state.changes(TableId::Countries.index()).unwrap();
    let slot = walk
        .state
        .table(TableId::Countries.index())
        .unwrap()
        .slot(2)
        .unwrap();
    assert_eq!(changes.changed, [slot]);
    let names = TableId::Countries;
    let bit = |name: &str| {
        1_u64
            << (0..64)
                .find(|&index| names.field_name(index) == Some(name))
                .unwrap()
    };
    assert_eq!(changes.masks, [bit("tag") | bit("stability_raw")]);
    assert_eq!(changes.deleted_keys, [3]);
}

fn opened(bytes: Vec<u8>) -> Recording {
    Recording::open(bytes).unwrap()
}

fn walk_alone(file: &Recording, segment: &Segment) -> io::Result<Vec<(u64, i32)>> {
    let mut hours = Vec::new();
    let mut state = State::new(schema()?)?;
    file.walk_segment(segment, &mut state, |_, manifest, _| {
        hours.push((manifest.branch, manifest.hour));
        Ok(true)
    })?;
    Ok(hours)
}

/// A page that cannot be read refuses the walks over it, naming the page; the recording
/// opens and its other runs walk as before.
#[test]
fn a_damaged_page_refuses_only_the_walks_over_it() {
    let runs = [(1, 100, 30), (2, 500, 20)];
    let (mut bytes, pages) = recording(&runs, every(&runs), 1, |_| {});
    let mut whole = opened(bytes.clone());
    whole.verify(true);
    assert!(whole.damage().is_empty());
    let mut state = State::new(schema().unwrap()).unwrap();
    let mut walked = 0;
    whole
        .walk(&mut state, |_, _, _| {
            walked += 1;
            Ok(true)
        })
        .unwrap();
    assert_eq!(walked, 50);
    bytes[pages[0][0].offset as usize] ^= 1;
    let damaged = opened(bytes);
    let page = DamageScope::Page {
        run: 0,
        first: 0,
        hours: 30,
    };
    assert_eq!(damaged.damage().len(), 1);
    assert_eq!(damaged.damage()[0].scope, page);
    let segments = damaged.segments();
    assert_eq!(segments.len(), 2);
    let refused = walk_alone(&damaged, &segments[0]).unwrap_err();
    assert_eq!(Damage::of(&refused).unwrap().scope, page);
    assert!(
        refused
            .to_string()
            .starts_with("Run 0 from ordinal 0 (30 hours): "),
        "{refused}"
    );
    let second: Vec<(u64, i32)> = (500..520).map(|hour| (2, hour)).collect();
    assert_eq!(walk_alone(&damaged, &segments[1]).unwrap(), second);
    assert!(damaged.walk(&mut state, |_, _, _| Ok(true)).is_err());
}

/// A restart restores its run's dictionary chunks in id order, and the run's last restart
/// ends its last segment.
#[test]
fn restarts_restore_their_run_s_next_dictionary_ids() {
    let runs = [(1, 0, 2 * envelope::UNIT_ORDINALS)];
    let (bytes, _) = recording(&runs, every(&runs), 1, |_| {});
    let file = opened(bytes);
    let segments = file.segments();
    let branch = file.run(0).unwrap().branch;
    let expected = [
        Segment {
            branch,
            run: 0,
            restart: None,
            first: 0,
            last: 719,
        },
        Segment {
            branch,
            run: 0,
            restart: Some(0),
            first: 720,
            last: 1439,
        },
    ];
    assert_eq!(segments, expected);
    assert_eq!(file.restart_after(&segments[0]), Some(0));
    assert_eq!(file.restart_after(&segments[1]), Some(1));
    let mut walked = State::new(schema().unwrap()).unwrap();
    file.walk_segment(&segments[1], &mut walked, |_, _, _| Ok(true))
        .unwrap();
    let mut restored = State::new(schema().unwrap()).unwrap();
    file.restore(0, 1, &mut restored).unwrap();
    assert_eq!(restored.dictionary().len(), 1440);
    assert_eq!(walked.dictionary().len(), 1440);
    let (bytes, _) = recording(&runs, every(&runs), 1, |page| {
        if let Some(restart) = &mut page.restart
            && restart.ordinal == 1
        {
            restart.ids.0 = 0;
        }
    });
    let file = opened(bytes);
    let mut state = State::new(schema().unwrap()).unwrap();
    assert!(file.restore(0, 0, &mut state).is_ok());
    let refused = file.restore(0, 1, &mut state).unwrap_err();
    assert_eq!(
        Damage::of(&refused).unwrap().scope,
        DamageScope::Dictionary { run: 0, chunk: 1 }
    );
}

/// A restart needs only the chunks its tables declare: one an unreadable page hides refuses
/// none of the run's later restarts.
#[test]
fn an_unreferenced_missing_old_chunk_does_not_refuse_a_restart() {
    let runs = [(1, 0, 3 * envelope::UNIT_ORDINALS)];
    for restart_units in [1, 2] {
        let (mut bytes, pages) = recording(&runs, every(&runs), restart_units, |_| {});
        bytes[pages[0][0].offset as usize] ^= 1;
        let file = opened(bytes);
        assert_eq!(file.damage().len(), 1);
        let last = *file.segments().last().unwrap();
        assert_eq!(
            (last.restart, last.first),
            (Some(1), 2 * envelope::UNIT_ORDINALS)
        );
        assert!(walk_alone(&file, &file.segments()[0]).is_err());
        assert_eq!(
            walk_alone(&file, &last).unwrap().len(),
            envelope::UNIT_ORDINALS as usize
        );
    }
}

/// A segment at its run's start walks from empty tables and a run-local dictionary, whatever
/// the state it is handed held.
#[test]
fn a_run_start_walks_from_empty_tables() {
    let runs = [(1, 0, 10), (2, 100, 6)];
    let (bytes, _) = recording(&runs, every(&runs), 1, |_| {});
    let file = opened(bytes);
    let mut state = State::new(schema().unwrap()).unwrap();
    for (segment, strings) in file.segments().iter().zip([10, 6]) {
        file.walk_segment(segment, &mut state, |_, _, _| Ok(true))
            .unwrap();
        assert_eq!(state.dictionary().len(), strings);
        assert_eq!(
            state.dictionary().get(0),
            Some(format!("run {} ordinal 0", segment.run).as_bytes())
        );
    }
}

fn begin(file: &Recording, segment: &Segment, state: &mut State) -> io::Result<()> {
    Walker::new(file)?.begin(segment, state)
}

/// A walk restores only the tables its recording selects: a damaged restart table it does
/// not select is never read, and reading the rows, changes or views of a table it skips is
/// refused. A recording selecting nothing clears what an earlier walk's state skipped.
#[test]
fn a_selection_restores_only_its_tables_and_refuses_reads_of_the_rest() {
    let runs = [(1, 0, 2 * envelope::UNIT_ORDINALS)];
    let damaged = TableId::Divisions.index() as u32;
    let (bytes, _) = recording(&runs, every(&runs), 1, move |page| {
        if let Some(restart) = &mut page.restart {
            for table in restart
                .tables
                .iter_mut()
                .filter(|table| table.table == damaged)
            {
                table.extent.digest[0] ^= 1;
            }
        }
    });
    let whole = opened(bytes.clone());
    let mut selected = opened(bytes);
    selected.select(&[TableId::Campaign, TableId::Countries]);
    let restart = *whole
        .segments()
        .iter()
        .find(|segment| segment.restart.is_some())
        .unwrap();
    let mut state = State::new(schema().unwrap()).unwrap();
    let refused = begin(&whole, &restart, &mut state).unwrap_err();
    assert_eq!(
        Damage::of(&refused).unwrap().scope,
        DamageScope::RestartTable {
            run: 0,
            after: 719,
            table: damaged,
        }
    );
    let mut state = State::new(schema().unwrap()).unwrap();
    begin(&selected, &restart, &mut state).unwrap();
    assert!(state.skips(TableId::Divisions.index()) && !state.skips(TableId::Countries.index()));
    assert!(crate::columns::table(&state, TableId::Divisions).is_err());
    assert!(crate::columns::changes(&state, TableId::Divisions).is_err());
    assert!(crate::columns::table(&state, TableId::Countries).is_ok());
    let views =
        |tables: &[TableId]| Views::new(tables, FormatVersion::CURRENT, CaptureMode::Full).unwrap();
    assert!(views(&[TableId::Countries]).restored(&state, 1, 0).is_ok());
    let refused = views(&[TableId::Divisions])
        .restored(&state, 1, 0)
        .unwrap_err();
    assert!(refused.to_string().contains("Divisions"), "{refused}");
    // A restore installs its recording's selection, and one selecting nothing clears it.
    let mut restored = State::new(schema().unwrap()).unwrap();
    let page = restart.restart.unwrap();
    selected.restore(restart.run, page, &mut restored).unwrap();
    assert!(crate::columns::table(&restored, TableId::Divisions).is_err());
    let clean = opened(recording(&runs, every(&runs), 1, |_| {}).0);
    clean.restore(restart.run, page, &mut restored).unwrap();
    assert!((0..TABLE_COUNT).all(|table| !restored.skips(table)));
    begin(&clean, &restart, &mut state).unwrap();
    assert!((0..TABLE_COUNT).all(|table| !state.skips(table)));
}

/// A recording's bytes in a file of their own, removed afterwards.
struct Saved(std::path::PathBuf);

impl Saved {
    fn new(bytes: &[u8]) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "columnar-frames-{}-{}.h4col",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, bytes).unwrap();
        Self(path)
    }

    fn open(&self) -> Recording {
        Recording::open_path(&self.0).unwrap()
    }

    fn append(&self, bytes: &[u8]) {
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(&self.0)
            .unwrap()
            .write_all(bytes)
            .unwrap();
    }
}

impl Drop for Saved {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn append_revision(bytes: &mut Vec<u8>, revision: u64) {
    let (extent, _) = envelope::last_root(bytes).unwrap();
    let stored = extent.stored_in(bytes).unwrap();
    let raw = zstd::bulk::decompress(stored, 1 << 20).unwrap();
    let mut root = Root::read(&raw).unwrap();
    root.revision = revision;
    let root = fixtures::append(bytes, Kind::Root, (0, 0, 0, 0), &root.write().unwrap());
    bytes.extend_from_slice(&envelope::footer(root.offset, root.stored, &root.digest));
}

/// Both inputs end where the last footer whose root verifies does.
#[test]
fn torn_tails_choose_the_last_committed_root_for_both_inputs() {
    let runs = [(1, 0, 10)];
    let (mut bytes, _) = recording(&runs, every(&runs), 1, |_| {});
    let original = bytes.len();
    append_revision(&mut bytes, 2);
    let committed = bytes.len();
    append_revision(&mut bytes, 3);
    bytes.truncate(bytes.len() - 13);
    let disk = Saved::new(&bytes);
    for reader in [opened(bytes.clone()), disk.open()] {
        assert_eq!(reader.len(), committed as u64);
        assert_eq!(reader.hours(), 10);
    }
    let (root, _) = envelope::last_root(&bytes).unwrap();
    bytes[root.offset as usize] ^= 1;
    assert_eq!(opened(bytes).len(), original as u64);
}

/// A recording opened from its file keeps the revision it opened while a writer appends the
/// next.
#[test]
fn an_open_recording_keeps_its_revision_while_its_file_grows() {
    let runs = [(1, 0, 10)];
    let (mut bytes, _) = recording(&runs, every(&runs), 1, |_| {});
    let disk = Saved::new(&bytes);
    let pinned = disk.open();
    let before = bytes.len();
    append_revision(&mut bytes, 2);
    disk.append(&bytes[before..]);
    assert_eq!(pinned.len(), before as u64);
    assert_eq!(
        walk_alone(&pinned, &pinned.segments()[0]).unwrap().len(),
        10
    );
    assert_eq!(disk.open().len(), bytes.len() as u64);
}

/// A unit a walk reads is always checked against its digest and the recording's extents;
/// one it does not read is not read.
#[test]
fn selected_units_always_check_their_digest_and_range() {
    for range in [false, true] {
        let bytes = fixtures::native_recording(1, move |page| {
            let table = page
                .tables
                .iter_mut()
                .find(|extent| extent.stream == TableId::Countries.index() as u32)
                .unwrap();
            if range {
                table.offset = u64::MAX;
            } else {
                table.digest[0] ^= 1;
            }
        });
        let mut selected = opened(bytes.clone());
        selected.select(&[TableId::Countries]);
        let mut state = State::new(schema().unwrap()).unwrap();
        let refused = selected.walk(&mut state, |_, _, _| Ok(true)).unwrap_err();
        assert!(matches!(
            Damage::of(&refused).unwrap().scope,
            DamageScope::Extent(extent) if extent.stream == TableId::Countries.index() as u32
        ));
        let mut unrelated = opened(bytes);
        unrelated.select(&[]);
        unrelated.walk(&mut state, |_, _, _| Ok(true)).unwrap();
    }
}

/// A unit whose envelope declares another raw length than its chunks' frames is refused as its
/// own damage, from a file and from memory, and one declaring more chunks than its page lists
/// refuses the page, before any buffer is sized by it; a restart table declaring another raw
/// length than its frame is refused as its own damage before any buffer is sized by it.
#[test]
fn declared_raw_lengths_are_checked_against_their_frames_before_buffers_are_sized() {
    let countries = TableId::Countries.index() as u32;
    let walked = |file: &Recording| {
        let mut state = State::new(schema().unwrap()).unwrap();
        file.walk(&mut state, |_, _, _| Ok(true)).unwrap_err()
    };
    let longer = fixtures::native_recording(1, move |page| {
        for block in &mut page.tables {
            block.raw += 1;
        }
    });
    // More chunks than its frames: each listed chunk's frame header declares its raw bytes.
    let split = fixtures::native_recording(1, move |page| {
        let mut chunks = vec![page.chunks[0]];
        for (at, block) in page.tables.iter_mut().enumerate() {
            let stored = page.chunks[1 + at];
            block.raw = 3 * envelope::CHUNK_BYTES;
            chunks.extend([stored - 2, 1, 1]);
        }
        page.chunks = chunks;
    });
    let (disk, split_disk) = (Saved::new(&longer), Saved::new(&split));
    for file in [opened(longer.clone()), disk.open(), opened(split.clone()), split_disk.open()] {
        let refused = walked(&file);
        assert!(
            matches!(
                Damage::of(&refused).unwrap().scope,
                DamageScope::Extent(extent) if extent.kind == Kind::Block
            ),
            "{refused}"
        );
        assert!(refused.to_string().ends_with("differs from its chunk's frame length"), "{refused}");
    }
    // No allocation could hold either: a refusal for lack of memory names no damage.
    for raw in [1 << 40, u64::MAX] {
        let unlisted = fixtures::native_recording(1, move |page| {
            for block in &mut page.tables {
                block.raw = raw;
            }
        });
        let restart = fixtures::native_recording(1, move |page| {
            page.restart.as_mut().unwrap().tables[countries as usize]
                .extent
                .raw = raw;
        });
        let (unlisted_disk, restart_disk) = (Saved::new(&unlisted), Saved::new(&restart));
        for file in [opened(unlisted.clone()), unlisted_disk.open()] {
            let refused = walked(&file);
            assert!(
                matches!(Damage::of(&refused).unwrap().scope, DamageScope::Page { run: 0, first: 0, .. }),
                "{refused}"
            );
        }
        for file in [opened(restart.clone()), restart_disk.open()] {
            let mut state = State::new(schema().unwrap()).unwrap();
            let refused = file.restore(0, 0, &mut state).unwrap_err();
            assert_eq!(
                Damage::of(&refused).unwrap().scope,
                DamageScope::RestartTable {
                    run: 0,
                    after: envelope::UNIT_ORDINALS - 1,
                    table: countries,
                },
                "{refused}"
            );
            assert!(refused.to_string().ends_with("differs from its frame's length"), "{refused}");
        }
    }
}

/// A dictionary chunk that cannot be read refuses the restores of the tables declaring it,
/// naming the chunk; a restore of other tables goes on.
#[test]
fn a_missing_dictionary_chunk_affects_its_declared_table_consumers() {
    let bytes = fixtures::native_recording(3, |page| {
        if page.first == 0 {
            page.restart.as_mut().unwrap().chunk.digest[0] ^= 1;
        }
    });
    let selected = opened(bytes.clone());
    let mut state = State::new(schema().unwrap()).unwrap();
    let refused = selected.restore(0, 2, &mut state).unwrap_err();
    assert_eq!(
        Damage::of(&refused).unwrap().scope,
        DamageScope::Dictionary { run: 0, chunk: 0 }
    );
    let mut unrelated = opened(bytes);
    unrelated.select(&[TableId::WorldTension]);
    unrelated.restore(0, 2, &mut state).unwrap();
}

#[test]
fn undeclared_dictionary_references_are_damage_even_when_the_chunk_exists() {
    let bytes = fixtures::native_recording(2, |page| {
        if page.first != 0 {
            page.restart.as_mut().unwrap().tables[TableId::Countries.index()]
                .chunks
                .clear();
        }
    });
    let file = opened(bytes);
    let mut state = State::new(schema().unwrap()).unwrap();
    let refused = file.restore(0, 1, &mut state).unwrap_err();
    assert!(refused.to_string().contains("dependencies"), "{refused}");
}

/// The dictionary id `country`'s tag holds.
fn tag(state: &State, country: u128) -> u32 {
    let column = (0..TableId::Countries.field_count())
        .find(|&field| TableId::Countries.field_name(field) == Some("tag"))
        .unwrap();
    let table = state.table(TableId::Countries.index()).unwrap();
    let slot = table.slot(country).unwrap();
    let Cell::Bytes(id) = table.column(column).unwrap().cell(slot).unwrap() else {
        panic!("a text tag");
    };
    id
}

/// A late restart holds the chunks its tables declare, not every string before it: the
/// others are read back when asked for.
#[test]
fn late_restarts_hold_only_the_chunks_their_tables_declare() {
    let file = opened(fixtures::native_recording(12, |_| {}));
    let mut state = State::new(schema().unwrap()).unwrap();
    file.restore(0, 11, &mut state).unwrap();
    let strings = 12 * envelope::UNIT_ORDINALS as usize + 1;
    assert_eq!(state.dictionary().len(), strings);
    let mut views = Views::new(
        &[TableId::Countries],
        FormatVersion::CURRENT,
        CaptureMode::Full,
    )
    .unwrap();
    let frame = views
        .restored(&state, 7, 100 + 12 * envelope::UNIT_ORDINALS as i32 - 1)
        .unwrap();
    assert_eq!(
        frame.tables[TableId::Countries.index()].sampled_at(),
        Some(100 + 12 * envelope::UNIT_ORDINALS as i32 - 2)
    );
    assert_eq!(
        state.dictionary().get(tag(&state, 1)),
        Some(b"old string".as_slice())
    );
    // Hour 3,000's string, in a chunk no table declares.
    let mut chunk = None;
    let id = (0..strings as u32)
        .find(|&id| {
            state
                .dictionary()
                .resolve(id, &mut chunk)
                .unwrap()
                .is_some_and(|text| text == b"new string 3000")
        })
        .unwrap();
    assert_eq!(state.dictionary().get(id), None);
}

/// The most a walk of `units` units holds of its dictionary, and the bytes of the strings it
/// appended.
fn held_walking(units: u32) -> (usize, usize) {
    let file = Recording::open(fixtures::native_recording(units, |_| {})).unwrap();
    let mut state = State::new(schema().unwrap()).unwrap();
    let (mut held, mut strings) = (0, 0);
    file.walk(&mut state, |state, manifest, _| {
        held = held.max(state.dictionary().held_bytes());
        strings += manifest
            .appends
            .iter()
            .map(|text| text.len())
            .sum::<usize>();
        Ok(true)
    })
    .unwrap();
    (held, strings)
}

/// A walk's dictionary holds no more for a long run than for a short one: strings no live
/// cell names are dropped once their restart's chunk holds them.
#[test]
fn a_walk_s_dictionary_does_not_grow_with_its_run() {
    let (short, _) = held_walking(4);
    let (long, strings) = held_walking(16);
    assert!(
        long < 2 * short,
        "{long} bytes held over 16 units, {short} over 4"
    );
    assert!(
        2 * long < strings,
        "{long} bytes held of {strings} appended"
    );
}

/// A run without restarts has no chunk to read a string back from: its walk keeps every
/// string it appended.
#[test]
fn a_run_without_restarts_keeps_every_string_it_appends() {
    let bytes = fixtures::native_recording(3, |page| page.restart = None);
    let file = Recording::open(bytes).unwrap();
    let mut state = State::new(schema().unwrap()).unwrap();
    let mut first = None;
    file.walk(&mut state, |state, _, _| {
        first.get_or_insert_with(|| tag(state, 2));
        Ok(true)
    })
    .unwrap();
    assert_eq!(
        state.dictionary().get(first.unwrap()),
        Some(b"new string 0".as_slice())
    );
}

#[test]
fn root_and_run_lists_hold_many_short_runs() {
    for count in [4, 256] {
        let runs: Vec<_> = (0..count).map(|run| (run as u64, 0, 1)).collect();
        let file = opened(recording(&runs, every(&runs), 1, |_| {}).0);
        assert_eq!(file.hours(), count as u64);
        assert_eq!(file.run_count(), count as u32);
        assert_eq!(file.segments().len(), count);
    }
}

/// Five runs and twenty segments, two runs of one branch, one after the other.
const CHURNED: [(u64, i32, u32); 5] = [
    (1, 100, 5),
    (1, 100 + 5 * envelope::UNIT_ORDINALS as i32, 4),
    (2, 50_000, 3),
    (3, 90_000, 6),
    (2, 200_000, 2),
];

fn watched() -> Views {
    Views::new(
        &[TableId::Countries, TableId::WorldTension],
        FormatVersion::CURRENT,
        CaptureMode::Full,
    )
    .unwrap()
}

fn seen(
    views: &mut Views,
    state: &State,
    manifest: &Manifest<'_>,
    starts: bool,
) -> io::Result<fixtures::Seen> {
    let frame = views.advance(state, manifest, starts)?;
    let sampled = frame.tables.iter().map(SharedView::sampled_at).collect();
    Ok((frame, sampled))
}

/// Every walk sees the frames and sample hours the writer saw, from a file and from memory,
/// of every run from its start, and of each segment from its restart.
#[test]
fn walks_see_the_writer_s_frames_over_many_runs_and_segments() {
    let (bytes, expected) = fixtures::churned(&CHURNED, 1);
    let disk = Saved::new(&bytes);
    {
        for file in [
            Recording::open(bytes.clone()).unwrap(),
            disk.open(),
        ] {
            assert!(file.run_count() > 4 && file.segments().len() > 16);
            assert_eq!(file.hours(), expected.len() as u64);
            let mut state = State::new(schema().unwrap()).unwrap();
            let mut views = watched();
            let mut walked = Vec::new();
            file.walk(&mut state, |state, manifest, starts| {
                walked.push(seen(&mut views, state, manifest, starts)?);
                Ok(true)
            })
            .unwrap();
            assert!(walked == expected, "a walk of every run differs");
            let mut at = 0;
            for segment in file.segments() {
                let mut views = watched();
                let mut hours = Vec::new();
                file.walk_segment(segment, &mut state, |state, manifest, starts| {
                    hours.push(seen(&mut views, state, manifest, starts)?);
                    Ok(true)
                })
                .unwrap();
                assert!(
                    hours[..] == expected[at..at + hours.len()],
                    "{segment:?} differs"
                );
                at += hours.len();
            }
            assert_eq!(at, expected.len());
        }
    }
}

/// Whether two dictionaries hold the same strings at every id, each read back from its
/// chunk where a dictionary dropped it.
fn same_strings(left: &Dictionary, right: &Dictionary) -> io::Result<bool> {
    let (mut left_chunk, mut right_chunk) = (None, None);
    for id in 0..left.len() as u32 {
        if left.resolve(id, &mut left_chunk)? != right.resolve(id, &mut right_chunk)? {
            return Ok(false);
        }
    }
    Ok(left.len() == right.len())
}

/// Each segment walked from its own start ends in the state of the restart after it, every
/// table and every string, whatever its dictionary dropped.
#[test]
fn every_segment_ends_in_its_next_restart_s_state_string_for_string() {
    let (bytes, _) = fixtures::churned(&CHURNED, 1);
    {
        let file = Recording::open(bytes.clone()).unwrap();
        for segment in file.segments() {
            let mut state = State::new(schema().unwrap()).unwrap();
            file.walk_segment(segment, &mut state, |_, _, _| Ok(true))
                .unwrap();
            let mut restored = State::new(schema().unwrap()).unwrap();
            let at = file.restart_after(segment).unwrap();
            file.restore(segment.run, at, &mut restored).unwrap();
            assert!(
                same_strings(state.dictionary(), restored.dictionary()).unwrap(),
                "{segment:?}"
            );
            assert!((0..TABLE_COUNT).all(|table| {
                state.table(table) == restored.table(table)
                    && state.status(table) == restored.status(table)
                    && state.sampled_at(table) == restored.sampled_at(table)
            }));
        }
    }
}

/// A walk whose dictionary doubled drops the strings of the pages before, and reads those that
/// come back, as cells or as reasons, from their chunks.
#[test]
fn a_walk_s_trimmed_dictionary_reads_returning_strings_back() {
    let (bytes, expected) = fixtures::churned(&[(1, 0, 6)], 1);
    let file = Recording::open(bytes).unwrap();
    let mut state = State::new(schema().unwrap()).unwrap();
    let mut views = watched();
    let (mut walked, mut first, mut dropped) = (Vec::new(), None, false);
    file.walk(&mut state, |state, manifest, starts| {
        let id = *first.get_or_insert_with(|| tag(state, 2));
        dropped |= state.dictionary().get(id).is_none();
        walked.push(seen(&mut views, state, manifest, starts)?);
        Ok(true)
    })
    .unwrap();
    assert!(dropped);
    assert!(walked == expected);
    assert_eq!(
        state
            .dictionary()
            .resolve(first.unwrap(), &mut None)
            .unwrap(),
        Some(b"new string 0".as_slice())
    );
    // The strings the fixture wrote, not the writer's frames: tags and a reason last held on
    // page 0, back on later pages.
    let tags = |hour: usize| {
        let (frame, _) = &walked[hour];
        assert_eq!(frame.hour, hour as i32);
        let TableView::Complete(TableRows::Countries(rows)) =
            &*frame.tables[TableId::Countries.index()]
        else {
            panic!("hour {hour}'s countries are not complete");
        };
        rows.iter()
            .map(|row| (row.id, std::str::from_utf8(&row.tag).unwrap()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        tags(800),
        [
            (1, "old string"),
            (2, "new string 800"),
            (3, "cycle 16"),
            (4, "returning"),
        ]
    );
    assert_eq!(
        tags(2000),
        [
            (1, "old string"),
            (2, "new string 2000"),
            (3, "cycle 0"),
            (4, "returning"),
        ]
    );
    assert_eq!(
        tags(4319),
        [(1, "old string"), (2, "new string 4319"), (3, "cycle 6")]
    );
    assert_eq!(
        walked[2100].0.tables[TableId::WorldTension.index()],
        TableView::Unavailable {
            reason: "refused 0".into()
        }
    );
}


/// A format-136 recording held every column format 137 did not append: its schema is this
/// build's but for those, and nothing since changed what its columns mean.
#[test]
fn format_136_columns_are_this_builds_but_the_appended() {
    use replay_data::rows::compatibility::AIR_HISTORY_SLOTS_FORMAT;
    let schema = schema().unwrap();
    let columns = columns_at(AIR_HISTORY_SLOTS_FORMAT).unwrap();
    let appended = [
        (TableId::States, 13),
        (TableId::Players, 4),
        (TableId::OperativeMissions, 18),
        (TableId::AgencyOperationBuffers, 4),
        (TableId::RaidIntelInputs, 4),
        (TableId::StrategicOperatives, 5),
        (TableId::AirGroundCrews, 1),
    ];
    for table in TableId::ALL {
        let expected = appended
            .iter()
            .find(|&&(held, _)| held == table)
            .map_or(schema[table.index()].len(), |&(_, columns)| columns);
        assert_eq!(columns[table.index()], expected, "{table:?}");
    }
    assert!(withheld_at(AIR_HISTORY_SLOTS_FORMAT).unwrap().is_empty());
    // A format-134 capture frames its tables as written and withholds by name those whose
    // layout changed since; every other table reads.
    use replay_data::rows::compatibility::EXACT_NAVAL_CONDITION_FORMAT;
    let withheld: Vec<usize> = withheld_at(EXACT_NAVAL_CONDITION_FORMAT)
        .unwrap()
        .into_iter()
        .map(|(table, reason)| {
            assert!(reason.contains(&format!("{:?}", TableId::ALL[table])), "{reason}");
            table
        })
        .collect();
    let mut expected = [TableId::AirNavalPatrolHistories, TableId::AirMissionInputs]
        .map(TableId::index);
    expected.sort_unstable();
    assert_eq!(withheld, expected);
    let earlier = columns_at(EXACT_NAVAL_CONDITION_FORMAT).unwrap();
    assert_eq!(earlier[TableId::AirNavalPatrolHistories.index()], 3);
    assert_eq!(earlier[TableId::Cryptology.index()], 4);
    assert_eq!(earlier[TableId::RaidAirScores.index()], 6);
    assert!(
        replay_columnar::envelope::schema_digest_prefix(&schema, &earlier).is_some(),
        "its columns are this build's first"
    );
    let current = columns_at(replay_data::FORMAT_VERSION).unwrap();
    assert_eq!(
        replay_columnar::envelope::schema_digest_prefix(&schema, &current),
        Some(replay_columnar::envelope::schema_digest(&schema))
    );
    assert_ne!(
        replay_columnar::envelope::schema_digest_prefix(&schema, &columns),
        Some(replay_columnar::envelope::schema_digest(&schema))
    );
}

/// A unit past one chunk reads back whole, its chunks decoded in turn or apart, from memory and
/// from a file; chunks listed out of place refuse their unit by name.
#[test]
fn units_past_one_chunk_read_back_whole_in_turn_and_apart() {
    use crate::columnar_file::{Block, FileWriter, Settings};
    let made = |seed: u64, length: usize| -> Vec<u8> {
        (0..length as u64)
            .map(|at| ((at.wrapping_mul(2_654_435_761) ^ seed) >> 7) as u8)
            .collect()
    };
    let records = made(1, 300);
    let table = TableId::Countries.index() as u32;
    let payload = made(2, (envelope::CHUNK_BYTES * 5 / 2) as usize);
    let written = |parallel: bool, swap: bool| {
        let identity = envelope::Identity {
            schema: envelope::schema_digest(&schema().unwrap()),
            campaign: b"chunks".to_vec(),
            content: Vec::new(),
            capture_mode: CaptureMode::Full.byte(),
        };
        let settings = Settings { level: 1, parallel };
        let none = vec![0_u64; TABLE_COUNT.div_ceil(64)];
        let tables = [(table, payload.clone())];
        let mut file = FileWriter::create(std::io::Cursor::new(Vec::new()), identity, TABLE_COUNT, settings)
            .unwrap()
            .with_tamper(move |page| {
                if swap {
                    assert_ne!(page.chunks[2], page.chunks[3]);
                    page.chunks.swap(2, 3);
                }
            });
        file.start_run(1, envelope::epoch::START, 1).unwrap();
        file.block(Block {
            first_hour: 1,
            last_hour: 10,
            hours: 10,
            records: &records,
            tables: &tables,
            sectioned: &none,
            read: &none,
            status_changed: &none,
            restart: None,
        })
        .unwrap();
        file.finish(|_| Ok(())).unwrap().file.into_inner()
    };
    let spread = |count: usize, job: &(dyn Fn(usize) + Sync)| {
        use rayon::prelude::*;
        (0..count).into_par_iter().for_each(job);
    };
    for (parallel, swap) in [(false, false), (true, false), (false, true)] {
        let bytes = written(parallel, swap);
        let disk = Saved::new(&bytes);
        for recording in [opened(bytes.clone()), disk.open()] {
            let page = recording.page(0, 0).unwrap();
            let counts: Vec<usize> = page.units().map(|(_, chunks)| chunks.len()).collect();
            assert_eq!(counts, [1, 3]);
            let units = recording.page_units(0, 0);
            for apart in [false, true] {
                let mut walker = Walker::new(&recording).unwrap();
                if apart {
                    walker.decode_apart(spread);
                }
                let loaded = walker.load_page(0, 0);
                if swap {
                    let refused = loaded.unwrap_err();
                    assert!(
                        matches!(
                            Damage::of(&refused).unwrap().scope,
                            DamageScope::Extent(extent) if extent.kind == Kind::Block && extent.stream == table
                        ),
                        "{refused}"
                    );
                    continue;
                }
                assert_eq!(loaded.unwrap(), records.len() + payload.len());
                assert_eq!(walker.loaded(0), Some(&records[..]));
                assert_eq!(walker.loaded(table as usize + 1), Some(&payload[..]));
            }
            match units {
                Ok(units) => {
                    assert!(!swap);
                    let raw: Vec<Vec<u8>> = units.into_iter().map(|(_, raw)| raw).collect();
                    assert_eq!(raw, [records.clone(), payload.clone()]);
                }
                Err(refused) => {
                    assert!(swap);
                    assert!(matches!(Damage::of(&refused).unwrap().scope, DamageScope::Extent(_)));
                }
            }
        }
    }
}
