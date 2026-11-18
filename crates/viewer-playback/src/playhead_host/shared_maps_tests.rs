use super::{Entry, MapSide, Opening};
#[cfg(test)]
use super::{BUDGET, OPENING, open, rows::tests::{hourly_steps, record}};
use crate::layers::{Budget, Carried, Gpu, Inputs, Layer, View};
use crate::playhead_host::mirror::Mirrors;
use playhead::Step;
use replay_data::{Hour, rows::TableId};
#[cfg(test)]
use replay_data::scope::CaptureMode;
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
        mpsc::Sender,
    },
    time::Duration,
};

/// A layer that reads one column of each of its tables and counts its composes.
pub struct Reads {
    tables: Vec<TableId>,
    composes: Arc<AtomicU32>,
}

impl Reads {
    pub fn new(tables: &[TableId]) -> (Box<dyn Layer>, Arc<AtomicU32>) {
        let composes = Arc::new(AtomicU32::new(0));
        (Box::new(Self { tables: tables.to_vec(), composes: Arc::clone(&composes) }), composes)
    }
}

impl Layer for Reads {
    fn name(&self) -> &'static str {
        "reads"
    }

    fn inputs(&mut self, _view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        for &table in &self.tables {
            inputs.table(table, true)?;
            inputs.index(table, 0)?;
        }
        Ok(())
    }

    fn compose(&mut self, _mirrors: &Mirrors) -> io::Result<()> {
        self.composes.fetch_add(1, Ordering::Relaxed);
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

/// A layer reading the countries whose every settle spends `spends` of the test's clock, and that
/// says on `dropped` which thread let it go.
struct Probe {
    spends: Duration,
    dropped: Option<Sender<Option<String>>>,
}

impl Layer for Probe {
    fn name(&self) -> &'static str {
        "probe"
    }

    fn inputs(&mut self, _view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        inputs.table(TableId::Countries, true)?;
        inputs.index(TableId::Countries, 0)
    }

    fn compose(&mut self, _mirrors: &Mirrors) -> io::Result<()> {
        Ok(())
    }

    fn apply(&mut self, _step: &Step, _mirrors: &Mirrors) {}

    fn settle(&mut self, _mirrors: &Mirrors, _budget: &mut Budget) -> io::Result<()> {
        crate::layers::clock::advance(self.spends);
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

impl Drop for Probe {
    fn drop(&mut self) {
        if let Some(dropped) = &self.dropped {
            let _ = dropped.send(std::thread::current().name().map(str::to_owned));
        }
    }
}

/// Both maps at `hour`, in turn: a map that takes no steps would hold the ring for the other. The
/// second may not have its tables yet: they join at a step past the hour it opened at.
fn both(first: &mut MapSide, second: &mut MapSide, hour: i32, seen: &mut Vec<String>) {
    first.catch_up(hour).unwrap();
    note("first", first, hour, seen);
    for _ in 0..50_000_000 {
        second.frame(hour).unwrap();
        note("second", second, hour, seen);
        if second.applied() == Some(hour) && !second.composing() {
            return;
        }
        std::thread::yield_now();
    }
    panic!("the second map did not reach hour {hour} ({seen:?})");
}

/// What a map's last frame did that resets a picture: a reset composed, or an exact seek asked.
fn note(name: &str, side: &MapSide, hour: i32, seen: &mut Vec<String>) {
    let frame = side.last_frame();
    if frame.exact || frame.resets > 0 {
        seen.push(format!("{name} at {hour}: exact {}, resets {}, applied {:?}, newest {:?}", frame.exact, frame.resets, frame.applied, side.entry().playhead().newest()));
    }
}

/// Frames of `first`, and of `second` when it is open, until `done` holds after one, at a head the
/// test moves as the naval layer's tests do: the hour after the first map's last applied once the
/// playhead has built it (a step of one hour a frame), held there while either map composes.
/// `each` runs after every frame; every frame that reset a picture is noted in `seen`.
fn play(entry: &Entry, first: &mut MapSide, second: Option<&mut MapSide>, seen: &mut Vec<String>, done: impl FnMut(&MapSide, Option<&MapSide>) -> bool, each: impl FnMut()) {
    play_to(entry, first, second, seen, Hour::MAX, done, each);
}

/// [`play`] with the head never past `cap`.
fn play_to(entry: &Entry, first: &mut MapSide, mut second: Option<&mut MapSide>, seen: &mut Vec<String>, cap: Hour, mut done: impl FnMut(&MapSide, Option<&MapSide>) -> bool, mut each: impl FnMut()) {
    for _ in 0..50_000_000u64 {
        let composing = first.composing() || second.as_deref().is_some_and(MapSide::composing);
        let head = match first.applied() {
            None => 0,
            Some(hour) if composing => hour,
            Some(hour) => entry.playhead().newest().unwrap_or(hour).min(hour + 1),
        }
        .min(cap);
        let stepped = first.frame(head).unwrap().steps > 0;
        note("first", first, head, seen);
        if let Some(second) = second.as_deref_mut() {
            second.frame(head).unwrap();
            note("second", second, head, seen);
        }
        each();
        if done(first, second.as_deref()) {
            return;
        }
        if !stepped {
            std::thread::yield_now();
        }
    }
    let (report, _, counters) = entry.playhead().held();
    panic!("the maps did not get there with the head at most {cap}: first at {:?}, second at {:?}, newest {:?}, preparing {}, {counters:?}; {seen:?}", first.applied(), second.as_deref().map(MapSide::applied), entry.playhead().newest(), report.preparing);
}

/// The reader an opening delivers, polled without waiting until it is there.
fn arrived(opening: &Opening) -> Option<MapSide> {
    for _ in 0..50_000_000u64 {
        if let Some(made) = opening.poll() {
            return made;
        }
        std::thread::yield_now();
    }
    panic!("the opening delivered nothing");
}

#[test]
fn maps_of_one_recording_share_its_playhead_each_with_its_own_reader_and_one_closing_resets_no_other() {
    let entry = open(record("shared maps", 16), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    assert!(Arc::ptr_eq(&entry, &open(record("shared maps", 16), hourly_steps()).unwrap()), "a second map of the same timeline gets the same playhead");
    let (layer, first_composes) = Reads::new(&[TableId::Countries]);
    let mut first = MapSide::new(Arc::clone(&entry), vec![layer], View::default()).unwrap();
    assert!(!entry.anchored());
    first.anchor(1, false, 0.0, true);
    assert!(entry.anchored(), "a map joining now takes this clock instead of seeking");
    let mut seen = Vec::new();
    for hour in 1..4 {
        first.catch_up(hour).unwrap();
        note("first", &first, hour, &mut seen);
    }
    assert_eq!(first_composes.load(Ordering::Relaxed), 1, "{seen:?}");
    seen.clear();
    let (layer, second_composes) = Reads::new(&[TableId::Countries, TableId::CountryLinks]);
    let mut second = MapSide::new(Arc::clone(&entry), vec![layer], View::default()).unwrap();
    for hour in 4..14 {
        both(&mut first, &mut second, hour, &mut seen);
        // The second map's lanes are prepared on the fetchers and join at a step built after; each
        // hour gives them a bounded while before the head moves on.
        for _ in 0..2_000_000 {
            if second.draws("reads") {
                break;
            }
            std::thread::yield_now();
            second.frame(hour).unwrap();
            note("second", &second, hour, &mut seen);
        }
    }
    let (report, _, counters) = entry.playhead().held();
    assert_eq!(second_composes.load(Ordering::Relaxed), 1, "the second map composes once, when its tables join (joins {}, join holds {}, preparing {}, second at {:?} holding {:?}; {seen:?})", counters.joins, counters.join_holds, report.preparing, second.applied(), second.held().mirrors);
    assert_eq!(first_composes.load(Ordering::Relaxed), 1, "the first map is neither sought nor reset by the second opening (playhead {counters:?}; {seen:?})");
    assert_eq!(first.applied(), second.applied(), "both follow the one clock");
    drop(second);
    for hour in 14..16 {
        first.catch_up(hour).unwrap();
        note("first", &first, hour, &mut seen);
    }
    assert_eq!(first_composes.load(Ordering::Relaxed), 1, "nor by it closing ({seen:?})");
    drop(first);
    assert_eq!(Arc::strong_count(&entry), 1, "closed, the maps hold nothing of the playhead");
}

/// A reader is made on an opening's thread while the first map plays (held at a gate, so the
/// overlap is certain), arrives, joins and composes once within the steps its tables take to join,
/// and resets nothing of the first map. An opening let go before its reader is made has the reader
/// dropped on its own thread, and the first map is all that holds the playhead.
#[test]
fn a_map_opened_off_the_frame_thread_arrives_while_another_plays_and_resets_nothing() {
    let records = 24;
    let entry = open(record("opening", records), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let (layer, first_composes) = Reads::new(&[TableId::Countries]);
    let mut first = MapSide::new(Arc::clone(&entry), vec![layer], View::default()).unwrap();
    // Playing, a table declared later joins as while playing; nothing reads the clock's hour.
    entry.playhead().play(1.0);
    entry.playhead().seek(0);
    let mut seen = Vec::new();
    play(&entry, &mut first, None, &mut seen, |first, _| !first.composing() && first.applied() >= Some(2), || {});
    let (gate, wait) = mpsc::channel::<()>();
    let (layer, second_composes) = Reads::new(&[TableId::Countries, TableId::CountryLinks]);
    let (made_on, joining) = (Arc::new(Mutex::new(None)), Arc::clone(&entry));
    let named = Arc::clone(&made_on);
    let opening = Opening::spawn(move || {
        wait.recv().unwrap();
        *named.lock().unwrap() = std::thread::current().name().map(str::to_owned);
        MapSide::new(joining, vec![layer], View::default()).ok()
    })
    .unwrap();
    let held_from = first.applied().unwrap();
    play(&entry, &mut first, None, &mut seen, |first, _| !first.composing() && first.applied() >= Some(held_from + 4), || {
        assert!(opening.poll().is_none(), "the reader is still being made while the first map plays");
    });
    gate.send(()).unwrap();
    let mut second = arrived(&opening).expect("the reader is made");
    assert_eq!(made_on.lock().unwrap().as_deref(), Some(OPENING), "made off the caller's thread");
    // Declared at the hour the first map stands at: CountryLinks, new to the lanes, joins at the
    // frames' hour; Countries, held for the first map, lands from memory at the newest step's end,
    // at most `ring + 1` steps of one hour on. The map composes once, with both home.
    let declared = first.applied().unwrap();
    let by = declared + hourly_steps().ring as Hour + 1;
    let last = by + 2;
    assert!(last < records as Hour, "the recording holds the steps the bound allows");
    // The head waits at `by` for the second map: nothing here races the fetchers against the frames.
    play_to(&entry, &mut first, Some(&mut second), &mut seen, by, |first, second| {
        let second = second.expect("open");
        second.draws("reads") && !second.composing() && second.applied() == first.applied()
    }, || {});
    play(&entry, &mut first, Some(&mut second), &mut seen, |first, _| !first.composing() && first.applied() >= Some(last), || {});
    assert_eq!(second_composes.load(Ordering::Relaxed), 1, "it composes once, when its tables join ({seen:?})");
    assert_eq!(first_composes.load(Ordering::Relaxed), 1, "the first map is never reset by it ({seen:?})");
    assert_eq!(first.applied(), second.applied(), "both follow the one clock");
    drop(second);
    let (gate, wait) = mpsc::channel::<()>();
    let (dropped, dropped_on) = mpsc::channel();
    let joining = Arc::clone(&entry);
    let abandoned = Opening::spawn(move || {
        wait.recv().unwrap();
        MapSide::new(joining, vec![Box::new(Probe { spends: Duration::ZERO, dropped: Some(dropped) })], View::default()).ok()
    })
    .unwrap();
    drop(abandoned);
    gate.send(()).unwrap();
    // A deadline that fails by name, not a wait: the message comes as the reader is dropped.
    let thread = dropped_on.recv_timeout(Duration::from_secs(60)).expect("the reader no longer wanted is dropped");
    assert_eq!(thread.as_deref(), Some(OPENING), "a reader no longer wanted is dropped by its opening's thread");
    assert_eq!(Arc::strong_count(&entry), 2, "only the first map holds the playhead");
    play(&entry, &mut first, None, &mut seen, |first, _| !first.composing() && first.applied() >= Some(last + 1), || {});
    assert_eq!(first_composes.load(Ordering::Relaxed), 1, "nor by the reader let go ({seen:?})");
}

/// A reader made on an opening's thread takes its share of the budget of the thread that draws
/// it: alone there, every frame's slice is the whole budget, though each settle spends a
/// millisecond of the test's clock. Had it joined the opening thread's budget, the drawing thread's
/// frame would never begin again, and each slice would be what the settles before it left.
#[test]
fn a_reader_made_on_another_thread_takes_its_share_of_the_budget_of_the_thread_that_draws_it() {
    crate::layers::clock::hold();
    let entry = open(record("opening budget", 12), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let joining = Arc::clone(&entry);
    let opening = Opening::spawn(move || MapSide::new(joining, vec![Box::new(Probe { spends: Duration::from_millis(1), dropped: None })], View::default()).ok()).unwrap();
    let mut side = arrived(&opening).expect("the reader is made");
    entry.playhead().play(1.0);
    entry.playhead().seek(0);
    let mut seen = Vec::new();
    let mut slices = Vec::new();
    let mut drawn = false;
    play(&entry, &mut side, None, &mut seen, |side, _| {
        let frame = side.last_frame();
        if drawn && !frame.composing {
            slices.push(frame.slice_ns);
        }
        drawn |= side.draws("probe");
        !side.composing() && side.applied() >= Some(8)
    }, || {});
    assert!(slices.len() >= 4, "too few frames settled: {slices:?}");
    let whole = BUDGET.as_nanos() as u64;
    assert!(slices.iter().all(|&ns| ns + 1_000 >= whole), "every slice is the whole {BUDGET:?}: {slices:?}");
}

#[cfg(test)]
use std::sync::{Mutex, mpsc};
