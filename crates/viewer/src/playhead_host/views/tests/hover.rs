
use super::{COUNTRIES, RECORDING, hourly_steps, open, record};
use crate::{
    app::hover_rows::{Family, HoverRows, Job, Key, Prepare, Retry, What},
    playhead_host::views::{Views, apart::Apart},
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

const DEADLINE: Duration = Duration::from_secs(120);

fn left(until: Instant) -> Duration {
    until.saturating_duration_since(Instant::now())
}

fn tags(frame: &DecodedFrame) -> String {
    frame.complete::<tables::Countries>().map_or_else(|_| "today".to_owned(), |rows| rows.iter().map(|row| String::from_utf8_lossy(&row.tag).into_owned()).collect::<Vec<_>>().join(","))
}

static WORKERS: Mutex<()> = Mutex::new(());

fn workers() -> std::sync::MutexGuard<'static, ()> {
    WORKERS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn job(build: impl FnMut(&Arc<DecodedFrame>, &Retry) -> String + Send + 'static) -> Job {
    let mut build = build;
    Job::new(Box::new(move |frame, retry| Arc::new(Tooltip { title: build(frame, retry), body: std::thread::current().name().unwrap_or_default().to_owned(), ..Tooltip::default() })), Arc::new(|| {}))
}

fn key(what: What) -> Key {
    Key::new(what, [0; 4], (false, false))
}

fn declares(frame: &DecodedFrame, tables: &[TableId]) -> bool {
    tables.iter().all(|table| !frame.tables[table.index()].is_not_declared())
}

fn holds_rows(hovers: &HoverRows, family: Family) -> bool {
    hovers.rows_wait(family, |_| true, Duration::ZERO).is_ok()
}

fn text(hovers: &mut HoverRows, family: Family, views: &Views, what: &What, until: Instant) -> Arc<Tooltip> {
    hovers.wait_within(family, views, key(what.clone()), |_| job(|frame, _| tags(frame)), left(until)).unwrap_or_else(|error| panic!("{family:?}: {error}"))
}

fn ask(hovers: &mut HoverRows, family: Family, views: &Views, what: &What) -> Option<Arc<Tooltip>> {
    hovers.ask(family, views, key(what.clone()), |_| job(|frame, _| tags(frame)))
}

fn held_at_one(name: &str) -> (Apart, Views) {
    let entry = open(record(name, 8), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    entry.playhead().seek(1);
    let mut fed = Apart::new(&entry, RECORDING.branch).unwrap();
    assert!(fed.drive(1), "the store never reached the held hour 1");
    let views = fed.store().handle();
    (fed, views)
}

fn step(fed: &mut Apart) {
    assert!(fed.drive(1), "the store never reached the held hour 1");
}

fn keeps(views: &Views, table: TableId) -> bool {
    views.held().iter().any(|(held, _)| *held == table)
}

const BY_KEY: &str = "GER,ENG,ENG";

#[test]
fn each_hover_family_makes_its_text_on_its_worker_from_rows_at_the_head_and_closes_with_its_hover() {
    let _workers = workers();
    let until = Instant::now() + DEADLINE;
    let (mut fed, views) = held_at_one("hover workers");
    let mut hovers = HoverRows::default();
    for (family, what) in [(Family::Counters, What::AirBase { province: 3, kind: 0 }), (Family::Naval, What::Mines(7)), (Family::Occupation, What::Occupation { state: 2, token: "a".into() }), (Family::Capture, What::Capture)] {
        let named = format!("the {} hover: loading: Countries", family.name());
        let opening = ask(&mut hovers, family, &views, &what);
        assert!(opening.is_none(), "{family:?}: nothing while its reader opens");
        assert_eq!(hovers.loading_of(family), Some("rows at the head"), "{family:?}: loading its rows, by name");
        assert!(!holds_rows(&hovers, family), "{family:?}: a reader opened between steps is given nothing until the next one");
        step(&mut fed);
        let first = hovers.rows_wait(family, |_| true, left(until)).unwrap_or_else(|error| panic!("{family:?}: its reader's first frame is made by the step: {error}"));
        assert!(!declares(&first, &[COUNTRIES]), "{family:?}: its first rows hold no table");
        assert!(ask(&mut hovers, family, &views, &what).is_none(), "{family:?}: nothing before a text is made");
        let learning = hovers.next(family, left(until)).unwrap_or_else(|error| panic!("{family:?}: {error}"));
        assert!(learning.is_none(), "{family:?}: rows without the table its leaf read make no text");
        assert_eq!(hovers.loading_of(family), Some("Countries"), "{family:?}: loading the table it learned, by name");
        assert!(hovers.loading().contains(&(named.clone(), 1)), "{family:?}: named in the window's refusals: {:?}", hovers.loading());
        step(&mut fed);
        hovers.rows_wait(family, |frame| declares(frame, &[COUNTRIES]), left(until)).unwrap_or_else(|error| panic!("{family:?}: the table it learned is in its rows at the next step: {error}"));
        let joined = text(&mut hovers, family, &views, &what, until);
        assert_eq!(joined.title, BY_KEY, "{family:?}: rows at the head once the table it learned joins");
        assert_eq!(joined.body, format!("{} hover", family.name()), "{family:?}: made on its worker");
        assert_eq!(hovers.shown(family).map(|made| made.hour), Some(1), "{family:?}: made over rows at the head's hour");
        assert_eq!(hovers.loading_of(family), None, "{family:?}: loaded");
        assert!(!hovers.loading().contains(&(named, 1)), "{family:?}: no longer in the window's refusals: {:?}", hovers.loading());
        hovers.end_frame(true);
        hovers.end_frame(true);
        assert_eq!(hovers.is_open(family), family == Family::Capture, "{family:?}: only the capture's stays open while capturing");
        hovers.end_frame(false);
        hovers.end_frame(false);
        assert!(!hovers.is_open(family), "{family:?}: closed, it holds nothing");
        assert_eq!(hovers.loading_of(family), None, "{family:?}: closed, it is loading nothing");
        assert!(HoverRows::join(family) >= 1, "{family:?}: its worker thread ends");
        step(&mut fed);
        assert!(!keeps(&views, COUNTRIES), "{family:?}: the view of Countries is let go once it closes");
    }
    let _ = ask(&mut hovers, Family::Naval, &views, &What::Accidents(9));
    step(&mut fed);
    let answered = Arc::new(Mutex::new(None::<Retry>));
    let first = hovers.wait_within(
        Family::Naval,
        &views,
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

struct Gate {
    state: Mutex<(usize, bool)>,
    changed: Condvar,
    prepared: Mutex<Vec<usize>>,
    until: Instant,
}

impl Gate {
    fn new(until: Instant) -> Self {
        Self { state: Mutex::default(), changed: Condvar::new(), prepared: Mutex::default(), until }
    }

    fn count(&self) -> usize {
        self.state.lock().unwrap().0
    }

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

#[test]
fn the_province_hover_is_made_only_from_rows_its_worker_prepared() {
    let _workers = workers();
    let until = Instant::now() + DEADLINE;
    let (mut fed, views) = held_at_one("province worker");
    let gate = Arc::new(Gate::new(until));
    let mut hovers = HoverRows::default();
    let head = &views;
    let what = What::Province { province: 5, mode: viewer_counter_products::map::province_tooltips::ModeHover::Nothing, routes: Vec::new() };
    let leaf = |gate: &Arc<Gate>| {
        job(|frame, _| {
            let _ = frame.complete::<tables::WeatherProvinces>();
            format!("{} {}", tags(frame), Arc::as_ptr(frame) as usize)
        })
        .preparing(Arc::clone(gate) as Arc<dyn Prepare>)
    };
    let opening = hovers.ask(Family::Province, head, key(what.clone()), |_| leaf(&gate));
    assert!(opening.is_none(), "nothing while its reader opens");
    assert_eq!(hovers.loading_of(Family::Province), Some("rows at the head"));
    step(&mut fed);
    let first = hovers.rows_wait(Family::Province, |_| true, left(until)).unwrap_or_else(|error| panic!("its reader's first frame is made by the step: {error}"));
    assert!(!declares(&first, &[TableId::WeatherProvinces]) && !declares(&first, &[COUNTRIES]), "its first rows hold no table");
    assert!(hovers.ask(Family::Province, head, key(what.clone()), |_| leaf(&gate)).is_none(), "nothing before a text is made");
    let learning = hovers.next(Family::Province, left(until)).unwrap_or_else(|error| panic!("rows of no table make no text and teach the tables: {error}"));
    assert!(learning.is_none(), "rows without the tables its leaf read make no text");
    assert!(hovers.loading_of(Family::Province).is_some_and(|loading| loading.contains("WeatherProvinces") && loading.contains("Countries")), "loading both tables it learned, by name: {:?}", hovers.loading_of(Family::Province));
    assert_eq!(gate.count(), 0, "rows without WeatherProvinces are never prepared");
    step(&mut fed);
    hovers.rows_wait(Family::Province, |frame| declares(frame, &[TableId::WeatherProvinces, COUNTRIES]), left(until)).unwrap_or_else(|error| panic!("the province hover's tables are in its rows at the next step: {error}"));
    let posted = hovers.ask(Family::Province, head, key(what.clone()), |_| leaf(&gate)).map(|text| text.title.clone());
    assert!(gate.asked(), "its rows go to its worker, which asks the gate to prepare them, within the test's deadline");
    let preparing = hovers.ask(Family::Province, head, key(what), |_| leaf(&gate)).map(|text| text.title.clone());
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

#[test]
fn a_live_hover_reads_no_rows_past_the_hour_shown() {
    let _workers = workers();
    let until = Instant::now() + DEADLINE;
    let (mut fed, views) = held_at_one("hover through the hour shown");
    let mut hovers = HoverRows::default();
    let (family, what) = (Family::Naval, What::Mines(7));
    let hour = |hovers: &mut HoverRows| -> replay_data::Hour {
        let text = hovers.wait_within(family, &views, key(what.clone()), |_| job(|frame, _| frame.hour.to_string()), left(until)).unwrap_or_else(|error| panic!("{error}"));
        text.title.parse().unwrap()
    };
    assert!(ask(&mut hovers, family, &views, &what).is_none(), "nothing while its reader opens");
    step(&mut fed);
    assert_eq!(hour(&mut hovers), 1, "made over the rows of the held hour");
    for at in [2, 3] {
        assert!(fed.drive(at), "the store never reached hour {at}");
        hovers.rows_wait(family, |frame| frame.hour == at, left(until)).unwrap_or_else(|error| panic!("its reader is given the rows of hour {at}: {error}"));
    }
    views.show_through(Some(2));
    assert_eq!(hour(&mut hovers), 2, "the text is made from the rows of the hour shown, not the newest");
    assert_eq!(hovers.shown(family).map(|made| made.hour), Some(2), "the text shown stands at the hour shown");
    views.show_through(None);
    assert_eq!(hour(&mut hovers), 3, "with no hour shown set, the newest rows");
    hovers.end_frame(false);
    hovers.end_frame(false);
    assert!(HoverRows::join(family) >= 1, "its worker thread ends");
}
