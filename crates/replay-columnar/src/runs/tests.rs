use super::*;
use crate::{
    leaf::Writer,
    manifest::{Manifest, SAMPLES, Section, Status},
    section::Changes,
    state::{Cell, Kind, Table},
    walk::State,
    writer::{Coverage, Dictionary as Interned, Evidence, Input, Sample as Sampled, TableWriter},
};
use engine_allocation::credit::MemoryBudget;

const UNITS: &[Kind] = &[
    Kind::U32,
    Kind::OptionalI64,
    Kind::Bytes,
    Kind::U32List,
    Kind::Opaque,
    Kind::Bool,
    Kind::I16,
    Kind::U64List,
];
const READS: &[Kind] = &[Kind::U32, Kind::Bytes];
const WIDE: &[Kind] = &[Kind::U8, Kind::I32];
const OTHER: &[Kind] = &[Kind::U32];
const INDEXED: [usize; 3] = [0, 1, 2];
/// The text columns of each indexed table.
const TEXT: [&[usize]; 3] = [&[2], &[1], &[]];
const NARROW: Layout = Layout { block: 8, runs: 24 };
/// Blocks wide enough for one value written to most of a block's rows to be a fill.
const BROAD: Layout = Layout {
    block: 64,
    runs: 40,
};
const RUNS: [(u64, i32, u32); 3] = [(1, 100, 260), (1, 500, 90), (2, 10_000, 130)];
const SEGMENT: u32 = 37;
/// The strings a cursor's dictionary holds before it drops those no live cell names.
const STRINGS: usize = 1 << 20;

/// What a cursor's tables may hold: far more than any fixture's.
fn tables_memory() -> MemoryBudget {
    MemoryBudget::new(1 << 30)
}

fn schema() -> Vec<Vec<Kind>> {
    [UNITS, READS, WIDE, OTHER]
        .iter()
        .map(|kinds| kinds.to_vec())
        .collect()
}

fn indexed_kinds() -> Vec<Vec<Kind>> {
    INDEXED
        .iter()
        .map(|&table| schema()[table].clone())
        .collect()
}

/// A walk's hour as the walker read it: its manifest's bytes and its sections'.
struct Read {
    manifest: Vec<u8>,
    sections: Vec<Vec<u8>>,
}

/// What a walk held of an indexed table after an hour.
#[derive(Clone, Debug)]
struct Held {
    table: Table,
    status: Status,
    sampled: Option<i32>,
    view: Option<Vec<u32>>,
}

/// One run of the fixture: its hours as read, what the walk held after each, and its strings.
struct Run {
    branch: u64,
    first: i32,
    reads: Vec<Read>,
    held: Vec<Vec<Held>>,
    strings: Vec<Vec<u8>>,
}

/// A unit's cells at an hour; an unrecorded optional is `None`.
struct Unit {
    key: u128,
    value: u32,
    optional: Option<Option<i64>>,
    text: Vec<u8>,
    list: Vec<u32>,
    opaque: Vec<u8>,
    flag: bool,
    small: i16,
    longs: Vec<u64>,
}

impl Unit {
    fn cells(&self) -> Vec<Input<'_>> {
        vec![
            Input::Value(Cell::U32(self.value)),
            match self.optional {
                Some(value) => Input::Value(Cell::OptionalI64(value)),
                None => Input::Unrecorded,
            },
            Input::Text(&self.text),
            Input::Value(Cell::U32List(&self.list)),
            Input::Value(Cell::Opaque(&self.opaque)),
            Input::Value(Cell::Bool(self.flag)),
            Input::Value(Cell::I16(self.small)),
            Input::Value(Cell::U64List(&self.longs)),
        ]
    }
}

/// Units that leave and come back, values that move one at a time, together, or most to one
/// value, and rows that come in bursts into the slots others freed.
fn units(ordinal: u32) -> Vec<Unit> {
    let mut rows = Vec::new();
    for key in 1..=70_u32 {
        if (key + ordinal / 9) % 13 == 0 {
            continue;
        }
        rows.push(Unit {
            key: u128::from(key) << 64 | u128::from(key * 3),
            value: if ordinal % 50 < 25 {
                7
            } else {
                key * 3 + ordinal / 4
            },
            optional: if (key + ordinal / 17) % 19 == 0 {
                None
            } else if (key + ordinal) % 5 == 0 {
                Some(None)
            } else {
                Some(Some(i64::from(key) * 100 + i64::from(ordinal / 3)))
            },
            text: format!("unit {key} rank {}", (ordinal / 20 + key) % 6).into_bytes(),
            list: (0..(key + ordinal / 10) % 4)
                .map(|item| item + key)
                .collect(),
            opaque: vec![(ordinal / 15) as u8; (key % 3) as usize],
            flag: (ordinal / 30 + key) % 2 == 0,
            small: if key % 7 == 0 {
                -1
            } else {
                (ordinal / 6) as i16
            },
            longs: if key % 4 == 0 {
                vec![u64::from(ordinal / 25), u64::MAX - u64::from(key)]
            } else {
                Vec::new()
            },
        });
    }
    for extra in 0..(ordinal / 25) % 4 {
        rows.push(Unit {
            key: u128::from(1_000 + extra + ordinal / 100 * 10),
            value: extra,
            optional: Some(Some(-1)),
            text: b"burst".to_vec(),
            list: vec![extra],
            opaque: Vec::new(),
            flag: true,
            small: 0,
            longs: Vec::new(),
        });
    }
    rows.sort_by_key(|row| row.key);
    rows
}

/// The read table's rows `keep` keeps, their strings `texts`.
fn read_rows(
    texts: &[Vec<u8>],
    ordinal: u32,
    keep: impl Fn(u32) -> bool,
) -> Vec<(u128, Vec<Input<'_>>)> {
    (1..=20_u32)
        .filter(|&key| keep(key))
        .map(|key| {
            (
                u128::from(key),
                vec![
                    Input::Value(Cell::U32(key * 10 + ordinal / 5)),
                    Input::Text(&texts[key as usize - 1]),
                ],
            )
        })
        .collect()
}

/// What an hour's tables wrote for its manifest.
#[derive(Default)]
struct Out {
    statuses: Vec<Status>,
    read: Vec<u64>,
    samples: Vec<(usize, Option<i32>)>,
    sections: Vec<(Section, Vec<u8>)>,
}

/// One table's hour: its rows under `coverage`, or none, and its evidence.
#[allow(clippy::too_many_arguments)]
fn table_hour(
    writer: &mut TableWriter,
    strings: &mut Interned,
    table: usize,
    coverage: Option<Coverage<'_>>,
    rows: &[(u128, Vec<Input<'_>>)],
    evidence: Evidence,
    hour: i32,
    out: &mut Out,
) {
    let prepared = coverage.map(|coverage| {
        let mut writing = writer.begin(&mut *strings, coverage).unwrap();
        for (key, cells) in rows {
            writing.row(*key, cells).unwrap();
        }
        writing.finish().unwrap()
    });
    if let Some(prepared) = &prepared
        && let Some(bytes) = &prepared.section
    {
        out.sections.push((
            Section {
                table,
                bytes: bytes.len() as u64,
                columns: prepared.columns.clone(),
                ops: prepared.ops,
            },
            bytes.clone(),
        ));
    }
    out.statuses[table] = evidence.status;
    if evidence.read {
        out.read[table / 64] |= 1 << (table % 64);
    }
    match evidence.sample {
        Sampled::FromReadBit => {}
        Sampled::Reset => out.samples.push((table, None)),
        Sampled::Carried(hour) => out.samples.push((table, Some(hour))),
    }
    writer.commit(prepared, evidence, hour).unwrap();
}

/// A run of `ordinals` hours of branch `branch` from hour `first`, its units each hour's
/// `units`, written as the native writer writes it and walked.
fn churn(branch: u64, first: i32, ordinals: u32, units: fn(u32) -> Vec<Unit>) -> Run {
    let schema = schema();
    let columns: Vec<usize> = schema.iter().map(Vec::len).collect();
    let mut writers: Vec<TableWriter> = schema
        .iter()
        .map(|kinds| TableWriter::new(kinds).unwrap())
        .collect();
    let mut strings = Interned::new();
    let mut state = State::new(schema.clone()).unwrap();
    let (mut reads, mut held) = (Vec::new(), Vec::new());
    for ordinal in 0..ordinals {
        let hour = first + ordinal as i32;
        let mut out = Out {
            statuses: vec![Status::NotRecorded; schema.len()],
            read: vec![0; schema.len().div_ceil(64)],
            ..Out::default()
        };
        let complete = Evidence {
            status: Status::Complete,
            read: true,
            sample: Sampled::FromReadBit,
        };
        let held_units = units(ordinal);
        let rows: Vec<_> = held_units
            .iter()
            .map(|unit| (unit.key, unit.cells()))
            .collect();
        table_hour(
            &mut writers[0],
            &mut strings,
            0,
            Some(Coverage::Complete),
            &rows,
            complete,
            hour,
            &mut out,
        );
        let texts: Vec<Vec<u8>> = (1..=20_u32)
            .map(|key| format!("read {key} {}", ordinal / 8 % 3).into_bytes())
            .collect();
        match ordinal % 40 {
            0..15 => table_hour(
                &mut writers[1],
                &mut strings,
                1,
                Some(Coverage::Complete),
                &read_rows(&texts, ordinal, |_| true),
                complete,
                hour,
                &mut out,
            ),
            15..25 => table_hour(
                &mut writers[1],
                &mut strings,
                1,
                Some(Coverage::Partial),
                &read_rows(&texts, ordinal, |key| (key + ordinal) % 3 != 0),
                Evidence {
                    status: Status::Truncated { missing: 2 },
                    ..complete
                },
                hour,
                &mut out,
            ),
            25..32 => {
                let reason = strings
                    .intern(format!("refused {}", ordinal / 40 % 3).as_bytes())
                    .unwrap();
                table_hour(
                    &mut writers[1],
                    &mut strings,
                    1,
                    None,
                    &[],
                    Evidence {
                        status: Status::Unavailable { reason },
                        read: false,
                        sample: Sampled::FromReadBit,
                    },
                    hour,
                    &mut out,
                );
            }
            _ => table_hour(
                &mut writers[1],
                &mut strings,
                1,
                None,
                &[],
                Evidence {
                    status: Status::NotRecorded,
                    read: false,
                    sample: Sampled::FromReadBit,
                },
                hour,
                &mut out,
            ),
        }
        let wide: Vec<(u128, Vec<Input<'_>>)> = (1..=12_u32)
            .map(|key| {
                (
                    u128::from(key),
                    vec![
                        Input::Value(Cell::U8(if key < 6 {
                            (ordinal / 30) as u8
                        } else {
                            key as u8
                        })),
                        Input::Value(Cell::I32(-(key as i32) * (ordinal / 45) as i32)),
                    ],
                )
            })
            .collect();
        let sample = if ordinal % 7 == 0 {
            Sampled::Carried(hour - 2)
        } else if ordinal % 11 == 0 {
            Sampled::Reset
        } else {
            Sampled::FromReadBit
        };
        table_hour(
            &mut writers[2],
            &mut strings,
            2,
            Some(Coverage::Complete),
            &wide,
            Evidence {
                status: Status::Complete,
                read: ordinal % 3 == 0,
                sample,
            },
            hour,
            &mut out,
        );
        table_hour(
            &mut writers[3],
            &mut strings,
            3,
            Some(Coverage::Complete),
            &[(1, vec![Input::Value(Cell::U32(ordinal))])],
            complete,
            hour,
            &mut out,
        );
        let appends = strings.take_appended();
        let manifest = Manifest {
            branch,
            hour,
            flags: if out.samples.is_empty() { 0 } else { SAMPLES },
            total_ns: u64::from(ordinal) * 7,
            pool_bytes: 3,
            dropped_frames: u64::from(ordinal % 5),
            appends: appends.iter().map(|append| &append[..]).collect(),
            statuses: out.statuses,
            read: out.read,
            samples: out.samples,
            sections: out
                .sections
                .iter()
                .map(|(section, _)| section.clone())
                .collect(),
        };
        let bytes = manifest.write(&columns).unwrap();
        let sections: Vec<Vec<u8>> = out.sections.into_iter().map(|(_, bytes)| bytes).collect();
        let read = Manifest::read(&bytes, &columns).unwrap();
        state.apply(&read, |at| Ok(&sections[at][..])).unwrap();
        held.push(
            INDEXED
                .iter()
                .map(|&table| Held {
                    table: state.table(table).unwrap().clone(),
                    status: state.status(table).unwrap(),
                    sampled: state.sampled_at(table),
                    view: state.truncated_view(table).map(<[u32]>::to_vec),
                })
                .collect(),
        );
        reads.push(Read {
            manifest: bytes,
            sections,
        });
    }
    let dictionary = state.dictionary();
    Run {
        branch,
        first,
        reads,
        held,
        strings: (0..dictionary.len() as u32)
            .map(|id| dictionary.get(id).unwrap().to_vec())
            .collect(),
    }
}

fn churned(runs: &[(u64, i32, u32)]) -> Vec<Run> {
    runs.iter()
        .map(|&(branch, first, ordinals)| churn(branch, first, ordinals, units))
        .collect()
}

/// A segment's walk's bytes in memory: what it stored, and its open pages' bytes.
#[derive(Default)]
struct Walked {
    stored: Memory,
    kept: Vec<u8>,
}

impl Store for Walked {
    fn put(&mut self, stored: &[u8], raw: u32) -> io::Result<Location> {
        self.stored.put(stored, raw)
    }
}

impl Disk for Walked {
    fn keep(&mut self, bytes: &[u8]) -> io::Result<u64> {
        let at = self.kept.len() as u64;
        self.kept.extend_from_slice(bytes);
        Ok(at)
    }

    fn kept(&mut self, at: u64, into: &mut [u8]) -> io::Result<()> {
        into.copy_from_slice(&self.kept[at as usize..][..into.len()]);
        Ok(())
    }
}

/// A segment's piece, and the bytes its walk stored.
type Walk = (Piece, Vec<u8>);

/// Run `run`'s pieces, a segment from each ordinal of `starts` through the hour before the
/// next or `end`, walked from its run's start.
fn pieces(layout: Layout, number: u32, run: &Run, starts: &[u32], end: Option<u32>) -> Vec<Walk> {
    let columns: Vec<usize> = schema().iter().map(Vec::len).collect();
    let mut state = State::new(schema()).unwrap();
    let mut builder: Option<(Builder, Walked)> = None;
    let mut pieces = Vec::new();
    let done = |(mut builder, mut disk): (Builder, Walked)| -> Walk {
        (builder.finish(&mut disk).unwrap(), disk.stored.0)
    };
    for (ordinal, read) in (0..).zip(&run.reads) {
        if Some(ordinal) == end {
            break;
        }
        if starts.contains(&ordinal) {
            if let Some(walked) = builder.take() {
                pieces.push(done(walked));
            }
            let mut next = Builder::new(layout, &INDEXED, indexed_kinds(), Box::new(Raw)).unwrap();
            next.start(state.dictionary().len()).unwrap();
            builder = Some((next, Walked::default()));
        }
        let manifest = Manifest::read(&read.manifest, &columns).unwrap();
        state
            .apply(&manifest, |at| Ok(&read.sections[at][..]))
            .unwrap();
        if let Some((builder, disk)) = &mut builder {
            builder
                .hour(number, ordinal, &state, &manifest, disk)
                .unwrap();
        }
    }
    if let Some(walked) = builder {
        pieces.push(done(walked));
    }
    pieces
}

/// The segments' first ordinals of `ordinals` hours.
fn segments(ordinals: u32, length: u32) -> Vec<u32> {
    (0..ordinals).step_by(length as usize).collect()
}

/// Pages kept in memory as stored.
#[derive(Default)]
struct Memory(Vec<u8>);

impl Store for Memory {
    fn put(&mut self, stored: &[u8], raw: u32) -> io::Result<Location> {
        let offset = self.0.len() as u64;
        self.0.extend_from_slice(stored);
        Ok(Location {
            offset,
            stored: stored.len() as u32,
            raw,
        })
    }
}

impl Pages for Memory {
    fn read(&self, location: Location, into: &mut Vec<u8>) -> io::Result<()> {
        assert_eq!(location.stored, location.raw);
        into.clear();
        into.extend_from_slice(&self.0[location.offset as usize..][..location.stored as usize]);
        Ok(())
    }
}

/// The index of `runs`, each run's pieces segment by segment as `pieces_of` gives them.
fn stitched(
    layout: Layout,
    runs: &[Run],
    mut pieces_of: impl FnMut(u32, &Run) -> Vec<Walk>,
) -> (Index, Memory) {
    let shapes: Vec<(u64, i32)> = runs.iter().map(|run| (run.branch, run.first)).collect();
    let mut index = Index::new(layout, &INDEXED, indexed_kinds(), &shapes).unwrap();
    let mut memory = Memory::default();
    for (number, run) in (0..).zip(runs) {
        for (piece, bytes) in pieces_of(number, run) {
            let at = memory.0.len() as u64;
            memory.0.extend_from_slice(&bytes);
            index.append(piece, at).unwrap();
        }
    }
    (index, memory)
}

fn segmented(layout: Layout, runs: &[Run]) -> (Index, Memory) {
    stitched(layout, runs, |number, run| {
        pieces(
            layout,
            number,
            run,
            &segments(run.reads.len() as u32, SEGMENT),
            None,
        )
    })
}

/// The cursor holds what the walk held after ordinal `ordinal` of `run`, string for string.
fn holds(cursor: &Cursor, run: &Run, ordinal: u32) {
    let state = cursor.state();
    for (at, &table) in INDEXED.iter().enumerate() {
        let expected = &run.held[ordinal as usize][at];
        let context = format!(
            "table {table} at ordinal {ordinal} of branch {}",
            run.branch
        );
        let held = state.table(table).unwrap();
        assert_eq!(held, &expected.table, "{context}");
        for slot in held.live().ones() {
            assert_eq!(
                held.slot(held.key(slot).unwrap()),
                Some(slot),
                "{context}: slot {slot}"
            );
        }
        assert_eq!(held.by_key.len(), held.live().count(), "{context}");
        assert_eq!(state.status(table), Some(expected.status), "{context}");
        assert_eq!(state.sampled_at(table), expected.sampled, "{context}");
        assert_eq!(
            state.truncated_view(table).map(<[u32]>::to_vec),
            expected.view,
            "{context}"
        );
        let text = |id: u32| {
            assert_eq!(
                state.dictionary().get(id),
                Some(run.strings[id as usize].as_slice()),
                "{context}: string {id}"
            );
        };
        if let Status::Unavailable { reason } = expected.status {
            text(reason);
        }
        for &column in TEXT[at] {
            for slot in held.live().ones() {
                if held.recorded(column, slot)
                    && let Some(Cell::Bytes(id)) = held.column(column).unwrap().cell(slot)
                {
                    text(id);
                }
            }
        }
    }
}

/// A walk of numbers: the same for the same seed.
struct Random(u64);

impl Random {
    fn below(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % bound
    }
}

/// Seeks hour by hour, then to hours far and near, forward, back and across runs, on one
/// worker and on several: every seek holds exactly what walking the recording held there,
/// statuses, sample hours, truncated views and strings with it, through fills, rows that leave
/// and come back into freed slots, and gaps between runs.
#[test]
fn seeks_hold_what_walking_held_at_every_hour() {
    let runs = churned(&RUNS);
    for layout in [NARROW, BROAD] {
        let (index, memory) = segmented(layout, &runs);
        // A dictionary held to a few bytes drops what no live cell names at every move.
        for (workers, strings) in [(1, STRINGS), (3, 64)] {
            let mut cursor = Cursor::new(
                &index,
                schema(),
                &INDEXED,
                workers,
                strings,
                tables_memory(),
            )
            .unwrap();
            for (number, run) in (0..).zip(&runs) {
                for ordinal in 0..run.reads.len() as u32 {
                    cursor
                        .seek(&index, &memory, number, ordinal, &|| {}, &|| false)
                        .unwrap();
                    assert_eq!(cursor.fresh(), ordinal == 0);
                    holds(&cursor, run, ordinal);
                }
            }
            let mut random = Random(u64::from(layout.block) + workers as u64);
            for _ in 0..600 {
                let number = random.below(runs.len() as u64) as u32;
                let run = &runs[number as usize];
                let ordinal = random.below(run.reads.len() as u64) as u32;
                cursor
                    .seek(&index, &memory, number, ordinal, &|| {}, &|| false)
                    .unwrap();
                holds(&cursor, run, ordinal);
            }
        }
    }
}

/// The slots of `page`'s block its table held at the page's first ordinal, as `piece` holds
/// its slot counts.
fn extent(piece: &Piece, page: &Closed, layout: Layout) -> u32 {
    let slots = *at(&piece.series[page.table as usize].slots, page.first)
        .unwrap()
        .1;
    slots
        .saturating_sub(page.block * layout.block)
        .min(layout.block)
}

/// `index` with the first page of its first table's first column and block in place of the
/// one it holds: `bytes`, kept in `memory`, from ordinal 0 through `last`.
fn replaced(index: &Index, memory: &mut Memory, bytes: &[u8], last: u32) -> Index {
    let mut index = index.clone();
    let location = Location {
        offset: memory.0.len() as u64,
        stored: bytes.len() as u32,
        raw: bytes.len() as u32,
    };
    memory.0.extend_from_slice(bytes);
    index.runs[0].strips[0].columns[0][0][0] = Page {
        first: 0,
        last,
        location,
    };
    index
}

/// A page holds a cell of every row its table holds: one naming fewer slots than its block
/// held at its first ordinal, or one leaving a row inserted later without a cell, is refused,
/// never read as the column's default.
#[test]
fn a_page_holding_no_cell_of_a_live_row_is_refused() {
    // Key 1 from the first hour, and key 2, in the next slot, from the second.
    let runs = [churn(1, 0, 3, |ordinal| {
        (1..=1 + ordinal.min(1)).map(plain).collect()
    })];
    let (index, mut memory) = segmented(NARROW, &runs);
    let refused = |error: io::Error| assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    // No slot, no unrecorded cell and no change: key 1's value would read as a recorded zero.
    let empty = replaced(&index, &mut memory, &[0, 0, 0], 0);
    let mut cursor = Cursor::new(&empty, schema(), &INDEXED, 1, STRINGS, tables_memory()).unwrap();
    refused(
        cursor
            .seek(&empty, &memory, 0, 0, &|| {}, &|| false)
            .unwrap_err(),
    );
    refused(rows_at(&empty, &memory, 0, 0, 0, &[1]).unwrap_err());
    // Key 1's value alone: right at the first hour, and short of key 2 from the second.
    let short = replaced(&index, &mut memory, &[1, 0, 1, 0], 0);
    for path in [&[0, 1][..], &[1]] {
        let mut cursor =
            Cursor::new(&short, schema(), &INDEXED, 1, STRINGS, tables_memory()).unwrap();
        for &ordinal in path {
            let moved = cursor.seek(&short, &memory, 0, ordinal, &|| {}, &|| false);
            match ordinal {
                0 => {
                    moved.unwrap();
                    holds(&cursor, &runs[0], 0);
                }
                _ => refused(moved.unwrap_err()),
            }
        }
    }
    assert_eq!(
        rows_at(&short, &memory, 0, 1, 0, &[1])
            .unwrap()
            .table
            .live()
            .count(),
        1
    );
    refused(rows_at(&short, &memory, 0, 1, 0, &[2]).unwrap_err());
}

/// A difference moves only a cell its page set whole before: a page whose first change of a row
/// inserted after its first ordinal is a difference, or a patch, sets no cell of that row and is
/// refused, never read as the default moved by it.
#[test]
fn a_difference_with_no_cell_set_before_is_refused() {
    // Key 1 from the first hour, and key 2, in the next slot, from the second.
    let runs = [churn(1, 0, 3, |ordinal| {
        (1..=1 + ordinal.min(1)).map(plain).collect()
    })];
    let (index, mut memory) = segmented(NARROW, &runs);
    let refused = |error: io::Error| assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    // Key 1's value, 1, at ordinal 0; at ordinal 1 a zero difference at key 2's slot.
    let moved = replaced(&index, &mut memory, &[1, 0, 1, 3, 1, 2, 12, 0], 1);
    let mut cursor = Cursor::new(&moved, schema(), &INDEXED, 1, STRINGS, tables_memory()).unwrap();
    cursor
        .seek(&moved, &memory, 0, 0, &|| {}, &|| false)
        .unwrap();
    holds(&cursor, &runs[0], 0);
    refused(
        cursor
            .seek(&moved, &memory, 0, 1, &|| {}, &|| false)
            .unwrap_err(),
    );
    let mut fresh = Cursor::new(&moved, schema(), &INDEXED, 1, STRINGS, tables_memory()).unwrap();
    refused(
        fresh
            .seek(&moved, &memory, 0, 1, &|| {}, &|| false)
            .unwrap_err(),
    );
    refused(rows_at(&moved, &memory, 0, 1, 0, &[2]).unwrap_err());
    // The same page setting key 2's value whole at ordinal 1 is read as written.
    let set = replaced(&index, &mut memory, &[1, 0, 1, 3, 1, 2, 8, 2], 1);
    let mut cursor = Cursor::new(&set, schema(), &INDEXED, 1, STRINGS, tables_memory()).unwrap();
    for ordinal in [0, 1, 0, 1] {
        cursor
            .seek(&set, &memory, 0, ordinal, &|| {}, &|| false)
            .unwrap();
        holds(&cursor, &runs[0], ordinal);
    }
}

/// A slot count is admitted only as far as the blocks its table's strips hold, and only as far
/// as its cursor's memory: an index naming slots no strip holds is refused as it is read, and
/// one held in memory is refused before its tables grow.
#[test]
fn a_slot_count_past_its_strips_is_refused_before_tables_grow() {
    let runs = [churn(1, 0, 3, |_| vec![plain(1)])];
    let (index, memory) = segmented(NARROW, &runs);
    let mut inflated = index.clone();
    let slots = &mut inflated.runs[0].series[0].slots;
    assert_eq!(slots.as_slice(), [(0, 1)]);
    slots[0].1 = u32::MAX;
    let refused = Index::read(&inflated.write().unwrap(), &schema()).unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::InvalidData);
    // A second block from the first hour, which no strip of the table holds.
    let mut wider = index.clone();
    wider.runs[0].series[0].slots[0].1 = NARROW.block + 1;
    assert!(Index::read(&wider.write().unwrap(), &schema()).is_err());
    assert!(Index::read(&index.write().unwrap(), &schema()).unwrap() == index);
    let budget = MemoryBudget::new(1 << 20);
    let mut cursor =
        Cursor::new(&inflated, schema(), &INDEXED, 1, STRINGS, budget.clone()).unwrap();
    let error = cursor
        .seek(&inflated, &memory, 0, 0, &|| {}, &|| false)
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
    assert!(budget.peak() <= 1 << 20);
    // The same budget holds the index as written.
    let mut cursor = Cursor::new(&index, schema(), &INDEXED, 1, STRINGS, budget.clone()).unwrap();
    for ordinal in [0, 2, 1] {
        cursor
            .seek(&index, &memory, 0, ordinal, &|| {}, &|| false)
            .unwrap();
        holds(&cursor, &runs[0], ordinal);
    }
    assert!(budget.used() > 0);
}

/// A page's changes lie within the ordinals its directory entry names: a page whose changes
/// pass its last ordinal, or end short of it, is refused wherever a move reads it, so a move
/// back never trusts a last its changes contradict; and a directory naming a page past its
/// run's ordinals is not read.
#[test]
fn a_page_whose_changes_disagree_with_its_directory_is_refused() {
    // The unit's value is 7 at the first hour and 9 from the second.
    let runs = [churn(1, 0, 3, |ordinal| {
        vec![Unit {
            value: if ordinal == 0 { 7 } else { 9 },
            ..plain(1)
        }]
    })];
    let (index, memory) = segmented(NARROW, &runs);
    let page = index.runs[0].strips[0].columns[0][0][0];
    assert_eq!((page.first, page.last), (0, 1));
    let with_last = |last: u32| {
        let mut index = index.clone();
        index.runs[0].strips[0].columns[0][0][0].last = last;
        index
    };
    let refused = |error: io::Error| assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    let early = with_last(0);
    let mut cursor = Cursor::new(&early, schema(), &INDEXED, 1, STRINGS, tables_memory()).unwrap();
    for ordinal in [1, 0] {
        refused(
            cursor
                .seek(&early, &memory, 0, ordinal, &|| {}, &|| false)
                .unwrap_err(),
        );
    }
    let late = with_last(2);
    let mut cursor = Cursor::new(&late, schema(), &INDEXED, 1, STRINGS, tables_memory()).unwrap();
    cursor
        .seek(&late, &memory, 0, 0, &|| {}, &|| false)
        .unwrap();
    holds(&cursor, &runs[0], 0);
    refused(
        cursor
            .seek(&late, &memory, 0, 2, &|| {}, &|| false)
            .unwrap_err(),
    );
    assert!(Index::read(&with_last(3).write().unwrap(), &schema()).is_err());
    assert!(Index::read(&index.write().unwrap(), &schema()).unwrap() == index);
}

/// A unit of `key` whose cells never change.
fn plain(key: u32) -> Unit {
    Unit {
        key: u128::from(key),
        value: key,
        optional: Some(Some(i64::from(key))),
        text: b"plain".to_vec(),
        list: vec![key],
        opaque: Vec::new(),
        flag: false,
        small: 0,
        longs: Vec::new(),
    }
}

/// A key that leaves a slot of one block and comes back into a freed slot of an earlier
/// block is found at its new slot, however the cursor reaches the hour: the earlier block's
/// strip is decoded first, and the later one's page still holds the key at its old slot.
#[test]
fn a_key_back_in_an_earlier_block_is_found_at_its_new_slot() {
    const KEYS: [&[u32]; 5] = [
        &[1, 2, 3, 4],
        &[2, 3, 4],
        &[2, 4],
        &[2, 3, 4],
        &[1, 2, 3, 4],
    ];
    let runs = [churn(1, 0, KEYS.len() as u32, |ordinal| {
        KEYS[ordinal as usize]
            .iter()
            .map(|&key| plain(key))
            .collect()
    })];
    // Key 3 leaves slot 2 at ordinal 2 and takes slot 0, the lowest free, at ordinal 3.
    assert_eq!(runs[0].held[3][0].table.slot(3), Some(0));
    let layout = Layout { block: 2, runs: 24 };
    let (index, memory) = segmented(layout, &runs);
    for path in [&[3, 4, 2][..], &[4, 0, 3, 1, 4]] {
        let mut cursor =
            Cursor::new(&index, schema(), &INDEXED, 1, STRINGS, tables_memory()).unwrap();
        for &ordinal in path {
            cursor
                .seek(&index, &memory, 0, ordinal, &|| {}, &|| false)
                .unwrap();
            holds(&cursor, &runs[0], ordinal);
        }
    }
}

/// One value written to most of a block's rows is one change, its exceptions named, and
/// seeks through it hold what the walk held.
#[test]
fn a_value_written_to_most_of_a_block_is_one_change() {
    let runs = churned(&RUNS[..1]);
    let pieces = pieces(BROAD, 0, &runs[0], &[0], None);
    let (mut fills, mut cells) = (0, 0);
    let mut items = cell::Items::default();
    let (piece, stored) = &pieces[0];
    for page in piece.pages.iter().filter(|page| page.column == 6) {
        let start = page.location.offset as usize;
        let bytes = &stored[start..start + page.location.stored as usize];
        let mut velocity = Vec::new();
        let from = cell::base(
            bytes,
            Some(Kind::I16),
            (page.first, page.last),
            extent(piece, page, BROAD),
            &mut velocity,
            &mut items,
            &mut |_| Ok(()),
        )
        .unwrap();
        cell::changes(
            bytes,
            Some(Kind::I16),
            BROAD.block,
            from,
            u32::MAX,
            &mut velocity,
            &mut items,
            &mut |write| {
                match write {
                    cell::Write::Fill { .. } => fills += 1,
                    _ => cells += 1,
                }
                Ok(())
            },
        )
        .unwrap();
    }
    // Most of the 70 rows take one value every sixth hour, the rest keep theirs.
    assert!(fills >= 30, "{fills} fills");
    assert!(cells < fills * 16, "{cells} cells beside {fills} fills");
    let (index, memory) = segmented(BROAD, &runs);
    let mut cursor = Cursor::new(&index, schema(), &INDEXED, 1, STRINGS, tables_memory()).unwrap();
    for ordinal in [5, 6, 7, 36, 42, 200, 41, 6] {
        cursor
            .seek(&index, &memory, 0, ordinal, &|| {}, &|| false)
            .unwrap();
        holds(&cursor, &runs[0], ordinal);
    }
}

/// A list that gains an item every other hour is written as the item, and an integer that
/// rises by one step an hour as a byte once its first step is written; seeks through them
/// hold what the walk held.
#[test]
fn growing_lists_and_steady_steps_are_written_as_what_changed() {
    const HOURS: u32 = 120;
    let runs = [churn(1, 0, HOURS, |ordinal| {
        (1..=20_u32)
            .map(|key| Unit {
                value: 1_000_000 + 4_096 * ordinal + key,
                list: (0..ordinal / 2 + key + 8).collect(),
                ..plain(key)
            })
            .collect()
    })];
    let layout = Layout {
        block: 64,
        runs: 100_000,
    };
    let pieces = pieces(layout, 0, &runs[0], &[0], None);
    let mut items = cell::Items::default();
    for column in [0, 3] {
        let (piece, stored) = &pieces[0];
        let pages: Vec<_> = piece
            .pages
            .iter()
            .filter(|page| page.table == 0 && page.column == column)
            .collect();
        assert_eq!(pages.len(), 1, "column {column}");
        let page = pages[0];
        let start = page.location.offset as usize;
        let bytes = &stored[start..start + page.location.stored as usize];
        let kind = UNITS[column as usize];
        let mut velocity = Vec::new();
        let (mut steps, mut patches, mut others) = (0, 0, 0);
        let from = cell::base(
            bytes,
            Some(kind),
            (page.first, page.last),
            extent(piece, page, layout),
            &mut velocity,
            &mut items,
            &mut |_| Ok(()),
        )
        .unwrap();
        cell::changes(
            bytes,
            Some(kind),
            layout.block,
            from,
            u32::MAX,
            &mut velocity,
            &mut items,
            &mut |write| {
                match write {
                    cell::Write::Delta { delta: 4_096, .. } => steps += 1,
                    cell::Write::Patch {
                        front,
                        back: 0,
                        middle: Cell::U32List(&[item]),
                        ..
                    } if item as usize == front => patches += 1,
                    _ => others += 1,
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(others, 0, "column {column}");
        if column == 0 {
            let changes = 20 * (HOURS as usize - 1);
            assert_eq!(steps, changes);
            // A group's two header bytes, then a byte of header and one of value a change.
            assert!(
                bytes.len() < changes * 2 + 4 * HOURS as usize + 100,
                "{} bytes",
                bytes.len()
            );
        } else {
            assert_eq!(patches, 20 * ((HOURS as usize - 1) / 2));
        }
    }
    let (index, memory) = segmented(layout, &runs);
    let mut cursor = Cursor::new(&index, schema(), &INDEXED, 1, STRINGS, tables_memory()).unwrap();
    for ordinal in [0, 1, 2, 3, 50, 51, 119, 7, 118, 119] {
        cursor
            .seek(&index, &memory, 0, ordinal, &|| {}, &|| false)
            .unwrap();
        holds(&cursor, &runs[0], ordinal);
    }
}

/// Whether `changes` name every row and cell of `after` that differs from `before`, and name
/// rows inserted or deleted only where they were.
fn names_every_difference(changes: &Changes, before: &Table, after: &Table, columns: usize) {
    for slot in after.live().ones() {
        let key = after.key(slot);
        if before.key(slot) != key {
            assert!(changes.inserted.binary_search(&slot).is_ok(), "slot {slot}");
            continue;
        }
        for column in 0..columns {
            let differs = before.column(column).unwrap().cell(slot)
                != after.column(column).unwrap().cell(slot)
                || before.recorded(column, slot) != after.recorded(column, slot);
            if differs {
                let at = changes.changed.binary_search(&slot).unwrap();
                assert_ne!(changes.masks[at] & 1 << column, 0, "slot {slot} {column}");
            }
        }
    }
    for slot in before.live().ones() {
        if after.key(slot) != before.key(slot) {
            let at = changes.deleted.binary_search(&slot).unwrap();
            assert_eq!(Some(changes.deleted_keys[at]), before.key(slot));
        }
    }
    for &slot in &changes.inserted {
        assert!(after.live().get(slot) && after.key(slot) != before.key(slot));
    }
    for &slot in &changes.deleted {
        assert!(before.live().get(slot) && after.key(slot) != before.key(slot));
    }
    for &slot in &changes.changed {
        assert!(after.live().get(slot) && after.key(slot) == before.key(slot));
    }
}

/// A move's changes name what differs between the two hours, as one hour of the walk names
/// what it changed, however far apart and in either direction.
#[test]
fn moves_name_every_row_and_cell_that_differs() {
    let runs = churned(&RUNS[..1]);
    let run = &runs[0];
    let (index, memory) = segmented(NARROW, &runs);
    let mut cursor = Cursor::new(&index, schema(), &INDEXED, 2, STRINGS, tables_memory()).unwrap();
    let mut random = Random(7);
    cursor
        .seek(&index, &memory, 0, 0, &|| {}, &|| false)
        .unwrap();
    let mut from = 0_u32;
    for step in 0..400 {
        let to = if step % 3 == 0 {
            (from + 1).min(run.reads.len() as u32 - 1)
        } else {
            random.below(run.reads.len() as u64) as u32
        };
        cursor
            .seek(&index, &memory, 0, to, &|| {}, &|| false)
            .unwrap();
        for (at, &table) in INDEXED.iter().enumerate() {
            names_every_difference(
                cursor.state().changes(table).unwrap(),
                &run.held[from as usize][at].table,
                &run.held[to as usize][at].table,
                schema()[table].len(),
            );
        }
        from = to;
    }
}

/// Segments walked apart, each on its own thread from its own start, write the pieces one
/// walk writes, and stitched they make the same index, byte for byte; an index of whole runs
/// seeks alike, and an index reads back as written.
#[test]
fn segments_written_apart_stitch_into_the_index_one_walk_writes() {
    let runs = churned(&RUNS);
    let serial = segmented(NARROW, &runs);
    let parallel = stitched(NARROW, &runs, |number, run| {
        let starts = segments(run.reads.len() as u32, SEGMENT);
        let alone: Vec<Vec<Walk>> = std::thread::scope(|scope| {
            let walks: Vec<_> = starts
                .iter()
                .enumerate()
                .map(|(at, &first)| {
                    let end = starts.get(at + 1).copied();
                    scope.spawn(move || pieces(NARROW, number, run, &[first], end))
                })
                .collect();
            walks.into_iter().map(|walk| walk.join().unwrap()).collect()
        });
        assert!(alone.iter().all(|pieces| pieces.len() == 1));
        let walked = pieces(NARROW, number, run, &starts, None);
        assert!(
            alone.iter().flatten().eq(walked.iter()),
            "a segment walked alone writes another piece"
        );
        alone.into_iter().flatten().collect()
    });
    assert!(parallel.0 == serial.0, "the stitched indexes differ");
    assert_eq!(parallel.1.0, serial.1.0);
    let written = serial.0.write().unwrap();
    assert_eq!(parallel.0.write().unwrap(), written);
    assert!(Index::read(&written, &schema()).unwrap() == serial.0);
    let whole = stitched(NARROW, &runs, |number, run| {
        pieces(NARROW, number, run, &[0], None)
    });
    let mut cursors = [
        Cursor::new(&serial.0, schema(), &INDEXED, 1, STRINGS, tables_memory()).unwrap(),
        Cursor::new(&whole.0, schema(), &INDEXED, 1, STRINGS, tables_memory()).unwrap(),
    ];
    let mut random = Random(3);
    for _ in 0..300 {
        let number = random.below(runs.len() as u64) as u32;
        let ordinal = random.below(runs[number as usize].reads.len() as u64) as u32;
        cursors[0]
            .seek(&serial.0, &serial.1, number, ordinal, &|| {}, &|| false)
            .unwrap();
        cursors[1]
            .seek(&whole.0, &whole.1, number, ordinal, &|| {}, &|| false)
            .unwrap();
        holds(&cursors[0], &runs[number as usize], ordinal);
        holds(&cursors[1], &runs[number as usize], ordinal);
    }
}

/// Seeks read at most one page of each strip, and what they read does not grow with the
/// recording: a run four times as long reads no more pages, nor bytes, on any seek.
#[test]
fn a_seek_reads_one_page_per_strip_whatever_the_recording_s_length() {
    let mut most = Vec::new();
    for ordinals in [240, 960] {
        let runs = churned(&[(1, 100, ordinals)]);
        let (index, memory) = segmented(NARROW, &runs);
        let mut cursor =
            Cursor::new(&index, schema(), &INDEXED, 1, STRINGS, tables_memory()).unwrap();
        let mut random = Random(11);
        let (mut pages, mut bytes) = (0, 0);
        for _ in 0..400 {
            let ordinal = random.below(u64::from(ordinals)) as u32;
            cursor
                .seek(&index, &memory, 0, ordinal, &|| {}, &|| false)
                .unwrap();
            let strips: usize = INDEXED
                .iter()
                .enumerate()
                .map(|(at, &table)| {
                    let held = &runs[0].held[ordinal as usize][at];
                    let views = usize::from(held.view.is_some());
                    (schema()[table].len() + 1) * NARROW.blocks(held.table.slots()) + views
                })
                .sum();
            let (read, raw) = cursor.read();
            assert!(read <= strips, "{read} pages of {strips} strips");
            pages = pages.max(read);
            bytes = bytes.max(raw);
        }
        most.push((pages, bytes));
    }
    assert!(most[1].0 <= most[0].0, "{most:?}");
    assert!(most[1].1 <= most[0].1 * 5 / 4, "{most:?}");
}

/// Rows looked up by key at an hour are the walk's rows there: only live ones, only those a
/// truncated table's view holds, each cell as held and recorded or not, with the hour's
/// status and sample hour.
#[test]
fn rows_looked_up_by_key_are_the_walk_s() {
    let runs = churned(&RUNS);
    let (index, memory) = segmented(NARROW, &runs);
    let mut random = Random(5);
    for _ in 0..300 {
        let number = random.below(runs.len() as u64) as u32;
        let run = &runs[number as usize];
        let ordinal = random.below(run.reads.len() as u64) as u32;
        let at = random.below(INDEXED.len() as u64) as usize;
        let table = INDEXED[at];
        let expected = &run.held[ordinal as usize][at];
        let mut keys: Vec<u128> = expected
            .table
            .live()
            .ones()
            .filter(|_| random.below(3) == 0)
            .filter_map(|slot| expected.table.key(slot))
            .collect();
        keys.extend([u128::MAX, 0, 1_003, u128::from(13_u32) << 64 | 39]);
        let keyed = rows_at(&index, &memory, number, ordinal, table, &keys).unwrap();
        assert_eq!(keyed.status, expected.status);
        assert_eq!(
            keyed.sampled,
            match expected.status {
                Status::Complete => expected.sampled,
                _ => None,
            }
        );
        let mut found: Vec<(u128, u32)> = keys
            .iter()
            .filter_map(|&key| {
                let slot = expected.table.slot(key)?;
                let viewed = expected
                    .view
                    .as_ref()
                    .is_none_or(|view| view.binary_search(&slot).is_ok());
                viewed.then_some((key, slot))
            })
            .collect();
        found.sort_unstable();
        found.dedup();
        assert_eq!(keyed.table.live().count(), found.len());
        for (dense, &(key, slot)) in found.iter().enumerate() {
            let dense = dense as u32;
            assert_eq!(keyed.table.key(dense), Some(key));
            for column in 0..schema()[table].len() {
                assert_eq!(
                    keyed.table.column(column).unwrap().cell(dense),
                    expected.table.column(column).unwrap().cell(slot),
                    "key {key:#x} column {column}"
                );
                assert_eq!(
                    keyed.table.recorded(column, dense),
                    expected.table.recorded(column, slot)
                );
            }
        }
    }
}

const MEBIBYTE: usize = 1 << 20;

/// An index of 64 rows in one block whose opaque cells, empty at the first hour, all take one
/// mebibyte at the second by a fill: what that expands to held, 64 MiB.
fn filled() -> (Index, Memory) {
    let runs = [churn(1, 0, 3, |_| (1..=64).map(plain).collect())];
    let (mut index, mut memory) = segmented(BROAD, &runs);
    let mut base = Writer::new();
    let mut held = cell::Held::default();
    let table = &runs[0].held[0][0].table;
    cell::put_base(&mut base, table, 4, Kind::Opaque, 0, 64, &mut held).unwrap();
    let (mut head, mut values) = (Writer::new(), Writer::new());
    let mut group = cell::Group::default();
    group.start();
    // No later change is written against it: the writer keeps none of the fill's cells.
    let big = vec![7_u8; MEBIBYTE];
    group
        .fill(&mut values, &[], Cell::Opaque(&big), 0, &mut held)
        .unwrap();
    group.close(&mut head, 0, 1).unwrap();
    let bytes = cell::assemble(&base, &head, &values, Vec::new())
        .unwrap()
        .into_bytes();
    let location = Location {
        offset: memory.0.len() as u64,
        stored: bytes.len() as u32,
        raw: bytes.len() as u32,
    };
    memory.0.extend_from_slice(&bytes);
    index.runs[0].strips[0].columns[4][0] = vec![Page {
        first: 0,
        last: 1,
        location,
    }];
    (index, memory)
}

/// Whether every opaque cell of the block `cursor` holds is the fill's mebibyte.
fn holds_the_fill(cursor: &Cursor) -> bool {
    let big = vec![7_u8; MEBIBYTE];
    let opaque = cursor.state().table(0).unwrap().column(4).unwrap();
    (0..64).all(|slot| opaque.cell(slot) == Some(Cell::Opaque(&big)))
}

/// A page's cells are admitted to their cursor's memory before they are held: a fill of a
/// mebibyte into every cell of a block is refused by name, holding nothing after, by a budget
/// short of what it expands to, before a cell of it is set; and is held within one that is not,
/// as the page a strip keeps for its changes is, until the cells shrink and give it back.
#[test]
fn a_fill_s_cells_are_admitted_before_they_are_held() {
    let (index, memory) = filled();
    let short = MemoryBudget::new(16 * MEBIBYTE);
    let mut cursor = Cursor::new(&index, schema(), &INDEXED, 2, STRINGS, short.clone()).unwrap();
    cursor
        .seek(&index, &memory, 0, 0, &|| {}, &|| false)
        .unwrap();
    // The strip keeps its page for the fill still to come.
    assert!(short.used() >= MEBIBYTE, "{}", short.used());
    let error = cursor
        .seek(&index, &memory, 0, 1, &|| {}, &|| false)
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
    assert!(beyond_memory(&error), "{error}");
    assert_eq!(short.used(), 0);
    let ample = MemoryBudget::new(256 * MEBIBYTE);
    let mut cursor = Cursor::new(&index, schema(), &INDEXED, 2, STRINGS, ample.clone()).unwrap();
    cursor
        .seek(&index, &memory, 0, 1, &|| {}, &|| false)
        .unwrap();
    assert!(holds_the_fill(&cursor));
    assert!(ample.used() >= 64 * MEBIBYTE, "{}", ample.used());
    cursor
        .seek(&index, &memory, 0, 0, &|| {}, &|| false)
        .unwrap();
    assert!(ample.used() < 8 * MEBIBYTE, "{}", ample.used());
}

/// A move its memory is short for while another holder holds it waits for it, holding nothing
/// meanwhile, and goes on once that is released; asked to stop while it waits, it stops,
/// holding nothing.
#[test]
fn a_move_short_of_memory_waits_for_it_holding_nothing() {
    let (index, memory) = filled();
    let budget = MemoryBudget::new(96 * MEBIBYTE);
    let other = budget.admit(64 * MEBIBYTE).unwrap();
    let mut cursor = Cursor::new(&index, schema(), &INDEXED, 1, STRINGS, budget.clone()).unwrap();
    let stopped = cursor
        .seek(&index, &memory, 0, 1, &|| {}, &|| true)
        .unwrap_err();
    assert_eq!(stopped.kind(), io::ErrorKind::Interrupted);
    assert_eq!(budget.used(), 64 * MEBIBYTE);
    let (waiting, waited) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        let (cursor, index, memory) = (&mut cursor, &index, &memory);
        let mover = scope.spawn(move || {
            cursor.seek(index, memory, 0, 1, &|| {}, &|| {
                let _ = waiting.send(());
                false
            })
        });
        waited.recv().unwrap();
        assert_eq!(budget.used(), 64 * MEBIBYTE);
        drop(other);
        mover.join().unwrap().unwrap();
    });
    assert!(holds_the_fill(&cursor));
    assert!(budget.used() >= 64 * MEBIBYTE, "{}", budget.used());
}

#[test]
fn a_share_gives_back_what_it_drops_and_needs_only_what_it_was_refused() {
    let ledger = Arc::new(cell::Ledger::new(MemoryBudget::new(1 << 20)));
    let mut first = cell::Share::of(&ledger).unwrap();
    let mut second = cell::Share::of(&ledger).unwrap();
    ledger.begin();
    first.take(768 << 10).unwrap();
    first.give(640 << 10);
    // What the first dropped went back, so the second's fill fits beside what it still holds.
    second.take(768 << 10).unwrap();
    let error = second.take(512 << 10).unwrap_err();
    assert!(cell::short_of(&error));
    // Needed at once: what both held when the second was refused, and what it asked for.
    assert_eq!(ledger.peak(), (128 << 10) + (768 << 10) + (512 << 10));
}

#[test]
fn a_retry_recycles_every_byte_a_share_drops_into_what_it_waited_for() {
    let ledger = Arc::new(cell::Ledger::new(MemoryBudget::new(1 << 20)));
    let mut first = cell::Share::of(&ledger).unwrap();
    let mut second = cell::Share::of(&ledger).unwrap();
    ledger.begin();
    // The retry waited for exactly what the move needed at once.
    *ledger.spare.lock().unwrap() = Some(ledger.budget.admit(1 << 20).unwrap());
    first.take(10 << 10).unwrap();
    first.give(9 << 10);
    // What the first dropped is the second's: the whole move fits what was waited for.
    second.take(1023 << 10).unwrap();
    assert_eq!(ledger.peak(), 1 << 20);
}

#[test]
fn a_move_needs_the_most_its_workers_held_and_asked_for_at_once() {
    let ledger = Arc::new(cell::Ledger::new(MemoryBudget::new(1 << 20)));
    let mut first = cell::Share::of(&ledger).unwrap();
    let mut second = cell::Share::of(&ledger).unwrap();
    ledger.begin();
    *ledger.spare.lock().unwrap() = Some(ledger.budget.admit(640 << 10).unwrap());
    first.take(768 << 10).unwrap();
    let error = second.take(512 << 10).unwrap_err();
    assert!(cell::short_of(&error));
    // The first worker unwinding before the move gives up does not shrink what it needed.
    first.give(640 << 10);
    assert_eq!(ledger.peak(), (768 << 10) + (512 << 10));
}

#[test]
fn a_dropped_share_leaves_the_ledger_with_what_it_held() {
    let ledger = Arc::new(cell::Ledger::new(MemoryBudget::new(1 << 20)));
    let mut kept = cell::Share::of(&ledger).unwrap();
    let mut dropped = cell::Share::of(&ledger).unwrap();
    dropped.take(256 << 10).unwrap();
    drop(dropped);
    ledger.begin();
    kept.take(900 << 10).unwrap();
    // A move after a worker is dropped needs only what the shares still there hold.
    assert_eq!(ledger.peak(), 900 << 10);
}

#[test]
fn in_a_retry_a_share_takes_exactly_what_it_holds() {
    let ledger = Arc::new(cell::Ledger::new(MemoryBudget::new(1 << 20)));
    let mut share = cell::Share::of(&ledger).unwrap();
    ledger.begin();
    *ledger.spare.lock().unwrap() = Some(ledger.budget.admit(4 << 10).unwrap());
    share.take(4 << 10).unwrap();
    share.take(10).unwrap();
    // No grain past what it holds: a retry's credit is its demand.
    assert_eq!(ledger.budget.used(), (4 << 10) + 10);
    assert_eq!(ledger.peak(), (4 << 10) + 10);
}
