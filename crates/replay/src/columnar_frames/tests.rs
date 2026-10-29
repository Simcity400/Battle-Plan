use super::*;
use crate::native_columns::{RowCells, TableInput, table_hour};
use replay_columnar::{
    envelope::{Identity, RestartDirectory, RestartTable},
    leaf::Writer,
    manifest::{self, Section},
    restart::{self, Acquisition},
    state::Bits,
    writer::{Dictionary as Strings, Sample, TableWriter},
};
use replay_data::rows::{CountryRow, DivisionRow, RowCodec, WorldTensionRow};

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

/// One extent of `raw` appended to `file`, compressed as the converter stores it.
fn append(
    file: &mut Vec<u8>,
    kind: Kind,
    (run, stream, first, hours): (u32, u32, u32, u32),
    raw: &[u8],
) -> Extent {
    let stored = zstd::bulk::compress(raw, 1).unwrap();
    let extent = Extent {
        kind,
        run,
        stream,
        first,
        hours,
        offset: file.len() as u64,
        stored: stored.len() as u64,
        raw: raw.len() as u64,
        digest: envelope::digest(&stored),
    };
    file.extend_from_slice(&stored);
    extent
}

/// A recording of `runs` (branch, first hour, ordinals) as the converter writes one, of
/// manifests alone: ordinal `n` of run `r` is the hour `hour(r, n)` gives (none leaves it
/// out), appending one dictionary string, and every `restart_units`th full page ends in a
/// restart. `tamper` changes each page before it is written. The file, and each run's page
/// extents.
fn recording(
    runs: &[(u64, i32, u32)],
    hour: impl Fn(u32, u32) -> Option<i32>,
    restart_units: u32,
    tamper: impl Fn(&mut IndexPage),
) -> (Vec<u8>, Vec<Vec<Extent>>) {
    let schema = schema().unwrap();
    let columns: Vec<usize> = schema.iter().map(Vec::len).collect();
    let identity = Identity {
        schema: envelope::schema_digest(&schema),
        campaign: b"campaign".to_vec(),
        content: Vec::new(),
        capture_mode: 0,
    };
    let none = Bits::from_words(vec![0; TABLE_COUNT.div_ceil(64)]).unwrap();
    let mut file = envelope::MAGIC.to_vec();
    file.extend_from_slice(&envelope::VERSION.to_le_bytes());
    let (mut directories, mut every_page) = (Vec::new(), Vec::new());
    for (run, &(branch, first_hour, ordinals)) in (0..).zip(runs) {
        let mut chain = envelope::chain_start(&identity.chain_bytes().unwrap(), run);
        let (mut pages, mut restarts, mut next_id) = (Vec::new(), 0, 0);
        // The strings first used since the run's last restart.
        let mut pending: Vec<Vec<u8>> = Vec::new();
        for first in (0..ordinals).step_by(envelope::UNIT_ORDINALS as usize) {
            let hours = (ordinals - first).min(envelope::UNIT_ORDINALS);
            let strings: Vec<Vec<u8>> = (first..first + hours)
                .map(|ordinal| format!("run {run} ordinal {ordinal}").into_bytes())
                .collect();
            pending.extend(strings.iter().cloned());
            let mut unit = Writer::new();
            for (ordinal, string) in (first..).zip(&strings) {
                let Some(hour) = hour(run, ordinal) else {
                    continue;
                };
                let manifest = Manifest {
                    branch,
                    hour,
                    flags: 0,
                    total_ns: 1,
                    pool_bytes: 0,
                    dropped_frames: 0,
                    appends: vec![string],
                    statuses: vec![Status::NotRecorded; TABLE_COUNT],
                    read: vec![0; TABLE_COUNT.div_ceil(64)],
                    samples: Vec::new(),
                    sections: Vec::new(),
                }
                .write(&columns)
                .unwrap();
                framing::put_manifest(&mut unit, &manifest).unwrap();
                chain = envelope::chain_hour(&chain, &manifest, std::iter::empty());
            }
            let units = vec![append(
                &mut file,
                Kind::Unit,
                (run, framing::MANIFESTS, first, hours),
                unit.bytes(),
            )];
            let restarts_here = hours == envelope::UNIT_ORDINALS
                && (first / envelope::UNIT_ORDINALS + 1) % restart_units == 0;
            let restart = restarts_here.then(|| {
                let after = first + hours - 1;
                let strings = std::mem::take(&mut pending);
                let mut chunk = Writer::new();
                chunk.varint(strings.len() as u64).unwrap();
                for string in &strings {
                    chunk.varint(string.len() as u64).unwrap();
                    chunk.raw(string).unwrap();
                }
                let chunk = append(
                    &mut file,
                    Kind::DictionaryChunk,
                    (run, 0, after, 0),
                    chunk.bytes(),
                );
                let tables = (0..)
                    .zip(&schema)
                    .map(|(table, kinds)| {
                        let bytes = restart::write(
                            &Table::new(kinds).unwrap(),
                            Status::NotRecorded,
                            &Acquisition::default(),
                            &[],
                        )
                        .unwrap();
                        RestartTable {
                            table,
                            extent: append(
                                &mut file,
                                Kind::RestartTable,
                                (run, table, after, 0),
                                &bytes,
                            ),
                            chunks: Vec::new(),
                        }
                    })
                    .collect();
                let restart = RestartDirectory {
                    after,
                    hour: first_hour + after as i32,
                    ordinal: restarts,
                    chunk,
                    ids: (next_id, strings.len() as u32),
                    tables,
                };
                restarts += 1;
                next_id += strings.len() as u32;
                restart
            });
            let mut page = IndexPage {
                run,
                first,
                hours,
                first_hour: first_hour + first as i32,
                last_hour: first_hour + (first + hours - 1) as i32,
                units,
                restart,
                sectioned: none.clone(),
                read: none.clone(),
                status_changed: none.clone(),
                chain,
            };
            tamper(&mut page);
            let raw = page.write().unwrap();
            pages.push(append(
                &mut file,
                Kind::IndexPage,
                (run, 0, first, hours),
                &raw,
            ));
        }
        let directory = RunDirectory {
            branch,
            epoch: envelope::epoch::START,
            first_hour,
            ordinals,
            chain,
            pages: pages.clone(),
        }
        .write(run)
        .unwrap();
        directories.push(append(
            &mut file,
            Kind::RunDirectory,
            (run, 0, 0, 0),
            &directory,
        ));
        every_page.push(pages);
    }
    let root = Root {
        identity,
        revision: 1,
        runs: directories,
        controls: Vec::new(),
    }
    .write()
    .unwrap();
    let root = append(&mut file, Kind::Root, (0, 0, 0, 0), &root);
    file.extend_from_slice(&envelope::footer(root.offset, root.stored, &root.digest));
    (file, every_page)
}

/// Every ordinal of `runs` at its run's hour.
fn every(runs: &[(u64, i32, u32)]) -> impl Fn(u32, u32) -> Option<i32> + '_ {
    |run, ordinal| Some(runs[run as usize].1 + ordinal as i32)
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

/// A page that cannot be read refuses the walks over it; the recording opens and its other
/// runs walk as before.
#[test]
fn a_damaged_page_refuses_only_the_walks_over_it() {
    let runs = [(1, 100, 30), (2, 500, 20)];
    let (mut bytes, pages) = recording(&runs, every(&runs), 1, |_| {});
    let mut whole = Recording::open(bytes.clone()).unwrap();
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
    let damaged = Recording::open(bytes).unwrap();
    assert_eq!(damaged.damage().len(), 1);
    let segments = damaged.segments();
    assert_eq!(segments.len(), 2);
    assert!(walk_alone(&damaged, &segments[0]).is_err());
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
        let mut file = Recording::open(bytes).unwrap();
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
    let file = Recording::open(bytes).unwrap();
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
    let file = Recording::open(bytes).unwrap();
    let mut state = State::new(schema().unwrap()).unwrap();
    assert!(file.restore(0, 0, &mut state).is_ok());
    assert!(file.restore(0, 1, &mut state).is_err());
}

/// An unreadable page holding no restart leaves the run's later restarts whole: their
/// ordinals and ids prove every chunk is there. One that held a restart refuses them.
#[test]
fn restores_need_every_chunk_not_every_page() {
    let runs = [(1, 0, 3 * envelope::UNIT_ORDINALS)];
    for (restart_units, restores) in [(2, true), (1, false)] {
        let (mut bytes, pages) = recording(&runs, every(&runs), restart_units, |_| {});
        bytes[pages[0][0].offset as usize] ^= 1;
        let file = Recording::open(bytes).unwrap();
        assert_eq!(file.damage().len(), 1);
        let segments = file.segments();
        let last = segments.last().unwrap();
        assert_eq!(
            (last.restart, last.first),
            (Some(1), 2 * envelope::UNIT_ORDINALS)
        );
        assert!(walk_alone(&file, &segments[0]).is_err());
        let walked = walk_alone(&file, last);
        assert_eq!(walked.is_ok(), restores, "{walked:?}");
        if restores {
            assert_eq!(walked.unwrap().len(), envelope::UNIT_ORDINALS as usize);
        }
    }
}

/// A segment at its run's start walks from empty tables and a run-local dictionary, whatever
/// the state it is handed held.
#[test]
fn a_run_start_walks_from_empty_tables() {
    let runs = [(1, 0, 10), (2, 100, 6)];
    let (bytes, _) = recording(&runs, every(&runs), 1, |_| {});
    let file = Recording::open(bytes).unwrap();
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
