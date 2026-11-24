use super::Lane;
use crate::{
    changes::{Changes, Raw},
    pages::{Pages, Run},
};
use replay_columnar::{
    manifest::Status,
    state::{Cell, Kind},
    store::{Store, TableChanges, TableState, Value},
    unit::{BLOCK, BlockPage, Encoder, PartSink, Settings, reader::Extents},
};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::Arc,
};

pub(crate) const KINDS: [Kind; 4] = [Kind::I32, Kind::OptionalI64, Kind::U32List, Kind::Bytes];

#[derive(Default)]
pub(crate) struct Memory {
    pub(crate) bytes: Vec<u8>,
    pub(crate) pages: Vec<BlockPage>,
}

impl PartSink for Memory {
    fn frame(&mut self, stored: &[u8]) -> io::Result<u64> {
        let offset = self.bytes.len() as u64;
        self.bytes.extend_from_slice(stored);
        Ok(offset)
    }

    fn block(&mut self, page: BlockPage) -> io::Result<()> {
        self.pages.push(page);
        Ok(())
    }

    fn read_back(&mut self, offset: u64, stored: u32, out: &mut Vec<u8>) -> io::Result<()> {
        out.clear();
        out.extend_from_slice(&self.bytes[offset as usize..offset as usize + stored as usize]);
        Ok(())
    }
}

pub(crate) struct Recording {
    pub(crate) bytes: Vec<u8>,
    pub(crate) runs: Vec<Run>,
    pub(crate) kinds: Vec<Vec<Kind>>,
}

impl Pages for Recording {
    fn extents(&self, _run: usize) -> &dyn Extents {
        &self.bytes
    }

    fn runs(&self) -> &[Run] {
        &self.runs
    }

    fn kinds(&self, table: usize) -> Option<&[Kind]> {
        self.kinds.get(table).map(Vec::as_slice)
    }
}

pub(crate) fn record(settings: Settings, blocks: u32, hour: impl FnMut(u32, &mut TableChanges<'_>)) -> (Arc<Recording>, Vec<TableState>) {
    record_hours(settings, blocks * BLOCK, hour)
}

pub(crate) fn record_hours(settings: Settings, hours: u32, mut hour: impl FnMut(u32, &mut TableChanges<'_>)) -> (Arc<Recording>, Vec<TableState>) {
    let (bytes, run, mut states) = record_run(settings, 1, 0, hours, |at, _, changes| hour(at, changes));
    (Arc::new(Recording { bytes, runs: vec![run], kinds: vec![KINDS.to_vec()] }), states.remove(0))
}

pub(crate) fn record_run(settings: Settings, tables: usize, first: i32, hours: u32, mut hour: impl FnMut(u32, usize, &mut TableChanges<'_>)) -> (Vec<u8>, Run, Vec<Vec<TableState>>) {
    let kinds = vec![KINDS.to_vec(); tables];
    let mut store = Store::new(kinds.clone(), vec![false; tables]);
    store.attach(Encoder::new(&kinds, settings).unwrap());
    let mut memory = Memory::default();
    let mut states = vec![Vec::new(); tables];
    store.begin_run(first);
    for at in 0..hours {
        let text = if first == 0 { format!("text {at}") } else { format!("text {first}/{at}") };
        store.begin_hour(first + at as i32, &[text.as_bytes()]).unwrap();
        for table in 0..tables {
            store.status(table, Status::Complete, true, Some(first + at as i32), true).unwrap();
            hour(at, table, &mut store.table(table).unwrap());
        }
        store.end_hour(&mut memory).unwrap();
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
        }
        for (table, held) in states.iter_mut().enumerate() {
            held.push(store.table_state(table).unwrap().clone());
        }
    }
    if !hours.is_multiple_of(BLOCK) {
        store.end_run(&mut memory).unwrap();
    }
    (memory.bytes, Run::new(first, memory.pages).unwrap(), states)
}

pub(crate) fn rows(table: &TableState) -> BTreeMap<u128, Vec<String>> {
    table.rows().map(|slot| (table.key(slot).unwrap(), (0..KINDS.len()).map(|column| format!("{:?}", table.value(column, slot))).collect())).collect()
}

fn give_all(lane: &mut Lane) {
    lane.fetch_through(i32::MAX).unwrap();
}

fn lane(recording: &Arc<Recording>) -> Lane {
    Lane::new(Arc::clone(recording) as Arc<dyn Pages>, 0, 0, &all(), true).unwrap()
}

pub(crate) fn all() -> Vec<usize> {
    (0..KINDS.len()).collect()
}

fn step_through(recording: &Arc<Recording>, states: &[TableState], step: u32, mut each: impl FnMut(&Lane, &Changes, &TableState)) {
    let mut lane = lane(recording);
    lane.seek(0).unwrap();
    give_all(&mut lane);
    let mut changes = Changes::default();
    let mut to = step - 1;
    while (to as usize) < states.len() {
        let head = lane.step(to as i32, &mut changes).unwrap();
        each(&lane, &changes, &states[head as usize]);
        if head as u32 == to {
            to += step;
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Owned {
    Unrecorded,
    Missing,
    Number(u64),
    Text(u32),
    List(Vec<u8>),
}

impl Owned {
    pub(crate) fn value(&self) -> Value<'_> {
        match self {
            Self::Unrecorded => Value::Unrecorded,
            Self::Missing => Value::Missing,
            Self::Number(bits) => Value::Number(*bits),
            Self::Text(id) => Value::Text(*id),
            Self::List(bytes) => Value::List(bytes),
        }
    }
}

pub(crate) fn owned(value: Value<'_>) -> Owned {
    match value {
        Value::Unrecorded => Owned::Unrecorded,
        Value::Missing => Owned::Missing,
        Value::Number(bits) => Owned::Number(bits),
        Value::Text(id) => Owned::Text(id),
        Value::List(bytes) => Owned::List(bytes.to_vec()),
    }
}

#[derive(Default)]
pub(crate) struct Mirror {
    pub(crate) rows: BTreeMap<u32, (u128, Vec<Owned>)>,
    pub(crate) texts: HashMap<u32, Vec<u8>>,
    pub(crate) status: Option<(Status, Option<i32>)>,
}

impl Mirror {
    pub(crate) fn rebuilt(lane: &Lane) -> Self {
        let cells = lane.cells();
        let rows = cells.rows().map(|slot| (slot, (cells.key(slot).expect("keyed"), lane.reads().iter().map(|&column| owned(cells.value(column, slot))).collect()))).collect();
        let texts = lane.texts().iter().map(|(id, text)| (id, text.to_vec())).collect();
        Self { rows, texts, status: Some(lane.status()) }
    }

    pub(crate) fn apply(&mut self, changes: &Changes) {
        let width = changes.columns().len();
        let deleted = changes.deleted();
        for (at, &slot) in deleted.slots().iter().enumerate() {
            assert_eq!(self.rows.remove(&slot).map(|(held, _)| held), Some(deleted.keys().expect("keyed")[at]), "a delete names the row held");
        }
        let inserted = changes.inserted();
        for (at, &slot) in inserted.slots().iter().enumerate() {
            assert!(self.rows.insert(slot, (inserted.keys().expect("keyed")[at], vec![Owned::Unrecorded; width])).is_none(), "an insert names a free slot");
        }
        for (at, column) in changes.columns().iter().enumerate() {
            for (slot, value) in column.values() {
                self.rows.get_mut(&slot).expect("a written slot holds a row").1[at] = owned(value);
            }
        }
        if let Some(old) = changes.renumber() {
            let mut moved = BTreeMap::new();
            for (new, &from) in old.iter().enumerate() {
                moved.insert(new as u32, self.rows.remove(&from).expect("a live row renumbered"));
            }
            assert!(self.rows.is_empty(), "every live row is renumbered");
            self.rows = moved;
        }
        self.texts.extend(changes.texts().iter().map(|(id, text)| (id, text.to_vec())));
        if let Some(status) = changes.status() {
            self.status = Some(status);
        }
    }

    pub(crate) fn assert_holds(&self, lane: &Lane) {
        let cells = lane.cells();
        let held: BTreeMap<u32, (u128, Vec<Owned>)> =
            cells.rows().map(|slot| (slot, (cells.key(slot).unwrap(), lane.reads().iter().map(|&column| owned(cells.value(column, slot))).collect()))).collect();
        assert_eq!(self.rows, held, "hour {:?}", lane.head());
        for (_, cells) in self.rows.values() {
            for cell in cells {
                if let Owned::Text(id) = cell {
                    assert!(self.texts.contains_key(id), "text {id} is named before a cell holds it");
                }
            }
        }
        assert_eq!(self.status, Some(lane.status()));
    }
}

fn assert_each_once(changes: &Changes) {
    for column in changes.columns() {
        let mut slots = column.slots.clone();
        slots.sort_unstable();
        slots.dedup();
        assert_eq!(slots.len(), column.slots.len(), "column {} names a slot twice", column.column);
    }
}

pub(crate) fn busy(at: u32, changes: &mut TableChanges<'_>) {
    if at == 0 {
        for slot in 0..8 {
            changes.insert_at(slot, 100 + u128::from(slot)).unwrap();
            changes.write(slot, 0, Cell::I32(0)).unwrap();
        }
    }
    for slot in 0..8 {
        changes.write(slot, 0, Cell::I32((at * 7 + slot) as i32)).unwrap();
    }
}

#[test]
fn a_step_crossing_a_part_boundary_names_each_slot_once() {
    let (recording, states) = record(Settings { cap: 64, ..Settings::at(3) }, 1, busy);
    assert!(recording.runs[0].pages[0].tables[0].parts.len() > 10);
    let mut steps = 0;
    step_through(&recording, &states, 24, |lane, changes, state| {
        assert_each_once(changes);
        assert_eq!(changes.columns()[0].len(), 8);
        assert_eq!(rows(lane.cells()), rows(state));
        steps += 1;
    });
    assert_eq!(steps, 30);
}

#[test]
fn short_lives_leave_no_trace_and_a_reused_slot_reports_its_new_row() {
    let (recording, states) = record(Settings::at(3), 1, |at, changes| match at {
        0 => {
            changes.insert_at(3, 30).unwrap();
            changes.write(3, 0, Cell::I32(1)).unwrap();
        }
        5 => {
            changes.insert_at(9, 99).unwrap();
            changes.write(9, 0, Cell::I32(9)).unwrap();
        }
        6 => changes.write(9, 0, Cell::I32(10)).unwrap(),
        7 => changes.delete(9).unwrap(),
        10 => changes.delete(3).unwrap(),
        11 => {
            changes.insert_at(3, 31).unwrap();
            changes.write(3, 0, Cell::I32(50)).unwrap();
            changes.write(3, 2, Cell::U32List(&[4, 5])).unwrap();
        }
        _ => {}
    });
    let mut lane = lane(&recording);
    lane.seek(0).unwrap();
    give_all(&mut lane);
    let mut changes = Changes::default();
    assert_eq!(lane.step(23, &mut changes).unwrap(), 23);
    assert_eq!((changes.deleted().hours(), changes.deleted().slots(), changes.deleted().keys()), (&[10][..], &[3][..], Some(&[30][..])));
    assert_eq!((changes.inserted().hours(), changes.inserted().slots(), changes.inserted().keys()), (&[11][..], &[3][..], Some(&[31][..])));
    assert_each_once(&changes);
    for column in changes.columns() {
        assert_eq!(column.slots(), &[3], "column {}", column.column());
    }
    assert_eq!(changes.columns()[0].value(0), Value::Number(50));
    assert_eq!(changes.columns()[2].value(0), Value::List(&[4, 0, 0, 0, 5, 0, 0, 0]));
    assert_eq!(lane.cells().value(2, 3), Value::List(&[4, 0, 0, 0, 5, 0, 0, 0]));
    assert_eq!(rows(lane.cells()), rows(&states[23]));
}

#[test]
fn a_packed_block_end_renumbers_from_the_live_bits() {
    let settings = Settings { restore_blocks: 1, whole_every: 1, pack_margin: 8, ..Settings::at(3) };
    let (recording, states) = record(settings, 2, |at, changes| {
        if at < 100 {
            changes.insert_at(at, u128::from(at) + 1000).unwrap();
            changes.write(at, 0, Cell::I32(at as i32)).unwrap();
        } else if at < 190 {
            changes.delete(at - 100).unwrap();
        } else if (BLOCK + 5..BLOCK + 50).contains(&at) {
            for slot in changes.held().rows().collect::<Vec<_>>() {
                changes.write(slot, 0, Cell::I32(at as i32)).unwrap();
            }
        }
    });
    assert!(recording.runs[0].pages[0].tables[0].packed);
    let mut lane = lane(&recording);
    let mut changes = Changes::default();
    lane.seek(0).unwrap();
    let mut mirror = Mirror::rebuilt(&lane);
    lane.clear_texts();
    give_all(&mut lane);
    let mut renumbered = 0;
    let mut to = 99;
    while to < 2 * BLOCK {
        let head = lane.step(to as i32, &mut changes).unwrap();
        mirror.apply(&changes);
        mirror.assert_holds(&lane);
        assert_eq!(rows(lane.cells()), rows(&states[head as usize]));
        if let Some(old) = changes.renumber() {
            renumbered += 1;
            assert_eq!(head, BLOCK as i32 - 1);
            assert_eq!(old, (90..100).collect::<Vec<u32>>().as_slice());
            assert_eq!(lane.cells().rows().collect::<Vec<_>>(), (0..10).collect::<Vec<u32>>());
            assert_eq!((0..10).map(|slot| lane.cells().key(slot).unwrap()).collect::<Vec<_>>(), (1090..1100).collect::<Vec<u128>>());
        }
        if head as u32 == to {
            to += 100;
        }
    }
    assert_eq!(renumbered, 1);
}

#[test]
fn a_keyed_lanes_view_finds_rows_by_key_and_names_their_strings() {
    let settings = Settings { restore_blocks: 1, whole_every: 1, pack_margin: 8, ..Settings::at(3) };
    let (recording, states) = record(settings, 2, |at, changes| {
        if at < 100 {
            changes.insert_at(at, u128::from(at) + 1000).unwrap();
            changes.write(at, 3, Cell::Bytes(at)).unwrap();
        } else if at < 190 {
            changes.delete(at - 100).unwrap();
        }
    });
    assert!(recording.runs[0].pages[0].tables[0].packed);
    let read = |lane: &Lane, head: usize| {
        let view = lane.view();
        assert!(lane.cells().bytes().index > 0, "hour {head}: the key index is kept");
        assert!(states[head].rows().next().is_some());
        for slot in states[head].rows() {
            let key = states[head].key(slot).unwrap();
            let at = view.slot(key).unwrap_or_else(|| panic!("hour {head}: row {key} found by its key"));
            let Value::Text(id) = view.value(3, at) else { panic!("hour {head}: row {key}'s text") };
            assert_eq!(view.string(id), Some(format!("text {id}").as_bytes()), "hour {head}: row {key}");
        }
    };
    for hour in [50, 99, 150, BLOCK - 1, BLOCK + 10] {
        let mut sought = lane(&recording);
        sought.seek(hour as i32).unwrap();
        read(&sought, hour as usize);
    }
    let mut stepped = lane(&recording);
    stepped.seek(0).unwrap();
    give_all(&mut stepped);
    let mut changes = Changes::default();
    let mut to = 99;
    while to < 2 * BLOCK {
        let head = stepped.step(to as i32, &mut changes).unwrap();
        if states[head as usize].rows().next().is_some() { read(&stepped, head as usize); }
        if head as u32 == to {
            to += 100;
        }
    }
}

#[test]
fn a_mirror_kept_from_change_sets_equals_the_cells() {
    let (recording, states) = record(Settings { cap: 256, ..Settings::at(3) }, 2, |at, changes| {
        if at % 30 == 0 {
            let slot = changes.insert(u128::from(at) + 7).unwrap();
            changes.write(slot, 0, Cell::I32(at as i32)).unwrap();
            changes.write(slot, 1, Cell::OptionalI64(Some(5))).unwrap();
            changes.write(slot, 2, Cell::U32List(&[1, 2, 3])).unwrap();
            changes.write(slot, 3, Cell::Bytes(at)).unwrap();
        }
        let live: Vec<u32> = changes.held().rows().collect();
        for (index, &slot) in live.iter().enumerate() {
            let seed = at as usize * 13 + index * 5;
            if seed.is_multiple_of(3) {
                changes.write(slot, 0, Cell::I32((seed % 101) as i32 - 50)).unwrap();
            }
            match seed % 17 {
                0 => changes.write(slot, 1, Cell::OptionalI64(None)).unwrap(),
                1 => changes.write(slot, 1, Cell::OptionalI64(Some(seed as i64))).unwrap(),
                2 => changes.unrecord(slot, 0).unwrap(),
                3 => changes.write(slot, 2, Cell::U32List(&[seed as u32, 9][..1 + seed % 2])).unwrap(),
                4 => changes.write(slot, 3, Cell::Bytes(at)).unwrap(),
                5 => changes.unrecord(slot, 2).unwrap(),
                _ => {}
            }
        }
        if at % 45 == 44 && live.len() > 2 {
            changes.delete(live[1]).unwrap();
        }
    });
    for step in [24_u32, 1, 37] {
        let mut lane = lane(&recording);
        let mut changes = Changes::default();
        lane.seek(0).unwrap();
        let mut mirror = Mirror::rebuilt(&lane);
        lane.clear_texts();
        mirror.assert_holds(&lane);
        give_all(&mut lane);
        let mut to = step - 1;
        while (to as usize) < states.len() {
            let head = lane.step(to as i32, &mut changes).unwrap();
            mirror.apply(&changes);
            mirror.assert_holds(&lane);
            assert_each_once(&changes);
            assert_eq!(rows(lane.cells()), rows(&states[head as usize]), "step {step}");
            to += step;
        }
    }
}

#[test]
fn a_step_waits_at_a_missing_part_and_goes_on_when_given_it() {
    let (recording, states) = record(Settings { cap: 64, ..Settings::at(3) }, 1, busy);
    let parts = &recording.runs[0].pages[0].tables[0].parts;
    let mut lane = lane(&recording);
    lane.seek(0).unwrap();
    let mut changes = Changes::default();
    let first_end = i32::from(parts[1].first) - 1;
    assert_eq!(lane.step(719, &mut changes).unwrap(), first_end);
    assert!(changes.stalled());
    assert_eq!(changes.short(), (719 - first_end) as u32);
    assert_eq!(rows(lane.cells()), rows(&states[first_end as usize]));
    assert_eq!(lane.step(719, &mut changes).unwrap(), first_end, "nothing more without the next part");
    assert!(changes.columns().iter().all(|column| column.is_empty()));
    give_all(&mut lane);
    assert_eq!(lane.step(719, &mut changes).unwrap(), 719);
    assert!(!changes.stalled());
    assert_eq!(rows(lane.cells()), rows(&states[719]));
}

#[test]
fn a_lane_reads_ahead_by_hours_and_lets_passed_parts_go() {
    let (recording, _) = record(Settings { cap: 64, ..Settings::at(3) }, 1, busy);
    let parts = &recording.runs[0].pages[0].tables[0].parts;
    let mut lane = lane(&recording);
    lane.seek(0).unwrap();
    assert_eq!(lane.held().parts, 1, "the seek reads the part under the head and nothing more");
    let through = i32::from(parts[3].first);
    let (_, read) = lane.fetch_through(through).unwrap();
    assert_eq!(read, 3, "the parts starting by the hour asked");
    assert!(lane.want(through).is_none());
    let mut changes = Changes::default();
    lane.step(through, &mut changes).unwrap();
    let mut retired = Vec::new();
    lane.retire_into(&mut retired);
    assert_eq!(retired.len(), 3, "the parts passed leave with their cursors");
    assert_eq!(lane.held().parts, 1);
    assert!(lane.want(through).is_some(), "at least the part after the last given");
}

#[test]
fn status_is_reported_only_when_it_changes() {
    let mut store = Store::new(vec![KINDS.to_vec()], vec![false]);
    store.attach(Encoder::new(&[KINDS.to_vec()], Settings::at(3)).unwrap());
    let mut memory = Memory::default();
    let mut held = Vec::new();
    store.begin_run(0);
    for at in 0..BLOCK {
        store.begin_hour(at as i32, &[]).unwrap();
        let (status, sampled) = match at {
            100..150 | 201..203 => (Status::NotRecorded, None),
            400..500 => (Status::Complete, Some(at as i32)),
            _ => (Status::Complete, Some((at - at % 48) as i32)),
        };
        store.status(0, status, true, sampled, true).unwrap();
        store.end_hour(&mut memory).unwrap();
        held.push((status, sampled));
    }
    store.end_block();
    let recording = Arc::new(Recording { bytes: memory.bytes, runs: vec![Run::new(0, memory.pages).unwrap()], kinds: vec![KINDS.to_vec()] });
    let mut lane = lane(&recording);
    let mut changes = Changes::default();
    lane.seek(0).unwrap();
    lane.reset_into(&mut changes).unwrap();
    assert_eq!(changes.status(), Some(held[0]));
    assert!(changes.reset() && changes.rows() == 0 && changes.cells() == 0 && changes.texts().is_empty(), "a reset names no row");
    give_all(&mut lane);
    let mut previous = held[0];
    let mut reported = 0;
    let mut from = 1;
    for to in (5..BLOCK).step_by(6) {
        lane.step(to as i32, &mut changes).unwrap();
        let now = held[to as usize];
        assert_eq!(lane.status(), now, "hour {to}");
        assert_eq!(changes.status(), (now != previous).then_some(now), "hour {to}");
        let left = (from..=to).find(|&hour| held[hour as usize].0 != previous.0).map(|hour| hour as i32);
        assert_eq!(changes.status_left(), left, "hour {to}");
        reported += u32::from(now != previous);
        previous = now;
        from = to + 1;
    }
    assert!(reported > 20);
}

#[test]
fn a_lane_holds_the_rows_an_hour_read_in_part() {
    let kinds = vec![KINDS.to_vec()];
    let mut store = Store::new(kinds.clone(), vec![false]);
    store.attach(Encoder::new(&kinds, Settings { cap: 64, restore_blocks: 1, ..Settings::at(3) }).unwrap());
    let mut memory = Memory::default();
    let mut states = Vec::new();
    let hours = 2 * BLOCK + 40;
    store.begin_run(0);
    for at in 0..hours {
        store.begin_hour(at as i32, &[]).unwrap();
        let partly = at % 5 == 2 || at % BLOCK == BLOCK - 1;
        store.status(0, if partly { Status::Truncated { missing: 3 } } else { Status::Complete }, true, Some(at as i32), true).unwrap();
        let mut changes = store.table(0).unwrap();
        let read: Vec<u32> = changes.held().rows().filter(|slot| !partly || (slot + at) % 3 == 0).collect();
        if partly {
            changes.read_rows(&read).unwrap();
        }
        if at == 0 {
            for slot in 0..8 {
                changes.insert_at(slot, 100 + u128::from(slot)).unwrap();
                changes.write(slot, 0, Cell::I32(0)).unwrap();
            }
        }
        for &slot in read.iter().filter(|&&slot| slot < 8) {
            changes.write(slot, 0, Cell::I32((at * 7 + slot) as i32)).unwrap();
        }
        if at % 10 == 2 {
            changes.insert_at(8, 900 + u128::from(at)).unwrap();
            changes.write(8, 0, Cell::I32(at as i32)).unwrap();
        } else if at % 10 == 6 && changes.held().rows().any(|slot| slot == 8) {
            changes.delete(8).unwrap();
        }
        if partly {
            changes.read_done();
        }
        store.end_hour(&mut memory).unwrap();
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
        }
        states.push(store.table_state(0).unwrap().clone());
    }
    store.end_run(&mut memory).unwrap();
    let recording = Arc::new(Recording { bytes: memory.bytes, runs: vec![Run::new(0, memory.pages).unwrap()], kinds });
    let read = |hour: usize| matches!(states[hour].status(), Status::Truncated { .. }).then(|| states[hour].view().unwrap_or(&[]).to_vec());
    assert!((0..hours as usize).filter(|&hour| read(hour).is_some_and(|rows| rows.contains(&8))).count() > 5, "hours read in part begin rows");
    for hour in (0..hours as usize).filter(|&hour| hour % 5 == 2 || hour % 5 == 3 || hour as u32 % BLOCK == BLOCK - 1) {
        let mut lane = lane(&recording);
        lane.seek(hour as i32).unwrap();
        assert_eq!(lane.read_rows().map(<[u32]>::to_vec), read(hour), "sought to hour {hour}");
    }
    for step in [1, 7] {
        let mut lane = lane(&recording);
        lane.seek(0).unwrap();
        give_all(&mut lane);
        let mut changes = Changes::default();
        let mut to = 0;
        while to + step < hours {
            to += step;
            let head = lane.step(to as i32, &mut changes).unwrap() as usize;
            assert_eq!(lane.read_rows().map(<[u32]>::to_vec), read(head), "stepped by {step} to hour {head}");
            to = head as u32;
        }
    }
}

#[test]
fn read_rows_changed_signals_swapped_truncated_witnesses_with_zero_complete_compares() {
    let kinds = vec![KINDS.to_vec()];
    let mut store = Store::new(kinds.clone(), vec![false]);
    store.attach(Encoder::new(&kinds, Settings { cap: 64, restore_blocks: 1, ..Settings::at(3) }).unwrap());
    let mut memory = Memory::default();
    store.begin_run(0);
    for at in 0..8_u32 {
        store.begin_hour(at as i32, &[]).unwrap();
        let status = if (3..=5).contains(&at) { Status::Truncated { missing: 1 } } else { Status::Complete };
        store.status(0, status, true, Some(at as i32), true).unwrap();
        let mut changes = store.table(0).unwrap();
        if at == 0 {
            for slot in 0..2 {
                changes.insert_at(slot, 100 + u128::from(slot)).unwrap();
                changes.write(slot, 0, Cell::I32(10 + slot as i32)).unwrap();
            }
        }
        match at {
            3 | 4 => {
                changes.read_rows(&[0]).unwrap();
                changes.read_done();
            }
            5 => {
                changes.read_rows(&[1]).unwrap();
                changes.read_done();
            }
            _ => {}
        }
        store.end_hour(&mut memory).unwrap();
    }
    store.end_run(&mut memory).unwrap();
    let recording = Arc::new(Recording { bytes: memory.bytes, runs: vec![Run::new(0, memory.pages).unwrap()], kinds });
    let mut lane = lane(&recording);
    lane.seek(0).unwrap();
    give_all(&mut lane);
    let mut changes = Changes::default();

    lane.step(1, &mut changes).unwrap();
    lane.step(2, &mut changes).unwrap();
    assert_eq!(lane.read_compares, 0, "complete hours never compare witness sets");
    assert!(!changes.read_rows_changed());

    lane.step(3, &mut changes).unwrap();
    assert!(changes.status().is_some(), "Complete -> Truncated is a status change");
    assert_eq!(lane.read_compares, 0);

    lane.step(4, &mut changes).unwrap();
    assert_eq!(changes.status().map(|(status, _)| status), Some(Status::Truncated { missing: 1 }));
    assert!(!changes.read_rows_changed(), "identical truncated witness sets do not invalidate");
    assert_eq!(lane.read_compares, 1);

    lane.step(5, &mut changes).unwrap();
    assert_eq!(changes.status().map(|(status, _)| status), Some(Status::Truncated { missing: 1 }));
    assert!(changes.read_rows_changed(), "swapped truncated witness sets invalidate downstream queries");
    assert_eq!(lane.read_compares, 2);

    lane.step(6, &mut changes).unwrap();
    lane.step(7, &mut changes).unwrap();
    assert_eq!(lane.read_compares, 2, "returning to complete hours performs no further compares");
}

#[test]
fn a_seek_equals_stepping_to_the_same_hour() {
    let settings = Settings { restore_blocks: 1, ..Settings::at(3) };
    let (recording, states) = record_hours(settings, 2 * BLOCK + 27, busy);
    assert_eq!(recording.runs[0].pages.len(), 3);
    for hour in [0, 1, 300, BLOCK - 1, BLOCK, BLOCK + 400, 2 * BLOCK - 1, 2 * BLOCK, 2 * BLOCK + 26] {
        let mut lane = lane(&recording);
        lane.seek(hour as i32).unwrap();
        assert_eq!(lane.head(), Some(hour as i32));
        assert_eq!(rows(lane.cells()), rows(&states[hour as usize]), "hour {hour}");
        assert_eq!(lane.status(), (states[hour as usize].status(), states[hour as usize].sampled_at()), "hour {hour}");
        let mut changes = Changes::default();
        lane.reset_into(&mut changes).unwrap();
        assert_eq!((changes.reset(), changes.head(), changes.rows(), changes.cells()), (true, Some(hour as i32), 0, 0), "a reset names no row");
        Mirror::rebuilt(&lane).assert_holds(&lane);
    }
}

fn events(at: u32, changes: &mut TableChanges<'_>) {
    if at == 0 {
        for slot in 0..4 {
            changes.insert_at(slot, 10 + u128::from(slot)).unwrap();
            changes.write(slot, 0, Cell::I32(slot as i32)).unwrap();
        }
    }
    if at == 30 {
        changes.delete(2).unwrap();
    }
    let born: Vec<(u32, u32)> = changes.held().rows().filter_map(|slot| changes.held().key(slot).filter(|&key| key >= 1000).map(|key| (slot, (key - 1000) as u32))).collect();
    for &(slot, birth) in &born {
        if at == birth + birth % 5 + 1 {
            changes.delete(slot).unwrap();
        } else if at.is_multiple_of(2) {
            changes.write(slot, 0, Cell::I32(at as i32 * 10)).unwrap();
            if at.is_multiple_of(7) {
                changes.unrecord(slot, 1).unwrap();
            }
        }
    }
    if at.is_multiple_of(3) {
        let slot = changes.insert(1000 + u128::from(at)).unwrap();
        changes.write(slot, 0, Cell::I32(at as i32)).unwrap();
        changes.write(slot, 1, Cell::OptionalI64(at.is_multiple_of(2).then_some(i64::from(at) * 3))).unwrap();
        changes.write(slot, 2, Cell::U32List(&[at, at + 1][..1 + at as usize % 2])).unwrap();
        changes.write(slot, 3, Cell::Bytes(at)).unwrap();
    }
}

type Passed = (i32, i32, u128, Vec<String>);

fn passed(states: &[TableState], from: usize, to: usize) -> Vec<Passed> {
    let before: std::collections::BTreeSet<u128> = if from == 0 { Default::default() } else { rows(&states[from - 1]).into_keys().collect() };
    let after = rows(&states[to]);
    let mut lives: BTreeMap<u128, (usize, usize, Vec<String>)> = BTreeMap::new();
    for (hour, state) in states.iter().enumerate().take(to + 1).skip(from) {
        for (key, cells) in rows(state) {
            if before.contains(&key) || after.contains_key(&key) {
                continue;
            }
            let held = lives.entry(key).or_insert((hour, hour, Vec::new()));
            (held.1, held.2) = (hour, cells);
        }
    }
    let mut out: Vec<Passed> = lives.into_iter().map(|(key, (born, last, cells))| (born as i32, last as i32, key, cells)).collect();
    out.sort();
    out
}

fn passing_of(changes: &Changes) -> Vec<Passed> {
    let passing = changes.passing();
    (0..passing.len()).map(|row| (passing.born()[row], passing.last()[row], passing.keys().expect("keyed")[row], passing.columns().iter().map(|column| format!("{:?}", column.value(row))).collect())).collect()
}

#[test]
fn a_passing_lane_gives_every_row_born_and_gone_in_a_step_with_its_last_cells() {
    let (recording, states) = record(Settings { cap: 64, ..Settings::at(3) }, 2, events);
    assert!(recording.runs[0].pages[0].tables[0].parts.len() > 5);
    for step in [24_u32, 7, 100] {
        let mut lane = lane(&recording);
        lane.set_passing(true);
        lane.seek(0).unwrap();
        give_all(&mut lane);
        let mut changes = Changes::default();
        let (mut to, mut from, mut seen) = (step - 1, 1_usize, 0);
        while (to as usize) < states.len() {
            let head = lane.step(to as i32, &mut changes).unwrap() as usize;
            let mut got = passing_of(&changes);
            assert!(got.windows(2).all(|pair| pair[0].0 <= pair[1].0), "birth order");
            for (_, _, key, _) in &got {
                assert!(!changes.inserted().keys().unwrap().contains(key) && !changes.deleted().keys().unwrap().contains(key), "row {key} passes and is no insert or delete");
            }
            got.sort();
            assert_eq!(got, passed(&states, from, head), "step of {step} hours to {head}");
            assert_eq!(rows(lane.cells()), rows(&states[head]));
            seen += got.len();
            from = head + 1;
            to += step;
        }
        assert!(seen > 100, "{seen} rows passed in steps of {step} hours");
    }
}

#[test]
fn a_slot_reused_inside_a_step_gives_one_passing_row_per_life_and_the_survivor_in_inserted() {
    let born = |changes: &mut TableChanges<'_>, key: u128, value: i32| {
        changes.insert_at(4, key).unwrap();
        changes.write(4, 0, Cell::I32(value)).unwrap();
        changes.write(4, 1, Cell::OptionalI64(None)).unwrap();
        changes.write(4, 2, Cell::U32List(&[value as u32])).unwrap();
        changes.write(4, 3, Cell::Bytes(2)).unwrap();
    };
    let (recording, states) = record(Settings::at(3), 1, |at, changes| match at {
        2 => born(changes, 40, 1),
        5 => {
            changes.delete(4).unwrap();
            born(changes, 41, 2);
        }
        6 => changes.write(4, 0, Cell::I32(3)).unwrap(),
        8 => changes.delete(4).unwrap(),
        10 => born(changes, 42, 4),
        _ => {}
    });
    let mut lane = lane(&recording);
    lane.set_passing(true);
    lane.seek(0).unwrap();
    give_all(&mut lane);
    let mut changes = Changes::default();
    assert_eq!(lane.step(23, &mut changes).unwrap(), 23);
    let got = passing_of(&changes);
    assert_eq!(got.iter().map(|(born, last, key, cells)| (*born, *last, *key, cells[0].clone())).collect::<Vec<_>>(), vec![(2, 4, 40, "Number(1)".to_owned()), (5, 7, 41, "Number(3)".to_owned())]);
    assert_eq!((changes.inserted().slots(), changes.inserted().keys(), changes.inserted().hours()), (&[4][..], Some(&[42][..]), &[10][..]));
    assert!(changes.deleted().is_empty());
    assert_eq!(got[1].3[1..], ["Missing".to_owned(), "List([2, 0, 0, 0])".to_owned(), got[0].3[3].clone()]);
    let Value::Text(id) = changes.passing().columns()[3].value(0) else { panic!("a text cell") };
    assert_eq!(changes.texts().iter().find(|&(held, _)| held == id).map(|(_, text)| text), Some(&b"text 2"[..]), "the text is named in the step");
    let mut sorted = got.clone();
    sorted.sort();
    assert_eq!(passed(&states, 1, 23), sorted);
}

fn seen(changes: &Changes) -> String {
    let columns: Vec<(usize, Vec<(u32, String)>)> = changes.columns().iter().map(|column| (column.column(), column.values().map(|(slot, value)| (slot, format!("{value:?}"))).collect())).collect();
    let lives = |lives: &crate::changes::Lives| (lives.hours().to_vec(), lives.slots().to_vec(), lives.keys().map(<[u128]>::to_vec));
    let texts: Vec<(u32, Vec<u8>)> = changes.texts().iter().map(|(id, text)| (id, text.to_vec())).collect();
    format!("{:?}", (changes.reset(), changes.head(), changes.short(), changes.stalled(), columns, lives(changes.deleted()), lives(changes.inserted()), changes.status(), changes.renumber(), texts))
}

#[test]
fn a_non_passing_lanes_change_sets_are_what_they_were() {
    let (recording, states) = record(Settings { cap: 64, ..Settings::at(3) }, 2, events);
    let (mut plain, mut passing) = (lane(&recording), lane(&recording));
    passing.set_passing(true);
    for lane in [&mut plain, &mut passing] {
        lane.seek(0).unwrap();
        give_all(lane);
    }
    let (mut a, mut b) = (Changes::default(), Changes::default());
    let mut to = 23;
    while (to as usize) < states.len() {
        plain.step(to, &mut a).unwrap();
        passing.step(to, &mut b).unwrap();
        assert!(a.passing().is_empty() && a.passing.columns.is_empty() && a.passing.born.capacity() == 0, "nothing held for passing");
        assert!(!b.passing().is_empty() || to < 30);
        assert_eq!(seen(&a), seen(&b), "to {to}");
        to += 24;
    }
}

#[test]
fn a_reset_and_a_seeks_replay_carry_no_passing_rows() {
    let (recording, states) = record(Settings { cap: 64, ..Settings::at(3) }, 2, events);
    for hour in [100_usize, 719, 1000] {
        let mut lane = lane(&recording);
        lane.set_passing(true);
        lane.seek(hour as i32).unwrap();
        let mut changes = Changes::default();
        lane.reset_into(&mut changes).unwrap();
        assert!(changes.passing().is_empty() && changes.rows() == 0);
        give_all(&mut lane);
        let head = lane.step(hour as i32 + 24, &mut changes).unwrap() as usize;
        let mut got = passing_of(&changes);
        got.sort();
        assert_eq!(got, passed(&states, hour + 1, head), "after a seek to {hour}");
    }
}

fn stored(bytes: &[u8], at: usize, width: u8, signed: bool) -> u64 {
    let width = width as usize;
    if width == 0 {
        return 0;
    }
    let mut held = [0_u8; 8];
    held[..width].copy_from_slice(&bytes[at * width..at * width + width]);
    let bits = u64::from_le_bytes(held);
    if signed && width < 8 {
        let shift = 64 - 8 * width as u32;
        ((bits << shift) as i64 >> shift) as u64
    } else {
        bits
    }
}

fn from_raw(written: &crate::changes::Written) -> Vec<(u32, Owned)> {
    let (missing, unrecorded) = (written.missing(), written.unrecorded());
    written
        .slots()
        .iter()
        .enumerate()
        .map(|(at, &slot)| {
            let index = at as u32;
            let cell = if unrecorded.binary_search(&index).is_ok() {
                Owned::Unrecorded
            } else if missing.binary_search(&index).is_ok() {
                Owned::Missing
            } else {
                match written.raw() {
                    Raw::Numbers { width, signed, bytes } => Owned::Number(stored(bytes, at, width, signed)),
                    Raw::Texts { width, bytes } => Owned::Text(stored(bytes, at, width, false) as u32),
                    Raw::Lists { ends, bytes } => Owned::List(bytes[if at == 0 { 0 } else { ends[at - 1] as usize }..ends[at] as usize].to_vec()),
                }
            };
            (slot, cell)
        })
        .collect()
}

#[test]
fn a_columns_raw_view_gives_the_cells_each_gives() {
    let (recording, _) = record(Settings { cap: 64, ..Settings::at(3) }, 2, |at, changes| {
        if at == 0 {
            for slot in 0..6 {
                changes.insert_at(slot, 10 + u128::from(slot)).unwrap();
            }
        }
        let value = match at {
            0..200 => at as i32 - 100,
            200..400 => (at as i32 - 300) * 300,
            _ => (at as i32 - 900) * 100_000,
        };
        for slot in 0..6 {
            changes.write(slot, 0, Cell::I32(value + slot as i32)).unwrap();
            let optional = match (at + slot) % 5 {
                0 => None,
                _ if at > 900 => Some(i64::from(at) * 10_000_000_000),
                _ => Some(i64::from(at)),
            };
            changes.write(slot, 1, Cell::OptionalI64(optional)).unwrap();
            if (at + slot).is_multiple_of(7) {
                changes.unrecord(slot, 1).unwrap();
            }
            if (at + slot).is_multiple_of(4) {
                changes.write(slot, 2, Cell::U32List(&[at, slot][..1 + slot as usize % 2])).unwrap();
                changes.write(slot, 3, Cell::Bytes(at)).unwrap();
            }
        }
        if at.is_multiple_of(5) {
            let slot = changes.insert(1000 + u128::from(at)).unwrap();
            changes.write(slot, 0, Cell::I32(value)).unwrap();
            changes.write(slot, 1, Cell::OptionalI64(None)).unwrap();
            changes.write(slot, 2, Cell::U32List(&[at])).unwrap();
            changes.write(slot, 3, Cell::Bytes(at)).unwrap();
        }
        if at > 2
            && (at - 2).is_multiple_of(5)
            && let Some(slot) = changes.slot(1000 + u128::from(at - 2))
        {
            changes.delete(slot).unwrap();
        }
    });
    let mut lane = lane(&recording);
    lane.set_passing(true);
    lane.seek(0).unwrap();
    give_all(&mut lane);
    let mut changes = Changes::default();
    let (mut widths, mut missing, mut unrecorded, mut texts, mut lists, mut passing) = (std::collections::BTreeSet::new(), 0, 0, 0, 0, 0);
    let mut to = 23;
    while to < 2 * BLOCK as i32 {
        lane.step(to, &mut changes).unwrap();
        for written in changes.columns().iter().chain(changes.passing().columns()) {
            let mut each = Vec::new();
            written.each(|slot, value| each.push((slot, owned(value))));
            assert_eq!(from_raw(written), each, "column {} to {to}", written.column());
            match written.raw() {
                Raw::Numbers { width, .. } if !written.is_empty() => {
                    widths.insert(width);
                }
                Raw::Texts { .. } => texts += written.len(),
                Raw::Lists { .. } => lists += written.len(),
                _ => {}
            }
            missing += written.missing().len();
            unrecorded += written.unrecorded().len();
        }
        passing += changes.passing().len();
        to += 24;
    }
    assert!([1, 2, 4, 8].iter().all(|width| widths.contains(width)), "widths seen: {widths:?}");
    assert!(missing > 0 && unrecorded > 0 && texts > 0 && lists > 0 && passing > 0);
}

#[test]
fn an_hours_change_sets_fold_to_the_steps_own() {
    let (recording, states) = record(Settings { cap: 256, ..Settings::at(3) }, 2, events);
    for step in [48_u32, 12, 1] {
        for sought in [0_u32, 377] {
            let mut lane = lane(&recording);
            lane.seek(sought as i32).unwrap();
            let mut mirror = Mirror::rebuilt(&lane);
            lane.clear_texts();
            mirror.assert_holds(&lane);
            give_all(&mut lane);
            let (mut changes, mut hours) = (Changes::default(), Vec::new());
            let mut to = sought + step;
            let mut dated = 0;
            while (to as usize) < states.len() {
                hours.clear();
                let before = lane.head().expect("sought") + 1;
                let head = lane.step_hours(to as i32, &mut changes, Some(&mut hours)).unwrap();
                let mut last = before - 1;
                for set in &hours {
                    let hour = set.head().expect("an hour's set stands at its hour");
                    assert!(hour > last && hour <= head, "hour {hour} of the step {before}..={head} is in it and after {last}");
                    assert_eq!(set.table(), changes.table());
                    assert!(set.texts().is_empty() && set.status().is_none() && set.renumber().is_none(), "an hour's set carries only its cells and rows");
                    last = hour;
                    mirror.apply(set);
                    dated += 1;
                }
                mirror.texts.extend(changes.texts().iter().map(|(id, text)| (id, text.to_vec())));
                if let Some(status) = changes.status() {
                    mirror.status = Some(status);
                }
                if let Some(old) = changes.renumber() {
                    let mut moved = BTreeMap::new();
                    for (new, &from) in old.iter().enumerate() {
                        moved.insert(new as u32, mirror.rows.remove(&from).expect("a live row renumbered"));
                    }
                    assert!(mirror.rows.is_empty(), "every live row is renumbered");
                    mirror.rows = moved;
                }
                mirror.assert_holds(&lane);
                assert_eq!(rows(lane.cells()), rows(&states[head as usize]), "step {step} from {sought}");
                to += step;
            }
            assert!(dated > 20, "the hours of {step} from {sought} dated {dated} change sets");
        }
    }
}
