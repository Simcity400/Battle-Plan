use super::{Consumer, Declared, Playhead, Reader, Settings, Timeline};
use crate::{
    LogState, Wanted,
    changes::Changes,
    lane::{
        Lane,
        log::tests::{Did, LOG, handed, read_directly, record_log, states as log_states},
        tests::{KINDS, Mirror, Owned, busy, owned, record_run, rows},
    },
    pages::{Pages, Run},
    state::{Rebuild, State},
    step::Step,
};
use replay_columnar::{
    state::{Cell, Kind},
    store::{TableChanges, TableState},
    unit::{BLOCK, Settings as Writing, reader::Extents},
};
use std::{
    collections::BTreeMap,
    io,
    ops::Deref,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

/// A run's frames; reads overlapping a closed range wait until it opens, counted while they wait,
/// and fail if it opened failing. Every read is counted.
struct Gated {
    bytes: Vec<u8>,
    closed: Mutex<(Option<(u64, u64)>, usize, bool)>,
    opened: Condvar,
    reads: AtomicUsize,
}

impl Gated {
    fn close(&self, from: u64, to: u64) {
        self.closed.lock().unwrap().0 = Some((from, to));
    }

    fn open(&self) {
        self.closed.lock().unwrap().0 = None;
        self.opened.notify_all();
    }

    /// Opens the closed range with every read waiting at it failing.
    fn fail(&self) {
        let mut closed = self.closed.lock().unwrap();
        closed.0 = None;
        closed.2 = true;
        self.opened.notify_all();
    }

    /// Returns once a read waits at the closed range.
    fn wait_blocked(&self) {
        let mut closed = self.closed.lock().unwrap();
        while closed.1 == 0 {
            closed = self.opened.wait(closed).unwrap();
        }
    }
}

impl Extents for Gated {
    fn read_at(&self, offset: u64, length: usize, out: &mut Vec<u8>) -> io::Result<()> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        let mut closed = self.closed.lock().unwrap();
        let shut = |closed: &(Option<(u64, u64)>, usize, bool)| closed.0.is_some_and(|(from, to)| offset < to && offset + length as u64 > from);
        if shut(&closed) {
            closed.1 += 1;
            self.opened.notify_all();
            while shut(&closed) {
                closed = self.opened.wait(closed).unwrap();
            }
            closed.1 -= 1;
            if closed.2 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "a damaged extent"));
            }
        }
        drop(closed);
        self.bytes.read_at(offset, length, out)
    }
}

struct Runs {
    extents: Vec<Gated>,
    runs: Vec<Run>,
    kinds: Vec<Vec<Kind>>,
}

impl Pages for Runs {
    fn extents(&self, run: usize) -> &dyn Extents {
        &self.extents[run]
    }

    fn runs(&self) -> &[Run] {
        &self.runs
    }

    fn kinds(&self, table: usize) -> Option<&[Kind]> {
        self.kinds.get(table).map(Vec::as_slice)
    }
}

fn gated(bytes: Vec<u8>) -> Gated {
    Gated { bytes, closed: Mutex::new((None, 0, false)), opened: Condvar::new(), reads: AtomicUsize::new(0) }
}

/// Runs of `tables` tables, each `(first hour, hours)`, every table [`busy`]; each run's states.
fn recording(writing: Writing, tables: usize, runs: &[(i32, u32)]) -> (Arc<Runs>, Vec<Vec<Vec<TableState>>>) {
    recording_of(writing, tables, runs, |at, _, changes| busy(at, changes))
}

/// [`recording`], each hour of each table written by `hour`.
fn recording_of(writing: Writing, tables: usize, runs: &[(i32, u32)], hour: impl Fn(u32, usize, &mut TableChanges<'_>)) -> (Arc<Runs>, Vec<Vec<Vec<TableState>>>) {
    let mut pages = Runs { extents: Vec::new(), runs: Vec::new(), kinds: vec![KINDS.to_vec(); tables] };
    let mut states = Vec::new();
    for &(first, hours) in runs {
        let (bytes, run, held) = record_run(writing, tables, first, hours, &hour);
        pages.extents.push(gated(bytes));
        pages.runs.push(run);
        states.push(held);
    }
    (Arc::new(pages), states)
}

/// A step a day, one frame a second, two steps read ahead.
fn settings() -> Settings {
    Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 24.0, rate: 1.0 }
}

/// A reader's state of one table: its columns of every live row by slot, kept from change sets.
#[derive(Clone, Debug, Default, PartialEq)]
struct Kept {
    columns: Vec<usize>,
    rows: BTreeMap<u32, (u128, Vec<Owned>)>,
    head: Option<i32>,
}

impl Kept {
    fn rebuilt(lane: &Lane, columns: &[usize]) -> Self {
        let cells = lane.cells();
        let rows = cells.rows().map(|slot| (slot, (cells.key(slot).expect("keyed"), columns.iter().map(|&column| owned(cells.value(column, slot))).collect()))).collect();
        Self { columns: columns.to_vec(), rows, head: lane.head() }
    }

    /// A change set, its columns found by index; a reset names no row.
    fn apply(&mut self, changes: &Changes) {
        for (at, &slot) in changes.deleted().slots().iter().enumerate() {
            assert_eq!(self.rows.remove(&slot).map(|(key, _)| key), Some(changes.deleted().keys().expect("keyed")[at]), "a delete names the row held");
        }
        for (at, &slot) in changes.inserted().slots().iter().enumerate() {
            let fresh = (changes.inserted().keys().expect("keyed")[at], vec![Owned::Unrecorded; self.columns.len()]);
            assert!(self.rows.insert(slot, fresh).is_none(), "an insert names a free slot");
        }
        for (at, &column) in self.columns.iter().enumerate() {
            let written = changes.columns().iter().find(|written| written.column() == column).expect("a change set holds every column its readers declared");
            for (slot, value) in written.values() {
                self.rows.get_mut(&slot).expect("a written slot holds a row").1[at] = owned(value);
            }
        }
        self.head = changes.head();
    }

    /// Its rows by key, as [`rows`] gives the recorder's, of its columns.
    fn by_key(&self) -> BTreeMap<u128, Vec<String>> {
        self.rows.values().map(|(key, cells)| (*key, cells.iter().map(|cell| format!("{:?}", cell.value())).collect())).collect()
    }

    fn holds(&self, state: &TableState) -> bool {
        let wanted: BTreeMap<u128, Vec<String>> = rows(state).into_iter().map(|(key, cells)| (key, self.columns.iter().map(|&column| cells[column].clone()).collect())).collect();
        self.by_key() == wanted
    }
}

/// Which reader rebuilt which table, in order.
type Log = Arc<Mutex<Vec<(u64, usize)>>>;

/// A rebuild of [`Kept`] that logs itself as `tag`.
fn kept(tag: u64, log: &Log) -> Rebuild {
    let log = Arc::clone(log);
    Arc::new(move |lane: &Lane, columns: &[usize]| {
        log.lock().unwrap().push((tag, lane.table()));
        Ok(State::new(Kept::rebuilt(lane, columns), 0))
    })
}

fn every(tables: &[usize]) -> Vec<Declared> {
    tables.iter().map(|&table| Declared { table, columns: (0..KINDS.len()).collect(), keyed: true, passing: false }).collect()
}

/// A step taken, with the states its consumer took from it.
struct Taken {
    step: Arc<Step>,
    states: BTreeMap<usize, Kept>,
}

impl Deref for Taken {
    type Target = Step;

    fn deref(&self) -> &Step {
        &self.step
    }
}

/// [`Consumer::take`], each reset table's state taken with its step.
fn take(consumer: &mut Consumer, head: i32, out: &mut Vec<Taken>) {
    let mut steps = Vec::new();
    consumer.take(head, &mut steps);
    for step in steps {
        let states = step.tables().iter().filter(|changes| changes.reset()).filter_map(|changes| consumer.state(&step, changes.table()).map(|state| (changes.table(), *state.downcast::<Kept>().expect("a kept state")))).collect();
        out.push(Taken { step, states });
    }
}

fn start(pages: &Arc<Runs>, settings: Settings, tables: &[usize], log: &Log) -> (Playhead, Consumer) {
    let playhead = Playhead::new(Arc::clone(pages) as Arc<dyn Pages>, Timeline::whole(&**pages).unwrap(), settings, None).unwrap();
    let reader = playhead.declare(every(tables), Some(kept(0, log))).unwrap();
    let consumer = playhead.consumer(reader);
    playhead.seek(playhead.timeline().first());
    (playhead, consumer)
}

/// Plays as frames do: the head moves `every` hours a frame from `from` to `to`, each frame waits
/// until the playhead has built through its head and takes the steps it reaches (a catch-up step
/// ends past the head of the frame that asked for it, and a later frame takes it); at the end,
/// the steps through `to`. Every step taken, in order.
fn play(playhead: &Playhead, consumer: &mut Consumer, from: i32, to: i32, every: i32, taken: &mut Vec<Taken>) {
    let mut head = from;
    loop {
        playhead.advance(head);
        playhead.wait_until(|| {
            let built = playhead.newest().is_some_and(|newest| newest >= head);
            take(consumer, head, taken);
            built
        });
        if head >= to {
            break;
        }
        head = (head + every).min(to);
    }
    playhead.wait_until(|| {
        take(consumer, if taken.last().is_some_and(|step| step.last() >= to) { to } else { i32::MAX }, taken);
        taken.last().is_some_and(|step| step.last() >= to)
    });
}

/// Takes steps until every table of the last is at its last hour: a table that stalled is caught
/// up by a step of no new hours once its parts come.
fn caught_up(playhead: &Playhead, consumer: &mut Consumer, taken: &mut Vec<Taken>) {
    playhead.wait_until(|| {
        take(consumer, i32::MAX, taken);
        taken.last().is_some_and(|step| step.tables().iter().all(|table| table.short() == 0))
    });
}

/// A reader's states kept from the steps it took alone: swapped in at a reset where it was given
/// one, kept otherwise.
fn mirrors(steps: &[Taken], mut each: impl FnMut(&Step, &BTreeMap<usize, Kept>)) {
    let mut held: BTreeMap<usize, Kept> = BTreeMap::new();
    for step in steps {
        held.extend(step.states.clone());
        for changes in step.tables() {
            if let Some(kept) = held.get_mut(&changes.table()) {
                kept.apply(changes);
            }
        }
        each(step, &held);
    }
}

fn no_rows(step: &Step) -> bool {
    step.tables().iter().filter(|changes| changes.reset()).all(|changes| changes.rows() == 0 && changes.cells() == 0 && changes.texts().is_empty())
}

/// A part that is not read when its hours are stepped stops its table there: the step records
/// the table short and stalled and the frame goes on; once read, the next step takes it up, and
/// a mirror kept from the steps equals the recorder's state wherever the table is not short.
#[test]
fn a_missing_part_stalls_its_table_and_the_next_step_takes_it_up() {
    let (pages, states) = recording(Writing { cap: 64, ..Writing::at(3) }, 1, &[(0, BLOCK)]);
    let parts = &pages.runs[0].pages[0].tables[0].parts;
    let held = parts.iter().position(|part| part.first > 200).expect("a part past the seek's read-ahead");
    let frames = &parts[held].frames;
    let from = frames.iter().map(|frame| frame.offset).min().unwrap();
    let to = frames.iter().map(|frame| frame.offset + u64::from(frame.stored)).max().unwrap();
    pages.extents[0].close(from, to);
    let log = Log::default();
    let (playhead, mut consumer) = start(&pages, settings(), &[0], &log);
    let missing = i32::from(parts[held].first);
    let mut taken = Vec::new();
    play(&playhead, &mut consumer, 0, missing + 48, 24, &mut taken);
    let stalled = taken.iter().find(|step| step.table(0).is_some_and(|table| table.stalled())).expect("a stalled step");
    let table = stalled.table(0).unwrap();
    let head = table.head().unwrap();
    assert!(head < missing);
    assert_eq!(table.short(), (stalled.last() - head) as u32);
    assert_eq!(stalled.built().stalled, 1);
    pages.extents[0].open();
    play(&playhead, &mut consumer, missing + 48, 719, 24, &mut taken);
    caught_up(&playhead, &mut consumer, &mut taken);
    let last = taken.last().unwrap().table(0).unwrap();
    assert_eq!((last.head(), last.short(), last.stalled()), (Some(719), 0, false));
    mirrors(&taken, |step, held| {
        let table = step.table(0).unwrap();
        assert!(held[&0].holds(&states[0][0][table.head().unwrap() as usize]), "step {}..{}", step.first(), step.last());
    });
    assert!(playhead.held().2.stalls > 0);
}

/// A seek abandons every step built: none of the old epoch is taken after it, and the first step
/// taken is a reset of every table at the hour sought, whose state equals the recorder's there.
#[test]
fn a_seek_abandons_the_ring() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 2, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let (playhead, mut consumer) = start(&pages, settings(), &[0, 1], &log);
    let mut taken = Vec::new();
    play(&playhead, &mut consumer, 0, 96, 24, &mut taken);
    let old = taken.last().unwrap().epoch();
    playhead.advance(400);
    // The ring holds its size of steps the consumer has not taken: the head at 400 builds no more.
    playhead.wait_until(|| playhead.kept() == 3);
    playhead.seek(1000);
    taken.clear();
    take(&mut consumer, i32::MAX, &mut taken);
    assert!(taken.iter().all(|step| step.epoch() != old), "nothing of the old epoch after the seek");
    playhead.wait_until(|| {
        take(&mut consumer, 1000, &mut taken);
        !taken.is_empty()
    });
    let reset = Arc::clone(&taken[0].step);
    assert!(reset.reset() && reset.epoch() != old);
    assert_eq!((reset.first(), reset.last()), (1000, 1000));
    play(&playhead, &mut consumer, 1000, 1200, 24, &mut taken);
    assert!(taken.iter().all(|step| step.epoch() == reset.epoch()));
    mirrors(&taken, |step, held| {
        for table in 0..2 {
            let at = step.table(table).unwrap().head().unwrap();
            assert!(held[&table].holds(&states[0][table][at as usize]), "table {table} at {at}");
        }
    });
}

/// A read that fails for a seek a newer seek abandoned is dropped with it: the newer seek, into an
/// intact run, lands its reset and the playhead goes on. Before, the abandoned read's error was
/// raised before its epoch was looked at, and the playhead stopped for good.
#[test]
fn a_failed_read_for_an_abandoned_seek_does_not_stop_the_playhead() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 2, &[(0, BLOCK), (1000, BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), settings(), None).unwrap();
    let reader = playhead.declare(every(&[0, 1]), Some(kept(0, &log))).unwrap();
    let mut consumer = playhead.consumer(reader);
    pages.extents[0].close(0, u64::MAX);
    playhead.seek(0);
    pages.extents[0].wait_blocked();
    playhead.seek(1000);
    pages.extents[0].fail();
    let mut taken = Vec::new();
    playhead.wait_until(|| {
        take(&mut consumer, 1000, &mut taken);
        !taken.is_empty()
    });
    let reset = &taken[0];
    assert!(reset.reset() && (reset.first(), reset.last()) == (1000, 1000));
    assert_eq!(playhead.error(), None);
    play(&playhead, &mut consumer, 1000, 1100, 24, &mut taken);
    mirrors(&taken, |step, held| {
        for table in 0..2 {
            let at = step.table(table).unwrap().head().unwrap();
            assert!(held[&table].holds(&states[1][table][(at - 1000) as usize]), "table {table} at {at}");
        }
    });
}

/// Holds the stepper in its hook once armed, until let go.
#[derive(Default)]
struct Hold {
    state: Mutex<(bool, bool)>,
    changed: Condvar,
}

impl Hold {
    fn arm(&self) {
        self.state.lock().unwrap().0 = true;
    }

    fn at_hook(&self) {
        let mut state = self.state.lock().unwrap();
        if !state.0 {
            return;
        }
        state.1 = true;
        self.changed.notify_all();
        while state.0 {
            state = self.changed.wait(state).unwrap();
        }
        state.1 = false;
    }

    fn wait_held(&self) {
        let mut state = self.state.lock().unwrap();
        while !state.1 {
            state = self.changed.wait(state).unwrap();
        }
    }

    fn release(&self) {
        self.state.lock().unwrap().0 = false;
        self.changed.notify_all();
    }
}

/// A table's last reader stops declaring it while its join's read fails: the failure reaches the
/// stepper with the declaration, and the join is let go with the table, unlooked at. The other
/// reader plays on. Before, the stepper took the failure before applying the declaration, and the
/// playhead stopped for good.
#[test]
fn a_failed_join_of_a_table_no_reader_declares_any_more_does_not_stop_the_playhead() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 2, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let hold = Arc::new(Hold::default());
    let held = Arc::clone(&hold);
    let hook: super::Hook = Box::new(move |_, _| held.at_hook());
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), whole_ahead(), Some(hook)).unwrap();
    let a = playhead.declare(every(&[0]), Some(kept(0, &log))).unwrap();
    let mut first = Side::new(&playhead, a);
    playhead.seek(0);
    frames(&playhead, &mut [&mut first], 0, 100);
    playhead.wait_until(|| playhead.ring_len() == 3);
    pages.extents[0].close(0, u64::MAX);
    let b = playhead.declare(every(&[1]), Some(kept(1, &log))).unwrap();
    pages.extents[0].wait_blocked();
    // The stepper is held in the hook of its next step, so the declaration and the failure reach it together.
    hold.arm();
    playhead.advance(124);
    take(&mut first.consumer, 124, &mut first.taken);
    hold.wait_held();
    playhead.redeclare(b, Vec::new()).unwrap();
    pages.extents[0].fail();
    {
        let mut control = playhead.shared.control.lock().unwrap();
        while control.done.is_empty() {
            control = playhead.shared.wake.wait(control).unwrap();
        }
    }
    hold.release();
    frames(&playhead, &mut [&mut first], 124, 400);
    assert_eq!(playhead.error(), None);
    follows(&first.taken, 0, &states[0][0]);
}

/// A seek runs each reader's rebuild once for each table it declared and for no other, on the
/// lane sought; the reset step names no row, and each reader takes its own states and no other's.
#[test]
fn a_seek_runs_each_readers_rebuild_once_for_each_table_it_declared() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 3, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), settings(), None).unwrap();
    let a = playhead.declare(every(&[0, 1]), Some(kept(0, &log))).unwrap();
    let b = playhead.declare(vec![Declared { table: 1, columns: vec![2, 0], keyed: true, passing: false }], Some(kept(1, &log))).unwrap();
    playhead.declare(every(&[0, 2]), None).unwrap();
    let (mut first, mut second) = (playhead.consumer(a), playhead.consumer(b));
    for (seek, hour) in [(1, 300), (2, 1000)] {
        playhead.seek(hour);
        let mut steps = Vec::new();
        playhead.wait_until(|| {
            first.take(hour, &mut steps);
            !steps.is_empty()
        });
        let reset = &steps[0];
        assert!(reset.reset() && reset.last() == hour);
        assert!(no_rows(reset), "no reset step holds rows");
        let mut logged = log.lock().unwrap().clone();
        logged.sort_unstable();
        assert_eq!(logged, [(0, 0), (0, 1), (1, 1)].iter().flat_map(|&held| std::iter::repeat_n(held, seek)).collect::<Vec<_>>(), "seek {seek}");
        assert!(second.state(reset, 0).is_none() && second.state(reset, 2).is_none() && first.state(reset, 2).is_none(), "no state for a table a reader did not declare, or without a rebuild");
        for (table, held) in states[0].iter().enumerate().take(2) {
            let state = first.state(reset, table).expect("its own state");
            assert!(first.state(reset, table).is_none(), "taken once");
            assert!(state.downcast::<Kept>().unwrap().holds(&held[hour as usize]));
        }
        let state = *second.state(reset, 1).expect("the second reader's own").downcast::<Kept>().unwrap();
        assert_eq!(state.columns, vec![0, 2]);
        assert!(state.holds(&states[0][1][hour as usize]));
        let mut rest = Vec::new();
        second.take(hour, &mut rest);
        drop((steps, rest));
    }
}

/// What a change set says of `columns` (its rows, their cells, its head), to compare two byte for
/// byte.
fn said(changes: &Changes, columns: &[usize]) -> String {
    let written: Vec<String> = changes.columns().iter().filter(|written| columns.contains(&written.column())).map(|written| format!("{}: {:?}", written.column(), written.values().map(|(slot, value)| (slot, owned(value))).collect::<Vec<_>>())).collect();
    let (gone, born) = (changes.deleted(), changes.inserted());
    format!("{} {:?} {} {:?} {:?} {:?} {:?} {:?} {written:?}", changes.reset(), changes.head(), changes.short(), gone.slots(), gone.keys(), born.slots(), born.keys(), changes.renumber())
}

/// Every step after the first (a seek's reset) holds hours, follows the one before, and gives
/// `table` of `columns` as a lane of those columns stepped alone over the same hours does, byte
/// for byte: no reset and no step it was not built with.
fn as_stepped_alone(pages: &Arc<Runs>, steps: &[Taken], table: usize, columns: &[usize]) {
    let mut lane = Lane::new(Arc::clone(pages) as Arc<dyn Pages>, 0, table, columns, true).unwrap();
    lane.seek(steps[0].last()).unwrap();
    lane.fetch_through(i32::MAX).unwrap();
    for pair in steps.windows(2) {
        let step = &pair[1];
        assert_eq!(step.first(), pair[0].last() + 1, "step {}..{} follows the one before", step.first(), step.last());
        let mut alone = Changes::default();
        lane.step(step.last(), &mut alone).unwrap();
        assert_eq!(said(step.table(table).unwrap(), columns), said(&alone, columns), "table {table} in step {}..{}", step.first(), step.last());
    }
}

/// While playing, a reader that adds columns to a table another reader keeps gets the table at
/// once, with its own state alone rebuilt, at the hour its consumer has, from a lane of the new
/// union sought from the file; the lane it replaces steps on meanwhile. The other reader sees none
/// of it: no reset, no step of no hours, and its columns' cells in every step byte for byte what a
/// lane of them stepped alone gives, so the state it keeps from the steps alone stays equal. Once
/// landed, the steps carry the union's columns. Before, the other reader took a reset of the
/// table, and the new lane joined at the newest step built.
#[test]
fn a_reader_adding_columns_gets_its_table_at_once_and_another_reader_of_it_sees_nothing() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 2, &[(0, BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), whole_ahead(), None).unwrap();
    let keeper = playhead.declare(vec![Declared { table: 0, columns: vec![0, 1], keyed: true, passing: false }, Declared { table: 1, columns: vec![0, 1, 2, 3], keyed: true, passing: false }], Some(kept(0, &log))).unwrap();
    let changer = playhead.declare(vec![Declared { table: 0, columns: vec![0], keyed: true, passing: false }], Some(kept(1, &log))).unwrap();
    let (mut kept_side, mut changed_side) = (Side::new(&playhead, keeper), Side::new(&playhead, changer));
    playhead.play(24.0);
    playhead.seek(0);
    frames(&playhead, &mut [&mut kept_side, &mut changed_side], 0, 120);
    playhead.wait_until(|| playhead.ring_len() == 3);
    let shown = kept_side.taken.last().unwrap().last();
    log.lock().unwrap().clear();
    let before = changed_side.taken.len();
    playhead.redeclare(changer, vec![Declared { table: 0, columns: vec![0, 2, 3], keyed: true, passing: false }]).unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    playhead.wait_until(|| {
        assert!(Instant::now() < deadline, "the columns declared at hour {shown} did not arrive with the head standing there");
        take(&mut kept_side.consumer, shown, &mut kept_side.taken);
        take(&mut changed_side.consumer, shown, &mut changed_side.taken);
        changed_side.taken[before..].iter().any(|step| step.states.contains_key(&0))
    });
    let joined = changed_side.taken[before..].iter().find(|step| step.states.contains_key(&0)).unwrap();
    assert_eq!((joined.first(), joined.last()), (shown + 1, shown), "a step of no hours at the hour taken");
    assert_eq!(joined.states[&0].columns, vec![0, 2, 3]);
    assert!(joined.states[&0].holds(&states[0][0][shown as usize]));
    assert_eq!(*log.lock().unwrap(), vec![(1, 0)], "only the changed reader rebuilds, and only the table it changed");
    frames(&playhead, &mut [&mut kept_side, &mut changed_side], shown, 600);
    follows(&changed_side.taken[before..], 0, &states[0][0]);
    as_stepped_alone(&pages, &kept_side.taken, 0, &[0, 1]);
    assert!(kept_side.taken.iter().skip(1).all(|step| step.states.is_empty()), "the keeper is given no state");
    let after = kept_side.taken.iter().find(|step| step.first() > shown + 3 * 24).unwrap().table(0).unwrap();
    assert_eq!(after.columns().iter().map(|written| written.column()).collect::<Vec<_>>(), vec![0, 1, 2, 3], "the union's columns once landed");
    mirrors(&kept_side.taken, |step, held| {
        for (&table, kept) in held {
            let at = step.table(table).unwrap().head().unwrap();
            assert!(kept.holds(&states[0][table][at as usize]), "table {table} of columns {:?} at {at}", kept.columns);
        }
    });
    assert_eq!(playhead.held().2.join_held_ns, 0);
}

/// Two runs as two pieces with hours between them: steps follow each other hour by hour, the
/// hours between are gap steps holding no table, the second piece starts with a reset of every
/// table for which each reader's rebuild runs once more for each table it declared, and a mirror
/// kept from the steps equals the recorder's state of the piece it is in.
#[test]
fn two_pieces_join_with_a_reset_and_report_the_gap_between() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 2, &[(0, BLOCK), (1000, BLOCK + 50)]);
    let log = Log::default();
    let (playhead, mut consumer) = start(&pages, settings(), &[0, 1], &log);
    let mut taken = Vec::new();
    play(&playhead, &mut consumer, 0, 400, 24, &mut taken);
    let mut logged = log.lock().unwrap().clone();
    logged.sort_unstable();
    assert_eq!(logged, vec![(0, 0), (0, 1)], "the seek's rebuilds");
    play(&playhead, &mut consumer, 900, 1000 + BLOCK as i32 + 49, 24, &mut taken);
    let mut logged = log.lock().unwrap().clone();
    logged.sort_unstable();
    assert_eq!(logged, vec![(0, 0), (0, 0), (0, 1), (0, 1)], "and the piece's, once a table");
    for pair in taken.windows(2) {
        assert_eq!(pair[1].first(), pair[0].last() + 1, "steps follow each other");
    }
    let gaps: Vec<&Taken> = taken.iter().filter(|step| step.gap()).collect();
    assert_eq!((gaps.first().unwrap().first(), gaps.last().unwrap().last()), (720, 999));
    assert!(gaps.iter().all(|step| step.tables().is_empty()));
    let joined = taken.iter().find(|step| step.first() == 1000).unwrap();
    assert!(joined.reset() && joined.last() == 1000 && joined.piece() == Some(1));
    assert!(joined.tables().iter().all(Changes::reset) && no_rows(joined));
    assert_eq!(joined.states.keys().copied().collect::<Vec<_>>(), vec![0, 1]);
    mirrors(&taken, |step, held| {
        for table in 0..2 {
            let Some(at) = step.table(table).map(|changes| changes.head().unwrap()) else { continue };
            let (run, hour) = if at < 1000 { (0, at) } else { (1, at - 1000) };
            assert!(held[&table].holds(&states[run][table][hour as usize]), "table {table} at {at}");
        }
    });
    assert_eq!(taken.last().unwrap().last(), playhead.timeline().last());
}

/// What a state holds when dropped: the thread it was dropped on.
struct Dropped(Arc<(Mutex<Vec<String>>, Condvar)>);

impl Drop for Dropped {
    fn drop(&mut self) {
        let (held, dropped) = &*self.0;
        held.lock().unwrap().push(std::thread::current().name().unwrap_or_default().to_owned());
        dropped.notify_all();
    }
}

/// A state is taken once, by its own reader's consumer only; a state no consumer takes (its
/// reader's consumer gone) is dropped with its step on a fetcher, off the frame.
#[test]
fn a_rebuilt_state_is_taken_once_by_its_reader_and_one_never_taken_is_dropped_on_a_fetcher() {
    let (pages, _) = recording(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, BLOCK)]);
    let log = Log::default();
    let drops = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
    let held = Arc::clone(&drops);
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), settings(), None).unwrap();
    let a = playhead.declare(every(&[0]), Some(kept(0, &log))).unwrap();
    let b = playhead.declare(every(&[0]), Some(Arc::new(move |_: &Lane, _: &[usize]| Ok(State::new(Dropped(Arc::clone(&held)), 0))))).unwrap();
    let (mut first, second) = (playhead.consumer(a), playhead.consumer(b));
    drop(second);
    playhead.seek(0);
    let mut steps = Vec::new();
    playhead.wait_until(|| {
        first.take(0, &mut steps);
        !steps.is_empty()
    });
    assert!(first.state(&steps[0], 0).unwrap().downcast::<Kept>().is_ok());
    assert!(first.state(&steps[0], 0).is_none(), "a state is taken once, and not by another reader's consumer");
    assert_eq!(playhead.shared.flight.read().0.0, 1, "the one not taken is still held");
    drop(steps);
    let mut taken = Vec::new();
    play(&playhead, &mut first, 0, 400, 24, &mut taken);
    let (dropped, signal) = &*drops;
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut dropped = dropped.lock().unwrap();
    while dropped.is_empty() {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "the state never taken was not dropped within a minute");
        dropped = signal.wait_timeout(dropped, left).unwrap().0;
    }
    assert_eq!(dropped.len(), 1);
    assert!(dropped[0].starts_with("playhead fetch"), "dropped on {}", dropped[0]);
}

/// The clock holds the head at a seek's hour however long the seek takes, until a consumer takes
/// its reset, and runs from that moment; a seek during a seek replaces it, the head holds at the
/// newer hour, and the first reset taken is the newer one's.
#[test]
fn the_clock_holds_at_a_seek_until_its_reset_is_taken_and_a_newer_seek_replaces_it() {
    let (pages, _) = recording(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), settings(), None).unwrap();
    let reader = playhead.declare(every(&[0]), Some(kept(0, &log))).unwrap();
    let mut consumer = playhead.consumer(reader);
    playhead.play(24.0);
    playhead.seek(300);
    playhead.seek(900);
    let later = Instant::now() + Duration::from_secs(3600);
    assert_eq!(playhead.head(later), 900, "held at the newer hour");
    playhead.advance(900);
    playhead.wait_until(|| playhead.ring_len() >= 1);
    assert_eq!(playhead.head(later), 900, "held while its reset waits to be taken");
    let mut steps = Vec::new();
    let before = Instant::now();
    consumer.take(900, &mut steps);
    let after = Instant::now();
    assert!(steps[0].reset() && steps[0].last() == 900, "the first reset taken is the newer seek's");
    assert_eq!(playhead.holding(), None);
    let ten = after + Duration::from_secs(10);
    let head = playhead.head(ten);
    let slack = (after - before).as_secs_f64() * 24.0;
    assert!(head >= 900 + 240 - 1 && f64::from(head) <= 900.0 + 240.0 + slack + 1.0, "runs from the moment its reset is taken: {head}");
}

/// The clock runs from the frame that shows a seek's reset: a frame that asks the head for its
/// time and takes the reset shows the hour sought, and a frame one step later shows one step
/// later, not a step less the moment the take came after the asking.
#[test]
fn the_clock_runs_from_the_frame_that_shows_the_reset() {
    let (pages, _) = recording(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), settings(), None).unwrap();
    let reader = playhead.declare(every(&[0]), Some(kept(0, &log))).unwrap();
    let mut consumer = playhead.consumer(reader);
    playhead.play(24.0);
    playhead.seek(900);
    playhead.advance(900);
    playhead.wait_until(|| playhead.ring_len() >= 1);
    let frame = Instant::now();
    let head = playhead.head(frame);
    let mut steps = Vec::new();
    consumer.take(head, &mut steps);
    assert!(steps[0].reset() && head == 900);
    assert_eq!(playhead.head(frame + Duration::from_secs(1)), 924, "a step a second after the frame that showed the reset");
    assert_eq!(playhead.head(frame + Duration::from_secs(10)), 1140);
}

/// A head run far ahead of every reader (a frame driving its head to the newest step, a clock
/// running while the only map composes) does not run the stepper with it: the ring holds no more
/// than its size of steps nobody has taken. With three built and none taken, the head goes to
/// 600; once one step is taken, the stepper builds the one step that leaves three untaken, to the
/// head, and no more. Before, room was counted past the head alone: the head at 600 let the
/// stepper build to 600 and two steps past it with nothing taken, and a head driven to the
/// newest step chased the stepper to the recording's end.
#[test]
fn a_head_run_ahead_of_every_reader_builds_no_more_than_the_ring_past_what_was_taken() {
    let (pages, _) = recording(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let (playhead, mut consumer) = start(&pages, whole_ahead(), &[0], &log);
    let mut taken = Vec::new();
    play(&playhead, &mut consumer, 0, 0, 24, &mut taken);
    playhead.wait_until(|| playhead.newest() == Some(3 * 24));
    playhead.advance(600);
    let mut first = Vec::new();
    consumer.take(24, &mut first);
    assert_eq!(first.len(), 1, "the step to hour 24");
    playhead.wait_until(|| playhead.newest().is_some_and(|newest| newest >= 600));
    assert_eq!(playhead.newest(), Some(600), "one step to the head once one was taken, none past it while nobody took them");
    assert_eq!(playhead.kept(), 3);
}

/// While nobody takes, the ring builds its size past what was taken; and steps given back are
/// built again rather than made anew. The whole file is read ahead (no step of no new hours), and
/// each frame moves the head a step, takes the step it reached and lets it go, then waits for the
/// ring to fill again, so the order of building and taking is fixed. At most the ring's three steps
/// not taken, the three a take holds and the one being built are alive at once, and a buffer is
/// made only when none is free: seven carry every step of the run.
#[test]
fn the_ring_holds_its_size_and_recycles_its_steps() {
    let (pages, _) = recording(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let (playhead, mut consumer) = start(&pages, whole_ahead(), &[0], &log);
    let mut taken = Vec::new();
    play(&playhead, &mut consumer, 0, 0, 24, &mut taken);
    drop(taken);
    playhead.wait_until(|| playhead.newest() == Some(3 * 24));
    assert_eq!(playhead.kept(), 3, "three steps past what was taken");
    let mut made = std::collections::BTreeSet::new();
    let mut steps = 0;
    let mut head = 0;
    // Inside the first block (a step ends at a block's end, hour 719), the ring three steps past
    // every hour taken.
    while head + 4 * 24 < BLOCK as i32 {
        head += 24;
        playhead.advance(head);
        let mut now = Vec::new();
        consumer.take(head, &mut now);
        assert!(now.last().is_some_and(|step| step.last() == head), "the step to hour {head} is there to take");
        steps += now.len();
        made.extend(now.iter().map(|step| Arc::as_ptr(step) as usize));
        drop(now);
        playhead.wait_until(|| playhead.newest() == Some(head + 3 * 24));
    }
    assert!(made.len() <= 7, "{} buffers carried {steps} steps; seven are alive at most", made.len());
}

/// A second consumer sees every step once, in order, the same step and not a copy. Both take
/// every step once the stepper has none left to build: the recording's last hour reached with no
/// table short, so no catch-up step of no new hours can follow.
#[test]
fn a_second_consumer_sees_every_step_once() {
    let (pages, _) = recording(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, BLOCK)]);
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), settings(), None).unwrap();
    let reader: Reader = playhead.declare(every(&[0]), None).unwrap();
    let (mut first, mut second) = (playhead.consumer(reader), playhead.consumer(reader));
    playhead.seek(0);
    let (mut seen, mut late) = (Vec::new(), Vec::new());
    let mut head = 0;
    while head < 719 {
        playhead.advance(head);
        playhead.wait_until(|| {
            first.take(head, &mut seen);
            second.take(head - 24, &mut late);
            playhead.newest().is_some_and(|newest| newest >= head)
        });
        head = (head + 24).min(719);
    }
    playhead.advance(719);
    playhead.wait_until(|| {
        second.take(719, &mut late);
        first.take(719, &mut seen);
        seen.last().is_some_and(|step| step.last() == 719 && step.tables().iter().all(|table| table.short() == 0))
    });
    second.take(719, &mut late);
    let spans = |steps: &[Arc<Step>]| steps.iter().map(|step| (step.seq(), step.first(), step.last(), step.tables().iter().map(Changes::short).max())).collect::<Vec<_>>();
    assert_eq!(spans(&seen), spans(&late), "the first consumer's steps against the second's (seq, first, last, most short)");
    for (a, b) in seen.iter().zip(&late) {
        assert!(Arc::ptr_eq(a, b), "step {} is shared, not copied", a.seq());
    }
    assert!(seen.windows(2).all(|pair| pair[1].seq() == pair[0].seq() + 1));
}

/// The head past the timeline's last hour is its last hour: the last step ends there and no step
/// is built after it.
#[test]
fn the_head_past_the_end_stops_at_the_last_hour() {
    let (pages, _) = recording(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, BLOCK + 100)]);
    let log = Log::default();
    let (playhead, mut consumer) = start(&pages, settings(), &[0], &log);
    let last = playhead.timeline().last();
    assert_eq!(last, BLOCK as i32 + 99);
    let mut taken = Vec::new();
    play(&playhead, &mut consumer, 0, 600, 24, &mut taken);
    playhead.advance(i32::MAX);
    playhead.wait_until(|| {
        take(&mut consumer, i32::MAX, &mut taken);
        taken.last().is_some_and(|step| step.last() == last)
    });
    caught_up(&playhead, &mut consumer, &mut taken);
    assert_eq!(playhead.newest(), Some(last));
    assert!(taken.iter().all(|step| step.last() <= last));
    assert_eq!(taken.last().unwrap().table(0).unwrap().head(), Some(last));
}

/// The hook runs on every step, the reset included, with the lanes at the step's end.
#[test]
fn the_hook_sees_every_step_with_the_lanes_at_its_end() {
    let (pages, _) = recording(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, BLOCK)]);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let held = Arc::clone(&seen);
    let hook: super::Hook = Box::new(move |step, lanes| {
        let lane = lanes.get(0).expect("a lane");
        held.lock().unwrap().push((step.first(), step.last(), step.reset(), lane.head(), step.table(0).and_then(Changes::head)));
    });
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), settings(), Some(hook)).unwrap();
    let reader = playhead.declare(vec![Declared { table: 0, columns: vec![0], keyed: false, passing: false }], None).unwrap();
    let mut consumer = playhead.consumer(reader);
    playhead.seek(0);
    let mut taken = Vec::new();
    playhead.advance(300);
    playhead.wait_until(|| {
        consumer.take(i32::MAX, &mut taken);
        taken.last().is_some_and(|step| step.last() >= 300)
    });
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0], (0, 0, true, Some(0), Some(0)), "the reset first");
    for step in &taken {
        let head = step.table(0).and_then(Changes::head);
        assert!(seen.contains(&(step.first(), step.last(), step.reset(), head, head)), "step {}..{}", step.first(), step.last());
    }
}

/// One run of four blocks of a log of [`LOG`] (table 0), several parts a block.
fn log_recording() -> Arc<Runs> {
    let (bytes, run, _) = record_log(Writing { cap: 48, ..Writing::at(3) }, 0, 4 * BLOCK);
    Arc::new(Runs { extents: vec![gated(bytes)], runs: vec![run], kinds: vec![LOG.to_vec()] })
}

fn log_declared(passing: bool) -> Vec<Declared> {
    vec![Declared { table: 0, columns: (0..LOG.len()).collect(), keyed: true, passing }]
}

/// Plays frames `every` hours apart from `from` to `to`, each waiting until the playhead has built
/// through its head; every step taken, in order, through the last that brings the table to `to`.
fn play_steps(playhead: &Playhead, consumer: &mut Consumer, from: i32, to: i32, every: i32, taken: &mut Vec<Arc<Step>>) {
    let mut head = from;
    loop {
        playhead.advance(head);
        playhead.wait_until(|| {
            consumer.take(head, taken);
            playhead.newest().is_some_and(|newest| newest >= head)
        });
        if head >= to {
            break;
        }
        head = (head + every).min(to);
    }
    playhead.wait_until(|| {
        consumer.take(i32::MAX, taken);
        taken.last().is_some_and(|step| step.last() >= to && step.tables().iter().all(|table| table.short() == 0))
    });
}

/// A seek abandons a log lane's ring like any lane's: nothing of the old epoch is taken after
/// it; the first step taken is a reset at the hour sought carrying the log's count and rows
/// listed there and no row, the reader's rebuild takes the live set from the lane, and every step
/// after hands over what the log's span gives, through hours where the table is short.
#[test]
fn a_seek_abandons_a_log_lanes_ring() {
    let pages = log_recording();
    let (did, _) = read_directly(&*pages, 0, 0);
    let states = log_states(&did, 0, 4 * BLOCK as i32 - 1);
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), settings(), None).unwrap();
    let rebuild: Rebuild = Arc::new(|lane: &Lane, _: &[usize]| Ok(State::new(lane.log().expect("a log lane").clone(), 0)));
    let reader = playhead.declare(log_declared(false), Some(rebuild)).unwrap();
    let mut consumer = playhead.consumer(reader);
    playhead.seek(0);
    let mut taken = Vec::new();
    play_steps(&playhead, &mut consumer, 0, 96, 24, &mut taken);
    let old = taken.last().unwrap().epoch();
    playhead.advance(400);
    // The ring holds its size of steps the consumer has not taken: the head at 400 builds no more.
    playhead.wait_until(|| playhead.kept() == 3);
    playhead.seek(1000);
    taken.clear();
    consumer.take(i32::MAX, &mut taken);
    assert!(taken.iter().all(|step| step.epoch() != old), "nothing of the old epoch after the seek");
    playhead.wait_until(|| {
        consumer.take(1000, &mut taken);
        !taken.is_empty()
    });
    let reset = Arc::clone(&taken[0]);
    assert!(reset.reset() && reset.epoch() != old);
    assert_eq!((reset.first(), reset.last()), (1000, 1000));
    let changes = reset.table(0).unwrap();
    let (count, listed) = &states[&1000];
    let log = changes.log().expect("a log's reset");
    assert!(changes.reset() && log.is_empty() && log.forgotten().is_empty());
    assert_eq!((log.count(), log.listed()), (*count, listed.len() as u64));
    let state = consumer.state(&reset, 0).expect("the reader's rebuilt state").downcast::<LogState>().expect("a log's state");
    assert_eq!(state.count(), *count);
    assert_eq!(state.live().ones().collect::<std::collections::BTreeSet<_>>(), *listed);
    play_steps(&playhead, &mut consumer, 1000, 1500, 24, &mut taken);
    assert!(taken.iter().all(|step| step.epoch() == reset.epoch()));
    let mut head = 1000;
    for step in &taken[1..] {
        let changes = step.table(0).unwrap();
        let reached = changes.head().unwrap();
        if reached <= head {
            assert!(handed(changes).is_empty(), "a step of no new hours for the table reports nothing");
            continue;
        }
        let wanted: BTreeMap<i32, Vec<Did>> = did.range(head + 1..=reached).map(|(hour, events)| (*hour, events.clone())).collect();
        assert_eq!(handed(changes), wanted, "step {}..{}", step.first(), step.last());
        assert_eq!(changes.log().unwrap().count(), states[&reached].0);
        head = reached;
    }
    assert!(head >= 1500, "the steps reach the last frame's head: {head}");
}

/// The playhead reads a log table through its log lane alone: the lanes the hook sees at every
/// step, the reset included, are log lanes that never opened a restore or cursor of a table
/// holding state.
#[test]
fn a_log_is_never_read_with_the_state_cursor() {
    let pages = log_recording();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let held = Arc::clone(&seen);
    let hook: super::Hook = Box::new(move |step, lanes| {
        let lane = lanes.get(0).expect("a lane");
        held.lock().unwrap().push((step.reset(), lane.log().is_some(), lane.state_reads, step.table(0).is_some_and(|changes| changes.log().is_some())));
    });
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), settings(), Some(hook)).unwrap();
    let reader = playhead.declare(log_declared(false), None).unwrap();
    let mut consumer = playhead.consumer(reader);
    playhead.seek(700);
    let mut taken = Vec::new();
    play_steps(&playhead, &mut consumer, 700, 1600, 24, &mut taken);
    let seen = seen.lock().unwrap().clone();
    assert!(seen.len() > 10 && seen[0].0, "the reset first, then the steps: {seen:?}");
    assert!(seen.iter().all(|&(_, log, state_reads, changes)| log && state_reads == 0 && changes), "{seen:?}");
}

/// Declaring a log table passing is refused by name, by a new reader and by one changing its
/// tables.
#[test]
fn declaring_a_log_passing_is_refused_by_name() {
    let pages = log_recording();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), settings(), None).unwrap();
    let refused = playhead.declare(log_declared(true), None).unwrap_err().to_string();
    assert!(refused.contains("Table 0 is a log") && refused.contains("not declared passing"), "{refused}");
    let reader = playhead.declare(log_declared(false), None).unwrap();
    let refused = playhead.redeclare(reader, log_declared(true)).unwrap_err().to_string();
    assert!(refused.contains("Table 0 is a log"), "{refused}");
}

/// Rows asked of the playhead by ordinal are read on a fetcher and answered owned, equal to the
/// rows read directly; an ordinal the log never appended is refused by name in the answer.
#[test]
fn listed_rows_are_read_on_a_fetcher_by_ordinal() {
    let pages = log_recording();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), settings(), None).unwrap();
    let wanted = Wanted { run: 0, table: 0, ordinals: vec![5, 900, 3, 2000, 1501], columns: vec![2, 0] };
    let asked = playhead.rows(wanted.clone()).wait().unwrap();
    let direct = crate::lane::log::rows(&*pages, &wanted).unwrap();
    assert_eq!((asked.ordinals(), asked.hours(), asked.keys()), (direct.ordinals(), direct.hours(), direct.keys()));
    assert_eq!(asked.ordinals(), &[3, 5, 900, 1501, 2000]);
    for (asked, direct) in asked.columns().iter().zip(direct.columns()) {
        assert_eq!(asked.values().map(|(slot, value)| format!("{slot} {value:?}")).collect::<Vec<_>>(), direct.values().map(|(slot, value)| format!("{slot} {value:?}")).collect::<Vec<_>>());
    }
    let refused = playhead.rows(Wanted { ordinals: vec![u64::from(u32::MAX)], ..wanted }).wait().unwrap_err().to_string();
    assert!(refused.contains("holds no row"), "{refused}");
}

/// One reader's consumer and the steps it took.
struct Side {
    consumer: Consumer,
    taken: Vec<Taken>,
}

impl Side {
    fn new(playhead: &Playhead, reader: Reader) -> Self {
        Self { consumer: playhead.consumer(reader), taken: Vec::new() }
    }

    /// The first step that carried this reader's state of `table`.
    fn joined(&self, table: usize) -> Option<&Taken> {
        self.taken.iter().find(|step| step.states.contains_key(&table))
    }
}

/// [`settings`] reading the whole run ahead at a seek or a join: no step waits on a read, so
/// every table is at each step's last hour.
fn whole_ahead() -> Settings {
    Settings { ahead: 100, ..settings() }
}

/// Frames as [`play`] plays them, every side taking each frame's steps (the ring moves only once
/// every consumer took a step).
fn frames(playhead: &Playhead, sides: &mut [&mut Side], from: i32, to: i32) {
    let mut head = from;
    loop {
        playhead.advance(head);
        playhead.wait_until(|| {
            // Built through the head before the takes, so each takes the step that reaches it.
            let built = playhead.newest().is_some_and(|newest| newest >= head);
            for side in sides.iter_mut() {
                take(&mut side.consumer, head, &mut side.taken);
            }
            built
        });
        if head >= to {
            break;
        }
        head = (head + 24).min(to);
    }
}

/// Waits, the head held at `head`, until `side` took its state of every table of `tables`.
fn arrive(playhead: &Playhead, side: &mut Side, head: i32, tables: &[usize]) {
    playhead.advance(head);
    playhead.wait_until(|| {
        take(&mut side.consumer, head, &mut side.taken);
        tables.iter().all(|&table| side.joined(table).is_some())
    });
}

/// From the step that carried its state on, `table` as `side` keeps it from the steps alone:
/// every step of hours past it holds the table (a step of none may join others alone), each the
/// hours after the one before and none twice, with the table at the step's last hour, equal to
/// the recorder's state there. A step of no hours holding it (a landing's catch-up to the hours
/// the frames passed) brings it no further than the hour taken.
fn follows(taken: &[Taken], table: usize, states: &[TableState]) {
    let joined = taken.iter().position(|step| step.states.contains_key(&table)).expect("its state");
    let mut kept = taken[joined].states[&table].clone();
    let mut at = kept.head.expect("sought");
    for step in &taken[joined + 1..] {
        let Some(changes) = step.table(table) else {
            assert!(step.first() > step.last(), "step {}..{} holds table {table}", step.first(), step.last());
            continue;
        };
        if step.first() <= step.last() {
            assert_eq!(step.first(), at + 1, "table {table}: the step after hour {at}");
            at = step.last();
        } else {
            at = changes.head().expect("sought");
            assert!(at <= step.last(), "table {table} brought to hour {at} past the hour taken, {}", step.last());
        }
        assert_eq!((changes.head(), changes.short()), (Some(at), 0), "table {table} at {at}");
        kept.apply(changes);
        assert!(kept.holds(&states[at as usize]), "table {table} at {at}");
    }
}

/// A table declared while the head is held arrives at once at the last hour a frame there took,
/// with its reader's rebuilt state, before the steps already built past the head and without the
/// head moving; no other table is sought again. Played on, the steps built before it arrived and
/// after carry the table hour after hour, none twice and none missing, and the hook saw its reset
/// with its lane there and each of those steps' change sets with the lane at their end.
#[test]
fn a_table_declared_while_paused_arrives_at_the_heads_hour_and_play_follows_it_hour_by_hour() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 2, &[(0, 2 * BLOCK)]);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let held = Arc::clone(&seen);
    let hook: super::Hook = Box::new(move |step, lanes| {
        held.lock().unwrap().push((step.first(), step.last(), lanes.get(1).and_then(Lane::head), step.table(1).map(|changes| (changes.reset(), changes.head()))));
    });
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), whole_ahead(), Some(hook)).unwrap();
    let log = Log::default();
    let a = playhead.declare(every(&[0]), Some(kept(0, &log))).unwrap();
    let mut first = Side::new(&playhead, a);
    playhead.seek(0);
    frames(&playhead, &mut [&mut first], 0, 100);
    playhead.wait_until(|| playhead.ring_len() == 3);
    let shown = first.taken.last().unwrap().last();
    assert!(shown <= 100 && playhead.newest().unwrap() > 100);
    let b = playhead.declare(every(&[1]), Some(kept(1, &log))).unwrap();
    let mut second = Side::new(&playhead, b);
    arrive(&playhead, &mut second, 100, &[1]);
    let joined = second.joined(1).unwrap();
    assert_eq!((joined.first(), joined.last(), joined.reset()), (shown + 1, shown, false), "a step of no hours at the last hour taken");
    assert!(joined.table(0).is_none(), "the other table is not in it");
    assert_eq!(joined.table(1).map(Changes::head), Some(Some(shown)));
    assert!(joined.states[&1].holds(&states[0][1][shown as usize]));
    assert_eq!(*log.lock().unwrap(), vec![(0, 0), (1, 1)], "no other table sought again");
    assert_eq!(playhead.held().2.joins_at_head, 1);
    frames(&playhead, &mut [&mut first, &mut second], 100, 600);
    follows(&second.taken, 1, &states[0][1]);
    let mut taken = first.taken.iter().filter(|step| step.first() <= step.last()).map(|step| (step.first(), step.last()));
    let mut at = taken.next().unwrap().1;
    for (from, to) in taken {
        assert_eq!(from, at + 1, "the first reader's steps go on hour after hour");
        at = to;
    }
    let seen = seen.lock().unwrap();
    assert!(seen.contains(&(shown + 1, shown, Some(shown), Some((true, Some(shown))))), "the hook saw the reset with its lane at the hour taken");
    for step in second.taken.iter().filter(|step| step.first() <= step.last()) {
        let head = step.table(1).and_then(Changes::head);
        assert!(seen.contains(&(step.first(), step.last(), head, Some((false, head)))), "the hook saw step {}..{} with the lane at its end", step.first(), step.last());
    }
}

/// A consumer looks at the steps built past the head and takes none by looking: what it is lent
/// is, in order, what its takes then bring, the same steps. Paused, a table another reader
/// declares lands at the head while the steps lent are still held: the join is not refused for
/// them, the held steps are left as they were (no table 1), and the steps taken past the join
/// carry the new table hour after hour.
#[test]
fn a_consumer_looks_ahead_without_taking_and_a_join_at_the_head_lands_past_steps_it_holds() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 2, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), whole_ahead(), None).unwrap();
    let a = playhead.declare(every(&[0]), Some(kept(0, &log))).unwrap();
    let mut first = Side::new(&playhead, a);
    playhead.seek(0);
    frames(&playhead, &mut [&mut first], 0, 100);
    playhead.wait_until(|| playhead.ring_len() == 3);
    let shown = first.taken.last().unwrap().last();
    let mut lent = Vec::new();
    first.consumer.ahead(&mut lent);
    assert_eq!(lent.len(), 3, "every step built and not taken is lent");
    assert!(lent.iter().all(|step| step.first() > shown), "only steps past the last taken");
    assert!(lent.windows(2).all(|pair| pair[1].first() == pair[0].last() + 1), "in order, hour after hour");
    let mut again = Vec::new();
    first.consumer.ahead(&mut again);
    assert!(lent.iter().zip(&again).all(|(a, b)| Arc::ptr_eq(a, b)) && again.len() == lent.len(), "looking twice lends the same steps: looking takes none");
    // A step of no hours (a stalled table's catch-up) may be lent among them; the hours lent are the rest.
    let spans: Vec<(i32, i32)> = lent.iter().filter(|step| step.first() <= step.last()).map(|step| (step.first(), step.last())).collect();
    let b = playhead.declare(every(&[1]), Some(kept(1, &log))).unwrap();
    let mut second = Side::new(&playhead, b);
    arrive(&playhead, &mut second, 100, &[1]);
    let joined = second.joined(1).unwrap();
    assert_eq!((joined.first(), joined.last()), (shown + 1, shown), "the join lands at the head, before the steps lent");
    assert_eq!(playhead.held().2.joins_at_head, 1, "landed once, not sought again for the steps held");
    assert!(lent.iter().all(|step| step.table(1).is_none()), "the steps held are unchanged");
    frames(&playhead, &mut [&mut first, &mut second], 100, 600);
    follows(&second.taken, 1, &states[0][1]);
    let taken: Vec<(i32, i32)> = first.taken.iter().filter(|step| step.first() > shown && step.first() <= step.last()).map(|step| (step.first(), step.last())).take(spans.len()).collect();
    assert_eq!(taken, spans, "the first reader takes the hours it was lent, in order");
}

/// A declaration that joins no table, made with the ring full and the head held (no step coming),
/// is shown to the hook at once in a step of no hours after the newest, the lanes there: a reader of
/// a table already stepped, or of none, is not left waiting for the clock to move.
#[test]
fn a_declaration_that_joins_no_table_is_shown_to_the_hook_at_once_while_paused() {
    let (pages, _) = recording(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, 2 * BLOCK)]);
    let (send, seen) = std::sync::mpsc::channel();
    let hook: super::Hook = Box::new(move |step, lanes| {
        let _ = send.send((step.first(), step.last(), step.gap(), lanes.get(0).and_then(Lane::head)));
    });
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), whole_ahead(), Some(hook)).unwrap();
    let log = Log::default();
    let a = playhead.declare(every(&[0]), Some(kept(0, &log))).unwrap();
    let mut first = Side::new(&playhead, a);
    playhead.seek(0);
    frames(&playhead, &mut [&mut first], 0, 100);
    playhead.wait_until(|| playhead.ring_len() == 3);
    let newest = playhead.newest().unwrap();
    while seen.try_recv().is_ok() {}
    let shown = |what: &str| seen.recv_timeout(Duration::from_secs(60)).unwrap_or_else(|_| panic!("{what}: the hook was shown nothing with the head held"));
    let _same = playhead.declare(every(&[0]), None).unwrap();
    assert_eq!(shown("a table already stepped"), (newest + 1, newest, false, Some(newest)), "a step of no hours after the newest, the lane there");
    let _none = playhead.declare(Vec::new(), None).unwrap();
    assert_eq!(shown("no table"), (newest + 1, newest, false, Some(newest)));
    assert_eq!(playhead.newest(), Some(newest), "no step built");
    assert_eq!(playhead.held().2.joins_at_head, 0, "nothing joined");
}

/// Kept exact while paused, what the hook is shown while the clock is paused is of the paused hour:
/// a pause with steps built past the head resets the lanes there (the hook sees that reset, the
/// clock holds at the hour paused), no step is built past it, and a declaration is shown at it.
#[test]
fn kept_exact_a_pause_past_the_steps_built_resets_at_the_head_and_builds_nothing_past_it() {
    let (pages, _) = recording(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, 2 * BLOCK)]);
    let (send, seen) = std::sync::mpsc::channel();
    let hook: super::Hook = Box::new(move |step, _lanes| {
        let _ = send.send((step.epoch(), step.first(), step.last(), step.reset()));
    });
    // Eight steps of a day past the head: the clock would run eight seconds to pass them.
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), Settings { ring: 8, ..whole_ahead() }, Some(hook)).unwrap();
    playhead.exact_when_paused();
    let log = Log::default();
    let reader = playhead.declare(every(&[0]), Some(kept(0, &log))).unwrap();
    let mut consumer = playhead.consumer(reader);
    playhead.play(24.0);
    playhead.seek(0);
    playhead.advance(0);
    let mut taken = Vec::new();
    playhead.wait_until(|| {
        take(&mut consumer, 0, &mut taken);
        !taken.is_empty()
    });
    playhead.wait_until(|| playhead.ring_len() == 8);
    let newest = playhead.newest().unwrap();
    while seen.try_recv().is_ok() {}
    playhead.pause();
    let paused = playhead.head(Instant::now());
    assert!(paused < newest, "paused at {paused}, before the newest step built ({newest})");
    let shown = || seen.recv_timeout(Duration::from_secs(60)).unwrap_or_else(|_| panic!("the hook was shown no reset at the paused head {paused}"));
    let reset = loop {
        let step = shown();
        if step.3 {
            break step;
        }
    };
    assert_eq!((reset.1, reset.2), (paused, paused), "a reset at the paused head, in place of the steps built past it");
    assert_eq!(playhead.head(Instant::now() + Duration::from_secs(3600)), paused, "the clock stays where it paused");
    playhead.advance(paused);
    playhead.wait_until(|| {
        take(&mut consumer, paused, &mut taken);
        taken.last().is_some_and(|step| step.reset() && step.last() == paused)
    });
    assert_eq!(playhead.newest(), Some(paused), "nothing built past the paused head");
    let _none = playhead.declare(Vec::new(), None).unwrap();
    assert_eq!(shown(), (reset.0, paused + 1, paused, false), "a declaration shown at the paused head, never at a step ahead of it");
    assert_eq!(playhead.newest(), Some(paused));
}

/// Several tables declared at once while paused all arrive at the hour taken, in one step: two
/// new to the playhead and one another reader keeps, whose new columns re-seek it. Only the declaring
/// reader rebuilds; the keeper's state, kept from the steps alone, stays equal through play.
#[test]
fn several_tables_declared_at_once_while_paused_arrive_together_and_a_keeper_keeps_its_state() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 3, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), whole_ahead(), None).unwrap();
    let a = playhead.declare(vec![Declared { table: 0, columns: vec![0, 1], keyed: true, passing: false }], Some(kept(0, &log))).unwrap();
    let mut keeper = Side::new(&playhead, a);
    playhead.seek(0);
    frames(&playhead, &mut [&mut keeper], 0, 300);
    playhead.wait_until(|| playhead.ring_len() == 3);
    let shown = keeper.taken.last().unwrap().last();
    let declared = vec![
        Declared { table: 0, columns: vec![2, 3], keyed: true, passing: false },
        Declared { table: 1, columns: vec![0, 2], keyed: true, passing: false },
        Declared { table: 2, columns: vec![0, 1, 2, 3], keyed: true, passing: false },
    ];
    let b = playhead.declare(declared, Some(kept(1, &log))).unwrap();
    let mut changer = Side::new(&playhead, b);
    arrive(&playhead, &mut changer, 300, &[0, 1, 2]);
    let carried: std::collections::BTreeSet<u64> = (0..3).map(|table| changer.joined(table).unwrap().seq()).collect();
    assert_eq!(carried.len(), 1, "one step brings every table declared at once, new to the playhead or not: a reader composes once");
    for table in 0..3 {
        let joined = changer.joined(table).unwrap();
        assert_eq!(joined.last(), shown);
        assert!(joined.states[&table].holds(&states[0][table][shown as usize]), "table {table}");
    }
    let mut logged = log.lock().unwrap().clone();
    logged.sort_unstable();
    assert_eq!(logged, vec![(0, 0), (1, 0), (1, 1), (1, 2)], "the keeper is not rebuilt");
    frames(&playhead, &mut [&mut keeper, &mut changer], 300, 900);
    for table in 0..3 {
        follows(&changer.taken, table, &states[0][table]);
    }
    let mut kept = keeper.taken[0].states[&0].clone();
    for step in &keeper.taken[1..] {
        if let Some(changes) = step.table(0) {
            kept.apply(changes);
            assert!(kept.holds(&states[0][0][kept.head.unwrap() as usize]), "the keeper's table 0 at {:?}", kept.head);
        }
    }
    assert_eq!(kept.head, keeper.taken.last().unwrap().table(0).unwrap().head());
}

/// A table declared while a seek runs arrives at the hour sought, with the seek's reset or just
/// after it, and play follows it.
#[test]
fn a_table_declared_during_a_seek_arrives_at_the_hour_sought() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 2, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), whole_ahead(), None).unwrap();
    let a = playhead.declare(every(&[0]), Some(kept(0, &log))).unwrap();
    let mut first = Side::new(&playhead, a);
    playhead.seek(0);
    frames(&playhead, &mut [&mut first], 0, 100);
    pages.extents[0].close(0, u64::MAX);
    playhead.seek(1000);
    pages.extents[0].wait_blocked();
    let b = playhead.declare(every(&[1]), Some(kept(1, &log))).unwrap();
    let mut second = Side::new(&playhead, b);
    pages.extents[0].open();
    arrive(&playhead, &mut second, 1000, &[1]);
    let joined = second.joined(1).unwrap();
    assert_eq!(joined.last(), 1000);
    assert!(joined.states[&1].holds(&states[0][1][1000]));
    frames(&playhead, &mut [&mut first, &mut second], 1000, 1300);
    follows(&second.taken, 1, &states[0][1]);
    assert!(first.taken.iter().any(|step| step.reset() && step.last() == 1000));
}

/// A table declared while paused and not arrived when a seek comes arrives with the seek, at the
/// hour sought: nothing of the abandoned join is taken and its reader's rebuild of it runs once.
#[test]
fn a_table_declared_while_paused_then_a_seek_before_it_arrives_comes_with_the_seek() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 2, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), whole_ahead(), None).unwrap();
    let a = playhead.declare(every(&[0]), Some(kept(0, &log))).unwrap();
    let mut first = Side::new(&playhead, a);
    playhead.seek(0);
    frames(&playhead, &mut [&mut first], 0, 100);
    playhead.wait_until(|| playhead.ring_len() == 3);
    pages.extents[0].close(0, u64::MAX);
    let b = playhead.declare(every(&[1]), Some(kept(1, &log))).unwrap();
    let mut second = Side::new(&playhead, b);
    pages.extents[0].wait_blocked();
    playhead.seek(1200);
    pages.extents[0].open();
    arrive(&playhead, &mut second, 1200, &[1]);
    let joined = second.joined(1).unwrap();
    assert_eq!(joined.last(), 1200);
    assert!(joined.states[&1].holds(&states[0][1][1200]));
    assert_eq!(second.taken.iter().filter(|step| step.states.contains_key(&1)).count(), 1);
    assert!(second.taken.iter().all(|step| step.epoch() == joined.epoch()), "nothing of the abandoned join");
    let mut logged = log.lock().unwrap().clone();
    logged.sort_unstable();
    assert_eq!(logged, vec![(0, 0), (0, 0), (1, 1)]);
    frames(&playhead, &mut [&mut first, &mut second], 1200, 1430);
    follows(&second.taken, 1, &states[0][1]);
}

/// Two consumers and a head moving a step a frame; one stands at hour 0 and takes nothing. The other gets every step at
/// its hour all the same: the stepper builds the ring's size past its head whatever the late one
/// has not taken. The steps it passed wait for the late one, shared, until they hold more bytes
/// than the lanes' cells, the state they change; then it lands at the newest step from memory, no
/// join made, and the ring lets them go: it never keeps more for the late one than that state and
/// the step that passed it. Taken, the landing holds its state equal to the recorder's at its hour,
/// and the steps after carry it on exactly. Before, every step was kept for it however far behind.
#[test]
fn a_consumer_late_to_take_holds_neither_the_stepper_nor_another_consumer_and_costs_at_most_the_state() {
    // 512 rows rewritten every hour: the steps the late one misses soon outweigh the state.
    let wide = |at: u32, _: usize, changes: &mut TableChanges<'_>| {
        for slot in 0..512_u32 {
            if at == 0 {
                changes.insert_at(slot, 100 + u128::from(slot)).unwrap();
            }
            changes.write(slot, 0, Cell::I32((at * 7 + slot) as i32)).unwrap();
        }
    };
    let (pages, states) = recording_of(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, 6 * BLOCK)], wide);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), whole_ahead(), None).unwrap();
    let (a, b) = (playhead.declare(every(&[0]), None).unwrap(), playhead.declare(every(&[0]), Some(kept(0, &log))).unwrap());
    let (mut early, mut late) = (playhead.consumer(a), playhead.consumer(b));
    playhead.play(24.0);
    playhead.seek(0);
    let mut reset = Vec::new();
    playhead.wait_until(|| {
        take(&mut late, 0, &mut reset);
        !reset.is_empty()
    });
    assert!(reset[0].reset() && reset[0].states.contains_key(&0), "the late one takes the seek's reset, then nothing");
    late.advance(0);
    let mut seen: Vec<Arc<Step>> = Vec::new();
    let mut frame = |head: i32, seen: &mut Vec<Arc<Step>>| {
        early.advance(head);
        playhead.wait_until(|| {
            let built = playhead.newest().is_some_and(|newest| newest >= head);
            early.take(head, seen);
            built && !early.reaches(head)
        });
    };
    let joins = playhead.held().2.joins;
    let last = 6 * BLOCK as i32 - 1;
    let mut head = 0;
    while playhead.held().2.landings == 0 {
        assert!(head < last, "the late one was never landed: the ring keeps {:?} bytes (all, largest) against lanes' cells of {}", playhead.kept_bytes(), playhead.held().0.held.cells);
        head = (head + 24).min(last);
        frame(head, &mut seen);
        let ((kept, largest), state) = (playhead.kept_bytes(), playhead.held().0.held.cells);
        assert!(state == 0 || kept <= state + largest, "at hour {head} the ring keeps {kept} bytes for the late one, past the lanes' cells of {state} and a step of {largest}");
    }
    assert!(playhead.held().2.landed_behind >= 1, "landed for being behind, not asked");
    assert_eq!(playhead.held().2.joins, joins, "landed from memory: no join from the file");
    let mut landed: Vec<Taken> = Vec::new();
    while landed.is_empty() {
        assert!(head < last, "the late one's landing never came to take (newest {:?})", playhead.newest());
        head += 24;
        frame(head, &mut seen);
        take(&mut late, head, &mut landed);
    }
    let hour = landed[0].last();
    assert!((landed[0].first(), landed[0].reset()) == (hour + 1, false) && landed[0].table(0).is_some_and(Changes::reset), "a step of no hours resetting its table, at {hour}");
    let mut held = landed[0].states.get(&0).cloned().expect("its state");
    assert!(held.holds(&states[0][0][hour as usize]), "the late one's state at {hour} is the recorder's");
    for _ in 0..5 {
        head = (head + 24).min(last);
        frame(head, &mut seen);
        take(&mut late, head, &mut landed);
    }
    assert!(landed.len() > 1, "the steps after the landing");
    for step in &landed[1..] {
        let changes = step.table(0).expect("its table carried on");
        held.apply(changes);
        let at = changes.head().unwrap();
        assert!(held.holds(&states[0][0][at as usize]), "the late one's state at {at} is the recorder's");
    }
    assert!(seen.windows(2).all(|pair| pair[1].seq() == pair[0].seq() + 1 && pair[1].first() == pair[0].last() + 1), "the early one took every step as built");
}

/// A reader of a log far behind holds no step past the state either: once its steps outweigh the
/// lanes' cells it is parked, its steps let go, and its log is read from the file from the hour it
/// stood at, then stepped in memory to where the other reader stands (the head standing there
/// meanwhile). Taken, those steps hand over every event of the hours it skipped as a fresh read of
/// the log gives them, the log's count with them, and the steps built after carry it on, none
/// twice. The ring never keeps more for it than the state and the step that passed it. Before, a
/// log reader was never landed: its steps waited for it without limit.
#[test]
fn a_log_reader_far_behind_is_caught_up_from_the_file_with_every_row_it_skipped() {
    let pages = log_recording();
    let last = 4 * BLOCK as i32 - 1;
    let (did, _) = read_directly(&*pages, 0, 0);
    let states = log_states(&did, 0, last);
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), whole_ahead(), None).unwrap();
    let rebuild: Rebuild = Arc::new(|lane: &Lane, _: &[usize]| Ok(State::new(lane.log().expect("a log lane").clone(), 0)));
    let (a, b) = (playhead.declare(log_declared(false), None).unwrap(), playhead.declare(log_declared(false), Some(rebuild)).unwrap());
    let (mut early, mut late) = (playhead.consumer(a), playhead.consumer(b));
    playhead.play(24.0);
    playhead.seek(0);
    let mut taken: Vec<Arc<Step>> = Vec::new();
    playhead.wait_until(|| {
        late.take(0, &mut taken);
        !taken.is_empty()
    });
    assert!(taken.len() == 1 && taken[0].reset(), "the late one takes the seek's reset, then nothing");
    late.advance(0);
    let mut seen: Vec<Arc<Step>> = Vec::new();
    let mut frame = |head: i32, seen: &mut Vec<Arc<Step>>| {
        early.advance(head);
        playhead.wait_until(|| {
            let built = playhead.newest().is_some_and(|newest| newest >= head);
            early.take(head, seen);
            built && !early.reaches(head)
        });
    };
    let mut head = 0;
    while playhead.held().2.landed_behind == 0 {
        assert!(head < last, "the late one was never parked: the ring keeps {:?} bytes (behind, largest) against lanes' cells of {}; {:?}", playhead.kept_bytes(), playhead.held().0.held.cells, playhead.held().2);
        head = (head + 24).min(last);
        frame(head, &mut seen);
        let ((kept, largest), state) = (playhead.kept_bytes(), playhead.held().0.held.cells);
        assert!(state == 0 || kept <= state + largest, "at hour {head} the ring keeps {kept} bytes, past the lanes' cells of {state} and a step of {largest}");
    }
    // The head stands while the late one's log is read from the file and caught up.
    playhead.wait_until(|| playhead.held().2.caught_up >= 1);
    let counters = playhead.held().2;
    assert!(counters.landed_behind >= 1 && counters.landings == 0, "caught up from the file, not landed from memory: {counters:?}");
    eprintln!("log catch-up from the file: {:.2} ms at the most", counters.catch_up_most_ns as f64 / 1e6);
    let through = (head + 5 * 24).min(last);
    while head < through {
        head = (head + 24).min(last);
        frame(head, &mut seen);
        late.take(head, &mut taken);
    }
    playhead.wait_until(|| {
        late.take(head, &mut taken);
        taken.last().and_then(|step| step.table(0)).and_then(Changes::head).is_some_and(|reached| reached >= head)
    });
    let mut reached = 0;
    for step in &taken[1..] {
        let changes = step.table(0).expect("its log in every step");
        let at = changes.head().unwrap();
        if at <= reached {
            assert!(handed(changes).is_empty(), "a step of no new hours for the log hands nothing");
            continue;
        }
        let wanted: BTreeMap<i32, Vec<Did>> = did.range(reached + 1..=at).map(|(hour, events)| (*hour, events.clone())).collect();
        assert_eq!(handed(changes), wanted, "step {}..{} hands the events a fresh read gives", step.first(), step.last());
        assert_eq!(changes.log().unwrap().count(), states[&at].0, "the log's count at {at}");
        reached = at;
    }
    assert!(reached >= head, "the late one reaches the other's head: {reached} of {head}");
}

/// A reader of a log far behind is caught up from the file while the other reader stands at an
/// hour where the log's lane stood short (no part had come when its steps were built). The catch-up
/// stops the reader's log where the lane stood there, so the steps after (the one that catches the
/// lane up among them) carry it on, and every event comes once, as a fresh read gives it: with
/// steps built past that hour (the head ran on), and with none (it stood there). Before, the
/// catch-up waited for the lane to be whole at that hour, which steps built past it never bring
/// back to it: the rows worker stood hundreds of hours behind the asked hour in the war24 equality
/// runs. And a reader landed short stood as if whole, so its next catch-up read past the hours its
/// log lacked.
#[test]
fn a_reader_caught_up_where_its_lane_stood_short_lands_with_the_head_standing_there() {
    caught_up_where_its_lane_stood_short(true);
    caught_up_where_its_lane_stood_short(false);
}

fn caught_up_where_its_lane_stood_short(built_past: bool) {
    let pages = log_recording();
    let last = 4 * BLOCK as i32 - 1;
    let (did, _) = read_directly(&*pages, 0, 0);
    let states = log_states(&did, 0, last);
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), settings(), None).unwrap();
    let rebuild: Rebuild = Arc::new(|lane: &Lane, _: &[usize]| Ok(State::new(lane.log().expect("a log lane").clone(), 0)));
    let (a, b) = (playhead.declare(log_declared(false), None).unwrap(), playhead.declare(log_declared(false), Some(rebuild)).unwrap());
    let (mut early, mut late) = (playhead.consumer(a), playhead.consumer(b));
    playhead.play(24.0);
    playhead.seek(0);
    let mut taken: Vec<Arc<Step>> = Vec::new();
    playhead.wait_until(|| {
        late.take(0, &mut taken);
        !taken.is_empty()
    });
    late.advance(0);
    // From here no read of the file ends until the gate opens: the steps past what the seek read
    // stand short, and the late one's catch-up read waits.
    pages.extents[0].close(0, u64::MAX);
    let mut seen: Vec<Arc<Step>> = Vec::new();
    let frame = |early: &mut Consumer, head: i32, seen: &mut Vec<Arc<Step>>| {
        early.advance(head);
        playhead.wait_until(|| {
            let built = playhead.newest().is_some_and(|newest| newest >= head);
            early.take(head, seen);
            built && !early.reaches(head)
        });
    };
    let stand = 144;
    for head in (24..=stand).step_by(24) {
        frame(&mut early, head, &mut seen);
    }
    assert!(seen.last().and_then(|step| step.table(0)).and_then(Changes::head).is_some_and(|head| head < stand), "the log's lane stands short at {stand}");
    // Its head runs on while it takes no further: steps are built past 144, short too, so the one
    // that catches the lane up, once the gate opens, ends past 144.
    if built_past {
        early.advance(stand + 96);
        playhead.wait_until(|| playhead.newest().is_some_and(|newest| newest >= stand + 96));
    }
    playhead.wait_until(|| playhead.held().2.landed_behind >= 1);
    assert_eq!(playhead.held().2.caught_up, 0, "its read waits at the gate");
    pages.extents[0].open();
    // The other stands at 144, where the lane stood short.
    playhead.wait_until(|| playhead.held().2.caught_up >= 1);
    let mut head = stand;
    while head < stand + 240 {
        head += 24;
        frame(&mut early, head, &mut seen);
        late.take(head, &mut taken);
    }
    playhead.wait_until(|| {
        late.take(head, &mut taken);
        taken.last().and_then(|step| step.table(0)).and_then(Changes::head).is_some_and(|reached| reached >= head)
    });
    let mut reached = 0;
    for step in &taken[1..] {
        let changes = step.table(0).expect("its log in every step");
        let at = changes.head().unwrap();
        if at <= reached {
            assert!(handed(changes).is_empty(), "a step of no new hours for the log hands nothing");
            continue;
        }
        let wanted: BTreeMap<i32, Vec<Did>> = did.range(reached + 1..=at).map(|(hour, events)| (*hour, events.clone())).collect();
        assert_eq!(handed(changes), wanted, "step {}..{} hands the events a fresh read gives", step.first(), step.last());
        assert_eq!(changes.log().unwrap().count(), states[&at].0, "the log's count at {at}");
        reached = at;
    }
    assert!(reached >= head, "the late one reaches the head: {reached} of {head}");
}

/// A seek's tables are prepared at once on as many threads as the machine has, not by the stepper
/// and its fetcher alone: with every read of the file held, as many tables wait in their restore
/// at once as there are threads for them, and the seek says how many prepared its tables. Before,
/// at most the stepper, its helpers and the fetchers prepared them: here two.
#[test]
fn a_seek_prepares_its_tables_on_threads_lent_to_it() {
    let tables = 8;
    let expected = tables.min(std::thread::available_parallelism().map_or(1, std::num::NonZero::get));
    let (pages, _) = recording(Writing { cap: 256, ..Writing::at(3) }, tables, &[(0, BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), settings(), None).unwrap();
    let reader = playhead.declare(every(&(0..tables).collect::<Vec<_>>()), Some(kept(0, &log))).unwrap();
    let mut consumer = playhead.consumer(reader);
    let gated = &pages.extents[0];
    gated.close(0, u64::MAX);
    playhead.seek(100);
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut closed = gated.closed.lock().unwrap();
    while closed.1 < expected {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "{} of {expected} tables were in their restore at once a minute after the seek", closed.1);
        closed = gated.opened.wait_timeout(closed, left).unwrap().0;
    }
    drop(closed);
    gated.open();
    playhead.wait_until(|| playhead.ring_len() >= 1);
    let mut steps = Vec::new();
    consumer.take(100, &mut steps);
    let sought = steps[0].built().sought.expect("the reset of a seek");
    assert!(steps[0].reset() && sought.tables as usize == tables);
    assert!(sought.threads as usize >= expected, "{} threads prepared the seek's tables", sought.threads);
}

/// A head held past the newest step (a paused clock, a driven head) is reached by the next step:
/// it ends at the head, not a step past it, so a consumer held there takes it. With one step of
/// ring and an hour a step, the head is moved five hours on while the ring is full, then held.
#[test]
fn a_head_held_past_the_newest_step_is_reached_by_the_next_step() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, BLOCK)]);
    let log = Log::default();
    let settings = Settings { step_threads: 1, fetch_threads: 1, ahead: 100, ring: 1, speed: 1.0, rate: 1.0 };
    let (playhead, mut consumer) = start(&pages, settings, &[0], &log);
    let mut taken = Vec::new();
    playhead.advance(0);
    playhead.wait_until(|| {
        take(&mut consumer, 0, &mut taken);
        !taken.is_empty() && playhead.ring_len() == 1
    });
    playhead.advance(5);
    playhead.wait_until(|| {
        take(&mut consumer, 5, &mut taken);
        taken.last().is_some_and(|step| step.last() == 5) || playhead.ring_len() == 1 && taken.last().is_some_and(|step| step.last() < 5) && playhead.newest().is_some_and(|newest| newest > 5)
    });
    let last = taken.last().unwrap();
    assert_eq!(last.last(), 5, "the step after the newest ends at the held head, not past it (newest {:?})", playhead.newest());
    let changes = last.table(0).unwrap();
    assert_eq!(changes.head(), Some(5));
    mirrors(&taken, |step, held| assert!(held[&0].holds(&states[0][0][step.table(0).unwrap().head().unwrap() as usize])));
}

/// A table declared while the clock is held joins where the frames are, not where the clock is.
/// The clock stays paused at the seek's hour while two frame sides advance their own head: one
/// has taken through hour 120, the other through 96. The join lands at 120, after the step the
/// second has still to take, and arrives there within a few seconds.
#[test]
fn a_table_declared_while_the_clock_is_held_joins_where_the_frames_are_not_where_the_clock_is() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 2, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), whole_ahead(), None).unwrap();
    let a = playhead.declare(every(&[0]), Some(kept(0, &log))).unwrap();
    let (mut ahead, mut behind) = (Side::new(&playhead, a), Side::new(&playhead, a));
    playhead.seek(0);
    frames(&playhead, &mut [&mut ahead, &mut behind], 0, 96);
    playhead.advance(120);
    playhead.wait_until(|| {
        take(&mut ahead.consumer, 120, &mut ahead.taken);
        ahead.taken.last().is_some_and(|step| step.last() == 120)
    });
    assert_eq!(playhead.head(Instant::now()), 0, "the clock is held where it was sought");
    assert!(behind.consumer.reaches(120) && !ahead.consumer.reaches(120), "the side behind has a step to take at the head, the one ahead none");
    let b = playhead.declare(every(&[1]), Some(kept(1, &log))).unwrap();
    let mut late = Side::new(&playhead, b);
    let until = Instant::now() + Duration::from_secs(5);
    playhead.wait_until(|| {
        assert!(Instant::now() < until, "the table declared while held did not arrive (joins at the head {})", playhead.held().2.joins_at_head);
        take(&mut late.consumer, 120, &mut late.taken);
        late.joined(1).is_some()
    });
    let joined = late.joined(1).unwrap();
    assert_eq!(joined.last(), 120, "the furthest hour a frame side took");
    assert!(joined.states[&1].holds(&states[0][1][120]));
    frames(&playhead, &mut [&mut ahead, &mut behind, &mut late], 120, 400);
    follows(&late.taken, 1, &states[0][1]);
}

/// While playing, tables a reader retires and declares again come back at the hour its consumer
/// has, in a step of no hours with its rebuilt states: declared from the hook as the step ending at
/// hour 264 is built, and again with the ring full. Table 0, which another reader keeps, comes from
/// a lane of the new union, and the keeper sees no reset of it; table 1, new to the lanes, is
/// sought alone. Each is followed from there.
#[test]
fn a_table_declared_again_while_playing_arrives_at_the_hour_its_consumer_has() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 2, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let late: Arc<std::sync::OnceLock<(std::sync::Weak<Playhead>, Reader)>> = Arc::default();
    let declared = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (hooked, declaring) = (Arc::clone(&late), Arc::clone(&declared));
    // It declares once: a declaration that joins nothing is shown to the hook in a step of no
    // hours at the newest step, which ends at 264 when that is the newest.
    let hook: super::Hook = Box::new(move |step, _| {
        if step.last() == 264
            && !declaring.load(Ordering::Acquire)
            && let Some((playhead, reader)) = hooked.get()
            && let Some(playhead) = playhead.upgrade()
        {
            playhead.redeclare(*reader, every(&[0, 1])).unwrap();
            declaring.store(true, Ordering::Release);
        }
    });
    let playhead = Arc::new(Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), whole_ahead(), Some(hook)).unwrap());
    let keeper = playhead.declare(vec![Declared { table: 0, columns: vec![0], keyed: true, passing: false }], Some(kept(0, &log))).unwrap();
    let reader = playhead.declare(every(&[0, 1]), Some(kept(1, &log))).unwrap();
    let (mut kept_side, mut side) = (Side::new(&playhead, keeper), Side::new(&playhead, reader));
    playhead.play(24.0);
    playhead.seek(0);
    frames(&playhead, &mut [&mut kept_side, &mut side], 0, 120);
    playhead.redeclare(reader, Vec::new()).unwrap();
    assert!(playhead.newest().unwrap() < 264, "the step the hook declares in is built after this");
    late.set((Arc::downgrade(&playhead), reader)).ok();
    // The hook declares as the step ending at 264 is built, up to the ring's three steps past the
    // frames. The frames play on until it has; then the head stands where they are, and the tables
    // land there once their lanes are read. A head driven on without waiting outruns the reads:
    // it reached the recording's end before a lane was prepared.
    let retired = side.taken.len();
    let arrived = |side: &Side, table: usize| side.taken[retired..].iter().position(|step| step.states.contains_key(&table)).map(|at| retired + at);
    let mut head = 120;
    while !declared.load(Ordering::Acquire) {
        assert!(head < 1200, "the hook never saw the step ending at 264 (newest {:?})", playhead.newest());
        frames(&playhead, &mut [&mut kept_side, &mut side], head, head + 24);
        head += 24;
    }
    let deadline = Instant::now() + Duration::from_secs(50);
    playhead.wait_until(|| {
        take(&mut kept_side.consumer, head, &mut kept_side.taken);
        take(&mut side.consumer, head, &mut side.taken);
        if (0..2).all(|table| arrived(&side, table).is_some()) {
            return true;
        }
        let (_, _, counters) = playhead.held();
        let taken: Vec<(i32, i32, Vec<usize>)> = side.taken[retired..].iter().map(|step| (step.first(), step.last(), step.states.keys().copied().collect())).collect();
        assert!(Instant::now() < deadline, "the tables declared at hour 264 did not come back with the head standing at {head}: arrived {:?}, newest {:?}, the steps taken since they were retired (first, last, states) {taken:?}, {counters:?}", (0..2).map(|table| arrived(&side, table)).collect::<Vec<_>>(), playhead.newest());
        false
    });
    for table in 0..2 {
        let at = arrived(&side, table).unwrap();
        let joined = &side.taken[at];
        assert_eq!(joined.first(), joined.last() + 1, "table {table}: a step of no hours at the hour taken");
        // Its state is where its lane was sought; the steps of no hours that come with it bring it
        // to the hour taken when the frames moved on while it was read.
        let mut whole = joined.states[&table].clone();
        for step in side.taken[at + 1..].iter().take_while(|step| step.first() > step.last()) {
            if let Some(changes) = step.table(table) {
                whole.apply(changes);
            }
        }
        assert_eq!(whole.head, Some(joined.last()), "table {table}: with the steps of no hours that come with it, at the hour taken");
        follows(&side.taken[at..], table, &states[0][table]);
    }
    playhead.redeclare(reader, Vec::new()).unwrap();
    frames(&playhead, &mut [&mut kept_side, &mut side], head, head + 120);
    head += 120;
    playhead.wait_until(|| playhead.ring_len() == 3);
    let shown = side.taken.last().unwrap().last();
    let before = side.taken.len();
    playhead.redeclare(reader, every(&[0, 1])).unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    playhead.wait_until(|| {
        assert!(Instant::now() < deadline, "the tables declared with the ring full did not arrive with the head standing at {shown}");
        take(&mut kept_side.consumer, shown, &mut kept_side.taken);
        take(&mut side.consumer, shown, &mut side.taken);
        (0..2).all(|table| side.taken[before..].iter().any(|step| step.states.contains_key(&table)))
    });
    for table in 0..2 {
        let joined = side.taken[before..].iter().find(|step| step.states.contains_key(&table)).unwrap();
        assert_eq!((joined.first(), joined.last(), joined.states[&table].head), (shown + 1, shown, Some(shown)), "declared with the ring full: table {table} at the hour taken");
    }
    frames(&playhead, &mut [&mut kept_side, &mut side], head, head + 6 * 24);
    for table in 0..2 {
        follows(&side.taken[before..], table, &states[0][table]);
    }
    assert!(kept_side.taken.iter().skip(1).all(|step| step.table(0).is_none_or(|changes| !changes.reset()) && step.states.is_empty()), "the keeper sees no reset of its table and is given no state");
    mirrors(&kept_side.taken, |step, held| {
        let at = step.table(0).unwrap().head().unwrap();
        assert!(held[&0].holds(&states[0][0][at as usize]), "the keeper's table 0 at {at}");
    });
}

/// Waits, the head where the frame left it, for the joins' reads to have had their turn: none under
/// way, or a landing check made since `since` with every join under way read and no read of one
/// out ([`super::Counters::ripe_checks`]). Fails by name within 50 s.
fn reads_turn(playhead: &Playhead, since: u64, head: i32) {
    let deadline = Instant::now() + Duration::from_secs(50);
    playhead.wait_until(|| {
        let counters = playhead.held().2;
        if counters.joins_open == 0 || counters.ripe_checks > since {
            return true;
        }
        assert!(Instant::now() < deadline, "the joins under way at hour {head} did not have their read and a landing check within 50 s: {counters:?}");
        false
    });
}

/// Waits, the head where the frame left it, for the stepper to have made the join a declaration
/// asks: its count past `before`. Fails by name within 50 s.
fn join_made(playhead: &Playhead, before: u64, head: i32) {
    let deadline = Instant::now() + Duration::from_secs(50);
    playhead.wait_until(|| {
        let counters = playhead.held().2;
        if counters.joins > before {
            return true;
        }
        assert!(Instant::now() < deadline, "no join was made for the declaration at hour {head} within 50 s: {counters:?}");
        false
    });
}

/// A map mode changed and changed back while playing: the head moves a step a frame and never
/// stands. A reader retires table 0, which another reader keeps two columns of, and declares it
/// again whole, the frame after which the head moves on while its lane is read. Between frames
/// the reads have their turn ([`reads_turn`]), so nothing hangs on which thread wins. The table is
/// back at the hour the declaring consumer had taken when it landed, at most one step past the
/// hour it was declared at, and taken at most two frames after it: the join's lane is read whole
/// at the frames' hour when its prepare starts (`whole_ahead`), so no read of it waits on the
/// head; the turn after the frame that moved the head on lets that read land at the frames' hour
/// then; the next take brings it. Its state with the steps of no hours that come with it is the
/// codec's at that hour, and every step after carries the table hour by hour. The keeper sees none
/// of it: no state, no step of no hours, its columns byte for byte a lane of them stepped alone,
/// and its rows the codec's at every step.
#[test]
fn a_table_retired_and_declared_again_whole_while_the_head_moves_lands_at_the_consumers_hour() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, 4 * BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), whole_ahead(), None).unwrap();
    let keeper = playhead.declare(vec![Declared { table: 0, columns: vec![0, 1], keyed: true, passing: false }], Some(kept(0, &log))).unwrap();
    let reader = playhead.declare(every(&[0]), Some(kept(1, &log))).unwrap();
    let (mut kept_side, mut side) = (Side::new(&playhead, keeper), Side::new(&playhead, reader));
    playhead.play(24.0);
    playhead.seek(0);
    frames(&playhead, &mut [&mut kept_side, &mut side], 0, 240);
    let mut head = 240;
    // One frame: the head a step on, both consumers taking to it, then (with `turn`) the reads'.
    let frame = |kept_side: &mut Side, side: &mut Side, head: &mut i32, turn: bool| {
        let since = playhead.held().2.ripe_checks;
        *head += 24;
        frames(&playhead, &mut [kept_side, side], *head, *head);
        if turn {
            reads_turn(&playhead, since, *head);
        }
    };
    log.lock().unwrap().clear();

    // Retired: the lane narrows to the keeper's columns, a join of no reader, landed by the turn
    // after the next frame.
    let joins = playhead.held().2.joins;
    playhead.redeclare(reader, Vec::new()).unwrap();
    join_made(&playhead, joins, head);
    frame(&mut kept_side, &mut side, &mut head, true);
    let counters = playhead.held().2;
    assert_eq!(counters.joins_open, 0, "the lane narrowed to the keeper's columns had landed by the turn after the frame at hour {head}: {counters:?}");
    frame(&mut kept_side, &mut side, &mut head, true);

    // Declared again whole: the head moves on at once while the lane is read.
    let declared = head;
    let before = side.taken.len();
    let joins = playhead.held().2.joins;
    playhead.redeclare(reader, every(&[0])).unwrap();
    join_made(&playhead, joins, head);
    frame(&mut kept_side, &mut side, &mut head, true);
    frame(&mut kept_side, &mut side, &mut head, true);
    let arrived = side.taken[before..].iter().position(|step| step.states.contains_key(&0)).map(|at| before + at);
    let Some(at) = arrived else {
        let taken: Vec<(i32, i32, Vec<usize>)> = side.taken[before..].iter().map(|step| (step.first(), step.last(), step.states.keys().copied().collect())).collect();
        panic!("table 0, declared again at hour {declared}, was not taken within two frames (head {head}): the steps taken since (first, last, states) {taken:?}, {:?}", playhead.held().2);
    };
    for _ in 0..12 {
        frame(&mut kept_side, &mut side, &mut head, true);
    }

    let landing = &side.taken[at];
    let hour = landing.last();
    assert_eq!(landing.first(), hour + 1, "a step of no hours");
    assert!((declared..=declared + 24).contains(&hour), "landed at hour {hour}: at most one step past the hour {declared} it was declared at");
    assert_eq!(side.taken[at - 1].last(), hour, "at the hour the declaring consumer had taken when it landed");
    let mut whole = landing.states[&0].clone();
    for step in side.taken[at + 1..].iter().take_while(|step| step.first() > step.last()) {
        if let Some(changes) = step.table(0) {
            whole.apply(changes);
        }
    }
    assert_eq!(whole.head, Some(hour), "with the steps of no hours that come with it, the table stands at hour {hour}");
    assert!(whole.holds(&states[0][0][hour as usize]), "whole: every column the codec's at hour {hour}");
    follows(&side.taken[at..], 0, &states[0][0]);
    assert!(side.taken[at..].iter().filter(|step| step.first() <= step.last()).count() >= 12, "the steps after the landing, hour by hour");
    assert_eq!(*log.lock().unwrap(), vec![(1, 0)], "only the declaring reader rebuilds: the narrowing join has no reader");

    as_stepped_alone(&pages, &kept_side.taken, 0, &[0, 1]);
    assert!(kept_side.taken.iter().skip(1).all(|step| step.states.is_empty() && step.first() <= step.last()), "the keeper is given no state and no step of no hours");
    mirrors(&kept_side.taken, |step, held| {
        let at = step.table(0).unwrap().head().unwrap();
        assert!(held[&0].holds(&states[0][0][at as usize]), "the keeper's table 0 at {at}");
    });
}

/// The unit models' case. A reader declared after a seek's reset was taken, the clock playing and
/// the head standing at the seek's hour (each frame asks it again) while the ring holds its steps
/// built ahead: the table the lanes hold for the map (owed, landed from memory at the newest step
/// built, which this head does not reach) is read for the reader at the hour it stands at, and the
/// table they do not hold is joined there; both arrive at that hour in a step of no hours with the
/// reader's state there, and follow hour by hour once the head moves; the map sees none of it.
/// Before, the owed table waited at the newest step built and held the joined one with it.
#[test]
fn a_reader_declared_after_a_seek_with_the_head_standing_there_has_its_tables_at_that_hour() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 2, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), whole_ahead(), None).unwrap();
    let map = playhead.declare(every(&[0]), Some(kept(0, &log))).unwrap();
    let mut map_side = Side::new(&playhead, map);
    playhead.play(24.0);
    playhead.seek(0);
    frames(&playhead, &mut [&mut map_side], 0, 0);
    playhead.wait_until(|| playhead.ring_len() == 3);
    assert!(playhead.newest().is_some_and(|newest| newest > 0), "the ring holds steps built past the seek's hour");
    log.lock().unwrap().clear();
    let late = playhead.declare(every(&[0, 1]), Some(kept(1, &log))).unwrap();
    let mut late_side = Side::new(&playhead, late);
    let deadline = Instant::now() + Duration::from_secs(50);
    playhead.wait_until(|| {
        // A frame at the head it stands at, as the map side makes one: the head stands.
        playhead.advance(0);
        std::thread::yield_now();
        take(&mut map_side.consumer, 0, &mut map_side.taken);
        take(&mut late_side.consumer, 0, &mut late_side.taken);
        if (0..2).all(|table| late_side.joined(table).is_some()) {
            return true;
        }
        let taken: Vec<(i32, i32, Vec<usize>)> = late_side.taken.iter().map(|step| (step.first(), step.last(), step.states.keys().copied().collect())).collect();
        assert!(Instant::now() < deadline, "the tables declared after the seek did not arrive with the head standing at its hour 0 within 50 s: newest {:?}, the reader took (first, last, states) {taken:?}, {:?}", playhead.newest(), playhead.held().2);
        false
    });
    for table in 0..2 {
        let joined = late_side.joined(table).unwrap();
        assert_eq!((joined.first(), joined.last()), (1, 0), "table {table}: a step of no hours at the seek's hour, where the head stands");
        assert_eq!(joined.states[&table].head, Some(0));
        assert!(joined.states[&table].holds(&states[0][table][0]), "table {table} at hour 0");
    }
    assert!(map_side.taken.iter().skip(1).all(|step| step.states.is_empty() && step.first() <= step.last()), "the map is given no state and no step of no hours");
    frames(&playhead, &mut [&mut map_side, &mut late_side], 0, 240);
    for table in 0..2 {
        follows(&late_side.taken, table, &states[0][table]);
    }
    as_stepped_alone(&pages, &map_side.taken, 0, &(0..KINDS.len()).collect::<Vec<_>>());
    assert!(log.lock().unwrap().iter().all(|&(tag, _)| tag == 1), "only the late reader rebuilds");
}

/// Case 1, the head moving. A table declared again (the reader retired it while another kept it
/// whole, so the lanes hold it as the reader reads it) while the head moves a step a frame, each
/// frame a new head: it comes from memory, in a step of no hours at the newest step built, which
/// the consumer takes when it takes that step, within a step of the newest one at the declaration.
/// No read is made for it (no join, and the file read no more), the stepper holds no step
/// (`join_holds` zero), and the other reader takes a step every frame throughout. Then the table
/// follows hour by hour, and the other reader's columns are byte for byte a lane stepped alone.
#[test]
fn a_table_declared_again_with_the_head_moving_lands_from_memory_at_the_newest_step_with_no_read() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), whole_ahead(), None).unwrap();
    let keeper = playhead.declare(every(&[0]), Some(kept(0, &log))).unwrap();
    let reader = playhead.declare(every(&[0]), Some(kept(1, &log))).unwrap();
    let (mut kept_side, mut side) = (Side::new(&playhead, keeper), Side::new(&playhead, reader));
    playhead.play(24.0);
    playhead.seek(0);
    frames(&playhead, &mut [&mut kept_side, &mut side], 0, 120);
    playhead.redeclare(reader, Vec::new()).unwrap();
    // Every frame a new head: a head asked again stands, and a standing head is read for (case 2).
    frames(&playhead, &mut [&mut kept_side, &mut side], 144, 168);
    // The ring full, the stepper builds nothing more until the head moves: the newest step at the
    // declaration is the one read below, not one built while the test reads it.
    playhead.wait_until(|| playhead.ring_len() == 3);
    let (joins, reads) = (playhead.held().2.joins, pages.extents[0].reads.load(Ordering::Relaxed));
    let before = side.taken.len();
    playhead.redeclare(reader, every(&[0])).unwrap();
    let declared = playhead.newest().unwrap();
    let mut head = 168;
    while side.taken[before..].iter().all(|step| !step.states.contains_key(&0)) {
        let taken: Vec<(i32, i32, Vec<usize>)> = side.taken[before..].iter().map(|step| (step.first(), step.last(), step.states.keys().copied().collect())).collect();
        assert!(head <= declared + 24, "the table declared again with the newest step at {declared} was not taken by the frame at {head}: taken since (first, last, states) {taken:?}, {:?}", playhead.held().2);
        let had = kept_side.taken.len();
        frames(&playhead, &mut [&mut kept_side, &mut side], head + 24, head + 24);
        head += 24;
        assert!(kept_side.taken.len() > had, "the other reader takes a step every frame");
    }
    let at = before + side.taken[before..].iter().position(|step| step.states.contains_key(&0)).unwrap();
    let counters = playhead.held().2;
    assert_eq!(counters.joins, joins, "no read: no join was made for it");
    assert_eq!(pages.extents[0].reads.load(Ordering::Relaxed), reads, "no read: the file was read no more");
    assert_eq!(counters.join_holds, 0, "no step was held for it");
    let landing = &side.taken[at];
    let hour = landing.last();
    assert_eq!(landing.first(), hour + 1, "a step of no hours");
    assert!((declared..=declared + 24).contains(&hour), "at the newest step built, {hour}, from the one at the declaration, {declared}");
    assert_eq!(side.taken[at - 1].last(), hour, "taken once the consumer took that step");
    assert_eq!(landing.states[&0].head, Some(hour));
    assert!(landing.states[&0].holds(&states[0][0][hour as usize]), "whole at hour {hour}");
    frames(&playhead, &mut [&mut kept_side, &mut side], head, head + 240);
    follows(&side.taken[at..], 0, &states[0][0]);
    as_stepped_alone(&pages, &kept_side.taken, 0, &(0..KINDS.len()).collect::<Vec<_>>());
    assert!(kept_side.taken.iter().skip(1).all(|step| step.states.is_empty()), "the other reader is given no state");
}

/// Case 2 met, then left. A table declared again while the head stands at 168 (each frame asks it
/// again), the ring full past it: its landing from memory at the newest step built lies past the
/// head, so a read is made for the reader at the hour it stands at. That read waits at the file
/// (the test's gate) while the head moves on; the consumer takes the landing from memory when it
/// takes the newest step, as a moving head does, and the read, unneeded, is let go: its answer,
/// once the gate opens, is dropped (`standing_dropped`). The table comes once, whole, then hour
/// by hour; no step is held.
#[test]
fn a_table_declared_again_while_the_head_stands_then_moves_takes_the_landing_from_memory_and_drops_the_read() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), whole_ahead(), None).unwrap();
    let keeper = playhead.declare(every(&[0]), Some(kept(0, &log))).unwrap();
    let reader = playhead.declare(every(&[0]), Some(kept(1, &log))).unwrap();
    let (mut kept_side, mut side) = (Side::new(&playhead, keeper), Side::new(&playhead, reader));
    playhead.play(24.0);
    playhead.seek(0);
    frames(&playhead, &mut [&mut kept_side, &mut side], 0, 120);
    playhead.redeclare(reader, Vec::new()).unwrap();
    frames(&playhead, &mut [&mut kept_side, &mut side], 120, 168);
    playhead.wait_until(|| playhead.ring_len() == 3);
    // The whole run was read at the seek: from here only a read made for the table reads the file,
    // and it waits at the gate.
    pages.extents[0].close(0, u64::MAX);
    let (reads, dropped) = (playhead.held().2.standing_reads, playhead.held().2.standing_dropped);
    let before = side.taken.len();
    playhead.redeclare(reader, every(&[0])).unwrap();
    let declared = playhead.newest().unwrap();
    let deadline = Instant::now() + Duration::from_secs(50);
    playhead.wait_until(|| {
        playhead.advance(168);
        std::thread::yield_now();
        take(&mut kept_side.consumer, 168, &mut kept_side.taken);
        take(&mut side.consumer, 168, &mut side.taken);
        let counters = playhead.held().2;
        assert!(Instant::now() < deadline, "no read was made for the table declared again with the head standing at 168 within 50 s: newest {:?}, {counters:?}", playhead.newest());
        counters.standing_reads > reads
    });
    assert!(side.taken[before..].iter().all(|step| !step.states.contains_key(&0)), "standing, it has not come yet: its read waits at the file");
    let mut head = 168;
    while side.taken[before..].iter().all(|step| !step.states.contains_key(&0)) {
        assert!(head <= declared + 24, "the head moving on from 168 did not take the landing from memory at the newest step, {declared}, by the frame at {head}: {:?}", playhead.held().2);
        frames(&playhead, &mut [&mut kept_side, &mut side], head + 24, head + 24);
        head += 24;
    }
    let at = before + side.taken[before..].iter().position(|step| step.states.contains_key(&0)).unwrap();
    let hour = side.taken[at].last();
    assert!((declared..=declared + 24).contains(&hour), "from memory at the newest step built, {hour}");
    assert!(side.taken[at].states[&0].holds(&states[0][0][hour as usize]), "whole at hour {hour}");
    pages.extents[0].open();
    let deadline = Instant::now() + Duration::from_secs(50);
    playhead.wait_until(|| {
        take(&mut kept_side.consumer, head, &mut kept_side.taken);
        take(&mut side.consumer, head, &mut side.taken);
        let counters = playhead.held().2;
        assert!(Instant::now() < deadline, "the read made for the standing head was not let go within 50 s of the gate opening: {counters:?}");
        counters.standing_dropped > dropped
    });
    frames(&playhead, &mut [&mut kept_side, &mut side], head, head + 240);
    assert_eq!(side.taken[before..].iter().filter(|step| step.states.contains_key(&0)).count(), 1, "the table came once");
    follows(&side.taken[at..], 0, &states[0][0]);
    assert_eq!(playhead.held().2.join_holds, 0, "no step was held");
    assert!(kept_side.taken.iter().skip(1).all(|step| step.states.is_empty()), "the other reader is given no state");
}

/// One reader stands while another steps on at the clock's pace. A table the standing reader
/// declares again, which the lanes hold for the other, lands from memory at the newest step, past
/// where it stands; a frame of it standing reads the table from the file at its own hour and hands
/// it a reset there, before the steps it still holds, which then carry the table on hour by hour.
/// It waits on no other reader, and no join is made. Before, the read was made only while the
/// furthest head stood short of the landing, and landed where the furthest reader stood: a reader
/// standing beside one that stepped on never got it (the map's text beside the dashboard's follow).
#[test]
fn a_reader_standing_beside_one_stepping_on_gets_a_table_it_declares_at_its_own_hour() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 2, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), whole_ahead(), None).unwrap();
    let ahead = playhead.declare(every(&[0, 1]), None).unwrap();
    let standing = playhead.declare(every(&[0]), Some(kept(1, &log))).unwrap();
    let (mut fast, mut slow) = (Side::new(&playhead, ahead), Side::new(&playhead, standing));
    playhead.play(24.0);
    playhead.seek(0);
    frames(&playhead, &mut [&mut fast, &mut slow], 0, 96);
    // The ring full past 96 first, so the landing from memory lies past where the reader stands.
    playhead.wait_until(|| playhead.ring_len() == whole_ahead().ring);
    let joins = playhead.held().2.joins;
    playhead.redeclare(standing, every(&[0, 1])).unwrap();
    let before = slow.taken.len();
    // The other steps on ten frames past it, then stands there, still ahead; the reader at 96 stands.
    let mut head = 96;
    let stand = |slow: &mut Side| {
        slow.consumer.advance(96);
        playhead.stand();
        take(&mut slow.consumer, 96, &mut slow.taken);
        slow.taken[before..].iter().any(|step| step.states.contains_key(&1))
    };
    while head < 96 + 240 {
        head += 24;
        fast.consumer.advance(head);
        playhead.wait_until(|| {
            take(&mut fast.consumer, head, &mut fast.taken);
            fast.taken.last().is_some_and(|step| step.last() >= head)
        });
        stand(&mut slow);
    }
    let deadline = Instant::now() + Duration::from_secs(50);
    playhead.wait_until(|| {
        fast.consumer.advance(head);
        take(&mut fast.consumer, head, &mut fast.taken);
        assert!(Instant::now() < deadline, "the reader standing at 96 did not get table 1 within 50 s, the other standing at {head}: {:?}", playhead.held().2);
        stand(&mut slow)
    });
    let at = before + slow.taken[before..].iter().position(|step| step.states.contains_key(&1)).unwrap();
    assert_eq!((slow.taken[at].first(), slow.taken[at].last()), (97, 96), "a step of no hours at the hour it stands at");
    assert!(slow.taken[at].states[&1].holds(&states[0][1][96]), "table 1 at 96 is the recorder's");
    let counters = playhead.held().2;
    assert!(counters.standing_reads >= 1 && counters.joins == joins, "read for the standing reader from the file, no join: {counters:?}");
    // Let go, it takes the steps it held: they carry table 1 on from 97.
    let mut at_hour = 96;
    while at_hour < head {
        at_hour = (at_hour + 24).min(head);
        slow.consumer.advance(at_hour);
        playhead.wait_until(|| {
            take(&mut slow.consumer, at_hour, &mut slow.taken);
            slow.taken.last().is_some_and(|step| step.last() >= at_hour)
        });
    }
    follows(&slow.taken[at..], 1, &states[0][1]);
    assert!(slow.taken[at..].iter().filter(|step| step.states.contains_key(&1)).count() == 1, "the table came once");
}

/// A table declared while playing with the head on the recording's last hour, where no step of
/// hours is built any more, still arrives: as a reset in a step of no hours, at that hour.
#[test]
fn a_table_declared_at_the_recordings_end_while_playing_arrives_in_a_step_of_no_hours() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 2, &[(0, BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), whole_ahead(), None).unwrap();
    let a = playhead.declare(every(&[0]), Some(kept(0, &log))).unwrap();
    let mut first = Side::new(&playhead, a);
    playhead.play(24.0);
    playhead.seek(0);
    let last = playhead.timeline().last();
    frames(&playhead, &mut [&mut first], 0, last);
    playhead.wait_until(|| {
        take(&mut first.consumer, last, &mut first.taken);
        first.taken.last().is_some_and(|step| step.last() == last)
    });
    let b = playhead.declare(every(&[1]), Some(kept(1, &log))).unwrap();
    let mut late = Side::new(&playhead, b);
    let until = Instant::now() + Duration::from_secs(5);
    playhead.wait_until(|| {
        assert!(Instant::now() < until, "the table declared at hour {last}, the last, did not arrive (joins {}, preparing {})", playhead.held().2.joins, playhead.held().0.preparing);
        take(&mut first.consumer, last, &mut first.taken);
        take(&mut late.consumer, last, &mut late.taken);
        late.joined(1).is_some()
    });
    let joined = late.joined(1).unwrap();
    assert_eq!((joined.first(), joined.last()), (last + 1, last), "a step of no hours at the last hour");
    assert_eq!(joined.table(1).and_then(Changes::head), Some(last));
    assert!(joined.states[&1].holds(&states[0][1][last as usize]));
}

/// A table a reader drops and declares again, back to back, so the stepper never applies the
/// declaration between (a hover closing and the next opening, a layer hidden and shown in one
/// frame), both declared from the hook as a step is built: once with the stepper between steps,
/// once with it ahead by the whole ring. The consumer may have let its own state go at the drop,
/// so the table comes again as a reset. It is given from memory at the newest step built; the head
/// stands from the declaration on (each frame asks it again), short of that step, so the table is
/// read at the hour it stands at and lands there when the read answers, the landing from memory
/// withdrawn. The head standing, the hour is decided, not raced.
/// Between the declaration and the landing the consumer's steps carry no reset and no state of the
/// table (only its change sets, which a consumer without the table's state passes over); then one
/// step of no hours at the hour it had taken, whole, and every step after follows it hour by hour.
/// The table it kept declared as it was gets no reset. Before, it came from memory at the newest
/// step built, in the step after it: a head standing short of that step never had it.
#[test]
fn a_table_dropped_and_declared_again_before_the_stepper_applies_arrives_as_a_reset_at_the_consumers_hour() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 2, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let late: Arc<std::sync::OnceLock<(std::sync::Weak<Playhead>, Reader)>> = Arc::default();
    let at = Arc::new(std::sync::atomic::AtomicI32::new(-1));
    let fired = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (hooked, acting, firing) = (Arc::clone(&late), Arc::clone(&at), Arc::clone(&fired));
    let hook: super::Hook = Box::new(move |step, _| {
        if step.last() == acting.load(Ordering::Relaxed)
            && let Some((playhead, reader)) = hooked.get()
            && let Some(playhead) = playhead.upgrade()
        {
            acting.store(-1, Ordering::Relaxed);
            playhead.redeclare(*reader, every(&[0])).unwrap();
            playhead.redeclare(*reader, every(&[0, 1])).unwrap();
            firing.store(true, Ordering::Release);
        }
    });
    let playhead = Arc::new(Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), whole_ahead(), Some(hook)).unwrap());
    let reader = playhead.declare(every(&[0, 1]), Some(kept(1, &log))).unwrap();
    let mut side = Side::new(&playhead, reader);
    late.set((Arc::downgrade(&playhead), reader)).ok();
    playhead.play(24.0);
    playhead.seek(0);
    frames(&playhead, &mut [&mut side], 0, 120);
    // With the head standing at `head`, the reset of table 1 the declaration brings, past `before`.
    let landed = |side: &mut Side, before: usize, head: i32, what: &str| -> usize {
        let deadline = Instant::now() + Duration::from_secs(50);
        playhead.wait_until(|| {
            // A frame at the head it stands at: the head stands.
            playhead.advance(head);
            std::thread::yield_now();
            take(&mut side.consumer, head, &mut side.taken);
            if side.taken[before..].iter().any(|step| step.states.contains_key(&1)) {
                return true;
            }
            let taken: Vec<(i32, i32, Vec<usize>)> = side.taken[before..].iter().map(|step| (step.first(), step.last(), step.states.keys().copied().collect())).collect();
            assert!(Instant::now() < deadline, "{what}: table 1 declared again did not come with the head standing at {head} within 50 s: newest {:?}, taken since (first, last, states) {taken:?}, {:?}", playhead.newest(), playhead.held().2);
            false
        });
        let at = before + side.taken[before..].iter().position(|step| step.states.contains_key(&1)).expect("arrived");
        assert!(side.taken[before..at].iter().all(|step| step.table(1).is_none_or(|changes| !changes.reset())), "{what}: no reset of table 1 before it lands");
        let landing = &side.taken[at];
        let hour = landing.last();
        assert_eq!(landing.first(), hour + 1, "{what}: a step of no hours");
        assert_eq!(side.taken[at - 1].last(), hour, "{what}: at the hour the consumer had taken");
        let mut whole = landing.states[&1].clone();
        for step in side.taken[at + 1..].iter().take_while(|step| step.first() > step.last()) {
            if let Some(changes) = step.table(1) {
                whole.apply(changes);
            }
        }
        assert_eq!(whole.head, Some(hour), "{what}: with the steps of no hours that come with it, at hour {hour}");
        assert!(whole.holds(&states[0][1][hour as usize]), "{what}: table 1 whole at hour {hour}");
        at
    };

    // Between steps: declared as the step to 216 is built; the frames move a step at a time until
    // the hook has run, then stand.
    assert!(playhead.newest().unwrap() < 216);
    let before = side.taken.len();
    at.store(216, Ordering::Relaxed);
    let mut head = 120;
    while !fired.swap(false, Ordering::AcqRel) {
        assert!(head < 600, "the hook never saw the step ending at 216 (newest {:?})", playhead.newest());
        frames(&playhead, &mut [&mut side], head + 24, head + 24);
        head += 24;
    }
    let first = landed(&mut side, before, head, "between steps");

    // With the ring full: declared as the step to 432 is built, the head standing at 360, three
    // steps short of it. The reset is at 360, where the head stands. The hook is armed before the
    // frames reach 360, as only a head at 360 lets that step be built.
    assert!(playhead.newest().unwrap() < 432);
    let before = side.taken.len();
    at.store(432, Ordering::Relaxed);
    frames(&playhead, &mut [&mut side], head, 360);
    playhead.wait_until(|| playhead.ring_len() == 3 && playhead.newest() == Some(432));
    assert!(fired.swap(false, Ordering::AcqRel), "the hook ran as the step to 432 was built");
    let second = landed(&mut side, before, 360, "with the ring full");
    assert_eq!((side.taken[second].first(), side.taken[second].last()), (361, 360), "with the ring full: at hour 360, where the head stands, not at the newest step");
    frames(&playhead, &mut [&mut side], 360, 528);

    follows(&side.taken[first..second], 1, &states[0][1]);
    follows(&side.taken[second..], 1, &states[0][1]);
    assert!(side.taken.iter().all(|step| step.last() <= 120 || !step.states.contains_key(&0)), "the table declared as it was is not joined again");
}

/// While the clock runs, a step a frame, a table a reader declares is its consumer's at once: with
/// the head standing still where the consumer has taken, the next take after the table's lane is
/// read brings a step of no hours at that hour with the reader's state there, equal to the
/// recorder's; and every step after carries the table hour by hour, equal to it at each hour as a
/// reader that had it from the start keeps it. The other consumer's steps are as they were built:
/// no step of no hours, no reset, and its table's cells byte for byte what a lane of it stepped
/// alone gives. Before, a table declared while playing joined past the steps built ahead of the
/// head: with the head standing still it never came.
#[test]
fn a_table_declared_while_playing_is_the_declaring_consumers_at_the_hour_it_has() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 2, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), whole_ahead(), None).unwrap();
    let a = playhead.declare(every(&[0]), Some(kept(0, &log))).unwrap();
    let mut first = Side::new(&playhead, a);
    playhead.play(24.0);
    playhead.seek(0);
    frames(&playhead, &mut [&mut first], 0, 240);
    playhead.wait_until(|| playhead.ring_len() == 3);
    let shown = first.taken.last().unwrap().last();
    let b = playhead.declare(every(&[1]), Some(kept(1, &log))).unwrap();
    let mut second = Side::new(&playhead, b);
    let deadline = Instant::now() + Duration::from_secs(60);
    playhead.wait_until(|| {
        assert!(Instant::now() < deadline, "the table declared at hour {shown} did not arrive with the head standing there (newest {:?})", playhead.newest());
        take(&mut first.consumer, shown, &mut first.taken);
        take(&mut second.consumer, shown, &mut second.taken);
        second.joined(1).is_some()
    });
    let joined = second.joined(1).unwrap();
    assert_eq!((joined.first(), joined.last(), joined.reset()), (shown + 1, shown, false), "a step of no hours at the hour taken");
    assert!(joined.table(0).is_none(), "the other table is not in it");
    assert_eq!(joined.states[&1].head, Some(shown));
    assert!(joined.states[&1].holds(&states[0][1][shown as usize]));
    assert_eq!(first.taken.last().unwrap().last(), shown, "the head stood still");
    frames(&playhead, &mut [&mut first, &mut second], shown, shown + 240);
    follows(&second.taken, 1, &states[0][1]);
    as_stepped_alone(&pages, &first.taken, 0, &(0..KINDS.len()).collect::<Vec<_>>());
    assert!(first.taken.iter().skip(1).all(|step| step.states.is_empty()), "the other reader is given no state");
    let counters = playhead.held().2;
    assert_eq!((counters.joins_at_head, counters.join_held_ns), (1, 0));
    assert_eq!(*log.lock().unwrap(), vec![(0, 0), (1, 1)], "no other table sought again");
}

/// Rows by key with their cells, as [`rows`] gives the recorder's.
fn by_key(mirror: &Mirror) -> BTreeMap<u128, Vec<String>> {
    mirror.rows.values().map(|(key, cells)| (*key, cells.iter().map(|cell| format!("{:?}", cell.value())).collect())).collect()
}

/// A consumer behind the head lands there from memory when it asks ([`Consumer::land`]), before its
/// steps outweigh the state (which would land it unasked). The whole file was read ahead at the
/// seek, and no read is made for it. It passes over every step it had not taken;
/// once the head reaches the newest step built when it asked, its next take is a step of no hours
/// there with its reader's state of each table, equal to the recorder's, every text cell's string
/// named though the lane named it long before; and the steps after carry its tables on. The other
/// consumer takes every step meanwhile as built, the stepper building on, and the ring lets go of
/// the steps the late one was behind by.
#[test]
fn a_consumer_far_behind_lands_at_the_head_from_memory_without_a_read() {
    let named = |at: u32, _: usize, changes: &mut TableChanges<'_>| {
        busy(at, changes);
        if at % 200 == 0 {
            for slot in 0..8 {
                changes.write(slot, 3, Cell::Bytes(at)).unwrap();
            }
        }
    };
    let (pages, states) = recording_of(Writing { cap: 256, ..Writing::at(3) }, 2, &[(0, 2 * BLOCK)], named);
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), whole_ahead(), None).unwrap();
    let map = playhead.declare(every(&[0, 1]), None).unwrap();
    let rebuilt: Rebuild = Arc::new(|lane: &Lane, _: &[usize]| Ok(State::new(Mirror::rebuilt(lane), 0)));
    let reader = playhead.declare(every(&[0, 1]), Some(rebuilt)).unwrap();
    let (mut map, mut late) = (playhead.consumer(map), playhead.consumer(reader));
    playhead.play(24.0);
    playhead.seek(0);
    let mut seen: Vec<Arc<Step>> = Vec::new();
    let mut frame = |head: i32, seen: &mut Vec<Arc<Step>>| {
        playhead.advance(head);
        playhead.wait_until(|| {
            map.take(head, seen);
            seen.last().is_some_and(|step| step.last() >= head)
        });
    };
    frame(0, &mut seen);
    let mut reset = Vec::new();
    late.take(0, &mut reset);
    assert!(reset.len() == 1 && reset[0].reset(), "the late one takes the seek's reset alone");
    for head in (24..=96).step_by(24) {
        frame(head, &mut seen);
    }
    assert!(playhead.kept() > 3, "the steps the late one is behind by are kept for it: {}", playhead.kept());
    assert_eq!(playhead.held().2.landed_behind, 0, "not yet behind by the state");
    let reads = pages.extents[0].reads.load(Ordering::Relaxed);
    let built = playhead.held().2.steps;
    late.land();
    playhead.wait_until(|| playhead.held().2.landings == 1);
    let mut head = 96;
    let mut landed: Vec<Arc<Step>> = Vec::new();
    while landed.is_empty() {
        assert!(head < 1200, "the late one did not land by hour {head} (newest {:?})", playhead.newest());
        head += 24;
        frame(head, &mut seen);
        late.take(head, &mut landed);
    }
    let landing = Arc::clone(&landed[0]);
    let hour = landing.last();
    // At the newest step built when the stepper landed it: 96 itself when it had built nothing
    // past the hour the map had taken, later otherwise. Its state is read at its own hour below.
    assert!((landing.first(), landing.reset()) == (hour + 1, false) && hour >= 96, "a step of no hours at or past where it asked, at {hour}");
    assert_eq!(pages.extents[0].reads.load(Ordering::Relaxed), reads, "no read was made for it");
    playhead.wait_until(|| playhead.held().2.steps > built);
    assert!(playhead.kept() <= 4, "the ring let go of the steps it was behind by: {}", playhead.kept());
    let mut held = BTreeMap::new();
    for table in 0..2 {
        assert!(landing.table(table).is_some_and(Changes::reset), "table {table}'s reset");
        let state = *late.state(&landing, table).expect("its state").downcast::<Mirror>().unwrap();
        assert_eq!(by_key(&state), rows(&states[0][table][hour as usize]), "table {table} at {hour}");
        let named: Vec<u32> = state.rows.values().flat_map(|(_, cells)| cells.iter().filter_map(|cell| if let Owned::Text(id) = cell { Some(*id) } else { None })).collect();
        assert!(named.iter().any(|&id| id + 50 < hour as u32), "a text cell named long before the landing");
        assert!(named.iter().all(|id| state.texts.contains_key(id)), "every text cell's string is named");
        held.insert(table, state);
    }
    for _ in 0..5 {
        head += 24;
        frame(head, &mut seen);
        late.take(head, &mut landed);
    }
    assert!(landed.len() > 5, "the steps after the landing");
    for step in &landed[1..] {
        for (&table, state) in &mut held {
            let changes = step.table(table).expect("its tables carried on");
            state.apply(changes);
            let at = changes.head().unwrap();
            assert_eq!(by_key(state), rows(&states[0][table][at as usize]), "table {table} at {at}");
        }
    }
    assert!(seen.iter().skip(1).all(|step| step.first() <= step.last() && !step.reset() && step.states.lock().unwrap().is_empty()), "the other consumer takes the steps as built");
    assert!(seen.windows(2).all(|pair| pair[1].seq() == pair[0].seq() + 1 && pair[1].first() == pair[0].last() + 1));
    assert_eq!(playhead.held().2.join_held_ns, 0);
}

/// A hundred tables declared at once while the clock runs are read at once, each on a thread of
/// its own while the machine has one: with every read of the file held, as many of their reads
/// wait together as there are threads lent to the joins, and the fetcher with no read to do. Let
/// go, all arrive in one step for their reader at the hour its consumer has, the head standing
/// still: in the time of their reads, not of the head's walk.
#[test]
fn a_hundred_tables_declared_together_are_read_at_once_and_arrive_together_at_the_consumers_hour() {
    let tables = 101;
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, tables, &[(0, BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), whole_ahead(), None).unwrap();
    let a = playhead.declare(every(&[0]), Some(kept(0, &log))).unwrap();
    let mut first = Side::new(&playhead, a);
    playhead.play(24.0);
    playhead.seek(0);
    frames(&playhead, &mut [&mut first], 0, 120);
    playhead.wait_until(|| playhead.ring_len() == 3);
    let shown = first.taken.last().unwrap().last();
    // Lent: the machine's threads past the stepper and the fetcher, one at the least; and the fetcher.
    let width = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let expected = (width.saturating_sub(2).max(1) + 1).min(tables - 1);
    let gated = &pages.extents[0];
    gated.close(0, u64::MAX);
    let b = playhead.declare(every(&(1..tables).collect::<Vec<_>>()), Some(kept(1, &log))).unwrap();
    let mut second = Side::new(&playhead, b);
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut closed = gated.closed.lock().unwrap();
    while closed.1 < expected {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "{} of {expected} tables' reads waited at once a minute after they were declared", closed.1);
        closed = gated.opened.wait_timeout(closed, left).unwrap().0;
    }
    drop(closed);
    gated.open();
    playhead.wait_until(|| {
        assert!(Instant::now() < deadline, "the tables declared at hour {shown} did not arrive with the head standing there");
        take(&mut first.consumer, shown, &mut first.taken);
        take(&mut second.consumer, shown, &mut second.taken);
        (1..tables).all(|table| second.joined(table).is_some())
    });
    let carried: std::collections::BTreeSet<*const Step> = (1..tables).map(|table| Arc::as_ptr(&second.joined(table).unwrap().step)).collect();
    assert_eq!(carried.len(), 1, "one step brings every table declared at once");
    for table in 1..tables {
        let joined = second.joined(table).unwrap();
        assert_eq!(joined.last(), shown);
        assert!(joined.states[&table].holds(&states[0][table][shown as usize]), "table {table}");
    }
    assert_eq!(first.taken.last().unwrap().last(), shown, "the head stood still");
}

/// A 24-hour step a frame at 1,440 hours a second (60 frames a second), eight steps built ahead,
/// the whole file read ahead.
fn fast() -> Settings {
    Settings { step_threads: 1, fetch_threads: 1, ahead: 100, ring: 8, speed: 1440.0, rate: 60.0 }
}

/// Moves the head an hour at a time from `from` to `to`, each side taking what each head reaches.
fn hour_by_hour(playhead: &Playhead, sides: &mut [&mut Side], from: i32, to: i32) {
    for head in from..=to {
        playhead.advance(head);
        playhead.wait_until(|| {
            // Built through the head before the takes, so each takes the step that reaches it.
            let built = playhead.newest().is_some_and(|newest| newest >= head);
            for side in sides.iter_mut() {
                take(&mut side.consumer, head, &mut side.taken);
            }
            built
        });
    }
    playhead.wait_until(|| {
        for side in sides.iter_mut() {
            take(&mut side.consumer, to, &mut side.taken);
        }
        sides.iter().all(|side| side.taken.last().is_some_and(|step| step.last() >= to))
    });
}

/// Every step of hours after the first follows the one before, each hour once.
fn every_hour_once(taken: &[Taken], who: &str) {
    let mut steps = taken.iter().filter(|step| step.first() <= step.last());
    let mut at = steps.next().expect("a reset").last();
    for step in steps {
        assert_eq!(step.first(), at + 1, "{who}: the step after hour {at}");
        at = step.last();
    }
}

/// When the pace falls from 1,440 to 24 hours a second, a step of 24 hours becomes a step of one,
/// and the steps built past the hour the frames have taken are built again at that size; no step
/// a consumer took is touched. Two consumers: the far one took two steps past the near one when
/// the pace changed. The far one's next steps are an hour each from the hour it had reached; the
/// near one takes the two 24-hour steps the far one took, the very same, then the hours. Each
/// takes every hour once, its tables equal to the recorder's at each step. Before, both took the
/// eight 24-hour steps built ahead first: eight seconds of 24-hour steps at 24 hours a second.
#[test]
fn a_slower_pace_builds_the_steps_past_the_frames_again_at_its_size_and_each_consumer_takes_every_hour_once() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 2, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), fast(), None).unwrap();
    let (a, b) = (playhead.declare(every(&[0, 1]), Some(kept(0, &log))).unwrap(), playhead.declare(every(&[0, 1]), Some(kept(1, &log))).unwrap());
    let (mut far, mut near) = (Side::new(&playhead, a), Side::new(&playhead, b));
    playhead.play(1440.0);
    playhead.seek(0);
    frames(&playhead, &mut [&mut far, &mut near], 0, 240);
    // The ring full past 240 first: the head asked next reaches 24-hour steps built ahead of it.
    playhead.wait_until(|| playhead.ring_len() == fast().ring);
    playhead.advance(288);
    playhead.wait_until(|| {
        take(&mut far.consumer, 288, &mut far.taken);
        far.taken.last().is_some_and(|step| step.last() == 288)
    });
    playhead.wait_until(|| playhead.newest() == Some(288 + 8 * 24));
    let (far_from, near_from) = (far.taken.len(), near.taken.len());
    playhead.play(24.0);
    playhead.wait_until(|| playhead.held().2.resteps == 1);
    hour_by_hour(&playhead, &mut [&mut far, &mut near], 241, 330);
    let hours = |taken: &[Taken]| taken.iter().map(|step| (step.first(), step.last(), step.restep())).collect::<Vec<_>>();
    let far_new = hours(&far.taken[far_from..]);
    assert_eq!(far_new, (289..=330).map(|hour| (hour, hour, 1)).collect::<Vec<_>>(), "the far consumer's steps after the change: an hour each from where it was");
    let near_new = hours(&near.taken[near_from..]);
    assert_eq!(near_new[..2], [(241, 264, 0), (265, 288, 0)], "the near consumer takes the steps the far one took");
    assert_eq!(near_new[2..], far_new[..], "then the same hours");
    for (step, held) in near.taken[near_from..near_from + 2].iter().zip(&far.taken[far_from - 2..far_from]) {
        assert!(Arc::ptr_eq(&step.step, &held.step), "step {}..{} is the very step the far one took", step.first(), step.last());
    }
    for (side, who) in [(&far, "far"), (&near, "near")] {
        every_hour_once(&side.taken, who);
        mirrors(&side.taken, |step, held| {
            for table in 0..2 {
                let at = step.table(table).unwrap().head().unwrap();
                assert!(held[&table].holds(&states[0][table][at as usize]), "{who}: table {table} at {at}");
            }
        });
    }
}

/// Paused, the steps ahead are built for the pace the clock will play. Given a pace of an hour a
/// step where 24-hour steps were built past the head, they are built again an hour each, and the
/// head moved an hour at a time takes, each time, one step ending at exactly that hour. Before,
/// it took nothing until the head reached the 24-hour step's end.
#[test]
fn paused_a_new_pace_builds_the_steps_ahead_again_and_each_hour_shows_that_hour() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), fast(), None).unwrap();
    let a = playhead.declare(every(&[0]), Some(kept(0, &log))).unwrap();
    let mut side = Side::new(&playhead, a);
    playhead.seek(0);
    frames(&playhead, &mut [&mut side], 0, 240);
    playhead.wait_until(|| playhead.newest() == Some(240 + 8 * 24));
    playhead.pace(24.0);
    playhead.wait_until(|| playhead.held().2.resteps == 1);
    for hour in 241..=260 {
        let from = side.taken.len();
        playhead.advance(hour);
        playhead.wait_until(|| {
            take(&mut side.consumer, hour, &mut side.taken);
            side.taken.last().is_some_and(|step| step.last() == hour)
        });
        let now: Vec<(i32, i32)> = side.taken[from..].iter().map(|step| (step.first(), step.last())).collect();
        assert_eq!(now, vec![(hour, hour)], "the head at hour {hour} takes one step of that hour");
    }
    every_hour_once(&side.taken, "paused");
    mirrors(&side.taken, |step, held| {
        let at = step.table(0).unwrap().head().unwrap();
        assert!(held[&0].holds(&states[0][0][at as usize]), "table 0 at {at}");
    });
}
