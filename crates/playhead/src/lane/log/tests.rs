use super::{LiveSet, Ordinal, Wanted, rows};
use crate::{
    changes::Changes,
    lane::{
        Lane,
        tests::{KINDS, Memory, Recording, record_run},
    },
    pages::{Pages, Run},
};
use replay_columnar::{
    manifest::Status,
    state::{Cell, Kind},
    store::Store,
    unit::{
        BLOCK, Encoder, Settings,
        reader::{LogEvent, log_span, log_state},
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub(crate) const LOG: [Kind; 4] = [Kind::I64, Kind::OptionalU32, Kind::Bytes, Kind::U32List];

pub(crate) const QUIET: u32 = 2;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Did {
    Appended { ordinal: Ordinal, key: u128, cells: Vec<String> },
    Forgotten(Ordinal),
}

pub(crate) type Done = Vec<(i32, bool, Ordinal)>;

pub(crate) fn log_hour(hour: u32, game: i32, changes: &mut replay_columnar::store::TableChanges<'_>, done: &mut Done) {
    static ITEMS: [u32; 6] = [3, 1, 4, 1, 5, 9];
    if hour / BLOCK == QUIET {
        return;
    }
    let listed = |changes: &replay_columnar::store::TableChanges<'_>| changes.held().listed().unwrap().ones().collect::<Vec<Ordinal>>();
    let forget = |changes: &mut replay_columnar::store::TableChanges<'_>, first: Ordinal, count: u64, done: &mut Done| {
        changes.forget_run(first, count).unwrap();
        done.extend((first..first + count).map(|ordinal| (game, false, ordinal)));
    };
    if hour % 7 == 0
        && let Some(&oldest) = listed(changes).first()
    {
        forget(changes, oldest, 1, done);
    }
    let count = hour % 4 + u32::from(hour % 13 == 0);
    for index in 0..count {
        let seed = u64::from(hour) * 7 + u64::from(index);
        let cells = [
            Some(Cell::I64(seed as i64 * -1_000_003)),
            (seed % 4 != 0).then(|| Cell::OptionalU32((seed % 3 != 0).then_some(seed as u32 * 11))),
            (seed % 5 != 1).then_some(Cell::Bytes(hour)),
            Some(Cell::U32List(&ITEMS[..(seed % 6) as usize])),
        ];
        let ordinal = changes.append(u128::from(hour) << 64 | u128::from(index), &cells).unwrap();
        done.push((game, true, ordinal));
        if hour % 13 == 0 && index + 1 == count {
            forget(changes, ordinal, 1, done);
        }
    }
    if hour % 50 == 0 {
        for &ordinal in listed(changes).iter().take(10).step_by(2) {
            forget(changes, ordinal, 1, done);
        }
    }
    let held = listed(changes);
    let excess = &held[..held.len().saturating_sub(30)];
    let mut at = 0;
    while at < excess.len() {
        let run = excess[at..].iter().enumerate().take_while(|&(offset, &ordinal)| ordinal == excess[at] + offset as Ordinal).count();
        forget(changes, excess[at], run as u64, done);
        at += run;
    }
}

pub(crate) fn record_log(settings: Settings, first: i32, hours: u32) -> (Vec<u8>, Run, Done) {
    let kinds = vec![LOG.to_vec()];
    let mut store = Store::new(kinds.clone(), vec![false]);
    store.declare_logs(&[true]).unwrap();
    store.attach(Encoder::new(&kinds, settings).unwrap());
    let mut memory = Memory::default();
    let mut done = Done::new();
    store.begin_run(first);
    for at in 0..hours {
        let text = format!("text {at}");
        store.begin_hour(first + at as i32, &[text.as_bytes()]).unwrap();
        store.status(0, Status::Complete, true, Some(first + at as i32), true).unwrap();
        log_hour(at, first + at as i32, &mut store.table(0).unwrap(), &mut done);
        store.end_hour(&mut memory).unwrap();
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
        }
    }
    if !hours.is_multiple_of(BLOCK) {
        store.end_run(&mut memory).unwrap();
    }
    (memory.bytes, Run::new(first, memory.pages).unwrap(), done)
}

fn recording(settings: Settings, hours: u32) -> (Arc<Recording>, Done) {
    let (bytes, run, done) = record_log(settings, 0, hours);
    (Arc::new(Recording { bytes, runs: vec![run], kinds: vec![LOG.to_vec()] }), done)
}

pub(crate) fn read_directly(pages: &dyn Pages, run: usize, table: usize) -> (BTreeMap<i32, Vec<Did>>, Vec<(Ordinal, Vec<Ordinal>)>) {
    let held = &pages.runs()[run];
    let kinds = pages.kinds(table).unwrap();
    let columns: Vec<usize> = (0..kinds.len()).collect();
    let table_pages = pages.table_pages(run, table, 0, held.blocks()).unwrap();
    let mut did: BTreeMap<i32, Vec<Did>> = BTreeMap::new();
    log_span(pages.extents(run), &table_pages, kinds, table, &columns, held.first_hour, i32::MIN..=i32::MAX, &mut |hour, event| {
        let entry = did.entry(hour).or_default();
        match event {
            LogEvent::Appended { ordinal, key, rows, at, .. } => entry.push(Did::Appended { ordinal, key, cells: columns.iter().map(|&column| format!("{:?}", rows.value(column, at))).collect() }),
            LogEvent::Forgotten { from, to, .. } => entry.extend((from..to).map(Did::Forgotten)),
            LogEvent::Named { .. } => {}
        }
    })
    .unwrap();
    let ends = (0..held.blocks())
        .map(|at| {
            let state = log_state(pages.extents(run), &table_pages, kinds, table, at).unwrap();
            (state.count().unwrap(), state.listed().unwrap().ones().collect())
        })
        .collect();
    (did, ends)
}

pub(crate) fn states(did: &BTreeMap<i32, Vec<Did>>, first: i32, last: i32) -> BTreeMap<i32, (Ordinal, BTreeSet<Ordinal>)> {
    let (mut count, mut listed, mut out) = (0, BTreeSet::new(), BTreeMap::new());
    for hour in first..=last {
        for event in did.get(&hour).into_iter().flatten() {
            match event {
                Did::Appended { ordinal, .. } => {
                    assert_eq!(*ordinal, count);
                    count += 1;
                    listed.insert(*ordinal);
                }
                Did::Forgotten(ordinal) => assert!(listed.remove(ordinal)),
            }
        }
        out.insert(hour, (count, listed.clone()));
    }
    out
}

pub(crate) fn handed(changes: &Changes) -> BTreeMap<i32, Vec<Did>> {
    let log = changes.log().expect("a log's change set");
    let mut did: BTreeMap<i32, Vec<Did>> = BTreeMap::new();
    for (place, ordinal, hour) in log.rows() {
        let cells = log.columns().iter().map(|written| format!("{:?}", written.value(place as usize))).collect();
        did.entry(hour).or_default().push(Did::Appended { ordinal, key: log.keys().expect("keyed")[place as usize], cells });
    }
    for run in log.forgotten() {
        did.entry(run.hour).or_default().extend((run.from..run.to).map(Did::Forgotten));
    }
    did
}

fn lane(recording: &Arc<Recording>) -> Lane {
    Lane::new(Arc::clone(recording) as Arc<dyn Pages>, 0, 0, &(0..LOG.len()).collect::<Vec<_>>(), true).unwrap()
}

fn step_and_compare(lane: &mut Lane, did: &BTreeMap<i32, Vec<Did>>, states: &BTreeMap<i32, (Ordinal, BTreeSet<Ordinal>)>, step: i32, to: i32, named: &mut BTreeSet<u32>) -> (usize, usize, usize) {
    lane.fetch_through(i32::MAX).unwrap();
    let mut changes = Changes::default();
    let (mut compared, mut both, mut runs) = (0, 0, 0);
    while lane.head().unwrap() < to {
        let from = lane.head().unwrap();
        let head = lane.step((from + step).min(to), &mut changes).unwrap();
        assert_eq!(head, (from + step).min(to));
        let wanted: BTreeMap<i32, Vec<Did>> = did.range(from + 1..=head).map(|(hour, events)| (*hour, events.clone())).collect();
        assert_eq!(handed(&changes), wanted, "step {}..{head}", from + 1);
        named.extend(changes.texts().iter().map(|(id, _)| id));
        let log = changes.log().unwrap();
        assert!(log.forgotten().windows(2).all(|pair| pair[0].hour < pair[1].hour || (pair[0].hour == pair[1].hour && pair[0].to < pair[1].from)), "runs in hour order, ascending and whole within an hour");
        both += usize::from(!log.is_empty() && !log.forgotten().is_empty());
        runs += log.forgotten().iter().filter(|run| run.to - run.from > 1).count();
        for (place, _, _) in log.rows() {
            if let replay_columnar::store::Value::Text(id) = log.column(2).unwrap().value(place as usize) {
                assert!(named.contains(&id), "string {id} of a row appended by hour {head} is named");
            }
        }
        let (count, listed) = &states[&head];
        let state = lane.log().unwrap();
        assert_eq!((state.count(), log.count(), log.listed()), (*count, *count, listed.len() as u64), "at {head}");
        assert_eq!(state.live().ones().collect::<BTreeSet<_>>(), *listed, "live at {head}");
        assert_eq!(log.appended().start + log.len() as Ordinal, *count);
        compared += 1;
        lane.step(head, &mut changes).unwrap();
        let log = changes.log().unwrap();
        assert!(log.is_empty() && log.forgotten().is_empty() && changes.head() == Some(head), "a catch-up step at {head} reports nothing");
    }
    (compared, both, runs)
}

fn follows_the_rule(done: &Done, did: &BTreeMap<i32, Vec<Did>>) {
    let mut by_hour: BTreeMap<i32, Vec<(bool, Ordinal)>> = BTreeMap::new();
    for &(hour, appended, ordinal) in done {
        by_hour.entry(hour).or_default().push((appended, ordinal));
    }
    assert!(by_hour.values().any(|events| events.first().is_some_and(|first| !first.0) && events.iter().any(|event| event.0)), "an hour forgets before it appends");
    assert!(by_hour.values().any(|events| events.iter().any(|&(appended, ordinal)| appended && events.contains(&(false, ordinal)))), "a row appended and forgotten in one hour");
    for (hour, events) in &by_hour {
        let appended: Vec<Ordinal> = events.iter().filter(|event| event.0).map(|event| event.1).collect();
        let mut forgotten: Vec<Ordinal> = events.iter().filter(|event| !event.0).map(|event| event.1).collect();
        forgotten.sort_unstable();
        let read: Vec<(bool, Ordinal)> = did[hour]
            .iter()
            .map(|event| match event {
                Did::Appended { ordinal, .. } => (true, *ordinal),
                Did::Forgotten(ordinal) => (false, *ordinal),
            })
            .collect();
        let wanted: Vec<(bool, Ordinal)> = appended.iter().map(|&ordinal| (true, ordinal)).chain(forgotten.iter().map(|&ordinal| (false, ordinal))).collect();
        assert_eq!(read, wanted, "hour {hour}: its appends, then its forgets ascending");
    }
    assert_eq!(did.len(), by_hour.len(), "no hour the writer did not write");
}

#[test]
fn a_log_lane_steps_equal_the_logs_span_and_state() {
    let (recording, done) = recording(Settings { cap: 48, ..Settings::at(3) }, 4 * BLOCK);
    let entries = recording.runs[0].held_pages().unwrap();
    assert!(entries.iter().all(|page| page.tables[0].log));
    assert!(entries[0].tables[0].parts.len() > 3, "several parts a block");
    let (did, ends) = read_directly(&*recording, 0, 0);
    let last = 4 * BLOCK as i32 - 1;
    let states = states(&did, 0, last);
    for (at, (count, listed)) in ends.iter().enumerate() {
        let end = (at as u32 * BLOCK + BLOCK - 1) as i32;
        assert_eq!((states[&end].0, states[&end].1.iter().copied().collect::<Vec<_>>()), (*count, listed.clone()), "the span's state equals the log's at block {at}'s end");
    }
    assert!(did.range((QUIET * BLOCK) as i32..((QUIET + 1) * BLOCK) as i32).next().is_none(), "a block with no event");
    assert!(did.values().any(|events| events.iter().any(|event| matches!(event, Did::Forgotten(ordinal) if events.iter().any(|other| matches!(other, Did::Appended { ordinal: appended, .. } if appended == ordinal))))), "a forget in its append's hour");
    follows_the_rule(&done, &did);
    for step in [1, 24, 240] {
        let mut lane = lane(&recording);
        lane.seek(0).unwrap();
        let mut named: BTreeSet<u32> = lane.texts().iter().map(|(id, _)| id).collect();
        let (compared, both, runs) = step_and_compare(&mut lane, &did, &states, step, last, &mut named);
        assert_eq!(compared, (last as usize).div_ceil(step as usize), "every step of {step} hours compared");
        assert!(runs > 0, "a run forgotten is handed over as one range");
        if step >= 24 {
            assert!(both > 0, "a step of {step} hours that appends and forgets");
        }
    }
}

#[test]
fn a_log_lane_seek_equals_the_logs_span_and_state() {
    let (recording, _) = recording(Settings { cap: 48, ..Settings::at(3) }, 4 * BLOCK);
    let (did, _) = read_directly(&*recording, 0, 0);
    let last = 4 * BLOCK as i32 - 1;
    let states = states(&did, 0, last);
    let hours = [0, 1, 37, 719, 720, 721, 1000, 1439, 1440, 1441, 1800, 2159, 2160, 2161, 2500, 2850, 2879, 1500, 300, 13];
    for hour in hours {
        let mut lane = lane(&recording);
        let sought = lane.seek(hour).unwrap();
        assert_eq!(lane.head(), Some(hour));
        assert_eq!(lane.state_reads, 0, "a log is never read as state");
        let mut reset = Changes::default();
        lane.reset_into(&mut reset).unwrap();
        let log = reset.log().unwrap();
        let (count, listed) = &states[&hour];
        assert!(reset.reset() && log.is_empty() && reset.rows() == 0, "a reset names no row");
        assert_eq!((log.count(), log.listed()), (*count, listed.len() as u64), "reset at {hour}");
        assert_eq!(lane.log().unwrap().live().ones().collect::<BTreeSet<_>>(), *listed, "live after a seek to {hour}");
        assert!(sought.blocks <= 1, "a seek reads at most its hour's block");
        let mut named: BTreeSet<u32> = lane.texts().iter().map(|(id, _)| id).collect();
        step_and_compare(&mut lane, &did, &states, 24, (hour + 72).min(last), &mut named);
    }
}

#[test]
fn listed_rows_by_ordinal_equal_the_logs_span() {
    let (recording, _) = recording(Settings { cap: 48, ..Settings::at(3) }, 4 * BLOCK);
    let (did, ends) = read_directly(&*recording, 0, 0);
    let appended: BTreeMap<Ordinal, (i32, u128, Vec<String>)> = did
        .iter()
        .flat_map(|(hour, events)| {
            events.iter().filter_map(move |event| match event {
                Did::Appended { ordinal, key, cells } => Some((*ordinal, (*hour, *key, cells.clone()))),
                Did::Forgotten(_) => None,
            })
        })
        .collect();
    let count = ends.last().unwrap().0;
    let block_end = ends[0].0;
    let mut asked: Vec<Ordinal> = (0..count).step_by(37).collect();
    asked.extend([0, 1, block_end - 1, block_end, count - 1]);
    let columns = vec![3, 0, 2];
    let read = rows(&*recording, &Wanted { run: 0, table: 0, ordinals: asked.clone(), columns: columns.clone() }).unwrap();
    asked.sort_unstable();
    asked.dedup();
    assert_eq!(read.ordinals(), asked.as_slice());
    let names: BTreeMap<u32, Vec<u8>> = read.texts().iter().map(|(id, text)| (id, text.to_vec())).collect();
    for (place, &ordinal) in read.ordinals().iter().enumerate() {
        let (hour, key, cells) = &appended[&ordinal];
        assert_eq!((read.hours()[place], read.keys()[place]), (*hour, *key), "row {ordinal}");
        for (written, &column) in read.columns().iter().zip(&columns) {
            assert_eq!(written.column(), column);
            let value = written.value(place);
            assert_eq!(format!("{value:?}"), cells[column], "row {ordinal} column {column}");
            if let replay_columnar::store::Value::Text(id) = value {
                assert_eq!(names.get(&id).map(Vec::as_slice), Some(format!("text {id}").as_bytes()), "row {ordinal} names string {id}");
            }
        }
    }
    assert!(read.read().blocks >= 3 && read.read().walked > 0);
    assert!(read.read().walked < read.read().parts, "a part holding no row asked is fetched for the next one's prefix, not walked");
    let refused = rows(&*recording, &Wanted { run: 0, table: 0, ordinals: vec![count], columns: vec![0] }).unwrap_err().to_string();
    assert!(refused.contains(&format!("holds no row {count} in run 0")), "{refused}");
    let (bytes, run, _) = record_run(Settings::at(3), 1, 0, 30, |_, _, _| {});
    let state = Arc::new(Recording { bytes, runs: vec![run], kinds: vec![KINDS.to_vec()] });
    let refused = rows(&*state, &Wanted { run: 0, table: 0, ordinals: vec![0], columns: vec![0] }).unwrap_err().to_string();
    assert!(refused.contains("Table 0 holds state"), "{refused}");
}

#[test]
fn a_live_set_follows_the_rows_listed_not_the_rows_appended() {
    let mut set = LiveSet::default();
    (0..1_000).for_each(|ordinal| set.append(ordinal));
    assert_eq!(set.forget(10, 20), 10);
    assert_eq!(set.forget(15, 25), 5, "only rows listed count");
    assert!(!set.get(12) && set.get(9) && set.get(25));
    assert_eq!(set.len(), 985);
    assert_eq!(set.forget(0, 999), 984);
    assert_eq!((set.len(), set.lowest(), set.ones().collect::<Vec<_>>()), (1, Some(999), vec![999]));
    set.append(100_000_000);
    assert_eq!(set.ones().collect::<Vec<_>>(), vec![999, 100_000_000]);
    assert!(set.bytes() <= 64 * 16, "{} bytes for two rows listed", set.bytes());
    let mut sparse = LiveSet::default();
    (0..64 * 200).step_by(64).for_each(|ordinal| sparse.append(ordinal));
    for ordinal in (64..64 * 200).step_by(128) {
        assert_eq!(sparse.forget(ordinal, ordinal + 1), 1);
    }
    assert_eq!(sparse.len(), 100);
    assert_eq!(sparse.ones().collect::<Vec<_>>(), (0..64 * 200).step_by(128).collect::<Vec<_>>());
}
