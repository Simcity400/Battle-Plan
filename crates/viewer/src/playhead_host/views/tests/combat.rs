//! The combat readers' sources: rows at the head of the open map, and nothing else; while those
//! rows do not hold a table a reader reads, it is loading by name; a table learned on the window
//! thread and from a worker's build; every reader closed by a frame that did not ask it. Every
//! reader that must be given rows has its tables before the playhead's seek; a table learned joins
//! at the held head, waited for by name within one deadline.

use super::{COUNTRIES, RECORDING, hourly_steps, record};
use crate::{
    app::combat_rows::CombatRows,
    playhead_host::{
        open,
        views::{AtHead, Need, Pace, Views, apart::Apart},
    },
};
use replay_data::rows::{CountryRow, TableId, mask, tables};
use replay_data::scope::CaptureMode;
use replay_frame::frame::{DecodedFrame, discovering};
use std::{sync::Arc, time::Duration};

const SHOWN: [&str; 3] = ["GER", "ENG", "ENG"];

/// Generous for a debug build on a machine at full load.
const DEADLINE: Duration = Duration::from_secs(120);

fn tags(frame: &DecodedFrame) -> Option<Vec<String>> {
    frame.complete::<tables::Countries>().ok().map(|rows| rows.iter().map(|row| String::from_utf8_lossy(&row.tag).into_owned()).collect())
}

fn shown() -> Option<Vec<String>> {
    Some(SHOWN.map(str::to_owned).to_vec())
}

/// A playhead over eight recorded hours with a view store of its own, as the map side's is,
/// standing at hour 1.
fn playhead(name: &str) -> (Apart, Views) {
    let entry = open(record(name, 8), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    entry.playhead().seek(1);
    let fed = Apart::new(&entry, RECORDING.branch).unwrap();
    let views = fed.store().handle();
    (fed, views)
}

/// The store's views at `hour`.
fn step(fed: &mut Apart, hour: replay_data::Hour) {
    assert!(fed.drive(hour), "the store never reached hour {hour}");
}

/// With no frame of today's at all, the window reads rows at the head's hour; while its reader
/// opens it has none and is loading by name; a frame that did not ask it closes it.
#[test]
fn the_combat_window_reads_rows_at_the_head_loading_by_name_and_closes_unasked() {
    let (mut fed, views) = playhead("combat window");
    let mut combat = CombatRows::default();
    *combat.window() = AtHead::new("combat window", &[COUNTRIES], Pace::Step);
    let head = &views;
    assert!(combat.window_rows(head).is_none(), "nothing while its rows load");
    assert_eq!(combat.window_loading(), Some("rows at the head"), "loading its rows, by name");
    assert!(combat.loading().contains(&("the combat window: loading: rows at the head".to_owned(), 1)), "named in the window's refusals: {:?}", combat.loading());
    for hour in 1..4 {
        step(&mut fed, hour);
        let read = combat.window_rows(head).map(|frame| (frame.hour, tags(&frame)));
        assert_eq!(read, Some((hour, shown())), "rows at the head's hour {hour}");
        assert_eq!(combat.window_loading(), None, "loaded at hour {hour}");
        combat.end_frame();
    }
    assert_eq!(combat.open(), (1, 0), "asked in the last frame: open");
    combat.end_frame();
    assert_eq!(combat.open(), (0, 0), "a frame that did not ask closes it");
    assert!(combat.loading().is_empty(), "closed, nothing is loading: {:?}", combat.loading());
}

/// A read over a table the window has not learned is thrown away: the table is learned, and until
/// it arrives the window has no rows and is loading that table by name, never another frame.
#[test]
fn a_read_over_a_table_not_learned_is_thrown_away_and_the_window_loads_it_by_name_until_it_arrives() {
    let (mut fed, views) = playhead("combat learning");
    let mut combat = CombatRows::default();
    *combat.window() = AtHead::new("combat window", &[], Pace::Step);
    let head = &views;
    assert!(combat.window_rows(head).is_none(), "nothing while its rows load");
    step(&mut fed, 1);
    let first = combat.window_rows(head).expect("rows at the head, made by the step");
    assert!(first.tables[COUNTRIES.index()].is_not_declared(), "Countries is not declared yet");
    let (read, missed) = discovering(|| tags(&first));
    assert_eq!((read, missed.clone()), (None, vec![TableId::Countries]), "the read over the undeclared table names it");
    combat.window_missed(&missed);
    assert_eq!(combat.window_loading(), Some("Countries"), "loading the table it learned, by name");
    assert!(combat.window_rows(head).is_none_or(|frame| !frame.tables[COUNTRIES.index()].is_not_declared()), "Countries is declared now: no rows until it holds it, and no other frame");
    // The side's mirrors hold every table the recording has, so a table learned is at the head:
    // the step that follows makes its rows, with no join to wait for.
    step(&mut fed, 2);
    combat.window().wait(|frame| !frame.tables[COUNTRIES.index()].is_not_declared(), DEADLINE).unwrap_or_else(|error| panic!("Countries arrives at the head: {error}"));
    let read = combat.window_rows(head).map(|frame| tags(&frame));
    assert_eq!(read, Some(shown()), "rows at the head holding Countries");
    assert_eq!(combat.window_loading(), None, "loaded");
}

/// A reader of one Countries column whose build on a worker reads every field: the table is named
/// as missed, learned whole (widening the column it held, not passed over as declared), and its
/// rows then hold every column. Between the learn and the arrival it has no rows, never rows
/// without the columns and never another frame.
#[test]
fn a_reader_built_on_a_worker_builds_from_rows_and_declares_what_its_build_missed() {
    let (mut fed, views) = playhead("combat worker");
    let mut learning = AtHead::with("combat worker", &[Need::columns(COUNTRIES, mask::<CountryRow>(&["stability_raw"]))], Pace::Step);
    assert!(learning.frame(&views).is_none(), "nothing while its rows load");
    step(&mut fed, 1);
    let build = |frame: Arc<DecodedFrame>| std::thread::spawn(move || discovering(|| tags(&frame))).join().unwrap();
    let first = learning.frame(&views).expect("rows at the head, made by the step");
    let (built, missed) = build(first);
    assert_eq!((built, missed.clone()), (None, vec![TableId::Countries]), "the worker's build of every field over rows of one column names the table");
    learning.learn(&missed);
    let whole = |frame: &DecodedFrame| {
        let view = &frame.tables[COUNTRIES.index()];
        !view.is_not_declared() && view.fields().is_none()
    };
    assert!(learning.frame(&views).is_none_or(|shown| whole(&shown)), "learned: no rows until the table arrives with every column, never rows without them");
    step(&mut fed, 2);
    let arrived = learning.wait(whole, DEADLINE).unwrap_or_else(|error| panic!("learned whole: Countries arrives with every column: {error}"));
    assert_eq!(build(arrived), (shown(), Vec::new()), "built on the worker from rows at the head, every column held");
}
