//! The hover families' readers at a held head, with the hover's reader the rows worker's only
//! one: every frame a reader is given is published because it opened or a table it declared
//! arrived, never on a cadence measured in wall time. No frame of today's is given to an ask over
//! rows at the head. Where two threads race (the reader's first frame, a table's join, a prepare)
//! the test holds one of them, observes, then releases it. Every wait is for a named state, within
//! one [`DEADLINE`] for the whole test.

use super::{COUNTRIES, COMBATS, RECORDING, hourly_steps, nothing, open, record};
use crate::{
    app::hover_rows::{Family, HoverRows, Job, Key, Prepare, Retry, What},
    playhead_host::{
        Entry,
        rows::{Cadence, Rows},
    },
};
use replay_data::{
    rows::{TableId, tables},
    scope::CaptureMode,
};
use replay_frame::frame::DecodedFrame;
use std::{
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
use viewer_foundation::tooltip_text::Tooltip;

/// Generous for a debug build on a machine at full load.
const DEADLINE: Duration = Duration::from_secs(120);

fn left(until: Instant) -> Duration {
    until.saturating_duration_since(Instant::now())
}

/// The text a test's leaf makes: the frame's Countries tags, or "today" for a frame without them.
fn tags(frame: &DecodedFrame) -> String {
    frame.complete::<tables::Countries>().map_or_else(|_| "today".to_owned(), |rows| rows.iter().map(|row| String::from_utf8_lossy(&row.tag).into_owned()).collect::<Vec<_>>().join(","))
}

/// A job whose text is `build`'s, with the thread it was made on as its body.
fn job(build: impl FnMut(&Arc<DecodedFrame>, &Retry) -> String + Send + 'static) -> Job {
    let mut build = build;
    Job::new(Box::new(move |frame, retry| Arc::new(Tooltip { title: build(frame, retry), body: std::thread::current().name().unwrap_or_default().to_owned(), ..Tooltip::default() })), Arc::new(|| {}))
}

fn key(what: What) -> Key {
    Key::new(what, [0; 4], (false, false))
}

/// Whether `frame` declares every one of `tables`.
fn declares(frame: &DecodedFrame, tables: &[TableId]) -> bool {
    tables.iter().all(|table| !frame.tables[table.index()].is_not_declared())
}

/// Whether `family`'s reader holds a frame now.
fn holds_rows(hovers: &HoverRows, family: Family) -> bool {
    hovers.rows_wait(family, |_| true, Duration::ZERO).is_ok()
}

/// The text `family`'s worker makes of `what` over rows at the head of `entry`, its rows and the
/// tables its leaf reads waited for.
fn text(hovers: &mut HoverRows, family: Family, entry: &Arc<Entry>, what: &What, until: Instant) -> Arc<Tooltip> {
    hovers.wait_within(family, (entry, RECORDING), key(what.clone()), |_| job(|frame, _| tags(frame)), left(until)).unwrap_or_else(|error| panic!("{family:?}: {error}"))
}

/// Asks `family`'s text of `what` over rows at the head of `entry`, as the window does a frame.
fn ask(hovers: &mut HoverRows, family: Family, entry: &Arc<Entry>, what: &What) -> Option<Arc<Tooltip>> {
    hovers.ask(family, (entry, RECORDING), key(what.clone()), |_| job(|frame, _| tags(frame)))
}

/// Rows at a head held at hour 1 whose only reader, once this returns, is the hover's: so a pause
/// of the rows worker holds the hover reader's frame and nothing else.
fn held_at_one(name: &str, until: Instant) -> (Arc<Entry>, Arc<Rows>) {
    let entry = open(record(name, 8), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    let watch = rows.reader(&[COMBATS], Cadence::EveryStep);
    rows.settled();
    rows.drive(1);
    entry.playhead().seek(1);
    watch.wait(|frame| frame.hour == 1, left(until)).unwrap_or_else(|error| panic!("the rows worker reaches the held hour 1: {error}"));
    drop(watch);
    (entry, rows)
}

const BY_KEY: &str = "GER,ENG,ENG";

/// With no frame of today's at all, each family's text is made on its own worker thread from rows
/// at the head. While its reader opens, and while the table its leaf read undeclared is learned and
/// joins, it shows nothing (no text from any other source) and is loading by name, in the window's
/// refusals; once the table joins, the text is the rows'. With no playhead map, today's frame gives
/// today's text. A leaf whose answer arrives later has its text made again without a new ask.
/// Closed (the capture only once the capture ends), the family holds no text, its worker thread
/// ends and its table is let go.
#[test]
fn each_hover_family_makes_its_text_on_its_worker_from_rows_at_the_head_and_closes_with_its_hover() {
    let until = Instant::now() + DEADLINE;
    let (entry, rows) = held_at_one("hover workers", until);
    let today = nothing(1);
    let mut hovers = HoverRows::default();
    for (family, what) in [(Family::Counters, What::AirBase { province: 3, kind: 0 }), (Family::Naval, What::Mines(7)), (Family::Occupation, What::Occupation { state: 2, token: "a".into() }), (Family::Capture, What::Capture)] {
        let named = format!("the {} hover: loading: Countries", family.name());
        let pause = rows.pause_before_publish();
        let opening = ask(&mut hovers, family, &entry, &what);
        assert!(opening.is_none(), "{family:?}: nothing while its reader opens");
        assert_eq!(hovers.loading_of(family), Some("rows at the head"), "{family:?}: loading its rows, by name");
        assert!(pause.holding(until), "{family:?}: the rows worker makes its new reader's first frame and holds it unpublished: {:?}", rows.error());
        pause.release();
        let first = hovers.rows_wait(family, |_| true, left(until)).unwrap_or_else(|error| panic!("{family:?}: its reader's first frame is published once released: {error}"));
        assert!(!declares(&first, &[COUNTRIES]), "{family:?}: its first rows hold no table");
        let pause = rows.pause_before_publish();
        assert!(ask(&mut hovers, family, &entry, &what).is_none(), "{family:?}: nothing before a text is made");
        let learning = hovers.next(family, left(until)).unwrap_or_else(|error| panic!("{family:?}: {error}"));
        assert!(learning.is_none(), "{family:?}: rows without the table its leaf read make no text");
        assert_eq!(hovers.loading_of(family), Some("Countries"), "{family:?}: loading the table it learned, by name");
        assert!(pause.holding(until), "{family:?}: the table it learned joins and the rows worker holds its reader's frame of it unpublished: {:?}", rows.error());
        let joining = ask(&mut hovers, family, &entry, &what);
        assert!(joining.is_none(), "{family:?}: no text from any other source while the table it learned has not joined (its reader holds rows: {})", holds_rows(&hovers, family));
        assert_eq!(hovers.loading_of(family), Some("Countries"), "{family:?}: still loading it");
        assert!(hovers.loading().contains(&(named.clone(), 1)), "{family:?}: named in the window's refusals: {:?}", hovers.loading());
        pause.release();
        hovers.rows_wait(family, |frame| declares(frame, &[COUNTRIES]), left(until)).unwrap_or_else(|error| panic!("{family:?}: the table it learned joins its reader at the head once released: {error}"));
        let joined = text(&mut hovers, family, &entry, &what, until);
        assert_eq!(joined.title, BY_KEY, "{family:?}: rows at the head once the table it learned joins");
        assert_eq!(joined.body, format!("{} hover", family.name()), "{family:?}: made on its worker");
        assert_eq!(hovers.shown(family).map(|made| made.hour), Some(1), "{family:?}: made over rows at the head's hour");
        assert_eq!(hovers.loading_of(family), None, "{family:?}: loaded");
        assert!(!hovers.loading().contains(&(named, 1)), "{family:?}: no longer in the window's refusals: {:?}", hovers.loading());
        let off = hovers.wait_today_within(family, &today, key(what.clone()), |_| job(|frame, _| tags(frame)), left(until)).unwrap_or_else(|error| panic!("{family:?}: {error}"));
        assert_eq!(off.title, "today", "{family:?}: no playhead map, today's frame");
        hovers.end_frame(true);
        hovers.end_frame(true);
        assert_eq!(hovers.is_open(family), family == Family::Capture, "{family:?}: only the capture's stays open while capturing");
        hovers.end_frame(false);
        hovers.end_frame(false);
        assert!(!hovers.is_open(family), "{family:?}: closed, it holds nothing");
        assert_eq!(hovers.loading_of(family), None, "{family:?}: closed, it is loading nothing");
        assert!(HoverRows::join(family) >= 1, "{family:?}: its worker thread ends");
        assert!(rows.released(COUNTRIES, until), "{family:?}: Countries is let go once it closes: {:?}", rows.error());
    }
    let answered = Arc::new(Mutex::new(None::<Retry>));
    let first = hovers.wait_within(
        Family::Naval,
        (&entry, RECORDING),
        key(What::Accidents(9)),
        |_| {
            let held = Arc::clone(&answered);
            job(move |_, retry| {
                let mut held = held.lock().unwrap();
                if held.is_some() {
                    return "answered".into();
                }
                *held = Some(Arc::clone(retry));
                "pending".into()
            })
        },
        left(until),
    );
    assert_eq!(first.unwrap().title, "pending");
    let retry = answered.lock().unwrap().clone().expect("the leaf kept its retry");
    retry();
    let again = hovers.next(Family::Naval, left(until)).unwrap().expect("a text");
    assert_eq!(again.title, "answered", "the answer makes the text again without a new ask");
    hovers.end_frame(false);
    hovers.end_frame(false);
    assert!(HoverRows::join(Family::Naval) >= 1);
}

/// Prepares a frame only once the test opens it, and names what it prepared.
struct Gate {
    /// Frames asked to be prepared, and whether it is open.
    state: Mutex<(usize, bool)>,
    changed: Condvar,
    prepared: Mutex<Vec<usize>>,
    /// The test's deadline, past which the worker stops waiting so it can never hang.
    until: Instant,
}

impl Gate {
    fn new(until: Instant) -> Self {
        Self { state: Mutex::default(), changed: Condvar::new(), prepared: Mutex::default(), until }
    }

    fn count(&self) -> usize {
        self.state.lock().unwrap().0
    }

    /// Waits until a frame is asked to be prepared; false at the deadline.
    fn asked(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        while state.0 == 0 {
            let left = left(self.until);
            if left.is_zero() {
                return false;
            }
            state = self.changed.wait_timeout(state, left).unwrap().0;
        }
        true
    }

    fn open(&self) {
        self.state.lock().unwrap().1 = true;
        self.changed.notify_all();
    }
}

impl Prepare for Gate {
    fn prepare(&self, frame: &Arc<DecodedFrame>) {
        let mut state = self.state.lock().unwrap();
        state.0 += 1;
        self.changed.notify_all();
        while !state.1 {
            let left = left(self.until);
            if left.is_zero() {
                drop(state);
                panic!("the province hover's worker waited at the gate for the test to open it, until the test's deadline");
            }
            state = self.changed.wait_timeout(state, left).unwrap().0;
        }
        drop(state);
        self.prepared.lock().unwrap().push(Arc::as_ptr(frame) as usize);
    }
}

/// The province hover's rows hold WeatherProvinces, whose temperatures its worker prepares before
/// it makes the text: nothing at all is shown while its rows lack the tables or while the prepare
/// runs, then only text made from rows that were prepared; closed, its worker ends.
#[test]
fn the_province_hover_is_made_only_from_rows_its_worker_prepared() {
    let until = Instant::now() + DEADLINE;
    let (entry, rows) = held_at_one("province worker", until);
    let gate = Arc::new(Gate::new(until));
    let mut hovers = HoverRows::default();
    let head = (&entry, RECORDING);
    let what = What::Province { province: 5, mode: viewer_counter_products::map::province_tooltips::ModeHover::Nothing };
    let leaf = |gate: &Arc<Gate>| {
        job(|frame, _| {
            let _ = frame.complete::<tables::WeatherProvinces>();
            format!("{} {}", tags(frame), Arc::as_ptr(frame) as usize)
        })
        .preparing(Arc::clone(gate) as Arc<dyn Prepare>)
    };
    let pause = rows.pause_before_publish();
    let opening = hovers.ask(Family::Province, head, key(what.clone()), |_| leaf(&gate));
    assert!(opening.is_none(), "nothing while its reader opens");
    assert_eq!(hovers.loading_of(Family::Province), Some("rows at the head"));
    assert!(pause.holding(until), "the rows worker makes the province reader's first frame and holds it unpublished: {:?}", rows.error());
    pause.release();
    let first = hovers.rows_wait(Family::Province, |_| true, left(until)).unwrap_or_else(|error| panic!("its reader's first frame is published once released: {error}"));
    assert!(!declares(&first, &[TableId::WeatherProvinces]) && !declares(&first, &[COUNTRIES]), "its first rows hold no table");
    assert!(hovers.ask(Family::Province, head, key(what.clone()), |_| leaf(&gate)).is_none(), "nothing before a text is made");
    let learning = hovers.next(Family::Province, left(until)).unwrap_or_else(|error| panic!("rows of no table make no text and teach the tables: {error}"));
    assert!(learning.is_none(), "rows without the tables its leaf read make no text");
    assert!(hovers.loading_of(Family::Province).is_some_and(|loading| loading.contains("WeatherProvinces") && loading.contains("Countries")), "loading both tables it learned, by name: {:?}", hovers.loading_of(Family::Province));
    assert_eq!(gate.count(), 0, "rows without WeatherProvinces are never prepared");
    hovers.rows_wait(Family::Province, |frame| declares(frame, &[TableId::WeatherProvinces, COUNTRIES]), left(until)).unwrap_or_else(|error| panic!("the province hover's tables join its reader at the head: {error}"));
    let posted = hovers.ask(Family::Province, head, key(what.clone()), |_| leaf(&gate)).map(|text| text.title.clone());
    assert!(gate.asked(), "its rows go to its worker, which asks the gate to prepare them, within the test's deadline");
    let preparing = hovers.ask(Family::Province, head, key(what.clone()), |_| leaf(&gate)).map(|text| text.title.clone());
    for shown in [posted, preparing] {
        assert!(shown.is_none(), "nothing from rows before they are prepared, and nothing from any other source: {shown:?}");
    }
    gate.open();
    let shown = hovers.next(Family::Province, left(until)).unwrap_or_else(|error| panic!("the gate opened, the worker makes the text over the rows it prepared: {error}")).expect("a text over the rows it prepared");
    assert!(shown.title.starts_with(BY_KEY), "rows at the head once prepared: {}", shown.title);
    let frame: usize = shown.title.rsplit(' ').next().unwrap().parse().unwrap();
    assert!(gate.prepared.lock().unwrap().contains(&frame), "the text shown was made from rows the worker prepared");
    hovers.end_frame(false);
    hovers.end_frame(false);
    assert!(!hovers.is_open(Family::Province));
    assert!(HoverRows::join(Family::Province) >= 1, "its worker thread ends");
}
