use super::*;
use crate::native_columns::{RowCells, TableInput, table_hour};
use replay_columnar::{
    manifest::{self, Section},
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
