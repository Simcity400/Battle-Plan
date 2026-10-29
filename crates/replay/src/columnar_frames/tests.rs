use super::fixtures::{every, recording};
use super::*;
use crate::native_columns::{RowCells, TableInput, table_hour};
use replay_columnar::{
    manifest::{self, Section},
    writer::{Dictionary as Strings, Sample, TableWriter},
};
use replay_data::rows::{CountryRow, DivisionRow, RowCodec, WorldTensionRow};
use std::sync::{
    Condvar, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc,
};

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
        self.views
            .advance(&self.state, &manifest, self.hour == 1)
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
    Recording::open(bytes, Limits::default()).unwrap()
}

/// A walk nobody stops.
static GOING: AtomicBool = AtomicBool::new(false);

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

/// A page's manifests are one per ordinal at its run's hour, even where the chain was
/// written over what they hold.
#[test]
fn a_walk_takes_one_manifest_per_ordinal_at_its_run_s_hour() {
    let runs = [(1, 100, 30)];
    let short = |_, ordinal: u32| (ordinal != 29).then_some(100 + ordinal as i32);
    let skipping = |_, ordinal: u32| Some(100 + ordinal as i32 + i32::from(ordinal >= 7));
    let hours: [&dyn Fn(u32, u32) -> Option<i32>; 2] = [&short, &skipping];
    for hour in hours {
        let (bytes, _) = recording(&runs, hour, 1, |_| {});
        let mut file = opened(bytes);
        file.verify(true);
        let segment = file.segments()[0];
        assert!(walk_alone(&file, &segment).is_err());
    }
}

/// A restart restores its run's dictionary chunks in id order, and the run's last restart
/// ends its last segment.
#[test]
fn restarts_restore_their_run_s_next_dictionary_ids() {
    let runs = [(1, 0, 2 * envelope::UNIT_ORDINALS)];
    let (bytes, _) = recording(&runs, every(&runs), 1, |_| {});
    let file = opened(bytes);
    let segments = file.segments();
    let expected = [
        Segment {
            run: 0,
            restart: None,
            first: 0,
            last: 719,
        },
        Segment {
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
    Walker::new(file, &GOING)?.begin(segment, state)
}

/// A walk restores only the tables its recording selects: a damaged restart table it does
/// not select is never read, and reading the rows, changes or views of a table it skips is
/// refused. A recording selecting nothing clears what an earlier walk's state skipped.
#[test]
fn a_selection_restores_only_its_tables_and_refuses_reads_of_the_rest() {
    let runs = [(1, 0, 2 * envelope::UNIT_ORDINALS)];
    let damaged = TableId::Divisions.index() as u32;
    let (bytes, _) = recording(&runs, every(&runs), 1, |page| {
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

    fn open(&self, limits: Limits) -> Recording {
        Recording::open_path(&self.0, limits).unwrap()
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
    for reader in [opened(bytes.clone()), disk.open(Limits::default())] {
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
    let pinned = disk.open(Limits::default());
    let before = bytes.len();
    append_revision(&mut bytes, 2);
    disk.append(&bytes[before..]);
    assert_eq!(pinned.len(), before as u64);
    assert_eq!(
        walk_alone(&pinned, &pinned.segments()[0]).unwrap().len(),
        10
    );
    assert_eq!(disk.open(Limits::default()).len(), bytes.len() as u64);
}

/// A unit a walk reads is always checked against its digest and the recording's extents;
/// one it does not read is not read.
#[test]
fn selected_units_always_check_their_digest_and_range() {
    for range in [false, true] {
        let bytes = fixtures::native_recording(1, |page| {
            let table = page
                .units
                .iter_mut()
                .find(|extent| extent.stream != 0)
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
            DamageScope::Extent(extent) if extent.stream == TableId::Countries.index() as u32 + 1
        ));
        let mut unrelated = opened(bytes);
        unrelated.select(&[]);
        unrelated.walk(&mut state, |_, _, _| Ok(true)).unwrap();
    }
}

/// A unit or restart table whose envelope declares another raw length than its frame is
/// refused as its own damage before any buffer is sized by what it declares, from a file and
/// from memory.
#[test]
fn declared_raw_lengths_are_checked_against_their_frames_before_buffers_are_sized() {
    let countries = TableId::Countries.index() as u32;
    // No allocation could hold either: a refusal for lack of memory names no damage.
    for raw in [1 << 40, u64::MAX] {
        let bytes = fixtures::native_recording(1, |page| {
            for unit in page.units.iter_mut().filter(|unit| unit.stream != 0) {
                unit.raw = raw;
            }
            page.restart.as_mut().unwrap().tables[countries as usize]
                .extent
                .raw = raw;
        });
        let disk = Saved::new(&bytes);
        for file in [opened(bytes.clone()), disk.open(Limits::default())] {
            let mut state = State::new(schema().unwrap()).unwrap();
            let refused = file.walk(&mut state, |_, _, _| Ok(true)).unwrap_err();
            assert!(
                matches!(
                    Damage::of(&refused).unwrap().scope,
                    DamageScope::Extent(extent) if extent.stream == countries + 1 && extent.raw == raw
                ),
                "{refused}"
            );
            assert!(
                refused
                    .to_string()
                    .ends_with("differs from its frame's length"),
                "{refused}"
            );
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

/// The most a walk of `units` units with a one-byte dictionary budget holds, and the bytes
/// of the strings it appended.
fn held_walking(units: u32) -> (usize, usize) {
    let limits = Limits {
        dictionary_bytes: 1,
        ..Limits::default()
    };
    let file = Recording::open(fixtures::native_recording(units, |_| {}), limits).unwrap();
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
/// string it appended, whatever its budget.
#[test]
fn a_run_without_restarts_keeps_every_string_it_appends() {
    let limits = Limits {
        dictionary_bytes: 1,
        ..Limits::default()
    };
    let bytes = fixtures::native_recording(3, |page| page.restart = None);
    let file = Recording::open(bytes, limits).unwrap();
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
/// with a dictionary budget that holds everything and with one that holds nothing past a
/// page: of every run from its start, and of each segment from its restart.
#[test]
fn walks_see_the_writer_s_frames_over_many_runs_and_segments() {
    let (bytes, expected) = fixtures::churned(&CHURNED, 1);
    let disk = Saved::new(&bytes);
    for dictionary_bytes in [Limits::default().dictionary_bytes, 1] {
        let limits = || Limits {
            dictionary_bytes,
            ..Limits::default()
        };
        for file in [
            Recording::open(bytes.clone(), limits()).unwrap(),
            disk.open(limits()),
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
/// table and every string, however little its dictionary holds.
#[test]
fn every_segment_ends_in_its_next_restart_s_state_string_for_string() {
    let (bytes, _) = fixtures::churned(&CHURNED, 1);
    for dictionary_bytes in [Limits::default().dictionary_bytes, 1] {
        let limits = Limits {
            dictionary_bytes,
            ..Limits::default()
        };
        let file = Recording::open(bytes.clone(), limits).unwrap();
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

/// A walk past its dictionary budget drops the strings of the pages before, and reads those
/// that come back, as cells or as reasons, from their chunks.
#[test]
fn a_walk_past_its_dictionary_budget_reads_returning_strings_back() {
    let (bytes, expected) = fixtures::churned(&[(1, 0, 6)], 1);
    let limits = Limits {
        dictionary_bytes: 1,
        ..Limits::default()
    };
    let file = Recording::open(bytes, limits).unwrap();
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

/// Opens a gate walkers wait at.
struct Gate(Mutex<bool>, Condvar);

impl Gate {
    fn new() -> Self {
        Self(Mutex::new(false), Condvar::new())
    }

    fn wait(&self) {
        let mut open = self.0.lock().unwrap();
        while !*open {
            open = self.1.wait(open).unwrap();
        }
    }

    fn open(&self) {
        *self.0.lock().unwrap() = true;
        self.1.notify_all();
    }
}

/// Walkers whose buffers the budget cannot hold at once wait, in turn, for those walking to
/// give theirs back: four walk, four wait, and every walk ends.
#[test]
fn walkers_past_the_budget_wait_for_those_walking() {
    let (bytes, _) = fixtures::churned(&[(1, 0, 8)], 1);
    // A walker's share is mostly its dictionary's budget: four fit, a fifth does not.
    let dictionary_bytes = 64 << 20;
    let budget = Arc::new(Budget::new(4 * dictionary_bytes + (4 << 20)));
    let limits = Limits {
        budget: budget.clone(),
        dictionary_bytes,
    };
    let file = Recording::open(bytes, limits).unwrap();
    assert_eq!(file.segments().len(), 8);
    let (gate, (entered, walking)) = (Gate::new(), mpsc::channel());
    std::thread::scope(|scope| {
        let walkers: Vec<_> = file
            .segments()
            .iter()
            .map(|segment| {
                let (gate, entered) = (&gate, entered.clone());
                let file = &file;
                scope.spawn(move || {
                    let mut walker = Walker::new(file, &GOING).unwrap();
                    let mut state = State::new(schema().unwrap()).unwrap();
                    walker.begin(segment, &mut state).unwrap();
                    let mut hours = 0;
                    walker
                        .walk(segment, &mut state, |_, _, _| {
                            if hours == 0 {
                                let _ = entered.send(());
                                gate.wait();
                            }
                            hours += 1;
                            Ok(true)
                        })
                        .unwrap();
                    hours
                })
            })
            .collect();
        for _ in 0..4 {
            walking.recv().unwrap();
        }
        budget.wait_for_waiters(4);
        assert_eq!(budget.walkers(), 4);
        gate.open();
        for walker in walkers {
            assert_eq!(walker.join().unwrap(), envelope::UNIT_ORDINALS);
        }
    });
    assert_eq!(budget.walkers(), 0);
}

/// Walkers waiting for the budget leave once their walk is stopped, the walker holding it
/// undisturbed.
#[test]
fn a_stopped_walk_leaves_the_budget_s_queue() {
    let (bytes, _) = fixtures::churned(&[(1, 0, 3)], 1);
    let budget = Arc::new(Budget::new(1));
    let limits = Limits {
        budget: budget.clone(),
        ..Limits::default()
    };
    let file = Recording::open(bytes, limits).unwrap();
    let (gate, (entered, walking)) = (Gate::new(), mpsc::channel());
    let stopped = AtomicBool::new(false);
    let segments = file.segments();
    std::thread::scope(|scope| {
        let (file, gate, stopped) = (&file, &gate, &stopped);
        let holder = scope.spawn(move || {
            let mut state = State::new(schema().unwrap()).unwrap();
            let mut first = true;
            file.walk_segment(&segments[0], &mut state, |_, _, _| {
                if std::mem::take(&mut first) {
                    entered.send(()).unwrap();
                    gate.wait();
                }
                Ok(true)
            })
        });
        walking.recv().unwrap();
        let waiters: Vec<_> = segments[1..]
            .iter()
            .map(|segment| {
                scope.spawn(move || {
                    let mut walker = Walker::new(file, stopped)?;
                    let mut state = State::new(schema()?)?;
                    walker.begin(segment, &mut state)?;
                    walker.walk(segment, &mut state, |_, _, _| Ok(true))
                })
            })
            .collect();
        budget.wait_for_waiters(2);
        stopped.store(true, Ordering::Relaxed);
        file.wake();
        for waiter in waiters {
            assert_eq!(
                waiter.join().unwrap().unwrap_err().kind(),
                io::ErrorKind::Interrupted
            );
        }
        gate.open();
        holder.join().unwrap().unwrap();
    });
    assert_eq!(budget.walkers(), 0);
}

/// A walker needing a larger share for a later page gives back the one it holds before it
/// waits: two walkers the budget admits one at a time, each growing past its first page while
/// the other waits, both end.
#[test]
fn a_walker_growing_its_share_gives_back_the_one_it_holds_first() {
    let runs = [
        (1, 0, 2 * envelope::UNIT_ORDINALS),
        (2, 0, 2 * envelope::UNIT_ORDINALS),
    ];
    let (bytes, _) = recording(&runs, every(&runs), 2, |_| {});
    let budget = Arc::new(Budget::new(1));
    let limits = Limits {
        budget: budget.clone(),
        ..Limits::default()
    };
    let file = Recording::open(bytes, limits).unwrap();
    let segments = file.segments();
    assert_eq!(segments.len(), 2);
    // Longer ordinals in its strings: each run's second page holds more than its first.
    for run in 0..2 {
        assert!(file.page(run, 1).unwrap().units[0].raw > file.page(run, 0).unwrap().units[0].raw);
    }
    let (gate, (entered, walking)) = (Gate::new(), mpsc::channel());
    std::thread::scope(|scope| {
        let walkers: Vec<_> = segments
            .iter()
            .map(|segment| {
                let (file, gate, entered) = (&file, &gate, entered.clone());
                scope.spawn(move || {
                    let mut walker = Walker::new(file, &GOING).unwrap();
                    let mut state = State::new(schema().unwrap()).unwrap();
                    walker.begin(segment, &mut state).unwrap();
                    let mut hours = 0;
                    walker
                        .walk(segment, &mut state, |_, _, _| {
                            if hours == 0 {
                                let _ = entered.send(());
                                gate.wait();
                            }
                            hours += 1;
                            Ok(true)
                        })
                        .unwrap();
                    hours
                })
            })
            .collect();
        // One walks its first page; the other waits ahead of its next share.
        walking.recv().unwrap();
        budget.wait_for_waiters(1);
        assert_eq!(budget.walkers(), 1);
        gate.open();
        for walker in walkers {
            assert_eq!(walker.join().unwrap(), 2 * envelope::UNIT_ORDINALS);
        }
    });
    assert_eq!(budget.walkers(), 0);
}
