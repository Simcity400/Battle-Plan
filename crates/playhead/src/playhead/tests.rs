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

    fn fail(&self) {
        let mut closed = self.closed.lock().unwrap();
        closed.0 = None;
        closed.2 = true;
        self.opened.notify_all();
    }

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

fn recording(writing: Writing, tables: usize, runs: &[(i32, u32)]) -> (Arc<Runs>, Vec<Vec<Vec<TableState>>>) {
    recording_of(writing, tables, runs, |at, _, changes| busy(at, changes))
}

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

fn settings() -> Settings {
    Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 24.0, rate: 1.0, ..Settings::default() }
}

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

    fn by_key(&self) -> BTreeMap<u128, Vec<String>> {
        self.rows.values().map(|(key, cells)| (*key, cells.iter().map(|cell| format!("{:?}", cell.value())).collect())).collect()
    }

    fn holds(&self, state: &TableState) -> bool {
        let wanted: BTreeMap<u128, Vec<String>> = rows(state).into_iter().map(|(key, cells)| (key, self.columns.iter().map(|&column| cells[column].clone()).collect())).collect();
        self.by_key() == wanted
    }
}

type Log = Arc<Mutex<Vec<(u64, usize)>>>;

fn kept(tag: u64, log: &Log) -> Rebuild {
    let log = Arc::clone(log);
    Arc::new(move |lane: &Lane, columns: &[usize]| {
        log.lock().unwrap().push((tag, lane.table()));
        Ok(State::new(Kept::rebuilt(lane, columns), 0))
    })
}

fn every(tables: &[usize]) -> Vec<Declared> {
    tables.iter().map(|&table| Declared { table, columns: (0..KINDS.len()).collect(), keyed: true, passing: false, hourly: false }).collect()
}

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

fn advance(playhead: &Playhead, head: i32) {
    while !playhead.shared.ask(None, head) {
        std::thread::yield_now();
    }
}

fn consumer_advance(consumer: &Consumer, head: i32) {
    let clamped = consumer.shared.timeline.clamp(head);
    loop {
        consumer.advance(head);
        let recorded = super::lock(&consumer.shared.control).asks.get(&Some(consumer.id)) == Some(&clamped);
        if recorded && consumer.asked.load(Ordering::Relaxed) {
            return;
        }
        std::thread::yield_now();
    }
}

fn reaches(consumer: &Consumer, head: i32) -> bool {
    loop {
        if let Some(hit) = consumer.reaches(head) {
            return hit;
        }
        std::thread::yield_now();
    }
}

fn ahead_nonempty(consumer: &Consumer, out: &mut Vec<Arc<Step>>) {
    let before = out.len();
    loop {
        consumer.ahead(out);
        if out.len() > before {
            return;
        }
        std::thread::yield_now();
    }
}

fn take_steps(consumer: &mut Consumer, head: i32, out: &mut Vec<Arc<Step>>) -> usize {
    let before = out.len();
    loop {
        consumer.take(head, out);
        if consumer.reaches(head) == Some(false) {
            break;
        }
        std::thread::yield_now();
    }
    let taken = out.len() - before;
    if taken > 0 {
        consumer.shared.nudge(|_| {});
    }
    taken
}

fn take(consumer: &mut Consumer, head: i32, out: &mut Vec<Taken>) {
    let mut steps = Vec::new();
    take_steps(consumer, head, &mut steps);
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

fn play(playhead: &Playhead, consumer: &mut Consumer, from: i32, to: i32, every: i32, taken: &mut Vec<Taken>) {
    let mut head = from;
    loop {
        advance(playhead, head);
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
        if taken.last().is_some_and(|step| step.last() >= to) {
            return true;
        }
        let mut ahead = Vec::new();
        super::lock(&consumer.shared.ring).ahead(consumer.id, &mut ahead);
        if let Some(through) = ahead.iter().map(|step| step.last()).find(|&last| last >= to) {
            take(consumer, through, taken);
        }
        taken.last().is_some_and(|step| step.last() >= to)
    });
}

fn caught_up(playhead: &Playhead, consumer: &mut Consumer, taken: &mut Vec<Taken>) {
    playhead.wait_until(|| {
        take(consumer, i32::MAX, taken);
        taken.last().is_some_and(|step| step.tables().iter().all(|table| table.short() == 0))
    });
}

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

#[test]
fn a_seek_abandons_the_ring() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 2, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let (playhead, mut consumer) = start(&pages, settings(), &[0, 1], &log);
    let mut taken = Vec::new();
    play(&playhead, &mut consumer, 0, 96, 24, &mut taken);
    let old = taken.last().unwrap().epoch();
    advance(&playhead, 400);
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
    hold.arm();
    advance(&playhead, 124);
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

#[test]
fn a_seek_runs_each_readers_rebuild_once_for_each_table_it_declared() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 3, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), settings(), None).unwrap();
    let a = playhead.declare(every(&[0, 1]), Some(kept(0, &log))).unwrap();
    let b = playhead.declare(vec![Declared { table: 1, columns: vec![2, 0], keyed: true, passing: false, hourly: false }], Some(kept(1, &log))).unwrap();
    playhead.declare(every(&[0, 2]), None).unwrap();
    let (mut first, mut second) = (playhead.consumer(a), playhead.consumer(b));
    for (seek, hour) in [(1, 300), (2, 1000)] {
        playhead.seek(hour);
        let mut steps = Vec::new();
        playhead.wait_until(|| {
            take_steps(&mut first, hour, &mut steps);
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
        take_steps(&mut second, hour, &mut rest);
        drop((steps, rest));
    }
}

fn said(changes: &Changes, columns: &[usize]) -> String {
    let written: Vec<String> = changes.columns().iter().filter(|written| columns.contains(&written.column())).map(|written| format!("{}: {:?}", written.column(), written.values().map(|(slot, value)| (slot, owned(value))).collect::<Vec<_>>())).collect();
    let (gone, born) = (changes.deleted(), changes.inserted());
    format!("{} {:?} {} {:?} {:?} {:?} {:?} {:?} {written:?}", changes.reset(), changes.head(), changes.short(), gone.slots(), gone.keys(), born.slots(), born.keys(), changes.renumber())
}

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

#[test]
fn a_reader_adding_columns_gets_its_table_at_once_and_another_reader_of_it_sees_nothing() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 2, &[(0, BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), whole_ahead(), None).unwrap();
    let keeper = playhead.declare(vec![Declared { table: 0, columns: vec![0, 1], keyed: true, passing: false, hourly: false }, Declared { table: 1, columns: vec![0, 1, 2, 3], keyed: true, passing: false, hourly: false }], Some(kept(0, &log))).unwrap();
    let changer = playhead.declare(vec![Declared { table: 0, columns: vec![0], keyed: true, passing: false, hourly: false }], Some(kept(1, &log))).unwrap();
    let (mut kept_side, mut changed_side) = (Side::new(&playhead, keeper), Side::new(&playhead, changer));
    playhead.play(24.0);
    playhead.seek(0);
    frames(&playhead, &mut [&mut kept_side, &mut changed_side], 0, 120);
    playhead.wait_until(|| playhead.ring_len() == 3);
    let shown = kept_side.taken.last().unwrap().last();
    log.lock().unwrap().clear();
    let before = changed_side.taken.len();
    playhead.redeclare(changer, vec![Declared { table: 0, columns: vec![0, 2, 3], keyed: true, passing: false, hourly: false }]).unwrap();
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

#[test]
fn two_pieces_join_with_a_crossing_and_report_the_gap_between() {
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
    assert!(!joined.reset() && joined.last() == 1000 && joined.piece() == Some(1), "the boundary is a step of its first hour");
    assert!(joined.tables().iter().all(|changes| !changes.reset()), "and no table of it is reset: none is a log");
    assert!(joined.states.is_empty(), "so every reader keeps the state it has");
    assert!(joined.tables().iter().all(|changes| changes.head() == Some(1000)), "every table stands at the piece's first hour");
    assert!(joined.tables().iter().any(|changes| changes.rows() > 0 || changes.cells() > 0), "and the crossing names what differs");
    let counters = playhead.held().2;
    assert_eq!((counters.pieces_crossed, counters.crossed_tables, counters.crossed_resets), (1, 2, 0));
    mirrors(&taken, |step, held| {
        for table in 0..2 {
            let Some(at) = step.table(table).map(|changes| changes.head().unwrap()) else { continue };
            let (run, hour) = if at < 1000 { (0, at) } else { (1, at - 1000) };
            assert!(held[&table].holds(&states[run][table][hour as usize]), "table {table} at {at}");
        }
    });
    assert_eq!(taken.last().unwrap().last(), playhead.timeline().last());
}

#[test]
fn a_crossing_writes_a_text_cell_whose_id_the_next_run_spells_anew() {
    let named = |at: u32, _: usize, changes: &mut TableChanges<'_>| {
        busy(at, changes);
        if at == 0 {
            for slot in 0..8 {
                changes.write(slot, 3, Cell::Bytes(0)).unwrap();
            }
        }
    };
    let (pages, _) = recording_of(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, BLOCK), (1000, BLOCK + 50)], named);
    let log = Log::default();
    let (playhead, mut consumer) = start(&pages, settings(), &[0], &log);
    let mut taken = Vec::new();
    play(&playhead, &mut consumer, 0, 400, 24, &mut taken);
    play(&playhead, &mut consumer, 900, 1024, 24, &mut taken);
    let joined = taken.iter().find(|step| step.first() == 1000).expect("the boundary's step");
    let changes = joined.table(0).expect("the table crosses");
    let written: usize = changes.columns().iter().filter(|written| written.column() == 3).map(|written| written.slots().len()).sum();
    assert_eq!(written, 8, "every text cell is written, though its id is the same in both runs");
    assert!(changes.texts().iter().any(|(id, text)| id == 0 && text == &b"text 1000/0"[..]), "and named as the run after it spells it");
}

#[test]
fn a_seek_across_a_boundary_resets_whole_and_the_hours_after_it_cross_again() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 2, &[(0, BLOCK), (1000, BLOCK + 50)]);
    let log = Log::default();
    let (playhead, mut consumer) = start(&pages, settings(), &[0, 1], &log);
    let mut taken = Vec::new();
    play(&playhead, &mut consumer, 0, 96, 24, &mut taken);
    let at = |hour: i32| if hour < 1000 { (0, hour as usize) } else { (1, (hour - 1000) as usize) };
    for sought in [1000, 1024, 300] {
        taken.clear();
        playhead.seek(sought);
        advance(&playhead, sought);
        playhead.wait_until(|| {
            take(&mut consumer, sought, &mut taken);
            taken.iter().any(|step| step.reset() && step.last() == sought)
        });
        let reset = taken.iter().find(|step| step.reset() && step.last() == sought).unwrap();
        assert!(reset.tables().iter().all(Changes::reset) && no_rows(reset), "a seek's first picture is a whole reset");
        assert_eq!(reset.states.keys().copied().collect::<Vec<_>>(), vec![0, 1], "with every reader's state");
        let (run, hour) = at(sought);
        for table in 0..2 {
            assert!(reset.states[&table].holds(&states[run][table][hour]), "table {table} sought to {sought}");
        }
    }
    play(&playhead, &mut consumer, 300, 1024, 24, &mut taken);
    let crossed = taken.iter().find(|step| step.first() == 1000).expect("the boundary crossed again after the seek back");
    assert!(!crossed.reset() && crossed.states.is_empty(), "as a step, with no state of its own");
    let mut at_boundary = 0;
    mirrors(&taken, |step, held| {
        for table in 0..2 {
            let Some(head) = step.table(table).map(|changes| changes.head().unwrap()) else { continue };
            let (run, hour) = at(head);
            assert!(held[&table].holds(&states[run][table][hour]), "table {table} at {head}");
            at_boundary += usize::from(head == 1000);
        }
    });
    assert_eq!(at_boundary, 2, "both tables stand at the recorder's state of the boundary's first hour");
    assert_eq!(playhead.held().2.pieces_crossed, 1, "the hours after the seek back crossed the boundary; no seek of them did");
}

struct Dropped(Arc<(Mutex<Vec<String>>, Condvar)>);

impl Drop for Dropped {
    fn drop(&mut self) {
        let (held, dropped) = &*self.0;
        held.lock().unwrap().push(std::thread::current().name().unwrap_or_default().to_owned());
        dropped.notify_all();
    }
}

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
        take_steps(&mut first, 0, &mut steps);
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
    advance(&playhead, 900);
    playhead.wait_until(|| playhead.ring_len() >= 1);
    assert_eq!(playhead.head(later), 900, "held while its reset waits to be taken");
    let mut steps = Vec::new();
    let before = Instant::now();
    take_steps(&mut consumer, 900, &mut steps);
    let after = Instant::now();
    assert!(steps[0].reset() && steps[0].last() == 900, "the first reset taken is the newer seek's");
    assert_eq!(playhead.holding(), None);
    let ten = after + Duration::from_secs(10);
    let head = playhead.head(ten);
    let slack = (after - before).as_secs_f64() * 24.0;
    assert!(head >= 900 + 240 - 1 && f64::from(head) <= 900.0 + 240.0 + slack + 1.0, "runs from the moment its reset is taken: {head}");
}

#[test]
fn a_publication_is_prepared_for_the_hour_the_head_will_reach_and_for_nothing_past_a_still_head() {
    let (pages, _) = recording(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), settings(), None).unwrap();
    let reader = playhead.declare(every(&[0]), Some(kept(0, &log))).unwrap();
    let mut consumer = playhead.consumer(reader);
    let making = Duration::from_millis(10);
    playhead.play(1440.0);
    playhead.seek(300);
    assert_eq!(playhead.head_from(300, making), None, "held at a seek, the hour sought is the hour prepared");
    advance(&playhead, 300);
    playhead.wait_until(|| playhead.ring_len() >= 1);
    let mut steps = Vec::new();
    take_steps(&mut consumer, 300, &mut steps);
    assert!(steps[0].reset());
    assert_eq!(playhead.head_from(300, making), Some(314), "at 1,440 hours a second ten milliseconds is fourteen hours");
    assert_eq!(playhead.head_from(300, Duration::ZERO), Some(300), "a publication that reaches the screen at once looks nowhere ahead");
    playhead.play(24.0);
    assert_eq!(playhead.head_from(300, making), Some(300), "the speed asked for at once: at 24 hours a second ten milliseconds is no hour");
    playhead.pause();
    assert_eq!(playhead.head_from(300, making), None, "paused, nothing is prepared past the hour shown");
    playhead.play(1440.0);
    playhead.seek(100);
    assert_eq!(playhead.head_from(100, making), None, "sought backwards, nothing of the hours it was playing is prepared");
    assert_eq!(playhead.head_from(playhead.timeline().last(), making), None, "nothing past the recording's last hour");
}

#[test]
fn the_clock_runs_from_the_frame_that_shows_the_reset() {
    let (pages, _) = recording(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), settings(), None).unwrap();
    let reader = playhead.declare(every(&[0]), Some(kept(0, &log))).unwrap();
    let mut consumer = playhead.consumer(reader);
    playhead.play(24.0);
    playhead.seek(900);
    advance(&playhead, 900);
    playhead.wait_until(|| playhead.ring_len() >= 1);
    let frame = Instant::now();
    let head = playhead.head(frame);
    let mut steps = Vec::new();
    take_steps(&mut consumer, head, &mut steps);
    assert!(steps[0].reset() && head == 900);
    assert_eq!(playhead.head(frame + Duration::from_secs(1)), 924, "a step a second after the frame that showed the reset");
    assert_eq!(playhead.head(frame + Duration::from_secs(10)), 1140);
}

#[test]
fn a_head_run_ahead_of_every_reader_builds_no_more_than_the_ring_past_what_was_taken() {
    let (pages, _) = recording(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let (playhead, mut consumer) = start(&pages, whole_ahead(), &[0], &log);
    let mut taken = Vec::new();
    play(&playhead, &mut consumer, 0, 0, 24, &mut taken);
    playhead.wait_until(|| playhead.newest() == Some(3 * 24));
    advance(&playhead, 600);
    let mut first = Vec::new();
    take_steps(&mut consumer, 24, &mut first);
    assert_eq!(first.len(), 1, "the step to hour 24");
    playhead.wait_until(|| playhead.newest().is_some_and(|newest| newest >= 600));
    assert_eq!(playhead.newest(), Some(600), "one step to the head once one was taken, none past it while nobody took them");
    assert_eq!(playhead.kept(), 3);
}

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
    while head + 4 * 24 < BLOCK as i32 {
        head += 24;
        advance(&playhead, head);
        let mut now = Vec::new();
        take_steps(&mut consumer, head, &mut now);
        assert!(now.last().is_some_and(|step| step.last() == head), "the step to hour {head} is there to take");
        steps += now.len();
        made.extend(now.iter().map(|step| Arc::as_ptr(step) as usize));
        drop(now);
        playhead.wait_until(|| playhead.newest() == Some(head + 3 * 24));
    }
    assert!(made.len() <= 7, "{} buffers carried {steps} steps; seven are alive at most", made.len());
}

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
        advance(&playhead, head);
        playhead.wait_until(|| {
            take_steps(&mut first, head, &mut seen);
            take_steps(&mut second, head - 24, &mut late);
            playhead.newest().is_some_and(|newest| newest >= head)
        });
        head = (head + 24).min(719);
    }
    advance(&playhead, 719);
    playhead.wait_until(|| {
        take_steps(&mut second, 719, &mut late);
        take_steps(&mut first, 719, &mut seen);
        seen.last().is_some_and(|step| step.last() == 719 && step.tables().iter().all(|table| table.short() == 0))
    });
    take_steps(&mut second, 719, &mut late);
    let spans = |steps: &[Arc<Step>]| steps.iter().map(|step| (step.seq(), step.first(), step.last(), step.tables().iter().map(Changes::short).max())).collect::<Vec<_>>();
    assert_eq!(spans(&seen), spans(&late), "the first consumer's steps against the second's (seq, first, last, most short)");
    for (a, b) in seen.iter().zip(&late) {
        assert!(Arc::ptr_eq(a, b), "step {} is shared, not copied", a.seq());
    }
    assert!(seen.windows(2).all(|pair| pair[1].seq() == pair[0].seq() + 1));
}

#[test]
fn the_head_past_the_end_stops_at_the_last_hour() {
    let (pages, _) = recording(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, BLOCK + 100)]);
    let log = Log::default();
    let (playhead, mut consumer) = start(&pages, settings(), &[0], &log);
    let last = playhead.timeline().last();
    assert_eq!(last, BLOCK as i32 + 99);
    let mut taken = Vec::new();
    play(&playhead, &mut consumer, 0, 600, 24, &mut taken);
    advance(&playhead, i32::MAX);
    playhead.wait_until(|| {
        take(&mut consumer, i32::MAX, &mut taken);
        taken.last().is_some_and(|step| step.last() == last)
    });
    caught_up(&playhead, &mut consumer, &mut taken);
    assert_eq!(playhead.newest(), Some(last));
    assert!(taken.iter().all(|step| step.last() <= last));
    assert_eq!(taken.last().unwrap().table(0).unwrap().head(), Some(last));
}

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
    let reader = playhead.declare(vec![Declared { table: 0, columns: vec![0], keyed: false, passing: false, hourly: false }], None).unwrap();
    let mut consumer = playhead.consumer(reader);
    playhead.seek(0);
    let mut taken = Vec::new();
    advance(&playhead, 300);
    playhead.wait_until(|| {
        take_steps(&mut consumer, i32::MAX, &mut taken);
        taken.last().is_some_and(|step| step.last() >= 300)
    });
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0], (0, 0, true, Some(0), Some(0)), "the reset first");
    for step in &taken {
        let head = step.table(0).and_then(Changes::head);
        assert!(seen.contains(&(step.first(), step.last(), step.reset(), head, head)), "step {}..{}", step.first(), step.last());
    }
}

fn log_recording() -> Arc<Runs> {
    let (bytes, run, _) = record_log(Writing { cap: 48, ..Writing::at(3) }, 0, 4 * BLOCK);
    Arc::new(Runs { extents: vec![gated(bytes)], runs: vec![run], kinds: vec![LOG.to_vec()] })
}

fn log_declared(passing: bool) -> Vec<Declared> {
    vec![Declared { table: 0, columns: (0..LOG.len()).collect(), keyed: true, passing, hourly: false }]
}

fn play_steps(playhead: &Playhead, consumer: &mut Consumer, from: i32, to: i32, every: i32, taken: &mut Vec<Arc<Step>>) {
    let mut head = from;
    loop {
        advance(playhead, head);
        playhead.wait_until(|| {
            take_steps(consumer, head, taken);
            playhead.newest().is_some_and(|newest| newest >= head)
        });
        if head >= to {
            break;
        }
        head = (head + every).min(to);
    }
    playhead.wait_until(|| {
        take_steps(consumer, i32::MAX, taken);
        taken.last().is_some_and(|step| step.last() >= to && step.tables().iter().all(|table| table.short() == 0))
    });
}

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
    advance(&playhead, 400);
    playhead.wait_until(|| playhead.kept() == 3);
    playhead.seek(1000);
    taken.clear();
    take_steps(&mut consumer, i32::MAX, &mut taken);
    assert!(taken.iter().all(|step| step.epoch() != old), "nothing of the old epoch after the seek");
    playhead.wait_until(|| {
        take_steps(&mut consumer, 1000, &mut taken);
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

struct Side {
    consumer: Consumer,
    taken: Vec<Taken>,
}

impl Side {
    fn new(playhead: &Playhead, reader: Reader) -> Self {
        Self { consumer: playhead.consumer(reader), taken: Vec::new() }
    }

    fn joined(&self, table: usize) -> Option<&Taken> {
        self.taken.iter().find(|step| step.states.contains_key(&table))
    }
}

fn whole_ahead() -> Settings {
    Settings { ahead: 100, ..settings() }
}

fn frames(playhead: &Playhead, sides: &mut [&mut Side], from: i32, to: i32) {
    let mut head = from;
    loop {
        advance(playhead, head);
        playhead.wait_until(|| {
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

fn arrive(playhead: &Playhead, side: &mut Side, head: i32, tables: &[usize]) {
    advance(playhead, head);
    playhead.wait_until(|| {
        take(&mut side.consumer, head, &mut side.taken);
        tables.iter().all(|&table| side.joined(table).is_some())
    });
}

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
    ahead_nonempty(&first.consumer, &mut lent);
    assert_eq!(lent.len(), 3, "every step built and not taken is lent");
    assert!(lent.iter().all(|step| step.first() > shown), "only steps past the last taken");
    assert!(lent.windows(2).all(|pair| pair[1].first() == pair[0].last() + 1), "in order, hour after hour");
    let mut again = Vec::new();
    ahead_nonempty(&first.consumer, &mut again);
    assert!(lent.iter().zip(&again).all(|(a, b)| Arc::ptr_eq(a, b)) && again.len() == lent.len(), "looking twice lends the same steps: looking takes none");
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

#[test]
fn a_paused_follow_builds_its_lead_and_seeds_a_new_query_at_the_paused_hour() {
    let (pages, _) = recording(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, 2 * BLOCK)]);
    let (sent, seen) = std::sync::mpsc::channel();
    let hook: super::Hook = Box::new(move |step, _| {
        if step.reset() { sent.send((step.epoch(), step.last())).unwrap(); }
    });
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), whole_ahead(), Some(hook)).unwrap();
    let reader = playhead.declare(every(&[0]), None).unwrap();
    let mut side = Side::new(&playhead, reader);
    playhead.seek(100);
    frames(&playhead, &mut [&mut side], 100, 100);
    let (first, hour) = seen.recv().unwrap();
    assert_eq!(hour, 100);
    playhead.wait_until(|| playhead.newest().is_some_and(|newest| newest > 100));
    playhead.seed_paused_query();
    let (next, hour) = seen.recv().unwrap();
    assert!(next > first);
    assert_eq!(hour, 100);
    frames(&playhead, &mut [&mut side], 100, 100);
    playhead.wait_until(|| playhead.newest().is_some_and(|newest| newest > 100));
    assert_eq!(playhead.head(Instant::now()), 100);
}

#[test]
fn several_tables_declared_at_once_while_paused_arrive_together_and_a_keeper_keeps_its_state() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 3, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), whole_ahead(), None).unwrap();
    let a = playhead.declare(vec![Declared { table: 0, columns: vec![0, 1], keyed: true, passing: false, hourly: false }], Some(kept(0, &log))).unwrap();
    let mut keeper = Side::new(&playhead, a);
    playhead.seek(0);
    frames(&playhead, &mut [&mut keeper], 0, 300);
    playhead.wait_until(|| playhead.ring_len() == 3);
    let shown = keeper.taken.last().unwrap().last();
    let declared = vec![
        Declared { table: 0, columns: vec![2, 3], keyed: true, passing: false, hourly: false },
        Declared { table: 1, columns: vec![0, 2], keyed: true, passing: false, hourly: false },
        Declared { table: 2, columns: vec![0, 1, 2, 3], keyed: true, passing: false, hourly: false },
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

#[test]
fn a_consumer_late_to_take_holds_neither_the_stepper_nor_another_consumer_and_costs_at_most_the_state() {
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
    consumer_advance(&late, 0);
    let mut seen: Vec<Arc<Step>> = Vec::new();
    let mut frame = |head: i32, seen: &mut Vec<Arc<Step>>| {
        consumer_advance(&early, head);
        playhead.wait_until(|| {
            let built = playhead.newest().is_some_and(|newest| newest >= head);
            take_steps(&mut early, head, seen);
            built && !reaches(&early, head)
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
        take_steps(&mut late, 0, &mut taken);
        !taken.is_empty()
    });
    assert!(taken.len() == 1 && taken[0].reset(), "the late one takes the seek's reset, then nothing");
    consumer_advance(&late, 0);
    let mut seen: Vec<Arc<Step>> = Vec::new();
    let mut frame = |head: i32, seen: &mut Vec<Arc<Step>>| {
        consumer_advance(&early, head);
        playhead.wait_until(|| {
            let built = playhead.newest().is_some_and(|newest| newest >= head);
            take_steps(&mut early, head, seen);
            built && !reaches(&early, head)
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
    playhead.wait_until(|| playhead.held().2.caught_up >= 1);
    let counters = playhead.held().2;
    assert!(counters.landed_behind >= 1 && counters.landings == 0, "caught up from the file, not landed from memory: {counters:?}");
    eprintln!("log catch-up from the file: {:.2} ms at the most", counters.catch_up_most_ns as f64 / 1e6);
    let through = (head + 5 * 24).min(last);
    while head < through {
        head = (head + 24).min(last);
        frame(head, &mut seen);
        take_steps(&mut late, head, &mut taken);
    }
    playhead.wait_until(|| {
        take_steps(&mut late, head, &mut taken);
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
        take_steps(&mut late, 0, &mut taken);
        !taken.is_empty()
    });
    consumer_advance(&late, 0);
    pages.extents[0].close(0, u64::MAX);
    let mut seen: Vec<Arc<Step>> = Vec::new();
    let frame = |early: &mut Consumer, head: i32, seen: &mut Vec<Arc<Step>>| {
        consumer_advance(early, head);
        playhead.wait_until(|| {
            let built = playhead.newest().is_some_and(|newest| newest >= head);
            take_steps(early, head, seen);
            built && !reaches(early, head)
        });
    };
    let stand = 144;
    for head in (24..=stand).step_by(24) {
        frame(&mut early, head, &mut seen);
    }
    assert!(seen.last().and_then(|step| step.table(0)).and_then(Changes::head).is_some_and(|head| head < stand), "the log's lane stands short at {stand}");
    if built_past {
        consumer_advance(&early, stand + 96);
        playhead.wait_until(|| playhead.newest().is_some_and(|newest| newest >= stand + 96));
    }
    playhead.wait_until(|| playhead.held().2.landed_behind >= 1);
    assert_eq!(playhead.held().2.caught_up, 0, "its read waits at the gate");
    pages.extents[0].open();
    playhead.wait_until(|| playhead.held().2.caught_up >= 1);
    let mut head = stand;
    while head < stand + 240 {
        head += 24;
        frame(&mut early, head, &mut seen);
        take_steps(&mut late, head, &mut taken);
    }
    playhead.wait_until(|| {
        take_steps(&mut late, head, &mut taken);
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
    take_steps(&mut consumer, 100, &mut steps);
    let sought = steps[0].built().sought.expect("the reset of a seek");
    assert!(steps[0].reset() && sought.tables as usize == tables);
    assert!(sought.threads as usize >= expected, "{} threads prepared the seek's tables", sought.threads);
}

#[test]
fn a_head_held_past_the_newest_step_is_reached_by_the_next_step() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, BLOCK)]);
    let log = Log::default();
    let settings = Settings { step_threads: 1, fetch_threads: 1, ahead: 100, ring: 1, speed: 1.0, rate: 1.0, ..Settings::default() };
    let (playhead, mut consumer) = start(&pages, settings, &[0], &log);
    let mut taken = Vec::new();
    advance(&playhead, 0);
    playhead.wait_until(|| {
        take(&mut consumer, 0, &mut taken);
        !taken.is_empty() && playhead.ring_len() == 1
    });
    advance(&playhead, 5);
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

#[test]
fn a_table_declared_while_the_clock_is_held_joins_where_the_frames_are_not_where_the_clock_is() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 2, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), whole_ahead(), None).unwrap();
    let a = playhead.declare(every(&[0]), Some(kept(0, &log))).unwrap();
    let (mut ahead, mut behind) = (Side::new(&playhead, a), Side::new(&playhead, a));
    playhead.seek(0);
    frames(&playhead, &mut [&mut ahead, &mut behind], 0, 96);
    advance(&playhead, 120);
    playhead.wait_until(|| {
        take(&mut ahead.consumer, 120, &mut ahead.taken);
        ahead.taken.last().is_some_and(|step| step.last() == 120)
    });
    assert_eq!(playhead.head(Instant::now()), 0, "the clock is held where it was sought");
    assert!(reaches(&behind.consumer, 120) && !reaches(&ahead.consumer, 120), "the side behind has a step to take at the head, the one ahead none");
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

#[test]
fn a_table_declared_again_while_playing_arrives_at_the_hour_its_consumer_has() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 2, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let late: Arc<std::sync::OnceLock<(std::sync::Weak<Playhead>, Reader)>> = Arc::default();
    let declared = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (hooked, declaring) = (Arc::clone(&late), Arc::clone(&declared));
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
    let keeper = playhead.declare(vec![Declared { table: 0, columns: vec![0], keyed: true, passing: false, hourly: false }], Some(kept(0, &log))).unwrap();
    let reader = playhead.declare(every(&[0, 1]), Some(kept(1, &log))).unwrap();
    let (mut kept_side, mut side) = (Side::new(&playhead, keeper), Side::new(&playhead, reader));
    playhead.play(24.0);
    playhead.seek(0);
    frames(&playhead, &mut [&mut kept_side, &mut side], 0, 120);
    playhead.redeclare(reader, Vec::new()).unwrap();
    assert!(playhead.newest().unwrap() < 264, "the step the hook declares in is built after this");
    late.set((Arc::downgrade(&playhead), reader)).ok();
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

#[test]
fn a_table_retired_and_declared_again_whole_while_the_head_moves_lands_at_the_consumers_hour() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, 4 * BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), whole_ahead(), None).unwrap();
    let keeper = playhead.declare(vec![Declared { table: 0, columns: vec![0, 1], keyed: true, passing: false, hourly: false }], Some(kept(0, &log))).unwrap();
    let reader = playhead.declare(every(&[0]), Some(kept(1, &log))).unwrap();
    let (mut kept_side, mut side) = (Side::new(&playhead, keeper), Side::new(&playhead, reader));
    playhead.play(24.0);
    playhead.seek(0);
    frames(&playhead, &mut [&mut kept_side, &mut side], 0, 240);
    let mut head = 240;
    let frame = |kept_side: &mut Side, side: &mut Side, head: &mut i32, turn: bool| {
        let since = playhead.held().2.ripe_checks;
        *head += 24;
        frames(&playhead, &mut [kept_side, side], *head, *head);
        if turn {
            reads_turn(&playhead, since, *head);
        }
    };
    log.lock().unwrap().clear();

    let joins = playhead.held().2.joins;
    playhead.redeclare(reader, Vec::new()).unwrap();
    join_made(&playhead, joins, head);
    frame(&mut kept_side, &mut side, &mut head, true);
    let counters = playhead.held().2;
    assert_eq!(counters.joins_open, 0, "the lane narrowed to the keeper's columns had landed by the turn after the frame at hour {head}: {counters:?}");
    frame(&mut kept_side, &mut side, &mut head, true);

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
        advance(&playhead, 0);
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
    frames(&playhead, &mut [&mut kept_side, &mut side], 144, 168);
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
    pages.extents[0].close(0, u64::MAX);
    let (reads, dropped) = (playhead.held().2.standing_reads, playhead.held().2.standing_dropped);
    let before = side.taken.len();
    playhead.redeclare(reader, every(&[0])).unwrap();
    let declared = playhead.newest().unwrap();
    let deadline = Instant::now() + Duration::from_secs(50);
    playhead.wait_until(|| {
        advance(&playhead, 168);
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
    playhead.wait_until(|| playhead.ring_len() == whole_ahead().ring);
    let joins = playhead.held().2.joins;
    playhead.redeclare(standing, every(&[0, 1])).unwrap();
    let before = slow.taken.len();
    let mut head = 96;
    let stand = |slow: &mut Side| {
        consumer_advance(&slow.consumer, 96);
        playhead.stand();
        take(&mut slow.consumer, 96, &mut slow.taken);
        slow.taken[before..].iter().any(|step| step.states.contains_key(&1))
    };
    while head < 96 + 240 {
        head += 24;
        consumer_advance(&fast.consumer, head);
        playhead.wait_until(|| {
            take(&mut fast.consumer, head, &mut fast.taken);
            fast.taken.last().is_some_and(|step| step.last() >= head)
        });
        stand(&mut slow);
    }
    let deadline = Instant::now() + Duration::from_secs(50);
    playhead.wait_until(|| {
        consumer_advance(&fast.consumer, head);
        take(&mut fast.consumer, head, &mut fast.taken);
        assert!(Instant::now() < deadline, "the reader standing at 96 did not get table 1 within 50 s, the other standing at {head}: {:?}", playhead.held().2);
        stand(&mut slow)
    });
    let at = before + slow.taken[before..].iter().position(|step| step.states.contains_key(&1)).unwrap();
    assert_eq!((slow.taken[at].first(), slow.taken[at].last()), (97, 96), "a step of no hours at the hour it stands at");
    assert!(slow.taken[at].states[&1].holds(&states[0][1][96]), "table 1 at 96 is the recorder's");
    let counters = playhead.held().2;
    assert!(counters.standing_reads >= 1 && counters.joins == joins, "read for the standing reader from the file, no join: {counters:?}");
    let mut at_hour = 96;
    while at_hour < head {
        at_hour = (at_hour + 24).min(head);
        consumer_advance(&slow.consumer, at_hour);
        playhead.wait_until(|| {
            take(&mut slow.consumer, at_hour, &mut slow.taken);
            slow.taken.last().is_some_and(|step| step.last() >= at_hour)
        });
    }
    follows(&slow.taken[at..], 1, &states[0][1]);
    assert!(slow.taken[at..].iter().filter(|step| step.states.contains_key(&1)).count() == 1, "the table came once");
}

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
    let landed = |side: &mut Side, before: usize, head: i32, what: &str| -> usize {
        let deadline = Instant::now() + Duration::from_secs(50);
        playhead.wait_until(|| {
            advance(&playhead, head);
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

fn by_key(mirror: &Mirror) -> BTreeMap<u128, Vec<String>> {
    mirror.rows.values().map(|(key, cells)| (*key, cells.iter().map(|cell| format!("{:?}", cell.value())).collect())).collect()
}

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
        advance(&playhead, head);
        playhead.wait_until(|| {
            take_steps(&mut map, head, seen);
            seen.last().is_some_and(|step| step.last() >= head)
        });
    };
    frame(0, &mut seen);
    let mut reset = Vec::new();
    take_steps(&mut late, 0, &mut reset);
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
        take_steps(&mut late, head, &mut landed);
    }
    let landing = Arc::clone(&landed[0]);
    let hour = landing.last();
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
        take_steps(&mut late, head, &mut landed);
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

fn fast() -> Settings {
    Settings { step_threads: 1, fetch_threads: 1, ahead: 100, ring: 8, speed: 1440.0, rate: 60.0, ..Settings::default() }
}

fn hour_by_hour(playhead: &Playhead, sides: &mut [&mut Side], from: i32, to: i32) {
    for head in from..=to {
        playhead.wait_until(|| playhead.newest().is_some_and(|newest| newest >= head));
        advance(playhead, head);
        for side in sides.iter_mut() {
            take(&mut side.consumer, head, &mut side.taken);
        }
    }
    playhead.wait_until(|| {
        for side in sides.iter_mut() {
            take(&mut side.consumer, to, &mut side.taken);
        }
        sides.iter().all(|side| side.taken.last().is_some_and(|step| step.last() >= to))
    });
}

fn every_hour_once(taken: &[Taken], who: &str) {
    let mut steps = taken.iter().filter(|step| step.first() <= step.last());
    let mut at = steps.next().expect("a reset").last();
    for step in steps {
        assert_eq!(step.first(), at + 1, "{who}: the step after hour {at}");
        at = step.last();
    }
}

#[test]
fn a_step_is_the_hours_of_one_publication_at_the_rate_the_host_holds() {
    let (pages, _) = recording(Writing { cap: 256, ..Writing::at(3) }, 2, &[(0, 2 * BLOCK)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), fast(), None).unwrap();
    let declared = playhead.declare(every(&[0, 1]), Some(kept(0, &log))).unwrap();
    let mut side = Side::new(&playhead, declared);
    playhead.play(1440.0);
    playhead.seek(0);
    frames(&playhead, &mut [&mut side], 0, 96);
    let lengths = |taken: &[Taken], from: usize| {
        taken[from..].iter().filter(|step| step.first() <= step.last()).map(|step| step.last() - step.first() + 1).collect::<Vec<_>>()
    };
    assert!(lengths(&side.taken, 1).iter().all(|&hours| hours == 24), "60 publications a second: {:?}", lengths(&side.taken, 1));
    playhead.wait_until(|| playhead.ring_len() == fast().ring);
    let resteps = playhead.held().2.resteps;
    playhead.frame_rate(fast().rate);
    assert_eq!(playhead.held().2.resteps, resteps, "the rate it already has builds no step again");
    playhead.frame_rate(144.0);
    playhead.wait_until(|| playhead.held().2.resteps > resteps);
    playhead.wait_until(|| playhead.ring_len() == fast().ring);
    let from = side.taken.len();
    hour_by_hour(&playhead, &mut [&mut side], 97, 196);
    assert!(lengths(&side.taken, from).iter().all(|&hours| hours == 10), "144 publications a second: {:?}", lengths(&side.taken, from));
    every_hour_once(&side.taken, "the only consumer");
}

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
    playhead.wait_until(|| playhead.ring_len() == fast().ring);
    advance(&playhead, 288);
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
        advance(&playhead, hour);
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

#[test]
fn a_declaration_keeps_the_next_pieces_lanes_for_the_tables_it_leaves_alone() {
    let (pages, _) = recording(Writing { cap: 256, ..Writing::at(3) }, 2, &[(0, BLOCK), (1000, BLOCK + 50)]);
    let log = Log::default();
    let (playhead, mut consumer) = start(&pages, settings(), &[0, 1], &log);
    let mut taken = Vec::new();
    play(&playhead, &mut consumer, 0, 700, 24, &mut taken);
    playhead.wait_until(|| playhead.held().2.piece_prepares > 0);
    let prepared = playhead.held().2.piece_prepares;
    assert_eq!(prepared, 2, "one lane a table");
    for _ in 0..5 {
        let seen = playhead.held().2.declarations_applied;
        let another = playhead.declare(every(&[0, 1]), None).unwrap();
        playhead.wait_until(|| playhead.held().2.declarations_applied > seen);
        drop(playhead.consumer(another));
    }
    assert_eq!(playhead.held().2.piece_prepares, prepared, "the piece's lanes stood");
}

#[test]
fn a_table_read_hourly_carries_its_hours_and_a_reset_or_a_crossing_carries_none() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 2, &[(0, BLOCK), (1000, BLOCK + 50)]);
    let log = Log::default();
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), settings(), None).unwrap();
    let declared = vec![
        Declared { table: 0, columns: (0..KINDS.len()).collect(), keyed: true, passing: false, hourly: true },
        Declared { table: 1, columns: (0..KINDS.len()).collect(), keyed: true, passing: false, hourly: false },
    ];
    let reader = playhead.declare(declared, Some(kept(0, &log))).unwrap();
    let mut consumer = playhead.consumer(reader);
    playhead.seek(0);
    let mut taken = Vec::new();
    play(&playhead, &mut consumer, 0, 400, 24, &mut taken);
    play(&playhead, &mut consumer, 900, 1000 + BLOCK as i32 + 49, 24, &mut taken);
    playhead.seek(300);
    advance(&playhead, 300);
    playhead.wait_until(|| {
        take(&mut consumer, 300, &mut taken);
        taken.iter().any(|step| step.reset() && step.last() == 300)
    });
    play(&playhead, &mut consumer, 300, 1024, 24, &mut taken);
    let mut held: BTreeMap<usize, Kept> = BTreeMap::new();
    let (mut dated, mut whole) = (0, 0);
    for step in &taken {
        held.extend(step.states.clone());
        assert!(step.hours_of(1).is_empty(), "a table read at the step's own grain carries no hour");
        let hours = step.hours_of(0);
        let covered = step.last() - held.get(&0).and_then(|kept| kept.head).unwrap_or(step.last());
        let coarse = step.reset() || step.first() == 1000 || covered > 24;
        assert_eq!(hours.is_empty(), coarse || step.table(0).is_none_or(|changes| changes.rows() == 0 && changes.cells() == 0), "step {}..={} reset {} covering {covered}", step.first(), step.last(), step.reset());
        let mut last = step.last() - covered;
        for set in hours {
            let hour = set.head().expect("an hour's set stands at its hour");
            assert!(hour > last && hour <= step.last() && set.table() == 0, "hour {hour} of {}..={}", step.first(), step.last());
            last = hour;
        }
        for changes in step.tables() {
            let Some(kept) = held.get_mut(&changes.table()) else { continue };
            match (changes.table() == 0, coarse) {
                (true, false) => {
                    dated += hours.len();
                    hours.iter().for_each(|set| kept.apply(set));
                    kept.head = changes.head();
                }
                _ => {
                    whole += usize::from(changes.table() == 0);
                    kept.apply(changes);
                }
            }
        }
        for table in 0..2 {
            let Some(at) = step.table(table).map(|changes| changes.head().expect("a stepped table has a head")) else { continue };
            let (run, hour) = if at < 1000 { (0, at) } else { (1, at - 1000) };
            assert!(held[&table].holds(&states[run][table][hour as usize]), "table {table} at {at} from its hours");
        }
    }
    assert!(dated > 100 && whole > 2, "{dated} hours dated and {whole} intervals taken whole");
}

#[test]
fn completed_steps_keep_exact_byte_bounds_through_parking_and_rewind() {
    let changes = |table, count: u32| {
        let mut changes = Changes::default();
        changes.begin(table, &[], false, false);
        for slot in 0..count {
            changes.inserted.push(1, slot, slot.into());
        }
        changes
    };
    let mut ring = crate::ring::Ring::new(16);
    let budget = crate::ByteBudget::new(256);
    ring.share_budget(budget.clone()).unwrap();
    let fast = ring.join(Reader(0));
    let slow = ring.join(Reader(1));
    ring.asks(fast);
    ring.asks(slow);
    let publish = |ring: &mut crate::ring::Ring, hour, count| {
        ring.publish(Arc::new(Step { epoch: ring.epoch, first: hour, last: hour, piece: Some(0), tables: vec![changes(0, count)], ..Step::default() })).unwrap().unwrap();
    };
    for hour in 1..=4 {
        publish(&mut ring, hour, hour as u32);
    }
    let mut before = Vec::new();
    ring.ahead(fast, &mut before);
    ring.complete(vec![(before[3].seq(), vec![changes(0, 1)]), (before[1].seq(), vec![changes(1, 2), changes(0, 6)])]).unwrap();
    assert_eq!(budget.used(), (13 + 2 + 4) * 8);
    assert_eq!(before[1].payload(), 2 * 8);
    ring.resize(16, 12 * 8);
    assert_eq!(ring.lead(1), (3, 12 * 8, true));
    assert_eq!(ring.lead(2), (2, 4 * 8, false));
    ring.set_bound(9 * 8);
    let mut taken = Vec::new();
    ring.take(fast, 2, &mut taken);
    assert_eq!(ring.parked(slow), None);
    ring.take(slow, 1, &mut taken);
    ring.take(fast, 3, &mut taken);
    assert_eq!(ring.parked(slow), Some(1));
    ring.land_at(slow, 3, vec![]).unwrap().unwrap();
    assert_eq!(ring.lead(3), (1, 8, false));
    ring.cut(before[3].seq());
    assert_eq!(budget.used(), 18 * 8);
    drop(taken);
    drop(before);
    assert_eq!(budget.used(), 0);
    publish(&mut ring, 4, 5);
    assert_eq!(ring.lead(3), (1, 5 * 8, false));
    ring.abandon();
    assert_eq!(budget.used(), 0);
    assert_eq!(ring.lead(0), (0, 0, false));
    publish(&mut ring, 1, 3);
    assert_eq!(ring.lead(0), (1, 3 * 8, false));
    let orphan = Arc::new(Step { tables: vec![changes(0, 4)], ..Step::default() });
    ring.give(Reader(9), 0, std::slice::from_ref(&orphan)).unwrap();
    ring.give(Reader(1), 0, std::slice::from_ref(&orphan)).unwrap();
    assert_eq!(budget.used(), 7 * 8);
    ring.abandon();
    assert_eq!(budget.used(), 4 * 8);
    drop(orphan);
    assert_eq!(budget.used(), 0);
}

#[test]
fn a_lead_builds_its_seconds_of_steps_past_the_head_sized_by_the_pace_and_stops_at_its_bytes() {
    let (pages, _) = recording(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, 2 * BLOCK)]);
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), Settings { ring: 2, rate: 60.0, ..whole_ahead() }, None).unwrap();
    let reader = playhead.declare(every(&[0]), None).unwrap();
    let mut consumer = playhead.consumer(reader);
    let mut taken = Vec::new();
    let from_the_start = |consumer: &mut Consumer, taken: &mut Vec<Taken>| {
        playhead.seek(0);
        advance(&playhead, 0);
        taken.clear();
        playhead.wait_until(|| {
            take(consumer, 0, taken);
            taken.iter().any(|step| step.reset())
        });
    };
    playhead.pace(240.0);
    from_the_start(&mut consumer, &mut taken);
    playhead.wait_until(|| playhead.ring_len() == 2);
    assert_eq!(playhead.lead_held().steps, 2, "no lead: the settings' ring");
    playhead.lead(super::Lead { seconds: 0.5, bytes: usize::MAX });
    assert_eq!(playhead.lead_held().steps, 30, "half a second at 240 h/s in steps of 4 h");
    playhead.wait_until(|| playhead.ring_len() == 30);
    let whole = playhead.lead_held();
    assert!(!whole.capped && whole.bytes > 0, "{whole:?}");
    playhead.pace(24.0);
    assert_eq!(playhead.lead_held().steps, 12, "steps of an hour at 24 h/s: half a second is 12");
    playhead.lead(super::Lead { seconds: 0.01, bytes: usize::MAX });
    assert_eq!(playhead.lead_held().steps, 2, "never fewer than the settings' ring");
    playhead.pace(240.0);
    playhead.lead(super::Lead { seconds: 0.5, bytes: whole.bytes / 3 });
    from_the_start(&mut consumer, &mut taken);
    playhead.wait_until(|| playhead.lead_held().capped);
    let capped = playhead.lead_held();
    assert!(capped.past < 30 && capped.bytes >= whole.bytes / 3, "stopped at its bytes, short of its steps: {capped:?}");
}

#[test]
fn a_contended_ring_refuses_frame_reads_without_consuming_pending_steps() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, BLOCK)]);
    let log = Log::default();
    let (playhead, mut consumer) = start(&pages, whole_ahead(), &[0], &log);
    let mut taken = Vec::new();
    playhead.wait_until(|| {
        take(&mut consumer, 0, &mut taken);
        taken.iter().any(|step| step.reset() && step.last() == 0)
    });
    playhead.wait_until(|| playhead.ring_len() == whole_ahead().ring);
    let mut expected = Vec::new();
    ahead_nonempty(&consumer, &mut expected);
    assert_eq!((expected[0].first(), expected[0].last()), (1, 24));
    assert!(!consumer.asked.load(Ordering::Relaxed));

    let (locked_tx, locked_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let shared = Arc::clone(&playhead.shared);
    let holder = std::thread::spawn(move || {
        let _ring = super::lock(&shared.ring);
        locked_tx.send(()).unwrap();
        let _ = release_rx.recv();
    });
    locked_rx.recv().unwrap();

    assert_eq!(consumer.reaches(24), None);
    let mut preview = Vec::new();
    consumer.ahead(&mut preview);
    assert!(preview.is_empty());
    let mut blocked = Vec::new();
    assert_eq!(consumer.take(24, &mut blocked), 0);
    assert!(blocked.is_empty());
    while super::lock(&playhead.shared.control).head != 24 {
        consumer.advance(24);
        std::thread::yield_now();
    }
    assert!(!consumer.asked.load(Ordering::Relaxed));

    release_tx.send(()).unwrap();
    holder.join().unwrap();

    assert!(!super::lock(&playhead.shared.ring).asked(consumer.id));
    assert!(reaches(&consumer, 24));
    let mut still_ahead = Vec::new();
    ahead_nonempty(&consumer, &mut still_ahead);
    assert!(Arc::ptr_eq(&still_ahead[0], &expected[0]));
    let mut after = Vec::new();
    assert_eq!(take_steps(&mut consumer, 24, &mut after), 1);
    assert_eq!(after.len(), 1);
    assert!(Arc::ptr_eq(&after[0], &expected[0]));
    assert_eq!((after[0].first(), after[0].last()), (1, 24));
    let mut held = taken[0].states[&0].clone();
    held.apply(after[0].table(0).unwrap());
    assert!(held.holds(&states[0][0][24]));
    while !consumer.asked.load(Ordering::Relaxed) {
        consumer.advance(24);
        std::thread::yield_now();
    }
    assert!(consumer.asked.load(Ordering::Relaxed));
    assert!(super::lock(&playhead.shared.ring).asked(consumer.id));
}

#[test]
fn a_contended_control_refuses_frame_advances_without_blocking_or_dropping_takes() {
    let (pages, states) = recording(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, BLOCK)]);
    let log = Log::default();
    let (playhead, mut consumer) = start(&pages, whole_ahead(), &[0], &log);
    let mut taken = Vec::new();
    playhead.wait_until(|| {
        take(&mut consumer, 0, &mut taken);
        taken.iter().any(|step| step.reset() && step.last() == 0)
    });
    playhead.wait_until(|| playhead.ring_len() == whole_ahead().ring);
    assert_eq!(super::lock(&playhead.shared.control).head, 0);
    assert!(!consumer.asked.load(Ordering::Relaxed));

    let (locked_tx, locked_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let shared = Arc::clone(&playhead.shared);
    let holder = std::thread::spawn(move || {
        let _control = super::lock(&shared.control);
        locked_tx.send(()).unwrap();
        let _ = release_rx.recv();
    });
    locked_rx.recv().unwrap();

    playhead.advance(96);
    consumer.advance(72);
    assert!(!consumer.asked.load(Ordering::Relaxed));
    let mut during = Vec::new();
    while during.is_empty() {
        consumer.take(24, &mut during);
        if during.is_empty() {
            std::thread::yield_now();
        }
    }
    assert_eq!(during.len(), 1);
    assert_eq!((during[0].first(), during[0].last()), (1, 24));

    release_tx.send(()).unwrap();
    holder.join().unwrap();

    assert_eq!(super::lock(&playhead.shared.control).head, 0);
    assert!(!consumer.asked.load(Ordering::Relaxed));
    assert!(!super::lock(&playhead.shared.ring).asked(consumer.id));

    while !consumer.asked.load(Ordering::Relaxed) || super::lock(&playhead.shared.control).head != 72 {
        consumer.advance(72);
        std::thread::yield_now();
    }
    assert!(consumer.asked.load(Ordering::Relaxed));
    assert!(super::lock(&playhead.shared.ring).asked(consumer.id));
    assert_eq!(super::lock(&playhead.shared.control).head, 72);
    let mut rest = Vec::new();
    playhead.wait_until(|| {
        take_steps(&mut consumer, 72, &mut rest);
        rest.last().is_some_and(|step| step.last() >= 72)
    });
    let mut held = taken[0].states[&0].clone();
    for step in during.iter().chain(rest.iter()) {
        held.apply(step.table(0).unwrap());
    }
    assert_eq!(held.head, Some(72));
    assert!(held.holds(&states[0][0][72]));
}

#[test]
fn drive_until_wakes_and_errors_when_a_hook_panics_after_an_empty_poll() {
    let (pages, _) = recording(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, BLOCK)]);
    let (step_tx, step_rx) = std::sync::mpsc::channel::<i32>();
    let (polled_tx, polled_rx) = std::sync::mpsc::channel::<()>();
    let hook: super::Hook = Box::new(move |step, _| {
        let _keep = &step_tx;
        let _ = polled_rx.recv();
        panic!("hook panicked at hour {}", step.last());
    });
    let playhead = Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), whole_ahead(), Some(hook)).unwrap();
    let reader = playhead.declare(every(&[0]), None).unwrap();
    let mut consumer = playhead.consumer(reader);
    playhead.seek(0);
    let mut signaled = false;
    let error = playhead
        .drive_until(&mut consumer, 0, || match step_rx.try_recv() {
            Ok(_) => std::ops::ControlFlow::Break(Ok(())),
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                if !signaled {
                    signaled = true;
                    polled_tx.send(()).unwrap();
                }
                std::ops::ControlFlow::Continue(())
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                std::ops::ControlFlow::Break(Err("step hook channel disconnected".into()))
            }
        })
        .expect_err("drive_until must wake and fail when the hook panics");
    assert!(error.contains("panicked") || error.contains("disconnected"), "unexpected error: {error}");
    let newest_error = playhead.wait_newest(0).expect_err("wait_newest must wake on stepper panic");
    assert!(newest_error.contains("panicked"), "unexpected wait_newest error: {newest_error}");
}

#[test]
fn wait_step_after_observes_same_hour_join_delivery_and_queued_ring_steps() {
    let (pages, _) = recording(Writing { cap: 256, ..Writing::at(3) }, 2, &[(0, 40)]);
    let log = Log::default();
    let playhead = Playhead::new(
        Arc::clone(&pages) as Arc<dyn Pages>,
        Timeline::whole(&*pages).unwrap(),
        Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 4, speed: 4.0, rate: 1.0, ..Settings::default() },
        None,
    )
    .unwrap();
    let reader = playhead.declare(every(&[0]), Some(kept(0, &log))).unwrap();
    let mut consumer = playhead.consumer(reader);
    playhead.seek(0);
    let last = playhead.timeline().last();
    let mut steps = Vec::new();
    let mid = 8;
    loop {
        let before = playhead.step_state();
        let head = before.0.unwrap_or(0).min(mid);
        consumer_advance(&consumer, head);
        take_steps(&mut consumer, head, &mut steps);
        if steps.last().is_some_and(|step| step.last() >= mid) {
            break;
        }
        assert!(consumer.wait_step_after(before, mid).unwrap(), "playhead stopped before step {mid}");
    }
    playhead.wait_until(|| playhead.ring_len() == 4);
    let published = super::lock(&playhead.shared.ring).next_seq();
    playhead.wait_until(|| playhead.step_state().1 >= published);
    assert!(playhead.newest().is_some_and(|newest| newest > mid));
    let paused_dropped = playhead.consumer(reader);
    let paused_id = paused_dropped.id;
    drop(paused_dropped);
    playhead.shared.nudge(|control| {
        control.lands.push(paused_id);
        control.caught_up_at = None;
    });
    assert!(
        !consumer.wait_step_after(playhead.step_state(), mid).unwrap(),
        "a paused dropped consumer's discarded landing restores caught_up_at instead of blocking wait_step_after",
    );
    loop {
        let before = playhead.step_state();
        let head = before.0.unwrap_or(0);
        consumer_advance(&consumer, head);
        take_steps(&mut consumer, head, &mut steps);
        if steps.last().is_some_and(|step| step.last() >= last) {
            break;
        }
        assert!(consumer.wait_step_after(before, last).unwrap(), "playhead reached end before step {last}");
    }
    let at_end = playhead.step_state();
    assert!(!consumer.wait_step_after(at_end, last).unwrap(), "at end with all tables caught up and ring drained, wait_step_after reports exhausted");
    let dropped = playhead.consumer(reader);
    let dropped_id = dropped.id;
    drop(dropped);
    playhead.shared.nudge(|control| {
        control.lands.push(dropped_id);
        control.caught_up_at = None;
    });
    assert!(
        !consumer.wait_step_after(playhead.step_state(), last).unwrap(),
        "a dropped consumer's discarded landing restores caught_up_at instead of blocking wait_step_after",
    );
    let before_redeclare = playhead.step_state();
    playhead.redeclare(reader, every(&[0, 1])).unwrap();
    assert!(
        consumer.wait_step_after(before_redeclare, last).unwrap(),
        "redeclaring table 1 at the last hour delivers a same-hour join step instead of reporting exhausted",
    );
    playhead.wait_until(|| {
        playhead.step_state().1 > before_redeclare.1
            && playhead.held().2.joins_open == 0
            && playhead.held().2.declarations_applied == playhead.declarations()
    });
    let queued = playhead.step_state();
    assert!(
        consumer.wait_step_after(queued, last).unwrap(),
        "a join step already queued in the ring is observed even when published_steps has not moved since",
    );
    let before_take = steps.len();
    take_steps(&mut consumer, last, &mut steps);
    assert!(steps[before_take..].iter().any(|step| step.table(1).is_some()), "the same-hour join step carries table 1");
    let drained = playhead.step_state();
    assert!(!consumer.wait_step_after(drained, last).unwrap(), "once the join step is drained at end, wait_step_after reports exhausted again");
}

#[test]
fn a_completing_join_does_not_clobber_a_concurrent_redeclares_caught_up_state() {
    #[derive(Default)]
    struct HookGate {
        state: std::sync::Mutex<(Option<usize>, bool, [bool; 3], bool)>,
        changed: std::sync::Condvar,
    }
    struct OpenGate(Arc<HookGate>);
    impl Drop for OpenGate {
        fn drop(&mut self) {
            let mut state = self.0.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            state.1 = true;
            state.3 = true;
            self.0.changed.notify_all();
        }
    }
    let gate = Arc::new(HookGate::default());
    let hook_gate = Arc::clone(&gate);
    let hook: super::Hook = Box::new(move |step, _| {
        for table in [1usize, 2] {
            if step.table(table).is_some() {
                let mut state = hook_gate.state.lock().unwrap();
                if state.2[table] || state.3 {
                    continue;
                }
                state.2[table] = true;
                state.0 = Some(table);
                state.1 = false;
                hook_gate.changed.notify_all();
                while !state.1 && !state.3 {
                    state = hook_gate.changed.wait(state).unwrap();
                }
            }
        }
    });
    let wait_hook = |target: usize| {
        let mut state = gate.state.lock().unwrap();
        while state.0 != Some(target) {
            state = gate.changed.wait(state).unwrap();
        }
    };
    let release_hook = || {
        let mut state = gate.state.lock().unwrap();
        state.1 = true;
        gate.changed.notify_all();
    };
    let (pages, _) = recording(Writing { cap: 256, ..Writing::at(3) }, 3, &[(0, 40)]);
    let log = Log::default();
    let playhead = Playhead::new(
        Arc::clone(&pages) as Arc<dyn Pages>,
        Timeline::whole(&*pages).unwrap(),
        Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 4, speed: 4.0, rate: 1.0, ..Settings::default() },
        Some(hook),
    )
    .unwrap();
    let _open = OpenGate(Arc::clone(&gate));
    let reader = playhead.declare(every(&[0]), Some(kept(0, &log))).unwrap();
    let mut consumer = playhead.consumer(reader);
    playhead.seek(0);
    let last = playhead.timeline().last();
    let mut steps = Vec::new();
    loop {
        let before = playhead.step_state();
        let head = before.0.unwrap_or(0);
        consumer_advance(&consumer, head);
        take_steps(&mut consumer, head, &mut steps);
        if steps.last().is_some_and(|step| step.last() >= last) {
            break;
        }
        assert!(consumer.wait_step_after(before, last).unwrap(), "playhead reached end before step {last}");
    }
    playhead.redeclare(reader, every(&[0, 1])).unwrap();
    wait_hook(1);
    let before_t1_land = playhead.step_state();
    playhead.redeclare(reader, every(&[0, 1, 2])).unwrap();
    {
        let mut control = super::lock(&playhead.shared.control);
        release_hook();
        while control.published_steps == before_t1_land.1 {
            assert!(control.error.is_none(), "playhead errored while landing table 1: {:?}", control.error);
            assert!(!control.stop, "playhead stopped while landing table 1");
            control = playhead.shared.wake.wait(control).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        assert_eq!(control.caught_up_at, None, "table 1's completion nudge did not clobber table 2's pending declaration");
    }
    wait_hook(2);
    let before_take = steps.len();
    take_steps(&mut consumer, last, &mut steps);
    assert!(steps[before_take..].iter().any(|step| step.table(1).is_some()), "table 1 landed while table 2 is still joining");
    assert!(steps[before_take..].iter().all(|step| step.table(2).is_none()), "table 2 has not landed yet");
    let after_t1 = playhead.step_state();
    let opener = {
        let gate = Arc::clone(&gate);
        std::thread::spawn(move || {
            let mut state = gate.state.lock().unwrap();
            state.1 = true;
            gate.changed.notify_all();
        })
    };
    assert!(consumer.wait_step_after(after_t1, last).unwrap(), "wait_step_after waits for table 2 instead of reporting exhausted");
    opener.join().unwrap();
    playhead.wait_until(|| {
        playhead.step_state().1 > after_t1.1
            && playhead.held().2.joins_open == 0
            && playhead.held().2.declarations_applied == playhead.declarations()
    });
    let before_t2 = steps.len();
    take_steps(&mut consumer, last, &mut steps);
    assert!(steps[before_t2..].iter().any(|step| step.table(2).is_some()), "table 2 landed after release");
    let drained = playhead.step_state();
    assert!(!consumer.wait_step_after(drained, last).unwrap(), "after both joins drain at end, wait_step_after reports exhausted");

    let mut stepper = super::Stepper::new(Arc::clone(&playhead.shared), None, Vec::new());
    stepper.epoch = playhead.epoch();
    stepper.selection_seen = playhead.declarations();
    let mut control = super::lock(&playhead.shared.control);
    let original_selection = control.selection;
    control.caught_up_at = None;
    control.selection = stepper.selection_seen + 1;
    stepper.publish_caught_up(&mut control, Some(last));
    assert_eq!(control.caught_up_at, None, "newer selection blocks setting caught_up_at");
    control.selection = original_selection;
    control.seek = Some((0, stepper.epoch + 1));
    stepper.publish_caught_up(&mut control, Some(last));
    assert_eq!(control.caught_up_at, None, "pending seek blocks setting caught_up_at");
    control.seek = None;
    control.lands.push(0);
    stepper.publish_caught_up(&mut control, Some(last));
    assert_eq!(control.caught_up_at, None, "pending land blocks setting caught_up_at");
    control.lands.clear();
    stepper.epoch = playhead.epoch().wrapping_add(1);
    stepper.publish_caught_up(&mut control, Some(last));
    assert_eq!(control.caught_up_at, None, "mismatched epoch blocks setting caught_up_at");
    control.newest = None;
    stepper.publish_newest(&mut control, last);
    assert_eq!(control.newest, None, "a step of an abandoned epoch leaves the newest hour a seek cleared");
    stepper.epoch = playhead.epoch();
    stepper.publish_caught_up(&mut control, Some(last));
    assert_eq!(control.caught_up_at, Some(last), "current control state accepts caught_up_at");
    stepper.publish_newest(&mut control, last);
    assert_eq!(control.newest, Some(last), "a step of the current epoch is the newest hour");
}

#[test]
fn taking_a_playing_seeks_reset_notifies_control_watchers_with_clock_unlocked() {
    let (pages, _) = recording(Writing { cap: 256, ..Writing::at(3) }, 1, &[(0, 48)]);
    let playhead = Arc::new(Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), settings(), None).unwrap());
    let reader = playhead.declare(every(&[0]), None).unwrap();
    let mut consumer = playhead.consumer(reader);
    consumer.set_blocking(true);
    playhead.play(24.0);
    playhead.seek(10);
    assert!(!playhead.playing(), "clock is held while seek reset has not been taken");
    playhead.wait_until(|| playhead.newest().is_some_and(|newest| newest >= 10));
    let control_wakes = Arc::new(AtomicUsize::new(0));
    let clock_locked = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let _watch = playhead.watch_control({
        let playhead = Arc::clone(&playhead);
        let control_wakes = Arc::clone(&control_wakes);
        let clock_locked = Arc::clone(&clock_locked);
        Arc::new(move || {
            control_wakes.fetch_add(1, Ordering::AcqRel);
            if playhead.clock_held_on_current_thread() {
                clock_locked.store(true, Ordering::Release);
            }
        })
    });
    {
        let _guard = super::lock_clock(&playhead.shared.clock);
        assert!(playhead.clock_held_on_current_thread(), "same-thread clock guard must be detected");
    }
    assert!(!playhead.clock_held_on_current_thread(), "clock guard dropped");
    let before_seq = playhead.control_seq();
    let mut steps = Vec::new();
    let taken = consumer.take(10, &mut steps);
    assert!(taken > 0 && steps.iter().any(|step| step.reset));
    assert!(playhead.playing(), "taking the seek reset clears the hold");
    assert!(playhead.control_seq() > before_seq, "clearing the hold bumps control_seq");
    assert!(control_wakes.load(Ordering::Acquire) > 0, "clearing the hold wakes control watchers");
    assert!(!clock_locked.load(Ordering::Acquire), "clock mutex must be unlocked before notifying watchers");
}

#[test]
fn watch_control_ignores_publications_and_rows_notifying_wakes_on_completion() {
    let (bytes, run, _) = record_log(Writing { cap: 256, ..Writing::at(3) }, 0, 24);
    let gated = gated(bytes);
    let pages = Arc::new(Runs { extents: vec![gated], runs: vec![run], kinds: vec![LOG.to_vec()] });
    let playhead = Arc::new(Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), settings(), None).unwrap());
    let reader = playhead.declare(vec![Declared { table: 0, columns: vec![0, 1], keyed: true, passing: false, hourly: false }], None).unwrap();
    let mut consumer = playhead.consumer(reader);
    consumer.set_blocking(true);
    playhead.seek(0);
    playhead.wait_until(|| playhead.newest().is_some());
    let mut steps = Vec::new();
    let _ = consumer.take(0, &mut steps);
    steps.clear();
    let all_wakes = Arc::new(AtomicUsize::new(0));
    let control_wakes = Arc::new(AtomicUsize::new(0));
    let _all = playhead.watch({
        let all_wakes = Arc::clone(&all_wakes);
        Arc::new(move || { all_wakes.fetch_add(1, Ordering::AcqRel); })
    });
    let _ctrl = playhead.watch_control({
        let control_wakes = Arc::clone(&control_wakes);
        Arc::new(move || { control_wakes.fetch_add(1, Ordering::AcqRel); })
    });
    let seq_before = playhead.control_seq();
    let act_before = playhead.activity();
    consumer.advance(23);
    playhead.wait_until(|| playhead.newest() == Some(23));
    let _ = consumer.take(23, &mut steps);
    assert!(playhead.activity() > act_before && all_wakes.load(Ordering::Acquire) > 0, "publications bump activity and wake watch");
    assert_eq!(playhead.control_seq(), seq_before, "publications must not bump control_seq");
    assert_eq!(control_wakes.load(Ordering::Acquire), 0, "watch_control must not wake on plain publications");

    pages.extents[0].close(0, u64::MAX);
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let asked = playhead.log_reads().rows_notifying(
        Wanted { run: 0, table: 0, ordinals: vec![0], columns: vec![0, 1] },
        Arc::new(move || { let _ = done_tx.send(()); }),
    );
    pages.extents[0].wait_blocked();
    assert!(asked.take().is_none(), "read is still blocked behind extent gate");
    pages.extents[0].open();
    done_rx.recv().unwrap();
    let rows = asked.take().expect("answer ready when on_done fires").unwrap();
    assert_eq!(rows.ordinals(), &[0]);
}
