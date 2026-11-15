use super::{MapSide, Units, mirror::Mirror, open};
use crate::layers::{Budget, Carried, Dirty, Gpu, Inputs, Layer, Num, View};
use crate::playhead_host::mirror::{Cell as Read, Mirrors};
use playhead::{
    Piece, Settings, Step, Timeline,
    changes::Changes,
    lane::Lane,
    pages::{Pages, Run},
};
use replay_columnar::{
    manifest::Status,
    state::{Cell, Kind},
    store::{Store, TableChanges, TableState, Value},
    unit::{BLOCK, BlockPage, Encoder, PartSink, Settings as Writing, reader::Extents},
};
use replay_data::rows::TableId;
use std::{
    io,
    sync::{
        Arc, Mutex, mpsc,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};

const KINDS: [Kind; 4] = [Kind::I32, Kind::OptionalI64, Kind::U32List, Kind::Bytes];

#[derive(Default)]
struct Memory {
    bytes: Vec<u8>,
    pages: Vec<BlockPage>,
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

struct Recording {
    bytes: Vec<u8>,
    runs: Vec<Run>,
    kinds: Vec<Vec<Kind>>,
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

/// `tables` tables of [`KINDS`] over `hours` hours from game hour 0, each hour's changes of each
/// table made by `hour` (hour `n` names string `n`); every table's state after every hour.
fn record(tables: usize, hours: u32, hour: impl FnMut(u32, usize, &mut Store)) -> (Arc<Recording>, Vec<Vec<TableState>>) {
    record_with(Writing::at(3), tables, hours, hour)
}

fn record_with(settings: Writing, tables: usize, hours: u32, mut hour: impl FnMut(u32, usize, &mut Store)) -> (Arc<Recording>, Vec<Vec<TableState>>) {
    let kinds = vec![KINDS.to_vec(); tables];
    let mut store = Store::new(kinds.clone(), vec![false; tables]);
    store.attach(Encoder::new(&kinds, settings).unwrap());
    let mut memory = Memory::default();
    let mut states = vec![Vec::new(); tables];
    store.begin_run(0);
    for at in 0..hours {
        let text = format!("text {at}");
        store.begin_hour(at as i32, &[text.as_bytes()]).unwrap();
        for table in 0..tables {
            hour(at, table, &mut store);
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
    let run = Run::new(0, memory.pages).unwrap();
    (Arc::new(Recording { bytes: memory.bytes, runs: vec![run], kinds }), states)
}

/// Every kind of cell a mirror keeps: numbers, a missing optional, unrecorded cells, text, lists,
/// rows inserted and deleted, a slot freed and taken by another row, and a stretch the table is
/// not recorded.
fn eventful(at: u32, table: usize, store: &mut Store) {
    let status = if (300..310).contains(&at) {
        Status::NotRecorded
    } else if (400..410).contains(&at) {
        Status::Unavailable { reason: at }
    } else {
        Status::Complete
    };
    store.status(table, status, true, Some(at as i32), true).unwrap();
    if status != Status::Complete {
        return;
    }
    let mut changes: TableChanges<'_> = store.table(table).unwrap();
    if at == 0 {
        for slot in 0..12 {
            changes.insert_at(slot, 1000 + u128::from(slot)).unwrap();
            changes.write(slot, 0, Cell::I32(slot as i32)).unwrap();
            changes.write(slot, 1, Cell::OptionalI64(Some(i64::from(slot) - 5))).unwrap();
            changes.write(slot, 2, Cell::U32List(&[slot, slot + 1])).unwrap();
            changes.write(slot, 3, Cell::Bytes(0)).unwrap();
        }
        return;
    }
    let slot = at % 12;
    if at % 37 == 0 && slot == (at / 37) % 12 {
        return;
    }
    changes.write(slot, 0, Cell::I32((at as i32) * if slot % 2 == 0 { 1 } else { -3 })).unwrap();
    match at % 5 {
        0 => changes.write(slot, 1, Cell::OptionalI64(None)).unwrap(),
        1 => changes.put_value(slot, 1, Value::Unrecorded).unwrap(),
        2 => changes.write(slot, 2, Cell::U32List(&[at, at * 2, at * 3])).unwrap(),
        3 => changes.write(slot, 3, Cell::Bytes(at)).unwrap(),
        _ => changes.write(slot, 1, Cell::OptionalI64(Some(-i64::from(at)))).unwrap(),
    }
    if at % 37 == 0 {
        let gone = (at / 37) % 12;
        changes.delete(gone).unwrap();
        changes.insert_at(gone, 5000 + u128::from(at)).unwrap();
        changes.write(gone, 0, Cell::I32(-(at as i32))).unwrap();
        changes.put_value(gone, 2, Value::Unrecorded).unwrap();
    }
    if at % 53 == 0 {
        let extra = 12 + (at / 53) % 4;
        if changes.slot(9000 + u128::from(extra)).is_some() {
            changes.delete(extra).unwrap();
        } else {
            changes.insert_at(extra, 9000 + u128::from(extra)).unwrap();
            changes.write(extra, 0, Cell::I32(extra as i32)).unwrap();
        }
    }
}

/// The mirror's cells against the lane's, every live row and column; the first difference.
fn differ(mirror: &Mirror, cells: &TableState, columns: &[usize]) -> Option<String> {
    let slots = (cells.slots() as usize).max(mirror.slots());
    for slot in 0..slots as u32 {
        let live = cells.live().get(slot);
        if mirror.live(slot) != live {
            return Some(format!("slot {slot}: live {} against {live}", mirror.live(slot)));
        }
        if !live {
            continue;
        }
        if mirror.key(slot) != cells.key(slot) {
            return Some(format!("slot {slot}: key {:?} against {:?}", mirror.key(slot), cells.key(slot)));
        }
        if mirror.slot(cells.key(slot).unwrap()) != Some(slot) {
            return Some(format!("slot {slot}: its key does not find it"));
        }
        for &column in columns {
            let held = mirror.owned(column, slot);
            let wanted = match cells.value(column, slot) {
                Value::Unrecorded => Read::Unrecorded,
                Value::Missing => Read::Missing,
                Value::Number(bits) => Read::Value(bits.to_le_bytes()[..width(KINDS[column])].to_vec()),
                Value::Text(id) => Read::Value(u64::from(id).to_le_bytes()[..4].to_vec()),
                Value::List(bytes) => Read::Value(bytes.to_vec()),
            };
            if held != wanted {
                return Some(format!("slot {slot} column {column}: {held:?} against {wanted:?}"));
            }
            if let Value::Text(id) = cells.value(column, slot)
                && mirror.string(id) != Some(format!("text {id}").as_bytes())
            {
                return Some(format!("slot {slot}: string {id} is {:?}", mirror.string(id)));
            }
        }
    }
    None
}

fn width(kind: Kind) -> usize {
    match kind {
        Kind::I32 => 4,
        _ => 8,
    }
}

#[test]
fn a_mirror_from_a_reset_equals_the_lane_and_stays_equal_over_steps() {
    let (recording, _) = record(1, 2 * BLOCK + 100, eventful);
    let pages: Arc<dyn Pages> = recording;
    let all = [0, 1, 2, 3];
    for (from, columns) in [(0, &all[..]), (BLOCK + 17, &all[..]), (40, &[0, 3][..])] {
        let mut lane = Lane::new(Arc::clone(&pages), 0, 0, columns, true).unwrap();
        lane.seek(from as i32).unwrap();
        let mut mirror = Mirror::rebuild(&lane, columns);
        assert_eq!(differ(&mirror, lane.cells(), columns), None, "rebuilt at hour {from}");
        let mut changes = Changes::default();
        let (mut to, mut steps, mut not_recorded, mut unavailable) = (from as i32 + 7, 0, 0, 0);
        while to < (2 * BLOCK + 100) as i32 {
            lane.fetch_through(to).unwrap();
            let head = lane.step(to, &mut changes).unwrap();
            mirror.apply(&changes);
            assert_eq!(differ(&mirror, lane.cells(), columns), None, "after the step to hour {head}");
            assert_eq!(mirror.head(), Some(head));
            assert_eq!(mirror.status(), Some(lane.status().0));
            if mirror.status() == Some(Status::NotRecorded) {
                not_recorded += 1;
                assert!(mirror.not_recorded().is_some_and(|why| why.contains("not recorded")));
                assert!(!mirror.complete());
            }
            if let Some(Status::Unavailable { reason }) = mirror.status() {
                unavailable += 1;
                let text = format!("text {reason}");
                assert_eq!(mirror.reason(), Some(text.as_str()), "the reason's text, not its id");
                assert!(mirror.not_recorded().is_some_and(|why| why.ends_with(&format!("is unavailable: {text}"))));
            }
            if head == to {
                to += 7;
            }
            steps += 1;
        }
        assert!(steps > 30);
        assert!(not_recorded > 0 || from > 310, "the stretch not recorded reached the mirror as such");
        assert!(unavailable > 0 || from > 410, "the stretch unavailable reached the mirror with its reason");
    }
}

/// A test layer: the sum of column 0 of table 0 over live rows, kept per row from the mirror, and
/// its composes counted; its compose waits for the gate when one is given.
struct Sum {
    column: Option<Num<i32>>,
    second: bool,
    /// Refused while the camera is high.
    refuse_high: bool,
    /// Declares nothing while the camera is high.
    hide_high: bool,
    /// The head of every mirror it settled from.
    read: Vec<Option<i32>>,
    /// The camera heights of the views it was told of.
    viewed: Vec<f32>,
    values: Vec<Option<i64>>,
    dirty: Dirty,
    composes: Arc<Mutex<u32>>,
    gate: Option<Arc<Mutex<mpsc::Receiver<()>>>>,
    /// Told when a compose begins, before it waits for the gate.
    entered: Option<mpsc::Sender<()>>,
    frame: u64,
}

impl Sum {
    fn new(composes: &Arc<Mutex<u32>>) -> Self {
        Self { column: None, second: false, refuse_high: false, hide_high: false, read: Vec::new(), viewed: Vec::new(), values: Vec::new(), dirty: Dirty::default(), composes: Arc::clone(composes), gate: None, entered: None, frame: 0 }
    }

    /// Composes only when the test sends on the gate it returns.
    fn gated(composes: &Arc<Mutex<u32>>) -> (Self, mpsc::Sender<()>) {
        let (release, gate) = mpsc::channel();
        (Self { gate: Some(Arc::new(Mutex::new(gate))), ..Self::new(composes) }, release)
    }

    fn total(&self) -> i64 {
        self.values.iter().flatten().sum()
    }
}

fn table(index: usize) -> TableId {
    TableId::ALL[index]
}

fn sum_of(mirror: &Mirror, column: Num<i32>) -> i64 {
    mirror.rows().filter_map(|slot| mirror.number(column, slot).value()).map(i64::from).sum()
}

impl Layer for Sum {
    fn name(&self) -> &'static str {
        if self.second { "second" } else { "sum" }
    }

    fn inputs(&mut self, view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        if self.hide_high && view.camera_height >= 1.0 {
            self.column = None;
            return Ok(());
        }
        if self.refuse_high && view.camera_height >= 1.0 {
            return Err(io::Error::other("refused while the camera is high"));
        }
        let at = usize::from(self.second);
        inputs.table(table(at), true)?;
        inputs.index(table(at), 0)?;
        if self.second && view.camera_height >= 1.0 {
            inputs.index(table(at), 1)?;
        }
        self.column = Some(Num::of(table(at), 0));
        Ok(())
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        if let Some(entered) = &self.entered {
            let _ = entered.send(());
        }
        if let Some(gate) = &self.gate {
            gate.lock().unwrap().recv().unwrap();
        }
        *self.composes.lock().unwrap() += 1;
        let column = self.column.unwrap();
        let mirror = mirrors.get(column.table).unwrap();
        self.values = (0..mirror.slots() as u32).map(|slot| if mirror.live(slot) { mirror.number(column, slot).value().map(i64::from) } else { None }).collect();
        self.dirty.clear();
        Ok(())
    }

    fn apply(&mut self, step: &Step, mirrors: &Mirrors) {
        let Some(column) = self.column else { return };
        let Some(changes) = step.table(column.table.index()) else { return };
        let _ = mirrors;
        let slots = changes.deleted().slots().iter().chain(changes.inserted().slots()).chain(changes.columns().iter().filter(|written| written.column() == 0).flat_map(|written| written.slots()));
        for &slot in slots {
            self.dirty.mark(slot, self.frame);
        }
    }

    fn settle(&mut self, mirrors: &Mirrors, budget: &mut Budget) -> io::Result<()> {
        self.frame += 1;
        let Some(column) = self.column else { return Ok(()) };
        let mirror = mirrors.get(column.table).unwrap();
        self.read.push(mirror.head());
        let values = &mut self.values;
        self.dirty.settle(budget, |slot| {
            if values.len() <= slot as usize {
                values.resize(slot as usize + 1, None);
            }
            values[slot as usize] = if mirror.live(slot) { mirror.number(column, slot).value().map(i64::from) } else { None };
            Ok(())
        })
    }

    fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }

    fn carried(&self) -> Carried {
        Carried { entities: self.dirty.len(), frames: self.dirty.since().map_or(0, |since| self.frame - since) }
    }

    fn held(&self) -> usize {
        self.values.capacity() * 16 + self.dirty.bytes()
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }

    fn viewed(&mut self, view: &View, _mirrors: &Mirrors) {
        self.viewed.push(view.camera_height);
    }
}

/// Every row of every table writes its column 0 each hour.
fn busy(at: u32, table: usize, store: &mut Store) {
    store.status(table, Status::Complete, true, Some(at as i32), true).unwrap();
    let mut changes = store.table(table).unwrap();
    if at == 0 {
        for slot in 0..40 {
            changes.insert_at(slot, 100 + u128::from(slot)).unwrap();
        }
    }
    for slot in 0..40 {
        changes.write(slot, 0, Cell::I32((at as i32) * 3 + slot as i32 + table as i32)).unwrap();
        changes.write(slot, 1, Cell::OptionalI64(Some(i64::from(at)))).unwrap();
    }
}

fn units(recording: Arc<Recording>, name: &str) -> Units {
    let kinds = Arc::new(recording.kinds.clone());
    let pages: Arc<dyn Pages> = recording;
    let timeline = Timeline::whole(&*pages).unwrap();
    Units { name: name.to_owned(), pages, timeline, kinds }
}

/// `recording` as a revision read whole: its pages and column kinds.
fn read_whole(recording: Arc<Recording>) -> super::units::Read {
    let kinds = Arc::new(recording.kinds.clone());
    let pages: Arc<dyn Pages> = recording;
    super::units::Read { pages, kinds, branches: Arc::from(Vec::new()) }
}

#[test]
fn callers_asking_for_one_recordings_units_at_once_share_one_build() {
    use std::sync::{atomic::{AtomicUsize, Ordering}, mpsc};
    let (recording, _) = record(1, BLOCK, busy);
    let builds = Arc::new(AtomicUsize::new(0));
    let (started, building) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let first = {
        let (recording, builds) = (Arc::clone(&recording), Arc::clone(&builds));
        std::thread::spawn(move || {
            super::units::shared("one build", || {
                builds.fetch_add(1, Ordering::SeqCst);
                started.send(()).unwrap();
                released.recv().unwrap();
                Ok(read_whole(recording))
            })
            .unwrap()
        })
    };
    building.recv().unwrap();
    // Asked while the first build runs: it waits for that build, never builds beside it.
    let second = {
        let (recording, builds) = (Arc::clone(&recording), Arc::clone(&builds));
        std::thread::spawn(move || {
            super::units::shared("one build", || {
                builds.fetch_add(1, Ordering::SeqCst);
                Ok(read_whole(recording))
            })
            .unwrap()
        })
    };
    release.send(()).unwrap();
    let (first, second) = (first.join().unwrap(), second.join().unwrap());
    assert_eq!(builds.load(Ordering::SeqCst), 1, "the map and follow opening one recording build its units once");
    assert!(Arc::ptr_eq(&first.pages, &second.pages), "and read the same pages");
}

/// Four hours a step, a frame a second.
fn quick() -> Settings {
    Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 4.0, rate: 1.0 }
}

/// Frames at the newest step built until `done` holds, yielding between frames that take nothing.
fn frames_until(side: &mut MapSide, mut done: impl FnMut(&MapSide, &super::Frame) -> bool) {
    for _ in 0..50_000_000u64 {
        crate::layers::recorded::reads_landed(side);
        let head = side.entry().playhead().newest().unwrap_or(0);
        let frame = side.frame(head).unwrap();
        if done(side, &frame) {
            return;
        }
        if frame.steps == 0 && frame.swapped.is_none() {
            std::thread::yield_now();
        }
    }
    panic!("the map side never reached what the test waits for");
}

fn sum_layer(side: &MapSide, at: usize) -> &Sum {
    side.parts().unwrap().1[at].any().downcast_ref::<Sum>().unwrap()
}

#[test]
fn a_reset_swaps_mirrors_and_composed_arrays_together_and_applies_the_steps_taken_meanwhile() {
    let (recording, states) = record(1, BLOCK + 200, busy);
    let entry = open(units(recording, "reset"), quick()).unwrap();
    let composes = Arc::new(Mutex::new(0));
    let (release, gate) = mpsc::channel();
    let mut layer = Sum::new(&composes);
    layer.gate = Some(Arc::new(Mutex::new(gate)));
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(layer)], View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(100);
    frames_until(&mut side, |_, frame| {
        if frame.resets > 0 {
            assert_eq!(frame.applied, Some(100), "the frame that takes the reset composes the hour sought, the steps taken with it left for after the swap");
        }
        frame.resets > 0
    });
    assert!(side.composing() && side.parts().is_none(), "the mirrors and layers are on the worker");
    let deferred = side.held().pending;
    let mut waited = 0;
    frames_until(&mut side, |side, frame| {
        assert!(frame.composing && frame.steps == 0, "nothing is applied while composing");
        assert_eq!(side.held().pending, deferred, "steps built meanwhile stay in the ring");
        waited += 1;
        side.entry().playhead().newest().is_some_and(|newest| newest >= 100 + 3 * 4)
    });
    assert!(waited > 0);
    release.send(()).unwrap();
    let mut swapped = false;
    frames_until(&mut side, |side, frame| {
        if frame.swapped.is_some() {
            assert_eq!((frame.steps, side.applied()), (0, Some(100)), "the swap frame shows the picture composed for the hour sought alone");
        }
        swapped |= frame.swapped.is_some();
        swapped && frame.steps > 0 && !side.composing() && side.held().pending == 0
    });
    let (mirrors, _) = side.parts().unwrap();
    let mirror = mirrors.get(table(0)).unwrap();
    let head = mirror.head().unwrap();
    assert!(head > 100 + 3 * 4 - 1, "the steps taken meanwhile are applied: head {head}");
    let column = Num::of(table(0), 0);
    assert_eq!(differ(mirror, &states[0][head as usize], &[0]), None);
    side.settle_all().unwrap();
    assert_eq!(sum_layer(&side, 0).total(), sum_of(side.parts().unwrap().0.get(table(0)).unwrap(), column));
    assert_eq!(*composes.lock().unwrap(), 1);
}

#[test]
fn a_layer_out_of_budget_keeps_its_dirty_entities_and_settles_them_with_the_newest_values() {
    let (recording, _) = record(1, BLOCK, busy);
    let entry = open(units(recording, "budget"), quick()).unwrap();
    let composes = Arc::new(Mutex::new(0));
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Sum::new(&composes))], View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    // Each step marks all 40 rows; with no budget one unit goes a frame (32 rows: the clock is read
    // every 32), the rest carried.
    side.set_budget(Duration::ZERO);
    frames_until(&mut side, |_, frame| frame.steps > 0 && frame.layers[0].carried > 0);
    let (before, mut late) = (sum_layer(&side, 0).total(), 0);
    frames_until(&mut side, |_, frame| {
        late = frame.layers[0].late;
        frame.steps > 0
    });
    assert_eq!(sum_layer(&side, 0).carried().entities, 40 - 32, "one unit made, the rest carried");
    assert!(late >= 1, "the carry-over is counted in frames");
    assert_ne!(sum_layer(&side, 0).total(), before, "the unit made shows its rows' newest values");
    side.set_budget(Duration::from_secs(1));
    frames_until(&mut side, |_, frame| frame.steps > 0 && frame.layers[0].carried == 0);
    let mirror = side.parts().unwrap().0.get(table(0)).unwrap();
    assert_eq!(sum_layer(&side, 0).total(), sum_of(mirror, Num::of(table(0), 0)), "settled late, with the newest values");
}

#[test]
fn changed_inputs_redeclare_and_reset_only_the_tables_whose_columns_changed() {
    let (recording, _) = record(2, BLOCK, busy);
    let entry = open(units(recording, "redeclare"), quick()).unwrap();
    let (first, second) = (Arc::new(Mutex::new(0)), Arc::new(Mutex::new(0)));
    let mut other = Sum::new(&second);
    other.second = true;
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Sum::new(&first)), Box::new(other)], View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    assert_eq!((*first.lock().unwrap(), *second.lock().unwrap()), (1, 1));
    let kept = side.parts().unwrap().0.get(table(0)).unwrap() as *const Mirror;
    assert!(!side.parts().unwrap().0.get(table(1)).unwrap().holds(1));
    side.set_view(View { camera_height: 2.0, ..View::default() }).unwrap();
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    let mirrors = side.parts().unwrap().0;
    assert!(mirrors.get(table(1)).unwrap().holds(1), "the table with a new column arrived reset");
    assert!(std::ptr::eq(mirrors.get(table(0)).unwrap(), kept), "the other table's mirror is kept");
    assert_eq!((*first.lock().unwrap(), *second.lock().unwrap()), (1, 2), "only the layer reading the reset table composed");
}

#[test]
fn a_column_the_schema_lacks_refuses_the_layer_by_name() {
    let kinds = replay::columnar_frames::schema().unwrap();
    let mut inputs = Inputs::new(&kinds, "political");
    let missing = inputs.number::<u32>(TableId::Provinces, "no_such_field").unwrap_err().to_string();
    assert!(missing.contains("political") && missing.contains("Provinces.no_such_field"), "{missing}");
    let mistyped = inputs.number::<i64>(TableId::Provinces, "owner").unwrap_err().to_string();
    assert!(mistyped.contains("Provinces.owner is a U32 column"), "{mistyped}");
    assert!(inputs.number::<u32>(TableId::Provinces, "owner").is_ok());
}

#[test]
fn a_hook_in_the_slot_runs_on_every_step_until_removed() {
    let (recording, _) = record(1, BLOCK, busy);
    let entry = open(units(Arc::clone(&recording), "hook"), quick()).unwrap();
    assert!(Arc::ptr_eq(&open(units(recording, "hook"), quick()).unwrap(), &entry), "a second caller gets the same playhead");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let held = Arc::clone(&seen);
    let id = entry.hooks().add(Box::new(move |step, lanes| held.lock().unwrap().push((step.last(), lanes.get(0).and_then(|lane| lane.head())))));
    let composes = Arc::new(Mutex::new(0));
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Sum::new(&composes))], View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |_, frame| frame.applied.is_some_and(|hour| hour >= 40));
    let steps = seen.lock().unwrap().clone();
    assert!(steps.len() >= 5);
    assert!(steps.iter().all(|&(last, lane)| lane == Some(last)), "the hook sees the lanes at the step's end: {steps:?}");
    entry.hooks().remove(id);
    let count = seen.lock().unwrap().len();
    frames_until(&mut side, |_, frame| frame.applied.is_some_and(|hour| hour >= 80));
    assert!(seen.lock().unwrap().len() <= count + 4, "a removed hook runs no more once the ring's steps are built");
}

/// A slot freed past twice the live rows by a margin, at a block the writer restores whole: the
/// block's end packs the live rows to slots 0 to live - 1, and the mirror remaps itself to them.
#[test]
fn a_renumber_at_a_packed_block_end_remaps_the_mirror() {
    let settings = Writing { restore_blocks: 1, whole_every: 1, pack_margin: 8, ..Writing::at(3) };
    let (recording, _) = record_with(settings, 1, 2 * BLOCK, |at, table, store| {
        store.status(table, Status::Complete, true, Some(at as i32), true).unwrap();
        let mut changes = store.table(table).unwrap();
        if at < 100 {
            changes.insert_at(at, u128::from(at) + 1000).unwrap();
            changes.write(at, 0, Cell::I32(at as i32)).unwrap();
            changes.write(at, 3, Cell::Bytes(at)).unwrap();
        } else if at < 190 {
            changes.delete(at - 100).unwrap();
        } else if (BLOCK + 5..BLOCK + 50).contains(&at) {
            for slot in changes.held().rows().collect::<Vec<_>>() {
                changes.write(slot, 0, Cell::I32(-(at as i32))).unwrap();
            }
        }
    });
    assert!(recording.runs[0].pages[0].tables[0].packed, "the fixture packs its first block's end");
    let pages: Arc<dyn Pages> = recording;
    let columns = [0, 1, 3];
    let mut lane = Lane::new(Arc::clone(&pages), 0, 0, &columns, true).unwrap();
    lane.seek(0).unwrap();
    let mut mirror = Mirror::rebuild(&lane, &columns);
    let mut changes = Changes::default();
    let (mut to, mut renumbered) = (99, 0);
    while to < 2 * BLOCK as i32 {
        lane.fetch_through(to).unwrap();
        let head = lane.step(to, &mut changes).unwrap();
        mirror.apply(&changes);
        assert_eq!(differ(&mirror, lane.cells(), &columns), None, "after the step to hour {head}");
        if changes.renumber().is_some() {
            renumbered += 1;
            assert_eq!(mirror.rows().collect::<Vec<_>>(), (0..10).collect::<Vec<u32>>());
            assert_eq!((0..10).map(|slot| mirror.key(slot).unwrap()).collect::<Vec<_>>(), (1090..1100).collect::<Vec<u128>>());
            assert_eq!(mirror.slot(1095), Some(5), "the key index follows the renumber");
        }
        if head == to {
            to += 100;
        }
    }
    assert_eq!(renumbered, 1);
}

/// The same packing with rows 0 to 4 left where they were: the renumber moves the five rows past
/// them alone, and the mirror equals the lane's cells, keys and index as a remap of every row.
#[test]
fn a_renumber_moves_only_the_rows_past_the_first_freed_slot() {
    let settings = Writing { restore_blocks: 1, whole_every: 1, pack_margin: 8, ..Writing::at(3) };
    let (recording, _) = record_with(settings, 1, 2 * BLOCK, |at, table, store| {
        store.status(table, Status::Complete, true, Some(at as i32), true).unwrap();
        let mut changes = store.table(table).unwrap();
        if at < 100 {
            changes.insert_at(at, u128::from(at) + 1000).unwrap();
            changes.write(at, 0, Cell::I32(at as i32)).unwrap();
            changes.write(at, 3, Cell::Bytes(at)).unwrap();
        } else if at < 190 {
            changes.delete(at - 95).unwrap();
        }
    });
    assert!(recording.runs[0].pages[0].tables[0].packed, "the fixture packs its first block's end");
    let pages: Arc<dyn Pages> = recording;
    let columns = [0, 1, 3];
    let mut lane = Lane::new(Arc::clone(&pages), 0, 0, &columns, true).unwrap();
    lane.seek(0).unwrap();
    let mut mirror = Mirror::rebuild(&lane, &columns);
    let mut changes = Changes::default();
    let (mut to, mut renumbered) = (99, 0);
    while to < 2 * BLOCK as i32 {
        lane.fetch_through(to).unwrap();
        let head = lane.step(to, &mut changes).unwrap();
        mirror.apply(&changes);
        assert_eq!(differ(&mirror, lane.cells(), &columns), None, "after the step to hour {head}");
        if changes.renumber().is_some() {
            renumbered += 1;
            assert_eq!(mirror.rows().collect::<Vec<_>>(), (0..10).collect::<Vec<u32>>());
            assert_eq!((0..10).map(|slot| mirror.key(slot).unwrap()).collect::<Vec<_>>(), (1000..1005).chain(1095..1100).collect::<Vec<u128>>());
            assert_eq!((mirror.slot(1003), mirror.slot(1097), mirror.slot(1050)), (Some(3), Some(7), None), "the key index follows the renumber");
            assert_eq!(mirror.moved(), 5, "the rows before the first freed slot stay where they are");
        }
        if head == to {
            to += 100;
        }
    }
    assert_eq!(renumbered, 1);
}

/// Hours between two pieces are a gap: every table reads as not recorded there, never as its
/// last rows, and the next piece's reset makes them recorded again.
#[test]
fn a_gap_step_shows_every_table_not_recorded_until_the_next_piece() {
    let (recording, _) = record(2, BLOCK, busy);
    let pages: Arc<dyn Pages> = Arc::clone(&recording) as Arc<dyn Pages>;
    let timeline = Timeline::new(&*pages, vec![Piece { run: 0, first: 0, last: 99 }, Piece { run: 0, first: 200, last: 399 }]).unwrap();
    let units = Units { name: "gap".into(), pages, timeline, kinds: Arc::new(recording.kinds.clone()) };
    let entry = open(units, quick()).unwrap();
    let composes = Arc::new(Mutex::new(0));
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Sum::new(&composes))], View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(60);
    let (mut gap, mut after) = (0, 0);
    frames_until(&mut side, |side, frame| {
        if let (Some(hour), Some((mirrors, _))) = (frame.applied, side.parts()) {
            let mirror = mirrors.get(table(0)).unwrap();
            if (100..200).contains(&hour) {
                gap += 1;
                assert_eq!(mirror.status(), None, "hour {hour} is in the gap");
                assert!(!mirror.complete() && mirror.not_recorded().is_some_and(|why| why.contains("gap")));
            } else if hour >= 200 {
                after += 1;
                assert_eq!(mirror.status(), Some(Status::Complete), "hour {hour} is in the second piece");
            } else {
                assert!(mirror.complete());
            }
        }
        after >= 3
    });
    assert!(gap > 0, "a gap step was taken");
}

/// An upstream layer (twice a column) and a dependent (the upstream's value plus one), stepped
/// together with budgets that starve both: the dependent never settles from a half-settled
/// upstream, and whenever both are whole it equals the dependent composed from scratch.
#[test]
fn a_dependent_layer_follows_its_upstream_and_equals_composed() {
    struct Twice {
        column: Option<Num<i32>>,
        values: Vec<Option<i64>>,
        dirty: Dirty,
        changed: crate::layers::Changed,
        frame: u64,
    }
    struct Plus {
        values: Vec<Option<i64>>,
        dirty: Dirty,
        settled_from_half: u64,
        frame: u64,
    }
    impl Layer for Twice {
        fn name(&self) -> &'static str {
            "twice"
        }
        fn inputs(&mut self, _view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
            inputs.table(table(0), false)?;
            inputs.index(table(0), 0)?;
            self.column = Some(Num::of(table(0), 0));
            Ok(())
        }
        fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
            let mirror = mirrors.get(table(0)).unwrap();
            let column = self.column.unwrap();
            self.values = (0..mirror.slots() as u32).map(|slot| mirror.live(slot).then(|| mirror.number(column, slot).value().map(|value| 2 * i64::from(value))).flatten()).collect();
            self.dirty.clear();
            Ok(())
        }
        fn apply(&mut self, step: &Step, _mirrors: &Mirrors) {
            if let Some(changes) = step.table(0) {
                for written in changes.columns().iter().filter(|written| written.column() == 0) {
                    for &slot in written.slots() {
                        self.dirty.mark(slot, self.frame);
                    }
                }
            }
        }
        fn settle(&mut self, mirrors: &Mirrors, budget: &mut Budget) -> io::Result<()> {
            self.frame += 1;
            let mirror = mirrors.get(table(0)).unwrap();
            let column = self.column.unwrap();
            let (values, changed) = (&mut self.values, &mut self.changed);
            self.dirty.settle(budget, |slot| {
                let value = mirror.live(slot).then(|| mirror.number(column, slot).value().map(|value| 2 * i64::from(value))).flatten();
                if values.len() <= slot as usize {
                    values.resize(slot as usize + 1, None);
                }
                if values[slot as usize] != value {
                    values[slot as usize] = value;
                    changed.mark("row", u64::from(slot));
                }
                Ok(())
            })
        }
        fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
            Ok(())
        }
        fn carried(&self) -> Carried {
            Carried { entities: self.dirty.len(), frames: 0 }
        }
        fn held(&self) -> usize {
            0
        }
        fn any(&self) -> &dyn std::any::Any {
            self
        }
        fn changed(&self) -> Option<&crate::layers::Changed> {
            Some(&self.changed)
        }
        fn clear_changed(&mut self) {
            self.changed.clear();
        }
    }
    impl Plus {
        fn from(upstream: &Twice) -> Vec<Option<i64>> {
            upstream.values.iter().map(|value| value.map(|value| value + 1)).collect()
        }
    }
    impl Layer for Plus {
        fn name(&self) -> &'static str {
            "plus"
        }
        fn inputs(&mut self, _view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
            inputs.after("twice");
            Ok(())
        }
        fn compose(&mut self, _mirrors: &Mirrors) -> io::Result<()> {
            unreachable!("a dependent composes with its upstream")
        }
        fn compose_with(&mut self, _mirrors: &Mirrors, upstream: &super::Upstream<'_>) -> io::Result<()> {
            self.values = Self::from(upstream.get::<Twice>().expect("registered before"));
            self.dirty.clear();
            Ok(())
        }
        fn apply(&mut self, _step: &Step, _mirrors: &Mirrors) {}
        fn apply_with(&mut self, _step: &Step, _mirrors: &Mirrors, upstream: &super::Upstream<'_>) {
            for slot in upstream.changed::<Twice>().of("row").iter().map(|&id| id as u32) {
                self.dirty.mark(slot, self.frame);
            }
        }
        fn settle(&mut self, _mirrors: &Mirrors, _budget: &mut Budget) -> io::Result<()> {
            unreachable!("a dependent settles with its upstream")
        }
        fn settle_with(&mut self, _mirrors: &Mirrors, upstream: &super::Upstream<'_>, budget: &mut Budget) -> io::Result<()> {
            self.frame += 1;
            let twice = upstream.get::<Twice>().unwrap();
            if twice.dirty.len() > 0 {
                self.settled_from_half += 1;
            }
            let values = &mut self.values;
            self.dirty.settle(budget, |slot| {
                if values.len() <= slot as usize {
                    values.resize(slot as usize + 1, None);
                }
                values[slot as usize] = twice.values.get(slot as usize).copied().flatten().map(|value| value + 1);
                Ok(())
            })
        }
        fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
            Ok(())
        }
        fn carried(&self) -> Carried {
            Carried { entities: self.dirty.len(), frames: 0 }
        }
        fn held(&self) -> usize {
            0
        }
        fn any(&self) -> &dyn std::any::Any {
            self
        }
    }
    let (recording, _) = record(1, BLOCK, busy);
    let entry = open(units(recording, "dependent"), quick()).unwrap();
    let twice = Twice { column: None, values: Vec::new(), dirty: Dirty::default(), changed: crate::layers::Changed::default(), frame: 0 };
    let plus = Plus { values: Vec::new(), dirty: Dirty::default(), settled_from_half: 0, frame: 0 };
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(twice), Box::new(plus)], View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    let check = |side: &MapSide| {
        let layers = side.parts().unwrap().1;
        let (twice, plus) = (layers[0].any().downcast_ref::<Twice>().unwrap(), layers[1].any().downcast_ref::<Plus>().unwrap());
        assert_eq!(plus.settled_from_half, 0, "the dependent never settles from a half-settled upstream");
        (twice.dirty.len() == 0 && plus.dirty.len() == 0).then(|| assert_eq!(plus.values, Plus::from(twice), "stepwise equals composed"))
    };
    let (mut starved, mut whole) = (0, 0);
    for round in 0..12 {
        side.set_budget(if round % 3 == 0 { Duration::from_secs(1) } else { Duration::ZERO });
        frames_until(&mut side, |side, frame| frame.steps > 0 && !side.composing());
        match check(&side) {
            Some(()) => whole += 1,
            None => starved += 1,
        }
    }
    assert!(starved > 0 && whole > 0, "starved {starved}, whole {whole}");
    side.set_budget(Duration::from_secs(1));
    frames_until(&mut side, |side, frame| frame.steps > 0 && check(side).is_some());
}

#[test]
fn two_anchors_of_the_same_hour_make_one_seek() {
    let (recording, _) = record(1, BLOCK, busy);
    let entry = open(units(recording, "anchors"), quick()).unwrap();
    entry.anchor(100, true, 4.0, true);
    let first = entry.playhead().holding().expect("held at the seek");
    entry.anchor(100, true, 4.0, true);
    assert_eq!(entry.playhead().holding(), Some(first), "the second anchor of hour 100 is dropped");
    entry.anchor(200, true, 4.0, true);
    let second = entry.playhead().holding().expect("held at the new seek");
    assert_ne!(second, first, "another hour seeks");
    entry.anchor(200, false, 4.0, false);
    entry.anchor(200, false, 4.0, true);
    assert_eq!(entry.playhead().holding(), Some(second), "paused at the hour it is seeking to: dropped");
}

#[test]
fn a_layer_refused_and_accepted_again_composes_instead_of_resuming() {
    let (recording, _) = record(1, BLOCK, busy);
    let entry = open(units(recording, "refused"), quick()).unwrap();
    let (kept, refusing) = (Arc::new(Mutex::new(0)), Arc::new(Mutex::new(0)));
    let mut second = Sum::new(&refusing);
    second.refuse_high = true;
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Sum::new(&kept)), Box::new(second)], View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    side.set_view(View { camera_height: 2.0, ..View::default() }).unwrap();
    assert_eq!(side.refusals().count(), 1);
    let mut stepped = 0;
    frames_until(&mut side, |_, frame| {
        stepped += frame.steps;
        stepped >= 5
    });
    side.set_view(View::default()).unwrap();
    frames_until(&mut side, |side, frame| frame.steps > 0 && !side.composing());
    side.settle_all().unwrap();
    let mirror = side.parts().unwrap().0.get(table(0)).unwrap();
    assert_eq!(sum_layer(&side, 1).total(), sum_of(mirror, Num::of(table(0), 0)), "accepted again, its arrays equal composed");
    assert_eq!(*refusing.lock().unwrap(), 2, "it composed again when accepted");
}

#[test]
fn a_camera_move_reaches_the_layers_without_a_redeclare_or_a_compose() {
    let (recording, _) = record(1, BLOCK, busy);
    let entry = open(units(recording, "camera"), quick()).unwrap();
    let composes = Arc::new(Mutex::new(0));
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Sum::new(&composes))], View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    for height in [3.0, 5.0] {
        side.set_view(View { camera_height: height, ..View::default() }).unwrap();
        frames_until(&mut side, |_, frame| frame.steps > 0);
    }
    assert_eq!(sum_layer(&side, 0).viewed, vec![3.0, 5.0], "each camera move reaches the layer once");
    assert_eq!(*composes.lock().unwrap(), 1, "no compose for a move that changes no input");
}

#[test]
fn a_map_and_follow_on_one_recording_make_one_seek_and_the_map_advances() {
    let (recording, _) = record(1, BLOCK, busy);
    let entry = open(units(recording, "map and follow"), quick()).unwrap();
    assert!(!entry.drawn(), "with no map, follow's driver advances");
    let composes = Arc::new(Mutex::new(0));
    let side = MapSide::new(Arc::clone(&entry), vec![Box::new(Sum::new(&composes))], View::default()).unwrap();
    assert!(entry.drawn(), "with a map, the map's frame advances and follow's driver does not");
    side.anchor(300, true, 4.0, true);
    let epoch = entry.playhead().holding().expect("held at the seek");
    entry.anchor(300, true, 4.0, true);
    assert_eq!(entry.playhead().holding(), Some(epoch), "follow's anchor of the same hour makes no second seek");
    drop(side);
    assert!(!entry.drawn(), "the map gone, follow's driver advances again");
}

#[test]
fn a_layer_shown_again_never_reads_a_mirror_its_tables_left() {
    let (recording, _) = record(1, 4 * BLOCK, busy);
    let entry = open(units(recording, "hidden"), quick()).unwrap();
    let composes = Arc::new(Mutex::new(0));
    let mut layer = Sum::new(&composes);
    layer.hide_high = true;
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(layer)], View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    side.set_view(View { camera_height: 2.0, ..View::default() }).unwrap();
    let mut stepped = 0;
    frames_until(&mut side, |_, frame| {
        stepped += frame.steps;
        stepped >= 6
    });
    assert!(side.parts().unwrap().0.get(table(0)).is_none(), "a table no drawn layer declares leaves the mirrors");
    let shown = entry.playhead().newest().unwrap();
    let before = sum_layer(&side, 0).read.len();
    side.set_view(View::default()).unwrap();
    // Its table left the lanes while hidden: shown again, it is read from the file and lands at the
    // hour the frames are at, which the frames give its read the turn to reach before they move on
    // (`reads_landed`); the layer composes with its rows then, and reads nothing before.
    let last = 4 * BLOCK as i32 - 1;
    frames_until(&mut side, |side, frame| (frame.steps > 0 && !side.composing() && sum_layer(side, 0).read.len() > before + 2) || frame.applied == Some(last));
    let read = &sum_layer(&side, 0).read[before..];
    assert!(read.iter().all(|&head| head.is_some_and(|head| head >= shown)), "shown again, it read only mirrors at or past hour {shown}: {read:?}");
    let mirror = side.parts().unwrap().0.get(table(0)).unwrap_or_else(|| panic!("shown again at hour {shown}, its table came back as a reset before hour {:?}", side.applied()));
    assert!(mirror.head().is_some_and(|head| head >= shown), "the table came back as a reset, not as the mirror it left");
    let back = read.iter().flatten().next().copied();
    assert!(back.is_some_and(|hour| hour < shown + 48), "it came back within the ring and the read-ahead of hour {shown}: first read at {back:?}");
    side.settle_all().unwrap();
    let mirror = side.parts().unwrap().0.get(table(0)).unwrap();
    assert_eq!(sum_layer(&side, 0).total(), sum_of(mirror, Num::of(table(0), 0)), "its arrays equal composed");
}

#[test]
fn a_table_two_layers_share_stays_current_when_one_stops_declaring_it() {
    let (recording, states) = record(1, BLOCK, busy);
    let entry = open(units(recording, "shared"), quick()).unwrap();
    let (first, second) = (Arc::new(Mutex::new(0)), Arc::new(Mutex::new(0)));
    let mut hiding = Sum::new(&second);
    hiding.hide_high = true;
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Sum::new(&first)), Box::new(hiding)], View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    side.set_view(View { camera_height: 2.0, ..View::default() }).unwrap();
    let mut stepped = 0;
    frames_until(&mut side, |_, frame| {
        stepped += frame.steps;
        stepped >= 4
    });
    let mirror = side.parts().unwrap().0.get(table(0)).expect("the other layer keeps the table");
    let head = mirror.head().unwrap();
    assert_eq!(differ(mirror, &states[0][head as usize], &[0]), None, "the shared table stays current");
}

/// The stepped harness's frames, one hour and several apart, across a block end: each is reached
/// (a step ends on every hour) and the mirror equals the recording there.
#[test]
fn stepped_frames_reach_every_hour_across_a_block_end() {
    let (recording, states) = record(1, 2 * BLOCK, busy);
    let entry = open(units(recording, "stepped"), quick()).unwrap();
    let composes = Arc::new(Mutex::new(0));
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Sum::new(&composes))], View::default()).unwrap();
    entry.anchor(0, false, 0.0, true);
    let hours = (0..4).chain((5..BLOCK + 40).step_by(7)).chain(BLOCK + 41..BLOCK + 44);
    for hour in hours {
        side.catch_up(hour as i32).unwrap_or_else(|error| panic!("hour {hour}: {error}"));
        let mirror = side.parts().unwrap().0.get(table(0)).unwrap();
        assert_eq!(mirror.head(), Some(hour as i32));
        assert_eq!(differ(mirror, &states[0][hour as usize], &[0]), None, "at hour {hour}");
        assert_eq!(sum_layer(&side, 0).total(), sum_of(mirror, Num::of(table(0), 0)), "the layer at hour {hour}");
    }
}

/// An upstream that publishes one id a frame and carries work for its first frames, and a
/// dependent that reads what it published only in `settle_with`: held while the upstream carries,
/// it must still see every id once it settles.
#[test]
fn a_held_dependent_sees_every_change_published_while_it_was_held() {
    struct Publisher {
        frame: u64,
        changed: crate::layers::Changed,
    }
    struct Reader {
        seen: Vec<u64>,
    }
    impl Layer for Publisher {
        fn name(&self) -> &'static str {
            "publisher"
        }
        fn inputs(&mut self, _view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
            inputs.table(table(0), false)
        }
        fn compose(&mut self, _mirrors: &Mirrors) -> io::Result<()> {
            Ok(())
        }
        fn apply(&mut self, _step: &Step, _mirrors: &Mirrors) {}
        fn settle(&mut self, _mirrors: &Mirrors, _budget: &mut Budget) -> io::Result<()> {
            self.frame += 1;
            self.changed.mark("frame", self.frame);
            Ok(())
        }
        fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
            Ok(())
        }
        fn carried(&self) -> Carried {
            Carried { entities: usize::from(self.frame < 5), frames: 0 }
        }
        fn held(&self) -> usize {
            0
        }
        fn any(&self) -> &dyn std::any::Any {
            self
        }
        fn changed(&self) -> Option<&crate::layers::Changed> {
            Some(&self.changed)
        }
        fn clear_changed(&mut self) {
            self.changed.clear();
        }
    }
    impl Layer for Reader {
        fn name(&self) -> &'static str {
            "reader"
        }
        fn inputs(&mut self, _view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
            inputs.after("publisher");
            Ok(())
        }
        fn compose(&mut self, _mirrors: &Mirrors) -> io::Result<()> {
            Ok(())
        }
        fn apply(&mut self, _step: &Step, _mirrors: &Mirrors) {}
        fn settle(&mut self, _mirrors: &Mirrors, _budget: &mut Budget) -> io::Result<()> {
            Ok(())
        }
        fn settle_with(&mut self, _mirrors: &Mirrors, upstream: &super::Upstream<'_>, _budget: &mut Budget) -> io::Result<()> {
            self.seen.extend_from_slice(upstream.changed::<Publisher>().of("frame"));
            Ok(())
        }
        fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
            Ok(())
        }
        fn carried(&self) -> Carried {
            Carried::default()
        }
        fn held(&self) -> usize {
            0
        }
        fn any(&self) -> &dyn std::any::Any {
            self
        }
    }
    let (recording, _) = record(1, BLOCK, busy);
    let entry = open(units(recording, "held"), quick()).unwrap();
    let publisher = Publisher { frame: 0, changed: crate::layers::Changed::default() };
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(publisher), Box::new(Reader { seen: Vec::new() })], View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    frames_until(&mut side, |side, _| side.parts().is_some_and(|(_, layers)| layers[0].any().downcast_ref::<Publisher>().unwrap().frame >= 8));
    let layers = side.parts().unwrap().1;
    let published = layers[0].any().downcast_ref::<Publisher>().unwrap().frame;
    let seen = &layers[1].any().downcast_ref::<Reader>().unwrap().seen;
    assert_eq!(seen, &(1..=published).collect::<Vec<u64>>(), "every id published, the held frames' included");
}

/// Frames at `head` until the mirror and the layer are exactly at it, never showing a later hour
/// and never dropping the old picture meanwhile; the frames it took.
fn frames_to_exact(side: &mut MapSide, head: i32, states: &[TableState]) -> usize {
    let before = side.parts().unwrap().0.get(table(0)).and_then(|mirror| mirror.head()).unwrap();
    for frames in 1..=200_000 {
        let frame = side.frame(head).unwrap();
        if let Some((mirrors, _)) = side.parts() {
            let at = mirrors.get(table(0)).and_then(|mirror| mirror.head());
            assert!(at.is_some_and(|at| at <= head.max(before)), "frame {frames} at {head} showed hour {at:?}, after {before}");
            assert!(side.draws("sum"), "the old picture stays while the exact one is made");
            if at == Some(head) && !side.composing() {
                side.settle_all().unwrap();
                let mirror = side.parts().unwrap().0.get(table(0)).unwrap();
                assert_eq!(differ(mirror, &states[head as usize], &[0]), None, "the mirror at hour {head}");
                assert_eq!(sum_layer(side, 0).total(), sum_of(mirror, Num::of(table(0), 0)), "the layer's arrays at hour {head}");
                return frames;
            }
        }
        if frame.steps == 0 && frame.swapped.is_none() {
            std::thread::yield_now();
        }
    }
    panic!("hour {head} was never shown exactly");
}

/// Played at 1,440 hours a second (24-hour steps), paused inside a step the playhead has built
/// past: the map comes to exactly the paused hour, and so do one hour forward and back.
#[test]
fn a_pause_inside_a_step_and_single_hours_while_paused_show_exactly_that_hour() {
    let (recording, states) = record(1, 2 * BLOCK, busy);
    let entry = open(units(recording, "paused"), Settings { speed: 1440.0, rate: 60.0, ..quick() }).unwrap();
    let composes = Arc::new(Mutex::new(0));
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Sum::new(&composes))], View::default()).unwrap();
    entry.anchor(0, true, 1440.0, true);
    frames_until(&mut side, |side, frame| frame.applied.is_some_and(|hour| hour >= 100) && !side.composing());
    let applied = side.parts().unwrap().0.get(table(0)).unwrap().head().unwrap();
    let paused = applied + 5;
    for _ in 0..50_000_000u64 {
        if entry.playhead().newest().is_some_and(|newest| newest > paused) {
            break;
        }
        std::thread::yield_now();
    }
    assert!(entry.playhead().newest().unwrap() > paused, "the playhead built past the paused hour");
    entry.anchor(paused, false, 1440.0, false);
    let started = std::time::Instant::now();
    let frames = frames_to_exact(&mut side, paused, &states[0]);
    eprintln!("pause: exact in {frames} frames, {:.2} ms", started.elapsed().as_secs_f64() * 1e3);
    entry.anchor(paused + 1, false, 1440.0, true);
    frames_to_exact(&mut side, paused + 1, &states[0]);
    entry.anchor(paused, false, 1440.0, true);
    frames_to_exact(&mut side, paused, &states[0]);
}

/// Keys in the shapes the recording's keyed tables use: small ids, (country, region),
/// (combat, side, ordinal), (group, wing) with 64-bit ids, and (country, region, level, index).
fn real_key_shapes() -> Vec<u128> {
    let mut keys: Vec<u128> = (1..=13_000).collect();
    keys.extend((0..1_500u128).flat_map(|country| (0..40u128).map(move |region| country << 32 | region)));
    keys.extend((0..2_000u128).flat_map(|combat| (0..2u128).flat_map(move |side| (0..24u128).map(move |ordinal| combat << 33 | side << 32 | ordinal))));
    keys.extend((0..400u128).flat_map(|group| (0..50u128).map(move |wing| (0x0000_7ff6_0000_0000 + group * 0x1000) << 64 | (0x0001_0000_0000 + wing * 0x40))));
    keys.extend((0..300u128).flat_map(|country| (0..20u128).flat_map(move |region| (0..4u128).map(move |level| country << 72 | region << 40 | level << 32 | 7))));
    keys.sort_unstable();
    keys.dedup();
    keys
}

/// Times `slot()` over a keyed mirror of every shape (run by name with --nocapture to read it);
/// asserts only that every key is found and an absent one is not.
#[test]
fn mirror_slot_lookup_time_over_real_key_shapes() {
    let keys = real_key_shapes();
    let held = keys.clone();
    let (recording, _) = record(1, 1, move |at, table, store| {
        store.status(table, Status::Complete, true, Some(at as i32), true).unwrap();
        let mut changes = store.table(table).unwrap();
        for (slot, &key) in held.iter().enumerate() {
            changes.insert_at(slot as u32, key).unwrap();
        }
    });
    let pages: Arc<dyn Pages> = recording;
    let mut lane = Lane::new(pages, 0, 0, &[0], true).unwrap();
    lane.seek(0).unwrap();
    let mirror = Mirror::rebuild(&lane, &[0]);
    let mut order: Vec<usize> = (0..keys.len()).collect();
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    for at in (1..order.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        order.swap(at, (seed % (at as u64 + 1)) as usize);
    }
    for (slot, &key) in keys.iter().enumerate() {
        assert_eq!(mirror.slot(key), Some(slot as u32));
    }
    assert_eq!(mirror.slot(u128::MAX), None);
    let rounds = 20;
    let started = std::time::Instant::now();
    let mut found = 0u64;
    for _ in 0..rounds {
        for &at in &order {
            found += u64::from(std::hint::black_box(mirror.slot(std::hint::black_box(keys[at]))).is_some());
        }
    }
    let ns = started.elapsed().as_nanos() as f64 / (rounds * keys.len()) as f64;
    assert_eq!(found, (rounds * keys.len()) as u64);
    eprintln!("slot(): {ns:.1} ns a lookup over {} keys", keys.len());
}

/// The key shapes hashed into a table of their size: no bucket of the low bits holds a long
/// chain, and the top seven bits (the table's control byte) take most of their values.
#[test]
fn real_key_shapes_do_not_collide_into_long_chains() {
    use std::hash::BuildHasher;
    let keys = real_key_shapes();
    let hasher = super::mirror::Keys::default();
    let buckets = keys.len().next_power_of_two();
    let mut load = vec![0u32; buckets];
    let mut control = [0u32; 128];
    for &key in &keys {
        let hash = hasher.hash_one(key);
        load[hash as usize & (buckets - 1)] += 1;
        control[(hash >> 57) as usize] += 1;
    }
    let longest = *load.iter().max().unwrap();
    assert!(longest <= 12, "{longest} keys share a bucket of {buckets}");
    let expected = keys.len() as f64 / 128.0;
    assert!(control.iter().all(|&count| (count as f64) < expected * 1.5 && (count as f64) > expected * 0.5), "control bytes {control:?}");
}


/// Two maps of one recording on one frame thread, each with more settle work than the frame has:
/// the frame's settle across both stays within its one budget, and both keep settling. The budget's
/// clock is the test's: each entity settled costs 3 µs of it, so the test counts work, not time.
#[test]
fn two_maps_share_one_frame_budget_and_neither_starves() {
    struct Costly {
        dirty: Dirty,
        frame: u64,
        settled: u64,
    }
    impl Layer for Costly {
        fn name(&self) -> &'static str {
            "costly"
        }
        fn inputs(&mut self, _view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
            inputs.table(table(0), false)?;
            inputs.index(table(0), 0)
        }
        fn compose(&mut self, _mirrors: &Mirrors) -> io::Result<()> {
            Ok(())
        }
        fn apply(&mut self, step: &Step, _mirrors: &Mirrors) {
            let Some(changes) = step.table(0) else { return };
            for &slot in changes.columns().iter().filter(|written| written.column() == 0).flat_map(|written| written.slots()) {
                self.dirty.mark(slot, self.frame);
            }
        }
        fn settle(&mut self, _mirrors: &Mirrors, budget: &mut Budget) -> io::Result<()> {
            self.frame += 1;
            let settled = &mut self.settled;
            self.dirty.settle(budget, |_| {
                crate::layers::clock::advance(Duration::from_micros(3));
                *settled += 1;
                Ok(())
            })
        }
        fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
            Ok(())
        }
        fn carried(&self) -> Carried {
            Carried { entities: self.dirty.len(), frames: self.dirty.since().map_or(0, |since| self.frame - since) }
        }
        fn held(&self) -> usize {
            0
        }
        fn any(&self) -> &dyn std::any::Any {
            self
        }
    }
    let (recording, _) = record(1, BLOCK, |at, table, store| {
        store.status(table, Status::Complete, true, Some(at as i32), true).unwrap();
        let mut changes = store.table(table).unwrap();
        if at == 0 {
            for slot in 0..2_000 {
                changes.insert_at(slot, u128::from(slot) + 1).unwrap();
            }
        }
        for slot in 0..2_000 {
            changes.write(slot, 0, Cell::I32(at as i32 + slot as i32)).unwrap();
        }
    });
    crate::layers::clock::hold();
    let entry = open(units(recording, "two maps"), quick()).unwrap();
    let budget = Duration::from_millis(2);
    let mut maps: Vec<MapSide> = (0..2)
        .map(|_| {
            let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Costly { dirty: Dirty::default(), frame: 0, settled: 0 })], View::default()).unwrap();
            side.set_budget(budget);
            side
        })
        .collect();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    let costly = |side: &MapSide| side.parts().map_or(0, |(_, layers)| layers[0].any().downcast_ref::<Costly>().unwrap().settled);
    let (mut frames, mut over) = (0, Vec::new());
    for _ in 0..5_000_000u64 {
        let head = entry.playhead().newest().unwrap_or(0);
        let parts: Vec<super::Frame> = maps.iter_mut().map(|side| side.frame(head).unwrap()).collect();
        let settling = parts.iter().all(|frame| !frame.composing && frame.swapped.is_none()) && maps.iter().all(|side| costly(side) > 0);
        if settling {
            let ns: u64 = parts.iter().map(|frame| frame.layers[0].settle_ns).sum();
            // Each map may pass the budget by what it settles between two of its checks (32 entities).
            if Duration::from_nanos(ns) > budget + Duration::from_micros(2 * 32 * 3) {
                over.push(ns);
            }
            frames += 1;
        }
        if frames >= 40 {
            break;
        }
        if parts.iter().all(|frame| frame.steps == 0 && frame.swapped.is_none()) {
            std::thread::yield_now();
        }
    }
    assert_eq!(frames, 40, "the maps settled for 40 frames");
    assert!(over.is_empty(), "frames whose settle across both maps overran one budget: {over:?}");
    // Each of these frames brings both maps a step's work, so each has work and a share for it: a
    // step past both maps' hours is waited for, as a loaded machine can leave one behind the other.
    let before: Vec<u64> = maps.iter().map(costly).collect();
    for _ in 0..20 {
        for _ in 0..50_000_000u64 {
            if maps.iter().all(|side| entry.playhead().newest() > side.applied()) {
                break;
            }
            std::thread::yield_now();
        }
        let head = entry.playhead().newest().unwrap();
        maps.iter_mut().for_each(|side| drop(side.frame(head).unwrap()));
    }
    assert!(maps.iter().zip(&before).all(|(side, &before)| costly(side) > before), "both maps kept settling");
}

/// One map, two independent layers, each given more settle work every step than the frame's budget
/// holds: the layers take the budget oldest dirty work first, so the one registered second is not
/// starved by the first.
#[test]
fn a_layer_registered_later_is_not_starved_by_one_before_it() {
    struct Busy {
        name: &'static str,
        table: usize,
        dirty: Dirty,
        frame: u64,
        settled: u64,
    }
    impl Layer for Busy {
        fn name(&self) -> &'static str {
            self.name
        }
        fn inputs(&mut self, _view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
            inputs.table(table(self.table), false)?;
            inputs.index(table(self.table), 0)
        }
        fn compose(&mut self, _mirrors: &Mirrors) -> io::Result<()> {
            Ok(())
        }
        fn apply(&mut self, step: &Step, _mirrors: &Mirrors) {
            let Some(changes) = step.table(self.table) else { return };
            for &slot in changes.columns().iter().filter(|written| written.column() == 0).flat_map(|written| written.slots()) {
                self.dirty.mark(slot, self.frame);
            }
        }
        fn settle(&mut self, _mirrors: &Mirrors, budget: &mut Budget) -> io::Result<()> {
            self.frame += 1;
            let settled = &mut self.settled;
            self.dirty.settle(budget, |_| {
                crate::layers::clock::advance(Duration::from_micros(3));
                *settled += 1;
                Ok(())
            })
        }
        fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
            Ok(())
        }
        fn carried(&self) -> Carried {
            Carried { entities: self.dirty.len(), frames: self.dirty.since().map_or(0, |since| self.frame - since) }
        }
        fn held(&self) -> usize {
            0
        }
        fn any(&self) -> &dyn std::any::Any {
            self
        }
    }
    let (recording, _) = record(2, BLOCK, |at, table, store| {
        store.status(table, Status::Complete, true, Some(at as i32), true).unwrap();
        let mut changes = store.table(table).unwrap();
        if at == 0 {
            for slot in 0..2_000 {
                changes.insert_at(slot, u128::from(slot) + 1).unwrap();
            }
        }
        for slot in 0..2_000 {
            changes.write(slot, 0, Cell::I32(at as i32 + slot as i32)).unwrap();
        }
    });
    crate::layers::clock::hold();
    let entry = open(units(recording, "starved"), quick()).unwrap();
    let busy = |name, table| Box::new(Busy { name, table, dirty: Dirty::default(), frame: 0, settled: 0 }) as Box<dyn Layer>;
    let mut side = MapSide::new(Arc::clone(&entry), vec![busy("first", 0), busy("second", 1)], View::default()).unwrap();
    side.set_budget(Duration::from_millis(2));
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    let settled = |side: &MapSide, at: usize| side.parts().unwrap().1[at].any().downcast_ref::<Busy>().unwrap().settled;
    // A frame only when a step is built, as at 1,440 hours a second a step is due every frame.
    for _ in 0..40 {
        for _ in 0..50_000_000u64 {
            if entry.playhead().newest() > side.applied() {
                break;
            }
            std::thread::yield_now();
        }
        side.frame(entry.playhead().newest().unwrap()).unwrap();
    }
    assert!(settled(&side, 0) > 0 && settled(&side, 1) > 0, "both layers settled: first {}, second {}", settled(&side, 0), settled(&side, 1));
}

/// With no budget at all, a layer made whole in one unit of work, dirty at every step, still
/// settles in every frame that took a step, and the layer that reads it (held while it carries
/// work) settles too. A frame takes every step due, so the frames that took one are counted.
/// Before, a spent budget left it dirty in every frame and its reader held for good, as the
/// engagements and the markers reading them stood at 1,440 hours a second.
#[test]
fn with_the_budget_spent_every_layer_makes_one_unit_of_progress_a_frame_and_its_readers_are_not_held() {
    struct Whole {
        dirty: bool,
        settled: u64,
    }
    impl Layer for Whole {
        fn name(&self) -> &'static str {
            "whole"
        }
        fn inputs(&mut self, _view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
            inputs.table(table(0), false)?;
            inputs.index(table(0), 0)
        }
        fn compose(&mut self, _mirrors: &Mirrors) -> io::Result<()> {
            Ok(())
        }
        fn apply(&mut self, step: &Step, _mirrors: &Mirrors) {
            self.dirty |= step.table(0).is_some_and(|changes| changes.columns().iter().any(|written| !written.is_empty()));
        }
        fn settle(&mut self, _mirrors: &Mirrors, budget: &mut Budget) -> io::Result<()> {
            if self.dirty && budget.left() {
                (self.dirty, self.settled) = (false, self.settled + 1);
            }
            Ok(())
        }
        fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
            Ok(())
        }
        fn carried(&self) -> Carried {
            Carried { entities: usize::from(self.dirty), frames: 0 }
        }
        fn held(&self) -> usize {
            0
        }
        fn any(&self) -> &dyn std::any::Any {
            self
        }
    }
    struct Reader {
        settled: u64,
    }
    impl Layer for Reader {
        fn name(&self) -> &'static str {
            "reader"
        }
        fn inputs(&mut self, _view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
            inputs.after("whole");
            Ok(())
        }
        fn compose(&mut self, _mirrors: &Mirrors) -> io::Result<()> {
            Ok(())
        }
        fn apply(&mut self, _step: &Step, _mirrors: &Mirrors) {}
        fn settle(&mut self, _mirrors: &Mirrors, _budget: &mut Budget) -> io::Result<()> {
            self.settled += 1;
            Ok(())
        }
        fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
            Ok(())
        }
        fn carried(&self) -> Carried {
            Carried::default()
        }
        fn held(&self) -> usize {
            0
        }
        fn any(&self) -> &dyn std::any::Any {
            self
        }
    }
    let (recording, _) = record(1, BLOCK, |at, table, store| {
        store.status(table, Status::Complete, true, Some(at as i32), true).unwrap();
        let mut changes = store.table(table).unwrap();
        if at == 0 {
            changes.insert_at(0, 1).unwrap();
        }
        changes.write(0, 0, Cell::I32(at as i32)).unwrap();
    });
    crate::layers::clock::hold();
    let entry = open(units(recording, "spent budget"), quick()).unwrap();
    let layers: Vec<Box<dyn Layer>> = vec![Box::new(Whole { dirty: false, settled: 0 }), Box::new(Reader { settled: 0 })];
    let mut side = MapSide::new(Arc::clone(&entry), layers, View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    side.set_budget(Duration::ZERO);
    let settled = |side: &MapSide| {
        let layers = side.parts().unwrap().1;
        (layers[0].any().downcast_ref::<Whole>().unwrap().settled, layers[1].any().downcast_ref::<Reader>().unwrap().settled)
    };
    let before = settled(&side);
    let mut stepped = 0;
    frames_until(&mut side, |side, frame| {
        stepped += u64::from(frame.steps > 0);
        let whole = side.parts().unwrap().1[0].any().downcast_ref::<Whole>().unwrap();
        assert!(frame.steps == 0 || !whole.dirty, "a frame that took a step left the whole layer dirty: its one unit did not go");
        stepped >= 10
    });
    let after = settled(&side);
    assert!(after.0 - before.0 >= stepped && after.1 - before.1 >= stepped, "both settled in every frame that took a step, with no budget: whole {:?}, reader {:?} over {stepped} frames that took a step", (before.0, after.0), (before.1, after.1));
}

/// The uploads' share is what they cost, at least a third of the map's slice and at most half.
#[test]
fn the_uploads_share_is_their_cost_between_a_third_and_half_the_slice() {
    let ms = Duration::from_millis;
    assert_eq!(super::upload_share(ms(3), &[]), ms(1), "a third at least");
    assert_eq!(super::upload_share(ms(3), &[Duration::from_micros(600), Duration::from_micros(700)]), Duration::from_micros(1_300));
    assert_eq!(super::upload_share(ms(3), &[ms(1), ms(2)]), Duration::from_micros(1_500), "at most half");
}

/// A layer passed over last frame with work waiting (the budget was spent before its turn) goes
/// before one whose work is older but that had its turn; with both having had it, oldest work first.
#[test]
fn a_layer_passed_over_last_frame_goes_before_older_work() {
    let none = vec![Vec::new(), Vec::new()];
    assert_eq!(super::oldest_first(&[50, 3], &[9, 6], &none, 10), vec![1, 0]);
    assert_eq!(super::oldest_first(&[50, 3], &[9, 9], &none, 10), vec![0, 1]);
}

/// A layer whose settle alone spends more than the whole slice in every frame, on the test's clock.
struct Hog;

impl Layer for Hog {
    fn name(&self) -> &'static str {
        "hog"
    }
    fn inputs(&mut self, _view: &View, _inputs: &mut Inputs<'_>) -> io::Result<()> {
        Ok(())
    }
    fn compose(&mut self, _mirrors: &Mirrors) -> io::Result<()> {
        Ok(())
    }
    fn apply(&mut self, _step: &Step, _mirrors: &Mirrors) {}
    fn settle(&mut self, _mirrors: &Mirrors, _budget: &mut Budget) -> io::Result<()> {
        crate::layers::clock::advance(Duration::from_millis(5));
        Ok(())
    }
    fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }
    fn carried(&self) -> Carried {
        Carried::default()
    }
    fn held(&self) -> usize {
        0
    }
    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A layer with uploads to make, as many as the test puts in `work`, each costing `cost` of the
/// test's clock and logged by name when made.
struct Pusher {
    name: &'static str,
    work: Arc<AtomicU32>,
    cost: Duration,
    log: Arc<Mutex<Vec<&'static str>>>,
}

impl Pusher {
    fn boxed(name: &'static str, work: &Arc<AtomicU32>, cost: Duration, log: &Arc<Mutex<Vec<&'static str>>>) -> Box<dyn Layer> {
        Box::new(Self { name, work: Arc::clone(work), cost, log: Arc::clone(log) })
    }

    fn push(&mut self, budget: &mut Budget) -> io::Result<()> {
        while self.work.load(Ordering::Relaxed) > 0 && budget.left() {
            crate::layers::clock::advance(self.cost);
            self.work.fetch_sub(1, Ordering::Relaxed);
            self.log.lock().unwrap().push(self.name);
        }
        Ok(())
    }
}

impl Layer for Pusher {
    fn name(&self) -> &'static str {
        self.name
    }
    fn inputs(&mut self, _view: &View, _inputs: &mut Inputs<'_>) -> io::Result<()> {
        Ok(())
    }
    fn compose(&mut self, _mirrors: &Mirrors) -> io::Result<()> {
        Ok(())
    }
    fn apply(&mut self, _step: &Step, _mirrors: &Mirrors) {}
    fn settle(&mut self, _mirrors: &Mirrors, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }
    fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }
    fn uploaded(&self) -> bool {
        self.work.load(Ordering::Relaxed) == 0
    }
    fn carried(&self) -> Carried {
        Carried::default()
    }
    fn held(&self) -> usize {
        0
    }
    fn any(&self) -> &dyn std::any::Any {
        self
    }
    fn any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// A map with a table-reading layer and `layers` after it, on the test's clock, played to its
/// first reset and composed.
fn drawn_on_held_clock(name: &str, layers: Vec<Box<dyn Layer>>) -> MapSide {
    crate::layers::clock::hold();
    let (recording, _) = record(1, BLOCK, busy);
    let entry = open(units(recording, name), quick()).unwrap();
    let mut all: Vec<Box<dyn Layer>> = vec![Box::new(Sum::new(&Arc::new(Mutex::new(0))))];
    all.extend(layers);
    let mut side = MapSide::new(Arc::clone(&entry), all, View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    side
}

/// The upload pass with each pusher making its uploads.
fn push_uploads(side: &mut MapSide) {
    side.upload_with(false, |layer, budget| match layer.any_mut().and_then(|layer| layer.downcast_mut::<Pusher>()) {
        Some(pusher) => pusher.push(budget),
        None => Ok(()),
    })
    .unwrap();
}

/// A layer whose settle alone spends more than the whole slice in every frame, beside a layer with
/// uploads to make in every frame: they are made in every frame all the same. Before, the uploads
/// had what the settles left of the slice, which was nothing, in every frame.
#[test]
fn an_overrunning_settle_leaves_another_layer_its_uploads_every_frame() {
    let (work, log) = (Arc::new(AtomicU32::new(0)), Arc::default());
    let mut side = drawn_on_held_clock("overrun", vec![Box::new(Hog), Pusher::boxed("late", &work, Duration::from_micros(100), &log)]);
    let head = side.applied().unwrap();
    for frame in 0..20 {
        work.store(3, Ordering::Relaxed);
        side.frame(head).unwrap();
        push_uploads(&mut side);
        assert_eq!(work.load(Ordering::Relaxed), 0, "frame {frame}: the uploads were made beside the overrunning settle");
        assert!(!side.last_frame().layers[2].upload_carried, "frame {frame}: nothing carried");
    }
}

/// Two layers with uploads carried: the one carried longer goes first. Each upload costs more than
/// the slice, so a frame makes only the one the first layer visited is always given. Before, the
/// layers uploaded in registration order: the second's last two waited for all of the first's.
#[test]
fn the_upload_carried_longer_goes_first() {
    let (first, second, log) = (Arc::new(AtomicU32::new(0)), Arc::new(AtomicU32::new(0)), Arc::new(Mutex::new(Vec::new())));
    let cost = Duration::from_millis(4);
    let mut side = drawn_on_held_clock("oldest upload", vec![Pusher::boxed("first", &first, cost, &log), Pusher::boxed("second", &second, cost, &log)]);
    let head = side.applied().unwrap();
    second.store(3, Ordering::Relaxed);
    side.frame(head).unwrap();
    push_uploads(&mut side);
    first.store(3, Ordering::Relaxed);
    for _ in 0..5 {
        side.frame(head).unwrap();
        push_uploads(&mut side);
    }
    assert_eq!(*log.lock().unwrap(), ["second", "second", "second", "first", "first", "first"]);
}

/// After a stretch of frames with a settle spending more than the whole slice and no uploads to
/// make, the uploads are still given at least their floor, a third of the slice: a layer with more
/// upload work than that makes at least that much of it in the next frame. Before, the uploads'
/// measured cost decayed over the stretch and they were left what the settle left: nothing.
#[test]
fn after_a_starved_stretch_the_uploads_share_is_not_below_its_floor() {
    let (work, log) = (Arc::new(AtomicU32::new(0)), Arc::default());
    let cost = Duration::from_micros(10);
    let mut side = drawn_on_held_clock("floor", vec![Box::new(Hog), Pusher::boxed("late", &work, cost, &log)]);
    let head = side.applied().unwrap();
    for _ in 0..30 {
        side.frame(head).unwrap();
        push_uploads(&mut side);
    }
    work.store(1_000_000, Ordering::Relaxed);
    let frame = side.frame(head).unwrap();
    push_uploads(&mut side);
    let made = 1_000_000 - work.load(Ordering::Relaxed);
    let floor = super::BUDGET / 3;
    assert!(Duration::from_nanos(frame.upload_share_ns) >= floor, "the share reserved is {} ns", frame.upload_share_ns);
    assert!(cost * made >= floor, "{made} uploads of {cost:?} made, under the floor of {floor:?}");
}

/// The layer whose settle alone takes more than the settles' whole share is named in the frame's
/// record, and only it.
#[test]
fn a_layer_whose_settle_overruns_the_whole_share_is_named_in_the_frame() {
    let mut side = drawn_on_held_clock("named", vec![Box::new(Hog), Box::new(Tableless { composed: 0 })]);
    let head = side.applied().unwrap();
    let frame = side.frame(head).unwrap();
    assert_eq!(frame.overran, ["hog"]);
    assert_eq!(frame.layers.iter().map(|part| part.overran).collect::<Vec<_>>(), [false, true, false]);
}

/// A layer that declares no table (it reads rows of its own, or only layers before it).
struct Tableless {
    composed: u32,
}

impl Layer for Tableless {
    fn name(&self) -> &'static str {
        "tableless"
    }
    fn inputs(&mut self, _view: &View, _inputs: &mut Inputs<'_>) -> io::Result<()> {
        Ok(())
    }
    fn compose(&mut self, _mirrors: &Mirrors) -> io::Result<()> {
        self.composed += 1;
        Ok(())
    }
    fn apply(&mut self, _step: &Step, _mirrors: &Mirrors) {}
    fn settle(&mut self, _mirrors: &Mirrors, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }
    fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }
    fn carried(&self) -> Carried {
        Carried::default()
    }
    fn held(&self) -> usize {
        0
    }
    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A map joining a playhead another map has anchored gets its tables by joins, not a reset: a
/// layer of it that declares no table has nothing to wait for and is drawn.
#[test]
fn a_layer_without_tables_on_a_map_joining_an_anchored_playhead_is_drawn() {
    let (recording, _) = record(1, BLOCK, busy);
    let entry = open(units(recording, "tableless join"), quick()).unwrap();
    let composes = Arc::new(Mutex::new(0));
    let mut first = MapSide::new(Arc::clone(&entry), vec![Box::new(Sum::new(&composes))], View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut first, |side, frame| frame.swapped.is_some() && !side.composing());
    let mut second = MapSide::new(Arc::clone(&entry), vec![Box::new(Tableless { composed: 0 })], View::default()).unwrap();
    for _ in 0..200_000 {
        let head = entry.playhead().newest().unwrap_or(0);
        first.frame(head).unwrap();
        second.frame(head).unwrap();
        if second.draws("tableless") {
            return;
        }
        std::thread::yield_now();
    }
    panic!("the joining map's layer without tables was never drawn");
}

/// Two maps of one playhead, the second composing for as long as the test holds its compose: the
/// composing map still takes the steps (into those it applies after the swap), so the first map
/// steps on far past what the step buffer holds.
#[test]
fn a_map_composing_does_not_hold_the_steps_of_another_map() {
    let (recording, _) = record(1, BLOCK, busy);
    let entry = open(units(recording, "composing holds"), quick()).unwrap();
    let composes = Arc::new(Mutex::new(0));
    let mut first = MapSide::new(Arc::clone(&entry), vec![Box::new(Sum::new(&composes))], View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut first, |side, frame| frame.swapped.is_some() && !side.composing());
    let (gate, held) = mpsc::channel();
    let mut gated = Sum::new(&Arc::new(Mutex::new(0)));
    gated.gate = Some(Arc::new(Mutex::new(held)));
    let mut second = MapSide::new(Arc::clone(&entry), vec![Box::new(gated)], View::default()).unwrap();
    for _ in 0..2_000_000 {
        let head = entry.playhead().newest().unwrap_or(0);
        first.frame(head).unwrap();
        second.frame(head).unwrap();
        if second.composing() {
            break;
        }
        std::thread::yield_now();
    }
    assert!(second.composing(), "the second map composes, held by the test");
    let from = first.applied().unwrap();
    let mut reached = from;
    for _ in 0..2_000_000 {
        let head = entry.playhead().newest().unwrap_or(0);
        first.frame(head).unwrap();
        second.frame(head).unwrap();
        reached = first.applied().unwrap();
        if reached >= from + 100 {
            break;
        }
        std::thread::yield_now();
    }
    gate.send(()).unwrap();
    assert!(reached >= from + 100, "the first map stepped from {from} only to {reached} while the second composed");
}

/// Two maps on one frame thread; the first composes through the second frame, asking no share.
/// Every map's frame begins the frame thread's frame, so in the third frame the two maps' shares
/// are one budget. Were the frame begun only by a map asking for its share, the second map would
/// begin it in the second frame, and in the third the first map's share (what that frame left)
/// and the second's (a new frame's, weighted by its lateness) would be 2.25 ms of a 2 ms budget.
#[test]
fn a_map_composing_through_a_frame_leaves_the_frame_thread_one_budget_a_frame() {
    use super::share;
    let total = Duration::from_millis(2);
    let (first, second) = (share::join(), share::join());
    // Frame 1: both draw; the second's oldest work has waited 5 frames.
    share::begin(first);
    let spent = share::slice(first, total);
    share::spent(first, spent, 0, true);
    share::begin(second);
    let spent = share::slice(second, total);
    share::spent(second, spent, 5, true);
    // Frame 2: the first composes (it settles nothing and asks no share); the second's has waited 50.
    share::begin(first);
    share::spent(first, Duration::ZERO, 0, true);
    share::begin(second);
    let spent = share::slice(second, total);
    share::spent(second, spent, 50, true);
    // Frame 3: both draw.
    share::begin(first);
    let one = share::slice(first, total);
    share::spent(first, one, 0, true);
    share::begin(second);
    let other = share::slice(second, total);
    share::spent(second, other, 50, true);
    assert!(one + other <= total + Duration::from_micros(1), "the third frame's shares are {one:?} and {other:?} of one {total:?}");
    share::leave(first);
    share::leave(second);
}

/// A layer whose worker makes the product of a step one settle after it is handed it (a worker
/// that keeps up): it carries nothing on the frame thread and says the hour its product shows.
struct Worked {
    next: Option<i32>,
    sent: Option<i32>,
    published: Option<i32>,
}

impl Layer for Worked {
    fn name(&self) -> &'static str {
        "worked"
    }
    fn inputs(&mut self, _view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        inputs.table(table(0), false)?;
        inputs.index(table(0), 0)
    }
    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        (self.next, self.sent, self.published) = (None, None, mirrors.get(table(0)).and_then(|mirror| mirror.head()));
        Ok(())
    }
    fn apply(&mut self, step: &Step, _mirrors: &Mirrors) {
        self.next = Some(step.last());
    }
    fn settle(&mut self, _mirrors: &Mirrors, _budget: &mut Budget) -> io::Result<()> {
        if let Some(sent) = self.sent.take() {
            self.published = Some(sent);
        }
        self.sent = self.next.take();
        Ok(())
    }
    fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }
    fn carried(&self) -> Carried {
        Carried::default()
    }
    fn published(&self) -> crate::layers::Published {
        use crate::layers::Published;
        match self.sent {
            Some(_) => self.published.map_or(Published::Nothing, Published::At),
            None => Published::Current,
        }
    }
    fn held(&self) -> usize {
        0
    }
    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

fn worked(side: &MapSide) -> &Worked {
    side.parts().unwrap().1.iter().find_map(|layer| layer.any().downcast_ref::<Worked>()).unwrap()
}

/// Frames only when a step is built, as at 1,440 hours a second a step is due every frame.
fn paced_frame(side: &mut MapSide, entry: &super::Entry) -> super::Frame {
    for _ in 0..50_000_000u64 {
        if entry.playhead().newest() > side.applied() {
            break;
        }
        std::thread::yield_now();
    }
    side.frame(entry.playhead().newest().unwrap()).unwrap()
}

/// Playing, a layer whose worker holds the newest step: the map is drawn at the hour the worker's
/// product shows (never the newer hour applied), and that hour keeps up with the head. Each frame
/// takes one step (the worker holds it: the map is at the step before), and a frame after it with
/// no step takes the worker's product back (the map is at the head): two known states a step.
#[test]
fn a_map_with_a_worker_layer_is_drawn_at_its_shown_hour_and_keeps_up_while_playing() {
    let (recording, _) = record(1, BLOCK, busy);
    let entry = open(units(recording, "worked playing"), quick()).unwrap();
    let composes = Arc::new(Mutex::new(0));
    let layers: Vec<Box<dyn Layer>> = vec![Box::new(Sum::new(&composes)), Box::new(Worked { next: None, sent: None, published: None })];
    let mut side = MapSide::new(Arc::clone(&entry), layers, View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    let first = side.last_frame().drawn;
    for _ in 0..20 {
        let head = side.applied().unwrap() + 4;
        for _ in 0..50_000_000u64 {
            if entry.playhead().newest().is_some_and(|newest| newest >= head) {
                break;
            }
            std::thread::yield_now();
        }
        let frame = side.frame(head).unwrap();
        assert_eq!(frame.applied, Some(head), "one step a frame");
        assert_eq!(frame.drawn, Some(head - 4), "the worker holds the step: drawn at its product's hour, not the {head} applied");
        let frame = side.frame(head).unwrap();
        assert_eq!(frame.drawn, Some(head), "the product is back: drawn at the head");
    }
    assert!(side.last_frame().drawn > first, "the drawn hour advances");
}

/// A layer drawing a product of rows at the head made on its worker: it says the hour those rows
/// stand at, `behind` hours before the hour applied.
struct Made {
    applied: Option<i32>,
    behind: Arc<std::sync::atomic::AtomicI32>,
}

impl Layer for Made {
    fn name(&self) -> &'static str {
        "made"
    }
    fn inputs(&mut self, _view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        inputs.table(table(0), false)?;
        inputs.index(table(0), 0)
    }
    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        self.applied = mirrors.get(table(0)).and_then(|mirror| mirror.head());
        Ok(())
    }
    fn apply(&mut self, step: &Step, _mirrors: &Mirrors) {
        self.applied = Some(step.last());
    }
    fn settle(&mut self, _mirrors: &Mirrors, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }
    fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }
    fn carried(&self) -> Carried {
        Carried::default()
    }
    fn published(&self) -> crate::layers::Published {
        self.applied.map_or(crate::layers::Published::Nothing, |hour| crate::layers::Published::Cadence(hour - self.behind.load(Ordering::Acquire)))
    }
    fn held(&self) -> usize {
        0
    }
    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

/// The window clock's drawn hour leaves a layer of rows at the head out; the oldest hour on screen
/// counts it while it is behind (rows of an older step), and does not once its rows stand at the
/// hour applied (no step since changed them).
#[test]
fn the_oldest_hour_on_screen_counts_a_layer_of_rows_at_the_head_while_it_is_behind() {
    let (recording, _) = record(1, BLOCK, busy);
    let entry = open(units(recording, "made behind"), quick()).unwrap();
    let behind = Arc::new(std::sync::atomic::AtomicI32::new(0));
    let layers: Vec<Box<dyn Layer>> = vec![Box::new(Sum::new(&Arc::new(Mutex::new(0)))), Box::new(Made { applied: None, behind: Arc::clone(&behind) })];
    let mut side = MapSide::new(Arc::clone(&entry), layers, View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    for lag in [3, 0, 5] {
        behind.store(lag, Ordering::Release);
        let head = side.applied().unwrap() + 4;
        for _ in 0..50_000_000u64 {
            if entry.playhead().newest().is_some_and(|newest| newest >= head) {
                break;
            }
            std::thread::yield_now();
        }
        let frame = side.frame(head).unwrap();
        assert_eq!(frame.applied, Some(head), "one step a frame");
        assert_eq!(frame.drawn, Some(head), "the window clock is not held by a layer of rows at the head");
        assert_eq!(frame.oldest, Some(head - lag), "the oldest hour on screen counts it {lag} hours behind");
    }
}

/// A dependent of a layer whose worker holds work in every frame settles every frame: the worker's
/// work is not carried, so the dependent reads the upstream's published product.
#[test]
fn a_dependent_settles_every_frame_while_its_upstream_worker_holds_work() {
    struct Follower {
        settles: u64,
    }
    impl Layer for Follower {
        fn name(&self) -> &'static str {
            "follower"
        }
        fn inputs(&mut self, _view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
            inputs.after("worked");
            Ok(())
        }
        fn compose(&mut self, _mirrors: &Mirrors) -> io::Result<()> {
            Ok(())
        }
        fn apply(&mut self, _step: &Step, _mirrors: &Mirrors) {}
        fn settle(&mut self, _mirrors: &Mirrors, _budget: &mut Budget) -> io::Result<()> {
            self.settles += 1;
            Ok(())
        }
        fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
            Ok(())
        }
        fn carried(&self) -> Carried {
            Carried::default()
        }
        fn held(&self) -> usize {
            0
        }
        fn any(&self) -> &dyn std::any::Any {
            self
        }
    }
    let (recording, _) = record(1, BLOCK, busy);
    let entry = open(units(recording, "follower"), quick()).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Worked { next: None, sent: None, published: None }), Box::new(Follower { settles: 0 })], View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    let follower = |side: &MapSide| side.parts().unwrap().1[1].any().downcast_ref::<Follower>().unwrap().settles;
    let before = follower(&side);
    for _ in 0..20 {
        paced_frame(&mut side, &entry);
        assert!(worked(&side).sent.is_some(), "the upstream's worker holds work");
    }
    assert_eq!(follower(&side) - before, 20, "the dependent settled in every frame");
}

/// Held at an hour, the map is drawn at it only once every layer shows it: a worker layer still
/// making the head's product keeps the map at its older hour until it is published.
#[test]
fn held_the_map_is_drawn_at_the_head_only_once_its_worker_layer_shows_it() {
    let (recording, _) = record(1, BLOCK, busy);
    let entry = open(units(recording, "worked held"), quick()).unwrap();
    let composes = Arc::new(Mutex::new(0));
    let layers: Vec<Box<dyn Layer>> = vec![Box::new(Sum::new(&composes)), Box::new(Worked { next: None, sent: None, published: None })];
    let mut side = MapSide::new(Arc::clone(&entry), layers, View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    let head = side.applied().unwrap() + 4;
    for _ in 0..50_000_000u64 {
        if entry.playhead().newest().is_some_and(|newest| newest >= head) {
            break;
        }
        std::thread::yield_now();
    }
    let frame = side.frame(head).unwrap();
    assert_eq!(frame.applied, Some(head), "the step to the held hour is applied");
    assert_ne!(frame.drawn, Some(head), "the worker has not shown it yet");
    let frame = side.frame(head).unwrap();
    assert_eq!(frame.drawn, Some(head), "drawn at the head once the worker shows it");
}

/// A layer that reads no table: its compose meets another layer's (it says it started and waits,
/// at most a minute, for the other to say so) and checks that the layer it reads composed first.
struct Meeting {
    name: &'static str,
    after: Option<&'static str>,
    meet: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
    composed: Arc<Mutex<Vec<&'static str>>>,
}

impl Meeting {
    fn boxed(name: &'static str, after: Option<&'static str>, meet: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>, composed: &Arc<Mutex<Vec<&'static str>>>) -> Box<dyn Layer> {
        Box::new(Self { name, after, meet, composed: Arc::clone(composed) })
    }
}

impl Layer for Meeting {
    fn name(&self) -> &'static str {
        self.name
    }
    fn inputs(&mut self, _view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        if let Some(upstream) = self.after {
            inputs.after(upstream);
        }
        Ok(())
    }
    fn compose(&mut self, _mirrors: &Mirrors) -> io::Result<()> {
        if let Some(upstream) = self.after
            && !self.composed.lock().unwrap().contains(&upstream)
        {
            return Err(io::Error::other(format!("{} composed before the {upstream} layer it reads", self.name)));
        }
        if let Some((started, other)) = &self.meet {
            let _ = started.send(());
            other.recv_timeout(Duration::from_secs(60)).map_err(|_| io::Error::other(format!("{} composed alone: the layer it meets had not started a minute later", self.name)))?;
        }
        self.composed.lock().unwrap().push(self.name);
        Ok(())
    }
    fn apply(&mut self, _step: &Step, _mirrors: &Mirrors) {}
    fn settle(&mut self, _mirrors: &Mirrors, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }
    fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }
    fn carried(&self) -> Carried {
        Carried::default()
    }
    fn held(&self) -> usize {
        0
    }
    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

/// At a reset, two layers that read nothing of one another compose at the same time: each waits
/// in its compose for the other to start. A layer reading one of them composes after it. Before,
/// the host's one worker composed every layer one after another, so the first waited a minute for
/// the second and was refused by name.
#[test]
fn layers_that_read_nothing_of_one_another_compose_at_once_and_a_dependent_after_its_upstream() {
    let (recording, _) = record(1, BLOCK, busy);
    let entry = open(units(recording, "composed at once"), quick()).unwrap();
    let composed = Arc::new(Mutex::new(Vec::new()));
    let ((to_u, from_a), (to_a, from_u)) = (mpsc::channel(), mpsc::channel());
    let layers = vec![
        Box::new(Sum::new(&Arc::new(Mutex::new(0)))) as Box<dyn Layer>,
        Meeting::boxed("a", None, Some((to_u, from_u)), &composed),
        Meeting::boxed("u", None, Some((to_a, from_a)), &composed),
        Meeting::boxed("d", Some("u"), None, &composed),
    ];
    let mut side = MapSide::new(Arc::clone(&entry), layers, View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    let refused: Vec<String> = side.refusals().map(|(name, why)| format!("{name}: {why}")).collect();
    assert!(refused.is_empty(), "refused: {refused:?}");
    assert!(["a", "u", "d"].iter().all(|name| side.draws(name)));
    let composed = composed.lock().unwrap();
    let at = |name: &str| composed.iter().position(|held| *held == name);
    assert!(at("u") < at("d"), "the dependent composed after its upstream: {composed:?}");
}

/// A seek while a reset composes supersedes it: that compose is discarded when it comes back,
/// never swapped in, and the newer seek's reset composes and is swapped in instead. No frame shows
/// an hour of the first seek. The newer seek comes once the first compose is inside the layer's
/// compose, so both composes run it. Before, the first compose was swapped in and shown, then the
/// second composed after it.
#[test]
fn a_second_seek_during_a_compose_discards_the_first() {
    let (recording, states) = record(1, BLOCK + 200, busy);
    let entry = open(units(recording, "superseded"), quick()).unwrap();
    let composes = Arc::new(Mutex::new(0));
    let (mut layer, release) = Sum::gated(&composes);
    let (entered, inside) = mpsc::channel();
    layer.entered = Some(entered);
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(layer)], View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(100);
    frames_until(&mut side, |_, frame| frame.resets > 0);
    assert!(side.composing());
    inside.recv_timeout(Duration::from_secs(60)).expect("the first seek's compose began within a minute");
    entry.playhead().seek(300);
    release.send(()).unwrap();
    let mut drawn = Vec::new();
    let mut discarded = false;
    frames_until(&mut side, |_, frame| {
        assert!(frame.swapped.is_none(), "the first seek's compose is never swapped in");
        discarded |= frame.superseded;
        drawn.extend(frame.drawn);
        frame.resets > 0
    });
    assert!(discarded, "the first seek's compose came back and was discarded");
    inside.recv_timeout(Duration::from_secs(60)).expect("the newer seek's compose began within a minute");
    release.send(()).unwrap();
    frames_until(&mut side, |side, frame| {
        drawn.extend(frame.drawn);
        frame.swapped.is_some() && !side.composing()
    });
    assert!(drawn.iter().all(|&hour| hour >= 300), "an hour of the first seek was shown: {drawn:?}");
    assert_eq!(*composes.lock().unwrap(), 2);
    let mirror = side.parts().unwrap().0.get(table(0)).unwrap();
    let head = mirror.head().unwrap();
    assert!(head >= 300, "the mirrors are at the newer seek: {head}");
    assert_eq!(differ(mirror, &states[0][head as usize], &[0]), None);
}

/// A layer whose compose leaves one whole upload, made when given budget at a cost of 4 ms of the
/// test's clock (more than the slice) and logged by name.
struct Whole {
    name: &'static str,
    owed: bool,
    log: Arc<Mutex<Vec<&'static str>>>,
}

impl Whole {
    fn boxed(name: &'static str, log: &Arc<Mutex<Vec<&'static str>>>) -> Box<dyn Layer> {
        Box::new(Self { name, owed: false, log: Arc::clone(log) })
    }

    fn push(&mut self, budget: &mut Budget) -> io::Result<()> {
        if self.owed && budget.left() {
            crate::layers::clock::advance(Duration::from_millis(4));
            self.owed = false;
            self.log.lock().unwrap().push(self.name);
        }
        Ok(())
    }
}

impl Layer for Whole {
    fn name(&self) -> &'static str {
        self.name
    }
    fn inputs(&mut self, _view: &View, _inputs: &mut Inputs<'_>) -> io::Result<()> {
        Ok(())
    }
    fn compose(&mut self, _mirrors: &Mirrors) -> io::Result<()> {
        self.owed = true;
        Ok(())
    }
    fn apply(&mut self, _step: &Step, _mirrors: &Mirrors) {}
    fn settle(&mut self, _mirrors: &Mirrors, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }
    fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }
    fn uploaded(&self) -> bool {
        !self.owed
    }
    fn carried(&self) -> Carried {
        Carried::default()
    }
    fn held(&self) -> usize {
        0
    }
    fn any(&self) -> &dyn std::any::Any {
        self
    }
    fn any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// The upload pass with each whole layer making its upload.
fn whole_uploads(side: &mut MapSide) {
    side.upload_with(false, |layer, budget| match layer.any_mut().and_then(|layer| layer.downcast_mut::<Whole>()) {
        Some(whole) => whole.push(budget),
        None => Ok(()),
    })
    .unwrap();
}

/// After a swap every layer owes its whole upload, each more than the slice: the swap frame makes
/// the political layer's, the one the user looks at first, and carries the rest to the next
/// frames; the political layer shows the picture made since the swap, the map not yet. Before,
/// the uploads went in registration order, so the layer registered before it went first.
#[test]
fn the_swap_frame_uploads_the_political_layer_first_and_carries_the_rest() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut side = drawn_on_held_clock("seen first", vec![Whole::boxed("buildings", &log), Whole::boxed("political", &log)]);
    whole_uploads(&mut side);
    assert_eq!(*log.lock().unwrap(), ["political"]);
    let last = side.last_frame();
    assert!(last.layers[1].upload_carried && !last.layers[2].upload_carried, "the rest are carried");
    assert!(last.layers[2].reset_shown && !last.reset_shown, "the political picture is the new one, the map's not yet");
    let head = side.applied().unwrap();
    side.frame(head).unwrap();
    whole_uploads(&mut side);
    assert_eq!(*log.lock().unwrap(), ["political", "buildings"]);
    assert!(side.last_frame().reset_shown, "every layer shows the picture made since the swap");
}

/// A reader that draws nothing, as the rows worker's: it declares no table and takes the steps.
fn bare_consumer(entry: &super::Entry) -> playhead::Consumer {
    let reader = entry.playhead().declare(Vec::new(), None).unwrap();
    entry.playhead().consumer(reader)
}

/// Takes the steps `head` reaches until a reset is among them, a minute at most.
fn take_reset(consumer: &mut playhead::Consumer, head: i32) {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut taken: Vec<Arc<Step>> = Vec::new();
    loop {
        consumer.take(head, &mut taken);
        if taken.iter().any(|step| step.reset()) {
            return;
        }
        taken.clear();
        assert!(Instant::now() < deadline, "no reset reached hour {head} within a minute");
        std::thread::yield_now();
    }
}

/// The shared clock is not held at the seek to `hour`: it runs on from it.
fn runs_from(entry: &super::Entry, hour: i32, held_by: &str) {
    assert_eq!(entry.playhead().holding(), None, "the clock is held at the seek by {held_by}");
    let later = Instant::now() + Duration::from_secs(10);
    assert!(entry.playhead().head(later) > hour, "the head stays at {hour}, held by {held_by}");
}

/// A map with no GPU, which never uploads, takes a seek's reset and composes it for as long as the
/// test holds the compose: the shared clock runs from the take, not from the map's picture. Its
/// swap shows the hour sought, the steps since left for the next frame. Before, the map's consumer
/// held the clock at the hour sought until the compose was swapped in.
#[test]
fn a_map_that_never_uploads_holds_no_clock_while_it_composes() {
    let (recording, _) = record(1, BLOCK, busy);
    let entry = open(units(recording, "never uploads"), quick()).unwrap();
    let (layer, release) = Sum::gated(&Arc::new(Mutex::new(0)));
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(layer)], View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(100);
    frames_until(&mut side, |side, frame| frame.resets > 0 && side.composing());
    runs_from(&entry, 100, "a map composing its reset");
    release.send(()).unwrap();
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    assert_eq!(side.applied(), Some(100), "the swap shows the picture composed for the hour sought");
}

/// A map that is never drawn (a hidden window, a map widget scrolled out of view) beside a reader
/// that draws nothing (the rows worker's): the reader's take of the reset starts the clock. Before,
/// the hidden map's consumer held the clock until a frame of its own showed the reset, which never
/// came.
#[test]
fn a_hidden_map_holds_no_clock_for_a_reader_beside_it() {
    let (recording, _) = record(1, BLOCK, busy);
    let entry = open(units(recording, "hidden map"), quick()).unwrap();
    let hidden = MapSide::new(Arc::clone(&entry), vec![Box::new(Sum::new(&Arc::new(Mutex::new(0))))], View::default()).unwrap();
    let mut rows = bare_consumer(&entry);
    entry.playhead().play(4.0);
    entry.playhead().seek(100);
    take_reset(&mut rows, 100);
    runs_from(&entry, 100, "a map never drawn");
    drop(hidden);
}

/// A map whose every layer is refused draws nothing: beside a reader that draws nothing, the
/// reader's take starts the clock, and the map's own frames take the reset with nothing to compose.
/// Before, the refused map's consumer held the clock until a frame of its own showed the reset.
#[test]
fn a_map_with_every_layer_refused_holds_no_clock() {
    let (recording, _) = record(1, BLOCK, busy);
    let entry = open(units(recording, "every layer refused"), quick()).unwrap();
    let mut refused = Sum::new(&Arc::new(Mutex::new(0)));
    refused.refuse_high = true;
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(refused)], View { camera_height: 2.0, ..View::default() }).unwrap();
    assert_eq!(side.refusals().count(), 1);
    let mut rows = bare_consumer(&entry);
    entry.playhead().play(4.0);
    entry.playhead().seek(100);
    take_reset(&mut rows, 100);
    runs_from(&entry, 100, "a map with every layer refused");
    frames_until(&mut side, |side, _| side.applied().is_some_and(|hour| hour >= 100));
    assert!(!side.composing(), "nothing to compose");
}

/// Two maps on one playhead: the clock is the app's, one for both, and a seek moves both. Each
/// composes the reset on its own; the second for as long as the test holds it. The clock runs from
/// the first take of the reset, and the first map shows the hour sought at its swap and plays on
/// while the second still composes, its old picture on screen meanwhile. Before, the clock stayed
/// at the hour sought until the first map's swap, though both had taken the reset.
#[test]
fn a_map_composing_slowly_holds_neither_the_clock_nor_another_map_on_its_playhead() {
    let (recording, _) = record(1, BLOCK, busy);
    let entry = open(units(recording, "slow second map"), quick()).unwrap();
    let mut first = MapSide::new(Arc::clone(&entry), vec![Box::new(Sum::new(&Arc::new(Mutex::new(0))))], View::default()).unwrap();
    let (slow, release) = Sum::gated(&Arc::new(Mutex::new(0)));
    let mut second = MapSide::new(Arc::clone(&entry), vec![Box::new(slow)], View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(100);
    let deadline = Instant::now() + Duration::from_secs(120);
    // One head a round, so both maps take the reset in the same round, before either swaps.
    loop {
        let head = entry.playhead().newest().unwrap_or(0);
        let (one, other) = (first.frame(head).unwrap(), second.frame(head).unwrap());
        if one.resets > 0 && other.resets > 0 {
            break;
        }
        assert!(Instant::now() < deadline, "the maps did not take the reset within two minutes");
        std::thread::yield_now();
    }
    runs_from(&entry, 100, "the maps' composes");
    loop {
        let head = entry.playhead().newest().unwrap_or(0);
        first.frame(head).unwrap();
        second.frame(head).unwrap();
        if !first.composing() && first.applied().is_some_and(|hour| hour >= 100 + 3 * 4) {
            break;
        }
        assert!(Instant::now() < deadline, "the first map did not play past the hour sought while the second composed");
        std::thread::yield_now();
    }
    assert!(second.composing(), "the second map still composes, its old picture on screen");
    release.send(()).unwrap();
}

/// The dashboard's follow host beside a map that is not drawn: the host's driver takes the reset,
/// which starts the clock. Before, the map's consumer held the clock until a frame of the map
/// showed the reset, so the followed widgets waited on a map that never drew.
#[test]
fn a_follow_host_is_not_held_by_a_map_beside_it() {
    let (recording, _) = record(1, BLOCK, busy);
    let entry = open(units(recording, "follow beside a map"), quick()).unwrap();
    let hidden = MapSide::new(Arc::clone(&entry), vec![Box::new(Sum::new(&Arc::new(Mutex::new(0))))], View::default()).unwrap();
    let held = dashboard_follow::host::held_tables(&**entry.pages(), entry.kinds().len());
    let anchored = |entry: &super::Entry, anchor: dashboard_follow::registry::Anchor| entry.anchor(anchor.hour, anchor.playing, anchor.speed, anchor.sought);
    let due = dashboard_follow::due::Due::new(Arc::new(dashboard_follow::push::Push::default()));
    let host = dashboard_follow::host::PlayheadHost::new(Arc::clone(&entry), super::Entry::playhead, anchored, super::Entry::drawn, dashboard_follow::Follow::new(64 << 10), due, held).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(100);
    let deadline = Instant::now() + Duration::from_secs(60);
    while entry.playhead().holding().is_some() {
        assert!(Instant::now() < deadline, "the clock was still held at the seek a minute later: held by the map beside the follow host");
        std::thread::yield_now();
    }
    runs_from(&entry, 100, "the map beside the follow host");
    drop(host);
    drop(hidden);
}

/// What the test lets a fed worker do: every job, or this many more.
type Permits = Arc<(Mutex<Option<u32>>, std::sync::Condvar)>;

/// A job for [`Ahead`]'s worker.
enum Fold {
    Reset { epoch: u64, values: Vec<Option<i32>> },
    Step { epoch: u64, step: Arc<Step> },
}

/// A product of [`Ahead`]'s worker: its compose's epoch, the hour it is current at (none: the
/// compose's) and the sum of column 0 over live rows there.
type Summed = (u64, Option<i32>, i64);

/// A layer fed the steps ahead: its worker keeps column 0 of table 0 per slot from the steps' change
/// sets alone and makes, for each step, the sum over live rows, stamped with the step's last hour
/// and its compose's epoch. Each job waits for a permit the test gives.
struct Ahead {
    fed: crate::layers::Fed<Summed>,
    epoch: u64,
    jobs: Option<mpsc::Sender<Fold>>,
    made: Option<mpsc::Receiver<Summed>>,
    permits: Permits,
    /// The product shown, and the newest hour of any product received, taken or not.
    shown: Option<Summed>,
    made_through: Option<i32>,
    changed: crate::layers::Changed,
}

impl Ahead {
    /// With `permits` jobs allowed its worker (none: every job).
    fn new(permits: Option<u32>) -> (Self, Permits) {
        let permits: Permits = Arc::new((Mutex::new(permits), std::sync::Condvar::new()));
        (Self { fed: crate::layers::Fed::default(), epoch: 0, jobs: None, made: None, permits: Arc::clone(&permits), shown: None, made_through: None, changed: crate::layers::Changed::default() }, permits)
    }

    fn run(jobs: mpsc::Receiver<Fold>, made: mpsc::Sender<Summed>, permits: Permits) {
        let (mut values, mut epoch, mut stamp): (Vec<Option<i32>>, u64, Option<i32>) = (Vec::new(), 0, None);
        for job in jobs {
            {
                let (held, wake) = &*permits;
                let mut left = held.lock().unwrap();
                while *left == Some(0) {
                    left = wake.wait(left).unwrap();
                }
                if let Some(left) = left.as_mut() {
                    *left -= 1;
                }
            }
            match job {
                Fold::Reset { epoch: next, values: whole } => (epoch, values, stamp) = (next, whole, None),
                Fold::Step { epoch: of, step } if of == epoch => {
                    if let Some(changes) = step.table(0) {
                        for &slot in changes.deleted().slots() {
                            values[slot as usize] = None;
                        }
                        for written in changes.columns().iter().filter(|written| written.column() == 0) {
                            for (slot, value) in written.values() {
                                if values.len() <= slot as usize {
                                    values.resize(slot as usize + 1, None);
                                }
                                values[slot as usize] = match value {
                                    Value::Number(bits) => Some(bits as i32),
                                    _ => None,
                                };
                            }
                        }
                    }
                    stamp = stamp.max(Some(step.last()));
                }
                Fold::Step { .. } => continue,
            }
            if made.send((epoch, stamp, values.iter().flatten().copied().map(i64::from).sum())).is_err() {
                return;
            }
        }
    }

    /// Takes in what the worker made, with `wait` until the product of the hour applied arrived,
    /// and shows what is due.
    fn exchange(&mut self, wait: bool) {
        if let Some(receive) = &self.made {
            loop {
                let made = if wait && !self.fed.arrived() { receive.recv().ok() } else { receive.try_recv().ok() };
                let Some(made) = made else { break };
                if made.0 == self.epoch {
                    self.made_through = self.made_through.max(made.1);
                    self.fed.receive(made.1, made);
                }
            }
        }
        let (mut shown, mut changed) = (self.shown, Vec::new());
        self.fed.take(|made| {
            shown = Some(made);
            changed.extend(made.1);
        });
        self.shown = shown;
        for hour in changed {
            self.changed.mark("hour", hour as u64);
        }
    }
}

/// Lets `permits` jobs more run on a fed worker; none: every job from now on.
fn permit(permits: &Permits, more: Option<u32>) {
    let (held, wake) = &**permits;
    let mut left = held.lock().unwrap();
    *left = more.map(|more| left.unwrap_or(0) + more);
    wake.notify_all();
}

impl Layer for Ahead {
    fn name(&self) -> &'static str {
        "ahead"
    }

    fn inputs(&mut self, _view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        inputs.table(table(0), true)?;
        inputs.index(table(0), 0)
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        if self.jobs.is_none() {
            let (jobs, receive) = mpsc::channel();
            let (send, made) = mpsc::channel();
            let permits = Arc::clone(&self.permits);
            std::thread::Builder::new().name("ahead test fold".into()).spawn(move || Ahead::run(receive, send, permits))?;
            (self.jobs, self.made) = (Some(jobs), Some(made));
        }
        let mirror = mirrors.get(table(0)).unwrap();
        let column = Num::of(table(0), 0);
        let values = (0..mirror.slots() as u32).map(|slot| if mirror.live(slot) { mirror.number(column, slot).value() } else { None }).collect();
        self.epoch += 1;
        (self.shown, self.made_through) = (None, None);
        self.fed.compose(mirror.head());
        self.jobs.as_ref().unwrap().send(Fold::Reset { epoch: self.epoch, values }).unwrap();
        self.exchange(true);
        Ok(())
    }

    fn apply(&mut self, step: &Step, _mirrors: &Mirrors) {
        self.fed.applied(step);
    }

    fn wants_ahead(&self) -> bool {
        true
    }

    fn ahead(&mut self, steps: &[Arc<Step>]) {
        let jobs = self.jobs.as_ref().unwrap();
        for step in self.fed.hand(steps) {
            jobs.send(Fold::Step { epoch: self.epoch, step: Arc::clone(step) }).unwrap();
        }
    }

    fn settle(&mut self, _mirrors: &Mirrors, budget: &mut Budget) -> io::Result<()> {
        self.exchange(budget.unlimited_now());
        Ok(())
    }

    fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }

    fn carried(&self) -> Carried {
        Carried::default()
    }

    fn published(&self) -> crate::layers::Published {
        self.fed.published()
    }

    fn held(&self) -> usize {
        0
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }

    fn changed(&self) -> Option<&crate::layers::Changed> {
        Some(&self.changed)
    }

    fn clear_changed(&mut self) {
        self.changed.clear();
    }
}

/// A layer reading [`Ahead`]: per frame, the last hour of the step it applied in it (if any) and
/// the hours [`Ahead`] published as changed when it settled.
struct Hears {
    stepped: Option<i32>,
    heard: Vec<(Option<i32>, Vec<u64>)>,
}

impl Layer for Hears {
    fn name(&self) -> &'static str {
        "hears"
    }
    fn inputs(&mut self, _view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        inputs.after("ahead");
        Ok(())
    }
    fn compose(&mut self, _mirrors: &Mirrors) -> io::Result<()> {
        Ok(())
    }
    fn apply(&mut self, step: &Step, _mirrors: &Mirrors) {
        self.stepped = Some(step.last());
    }
    fn settle(&mut self, _mirrors: &Mirrors, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }
    fn settle_with(&mut self, _mirrors: &Mirrors, upstream: &super::Upstream<'_>, _budget: &mut Budget) -> io::Result<()> {
        self.heard.push((self.stepped.take(), upstream.changed::<Ahead>().of("hour").to_vec()));
        Ok(())
    }
    fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }
    fn carried(&self) -> Carried {
        Carried::default()
    }
    fn held(&self) -> usize {
        0
    }
    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

fn ahead(side: &MapSide) -> &Ahead {
    side.parts().unwrap().1.iter().find_map(|layer| layer.any().downcast_ref::<Ahead>()).unwrap()
}

/// A map drawing [`Ahead`] (and [`Hears`] after it), its worker allowed `permits` jobs (none: every
/// one), played from hour 0 on a running clock and composed; the frames' heads are the test's.
fn ahead_side(name: &str, permits: Option<u32>) -> (MapSide, Arc<super::Entry>, Permits) {
    let (recording, _) = record(1, BLOCK, busy);
    let entry = open(units(recording, name), quick()).unwrap();
    let (layer, permits) = Ahead::new(permits);
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(layer), Box::new(Hears { stepped: None, heard: Vec::new() })], View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    (side, entry, permits)
}

/// Frames at the hour applied (taking no step) until [`Ahead`]'s worker has made the product of
/// `hour`: the frames hand it the steps the playhead built past the head.
fn made_ahead(side: &mut MapSide, hour: i32) {
    for _ in 0..50_000_000u64 {
        let applied = side.applied().unwrap();
        side.frame(applied).unwrap();
        if ahead(side).made_through >= Some(hour) {
            return;
        }
        std::thread::yield_now();
    }
    panic!("the worker never made the product of hour {hour}");
}

/// Frames at `hour` until [`Ahead`] shows it.
fn until_current(side: &mut MapSide, hour: i32) {
    for _ in 0..50_000_000u64 {
        side.frame(hour).unwrap();
        if ahead(side).published() == crate::layers::Published::Current {
            return;
        }
        std::thread::yield_now();
    }
    panic!("the layer never showed hour {hour}");
}

/// The sum of column 0 over table 0's live rows in the map's mirror now.
fn mirrored(side: &MapSide) -> i64 {
    sum_of(side.parts().unwrap().0.get(table(0)).unwrap(), Num::of(table(0), 0))
}

/// Playing at a step a frame, a layer fed the steps ahead shows, in the frame that applies a step,
/// its worker's product of that step's hour: current, the map drawn at the hour applied, and equal
/// to the mirror there. Before, the layer handed the step to its worker in that frame and showed
/// the step before (the map a step behind the head).
#[test]
fn a_step_made_ahead_is_shown_current_in_the_frame_that_applies_it() {
    let (mut side, _entry, _permits) = ahead_side("ahead current", None);
    for _ in 0..10 {
        let next = side.applied().unwrap() + 4;
        made_ahead(&mut side, next);
        let frame = side.frame(next).unwrap();
        assert_eq!((frame.applied, frame.steps), (Some(next), 1), "one step a frame");
        assert_eq!(frame.layers[0].ready_at_hour, Some(true), "the product of hour {next} was ready in the frame that applied it");
        assert_eq!(ahead(&side).published(), crate::layers::Published::Current);
        assert_eq!(frame.drawn, Some(next), "the map is drawn at the hour applied");
        assert_eq!(ahead(&side).shown.map(|(_, hour, sum)| (hour, sum)), Some((Some(next), mirrored(&side))), "the product shown is the mirror's at {next}");
    }
}

/// With its worker held, a fed layer shows the newest hour its worker made and never a later one,
/// and the frame does not wait for it: products made ahead are shown only at their hour, and a step
/// the worker has not folded leaves the layer at the hour before, with the frame's record saying so.
#[test]
fn a_held_worker_leaves_the_layer_at_the_newest_hour_it_made_and_never_shows_one_ahead() {
    let (mut side, entry, permits) = ahead_side("ahead held", Some(1));
    let from = side.applied().unwrap();
    for _ in 0..50_000_000u64 {
        if entry.playhead().newest() >= Some(from + 12) {
            break;
        }
        std::thread::yield_now();
    }
    permit(&permits, Some(2));
    made_ahead(&mut side, from + 8);
    let frame = side.frame(from + 4).unwrap();
    assert_eq!(frame.applied, Some(from + 4));
    assert_eq!(ahead(&side).shown.map(|(_, hour, _)| hour), Some(Some(from + 4)), "the product of {} is made; not shown before its hour", from + 8);
    assert_eq!(frame.drawn, Some(from + 4));
    let frame = side.frame(from + 8).unwrap();
    assert_eq!((frame.drawn, frame.layers[0].ready_at_hour), (Some(from + 8), Some(true)));
    // The worker is held until the permit below: a frame that waited for it would never return.
    let frame = side.frame(from + 12).unwrap();
    assert_eq!(frame.applied, Some(from + 12));
    assert_eq!(frame.layers[0].ready_at_hour, Some(false), "the step to {} is not folded", from + 12);
    assert_eq!(ahead(&side).published(), crate::layers::Published::At(from + 8));
    assert_eq!(frame.drawn, Some(from + 8), "the map stays at the newest hour the worker made");
    let shown = ahead(&side).shown.unwrap();
    assert_eq!(shown.1, Some(from + 8));
    permit(&permits, None);
    until_current(&mut side, from + 12);
    assert_eq!(ahead(&side).shown.map(|(_, hour, sum)| (hour, sum)), Some((Some(from + 12), mirrored(&side))));
}

/// Two steps taken in one frame show the second's product, made ahead.
#[test]
fn two_steps_taken_in_one_frame_show_the_seconds_product() {
    let (mut side, _entry, _permits) = ahead_side("ahead two", None);
    let from = side.applied().unwrap();
    made_ahead(&mut side, from + 8);
    let frame = side.frame(from + 8).unwrap();
    assert_eq!((frame.applied, frame.steps), (Some(from + 8), 2));
    assert_eq!(frame.layers[0].ready_at_hour, Some(true));
    assert_eq!(ahead(&side).shown.map(|(_, hour, sum)| (hour, sum)), Some((Some(from + 8), mirrored(&side))));
    assert_eq!(frame.drawn, Some(from + 8));
}

/// A seek while products made ahead wait for their hours shows none of them: after the seek's
/// compose, every product shown is of that compose, and current at the hour applied.
#[test]
fn a_seek_shows_none_of_the_products_made_ahead_before_it() {
    let (mut side, entry, _permits) = ahead_side("ahead seek", None);
    let from = side.applied().unwrap();
    made_ahead(&mut side, from + 8);
    let before = ahead(&side).epoch;
    entry.playhead().seek(from + 2);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    let epoch = ahead(&side).epoch;
    assert!(epoch > before, "the seek composed the layer again");
    for _ in 0..6 {
        let next = side.applied().unwrap() + 4;
        made_ahead(&mut side, next);
        side.frame(next).unwrap();
        let shown = ahead(&side).shown.unwrap();
        assert_eq!(shown.0, epoch, "a product made before the seek was shown at hour {next}");
        assert_eq!((shown.1, shown.2), (Some(next), mirrored(&side)));
    }
}

/// A dependent hears what a fed layer changed for a step in the frame that applies the step, not
/// a frame later.
#[test]
fn a_dependent_hears_a_fed_layers_change_in_the_frame_of_its_step() {
    let (mut side, _entry, _permits) = ahead_side("ahead heard", None);
    for _ in 0..5 {
        let next = side.applied().unwrap() + 4;
        made_ahead(&mut side, next);
        side.frame(next).unwrap();
        let hears = side.parts().unwrap().1[1].any().downcast_ref::<Hears>().unwrap();
        let (stepped, heard) = hears.heard.last().unwrap();
        assert_eq!(*stepped, Some(next), "the dependent settled in the frame that applied {next}");
        assert!(heard.contains(&(next as u64)), "the change of hour {next} reached it in that frame: {heard:?}");
    }
}

/// A layer current with the host but owing marks: each step whose last hour is at or past `from`
/// marks it, and a settle takes a mark only once it has stood `lag` of the layer's settles. Its age
/// is counted in its settles, the frame advancing when a settle is done, as the layers do.
struct Owes {
    from: i32,
    lag: u64,
    /// Steps it marks at most; none: every one from `from` on.
    marks: Option<u32>,
    frame: u64,
    owed: std::collections::VecDeque<u64>,
}

impl Owes {
    fn new(from: i32, lag: u64, marks: Option<u32>) -> Self {
        Self { from, lag, marks, frame: 0, owed: std::collections::VecDeque::new() }
    }
}

impl Layer for Owes {
    fn name(&self) -> &'static str {
        "owes"
    }
    fn inputs(&mut self, _view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        inputs.table(table(0), false)?;
        inputs.index(table(0), 0)
    }
    fn compose(&mut self, _mirrors: &Mirrors) -> io::Result<()> {
        self.owed.clear();
        Ok(())
    }
    fn apply(&mut self, step: &Step, _mirrors: &Mirrors) {
        if step.last() >= self.from && self.marks != Some(0) {
            self.owed.push_back(self.frame);
            self.marks = self.marks.map(|left| left - 1);
        }
    }
    fn settle(&mut self, _mirrors: &Mirrors, _budget: &mut Budget) -> io::Result<()> {
        while self.owed.front().is_some_and(|&marked| self.frame - marked >= self.lag) {
            self.owed.pop_front();
        }
        self.frame += 1;
        Ok(())
    }
    fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }
    fn carried(&self) -> Carried {
        Carried { entities: self.owed.len(), frames: self.owed.front().map_or(0, |&marked| self.frame - marked) }
    }
    fn held(&self) -> usize {
        0
    }
    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A frame taking the next step of four hours on the held clock: its applied hour.
fn next_step(side: &mut MapSide) -> (super::Frame, i32) {
    let head = side.applied().unwrap() + 4;
    for _ in 0..50_000_000u64 {
        if side.entry().playhead().newest().is_some_and(|newest| newest >= head) {
            break;
        }
        std::thread::yield_now();
    }
    let frame = side.frame(head).unwrap();
    assert_eq!(frame.applied, Some(head), "one step a frame");
    (frame, head)
}

/// A layer that owes one mark for `k` of its settles, then takes it: meanwhile it shows the hour
/// before the step that marked it (k frames ago, one step a frame), then the hour applied.
#[test]
fn a_layer_owing_one_mark_for_k_settles_shows_the_hour_before_it_then_the_hour_applied() {
    let from = 8;
    let side = &mut drawn_on_held_clock("owes once", vec![Box::new(Owes::new(from, 3, Some(1)))]);
    while side.applied().unwrap() + 4 < from {
        next_step(side);
    }
    let before = side.applied().unwrap();
    for frames in 1..=3 {
        let (frame, applied) = next_step(side);
        assert_eq!(frame.layers[1].carried, 1, "the mark is owed {frames} settles");
        assert_eq!(frame.layers[1].shown, Some(before), "owing the mark from the step after {before}, it shows {before}, not {applied}");
        assert_eq!(frame.drawn, Some(before));
    }
    let (frame, applied) = next_step(side);
    assert_eq!(frame.layers[1].carried, 0);
    assert_eq!((frame.layers[1].shown, frame.drawn), (Some(applied), Some(applied)), "taken, it shows the hour applied");
}

/// A layer that always owes the mark of the step before the last (it takes each a settle late):
/// it shows the hour of the step before, moving every frame. Before, a layer current and carrying
/// anything kept the hour it last carried nothing, for as long as it carried: here, for ever.
#[test]
fn a_layer_always_one_step_behind_shows_the_step_before_and_moves_with_the_head() {
    let side = &mut drawn_on_held_clock("owes always", vec![Box::new(Owes::new(i32::MIN, 1, None))]);
    let mut before = side.applied().unwrap();
    for _ in 0..12 {
        let (frame, applied) = next_step(side);
        assert_eq!(frame.layers[1].carried, 1, "it owes the step it just applied");
        assert_eq!(frame.layers[1].shown, Some(before), "it shows the hour before the mark it owes");
        assert_eq!(frame.drawn, Some(before));
        before = applied;
    }
}

/// A layer that owes nothing shows the hour applied every frame.
#[test]
fn a_layer_owing_nothing_shows_the_hour_applied() {
    let side = &mut drawn_on_held_clock("owes nothing", vec![Box::new(Owes::new(i32::MAX, 1, None))]);
    for _ in 0..6 {
        let (frame, applied) = next_step(side);
        assert_eq!((frame.layers[1].carried, frame.layers[1].shown, frame.drawn), (0, Some(applied), Some(applied)));
    }
}

/// A layer that counts its declarations; with `takes`, it takes a moved camera itself, its
/// declaration deciding only whether the camera is below a height of 100.
struct Camera {
    name: &'static str,
    takes: bool,
    declared: Arc<AtomicU32>,
}

impl Layer for Camera {
    fn name(&self) -> &'static str {
        self.name
    }
    fn placed(&mut self, view: &View) -> Option<u64> {
        self.takes.then(|| u64::from(view.camera_height < 100.0))
    }
    fn inputs(&mut self, _view: &View, _inputs: &mut Inputs<'_>) -> io::Result<()> {
        self.declared.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
    fn compose(&mut self, _mirrors: &Mirrors) -> io::Result<()> {
        Ok(())
    }
    fn apply(&mut self, _step: &Step, _mirrors: &Mirrors) {}
    fn settle(&mut self, _mirrors: &Mirrors, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }
    fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }
    fn carried(&self) -> Carried {
        Carried::default()
    }
    fn held(&self) -> usize {
        0
    }
    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

/// The camera moving with nothing else of the view changing declares no layer that takes the
/// placement itself, and only again alone the one that reads the camera in its inputs; a camera
/// crossing what a layer's declaration decides, or another change of the view, declares the view
/// whole. Before, every frame the camera moved ran every layer's inputs, and what they did there (a
/// layer marking everything to make again) at a held hour.
#[test]
fn a_moved_camera_declares_again_only_what_the_camera_decides() {
    let (recording, _) = record(1, BLOCK, busy);
    let entry = open(units(recording, "camera placed"), quick()).unwrap();
    let (taker, reader) = (Arc::new(AtomicU32::new(0)), Arc::new(AtomicU32::new(0)));
    let layers: Vec<Box<dyn Layer>> = vec![
        Box::new(Camera { name: "taker", takes: true, declared: Arc::clone(&taker) }),
        Box::new(Camera { name: "reader", takes: false, declared: Arc::clone(&reader) }),
    ];
    let mut side = MapSide::new(Arc::clone(&entry), layers, View { camera_height: 10.0, ..View::default() }).unwrap();
    let counts = || (taker.load(Ordering::Relaxed), reader.load(Ordering::Relaxed));
    assert_eq!(counts(), (1, 1));
    side.set_view(View { camera_height: 50.0, viewport: [800, 600], ..View::default() }).unwrap();
    assert_eq!(counts(), (1, 2), "a moved camera: the taker takes it, the reader declares again alone");
    side.set_view(View { camera_height: 150.0, viewport: [800, 600], ..View::default() }).unwrap();
    assert_eq!(counts(), (2, 3), "the camera crossed what the taker's declaration decides: the view is declared whole");
    side.set_view(View { camera_height: 150.0, viewport: [800, 600], fog: !View::default().fog, ..View::default() }).unwrap();
    assert_eq!(counts(), (3, 4), "another change of the view declares it whole");
}
