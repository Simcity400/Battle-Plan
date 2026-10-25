use super::direct_tests::{LAYOUTS, alias_fixture};
use super::*;
use crate::container::{
    SeekResult,
    aliases::private::{Followed, hooks},
    check::{self, Step},
    decoded_sections::Key,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc,
};

const SELECTED: &[TableId] = &[
    TableId::Divisions,
    TableId::DivisionMovement,
    TableId::LandCombatDivisionModifiers,
];

fn cancelled(flag: &AtomicBool) -> io::Result<()> {
    if flag.load(Ordering::Acquire) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Dashboard request cancelled",
        ))
    } else {
        Ok(())
    }
}

fn bytes(frames: &[Frame]) -> Vec<u8> {
    let mut out = Vec::new();
    for frame in frames {
        frame.encode(&mut out).unwrap();
    }
    out
}

fn reference(expected: &[Frame], index: &BlockIndex, through: Hour) -> Vec<Frame> {
    expected
        .iter()
        .filter(|frame| {
            frame.branch == index.branch && index.first_hour <= frame.hour && frame.hour <= through
        })
        .cloned()
        .collect()
}

fn assert_seek(result: SeekResult, expected: &[Frame], selected: &[TableId]) {
    let SeekResult::Frame(actual) = result else {
        panic!("the synthetic recording must produce a frame: {result:?}");
    };
    let mut decoder = crate::frame::FrameDecoder::new();
    let mut reference = None;
    for frame in expected {
        reference = Some(decoder.advance(frame).unwrap().clone());
    }
    let reference = reference.unwrap();
    assert_eq!(actual.hour, reference.hour);
    assert_eq!(actual.branch, reference.branch);
    for &table in selected.iter().chain([TableId::Campaign].iter()) {
        assert_eq!(
            actual.tables[table.index()],
            reference.tables[table.index()],
            "{table:?}"
        );
    }
}

#[derive(Clone, Copy, Debug)]
enum Stop {
    Table,
    Raw,
    Group,
    Alias,
    CatchUp,
    Keys,
}

impl Stop {
    fn matches(self, step: Step) -> bool {
        match (self, step) {
            (Self::Table, Step::Table(TableId::Divisions)) => true,
            (Self::Raw, Step::Raw(TableId::Divisions, 2)) => true,
            (Self::Group, Step::Group(_, 2)) => true,
            (Self::Alias, Step::Alias(_, 2)) => true,
            (Self::CatchUp, Step::CatchUp(_, 2)) => true,
            (Self::Keys, Step::Keys(2)) => true,
            _ => false,
        }
    }
}

#[derive(Clone, Copy)]
enum Route {
    Seek,
    Recorded,
    Changes,
    Kept,
}

fn read_request(
    replay: &Replay,
    index: &BlockIndex,
    route: Route,
    check: impl FnMut() -> io::Result<()>,
) -> io::Result<()> {
    match route {
        Route::Seek => replay
            .seek_selected_with(index.branch, index.last_hour, Some(SELECTED), check)
            .map(|_| ()),
        Route::Recorded => replay
            .read_block_recorded_selected_with(index, SELECTED, check)
            .map(|_| ()),
        Route::Changes => replay.read_block_changes(
            index,
            &[crate::container::TableFields::all(TableId::Divisions)],
            |_| Ok(()),
            check,
        ),
        Route::Kept => replay.read_block_kept(index, SELECTED, check).map(|_| ()),
    }
}

fn cancelled_resolution(stop: Stop) {
    cancelled_read(stop, Route::Seek);
}

fn cancelled_read(stop: Stop, route: Route) {
    hooks::with(|state| *state = hooks::Hooks::default());
    let (probe, _) = alias_fixture(Layout::Chosen);
    probe.cache_decoded_sections(false);
    let index = &probe.index[0];
    let mut observed = Vec::new();
    let before = check::started();
    read_request(&probe, index, route, || {
        if let Some(step) = check::step() {
            observed.push((step, check::started() - before));
        }
        Ok(())
    })
    .unwrap();
    let (target, target_count) = observed
        .iter()
        .copied()
        .find(|(step, _)| stop.matches(*step))
        .unwrap_or_else(|| panic!("the fixture must reach {stop:?}: {observed:?}"));
    assert!(
        check::started() - before > target_count,
        "cancellation must skip later work"
    );

    let (replay, expected) = alias_fixture(Layout::Chosen);
    replay.cache_decoded_sections(true);
    replay.log_section_decodes();
    let index = &replay.index[0];
    let native = reference(&expected, index, index.last_hour);
    let flag = AtomicBool::new(false);
    let (arrived, at_gate) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let (result, final_count, calls_after_stop) = std::thread::scope(|scope| {
        let replay = &replay;
        let flag = &flag;
        let worker = scope.spawn(move || {
            let mut paused = false;
            let mut calls_after_stop = 0;
            let result = read_request(replay, index, route, || {
                if paused {
                    calls_after_stop += 1;
                } else if check::step() == Some(target) {
                    paused = true;
                    arrived.send(check::started()).unwrap();
                    released.recv().unwrap();
                }
                cancelled(flag)
            });
            (result, check::started(), calls_after_stop)
        });
        let at_cancel = at_gate
            .recv()
            .expect("the request must reach the resolution gate");
        // Looking up the cache while gated proves the callback holds no cache mutex.
        let held =
            replay
                .decoded_sections
                .ready(Key::new(replay, index, TableId::Divisions), 1, false);
        flag.store(true, Ordering::Release);
        release.send(()).unwrap();
        let (result, final_count, calls_after_stop) = worker.join().unwrap();
        assert!(held.is_none());
        assert_eq!(
            final_count, at_cancel,
            "no next table/hour/source step may start"
        );
        (result, final_count, calls_after_stop)
    });
    let error = result.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(error.to_string(), "Dashboard request cancelled");
    assert!(final_count > 0);
    assert_eq!(
        calls_after_stop, 0,
        "the stop must not retry through a fallback"
    );
    assert!(!replay.is_verified(index));
    for table in TableId::ALL {
        let key = Key::new(&replay, index, table);
        let prefix = replay.decoded_sections.ready(key, 1, false);
        let complete = replay.decoded_sections.ready(key, native.len(), false);
        assert_eq!(
            prefix.is_some(),
            complete.is_some(),
            "partial cache column: {table:?}"
        );
        if let Some(complete) = complete {
            for (at, frame) in native.iter().enumerate() {
                assert_eq!(
                    **complete.get(at),
                    frame.tables[table.index()],
                    "cached {table:?} at {at}"
                );
            }
        }
    }
    for table in [
        TableId::Divisions,
        TableId::DivisionStatus,
        TableId::LandCombatDivisions,
    ] {
        assert!(
            replay
                .decoded_sections
                .ready(Key::new(&replay, index, table), 1, false)
                .is_none(),
            "an interrupted group must publish no member: {table:?}"
        );
    }
    if matches!(route, Route::Seek) {
        assert!(
            replay.section_decodes().is_empty(),
            "point reads never enter the section cache"
        );
    } else if !matches!(stop, Stop::Table) {
        assert!(
            replay
                .section_decodes()
                .iter()
                .any(|decode| !decode.returned),
            "cancellation must unwind an active decoded-section read"
        );
    }
    let following = replay
        .seek_selected_with(index.branch, index.last_hour, Some(SELECTED), || Ok(()))
        .unwrap();
    assert_seek(following, &native, SELECTED);
    let frames = replay
        .read_block_selected_through_with(index, None, index.last_hour, || Ok(()))
        .unwrap();
    assert_eq!(
        bytes(&frames),
        bytes(&native),
        "following request must match independent native encoding"
    );
}

#[test]
fn cancelling_before_a_table_starts_no_later_resolution() {
    cancelled_resolution(Stop::Table);
}

#[test]
fn cancelling_a_raw_hour_starts_no_later_resolution() {
    cancelled_resolution(Stop::Raw);
}

#[test]
fn cancelling_a_group_hour_starts_no_later_resolution() {
    cancelled_resolution(Stop::Group);
}

#[test]
fn cancelling_an_alias_hour_starts_no_later_resolution() {
    cancelled_resolution(Stop::Alias);
}

#[test]
fn cancelling_a_source_catch_up_starts_no_later_resolution() {
    cancelled_resolution(Stop::CatchUp);
}

#[test]
fn cancelling_a_source_key_hour_starts_no_later_resolution() {
    cancelled_resolution(Stop::Keys);
}

#[test]
fn cancelling_a_recorded_group_hour_starts_no_later_resolution() {
    cancelled_read(Stop::Group, Route::Recorded);
}

fn cancelled_verification(route: Route, before_publication: bool) {
    hooks::with(|state| *state = hooks::Hooks::default());
    let (probe, _) = alias_fixture(Layout::Chosen);
    let index = &probe.index[0];
    assert!(!probe.is_verified(index));
    let mut sections = Vec::new();
    let mut checked_publication = false;
    read_request(&probe, index, route, || {
        if let Some(Step::Verify(id)) = check::step() {
            sections.push(id);
        } else if sections.len() == SECTIONS && !checked_publication {
            assert!(
                !probe.is_verified(index),
                "check before publishing verification"
            );
            checked_publication = true;
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(sections, (0..SECTIONS).collect::<Vec<_>>());
    assert!(checked_publication);
    assert!(probe.is_verified(index));

    let (replay, expected) = alias_fixture(Layout::Chosen);
    let index = &replay.index[0];
    let flag = AtomicBool::new(false);
    let mut sections = Vec::new();
    let mut at_cancel = None;
    let mut calls_after_stop = 0;
    let result = read_request(&replay, index, route, || {
        if flag.load(Ordering::Acquire) {
            calls_after_stop += 1;
        }
        if let Some(Step::Verify(id)) = check::step() {
            sections.push(id);
            if id == 1 && !before_publication {
                at_cancel = Some(check::started());
                flag.store(true, Ordering::Release);
            }
        } else if before_publication && sections.len() == SECTIONS {
            at_cancel = Some(check::started());
            flag.store(true, Ordering::Release);
        }
        cancelled(&flag)
    });
    let error = result.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(error.to_string(), "Dashboard request cancelled");
    let expected_sections = if before_publication {
        (0..SECTIONS).collect::<Vec<_>>()
    } else {
        vec![0, 1]
    };
    assert_eq!(
        sections, expected_sections,
        "no later verification section may start"
    );
    assert_eq!(Some(check::started()), at_cancel);
    assert_eq!(calls_after_stop, 0);
    assert!(!replay.is_verified(index));
    read_request(&replay, index, route, || Ok(())).unwrap();
    assert!(replay.is_verified(index));
    assert_seek(
        replay
            .seek_selected(index.branch, index.last_hour, Some(SELECTED))
            .unwrap(),
        &reference(&expected, index, index.last_hour),
        SELECTED,
    );
}

#[test]
fn a_changes_request_stops_verification_before_publication() {
    for before_publication in [false, true] {
        cancelled_verification(Route::Changes, before_publication);
    }
}

#[test]
fn a_kept_request_stops_verification_before_publication() {
    for before_publication in [false, true] {
        cancelled_verification(Route::Kept, before_publication);
    }
}

#[test]
fn unchecked_seeks_never_enable_decode_callbacks() {
    hooks::with(|state| *state = hooks::Hooks::default());
    for cache in [false, true] {
        for tables in [None, Some(SELECTED)] {
            let (replay, expected) = alias_fixture(Layout::Chosen);
            replay.cache_decoded_sections(cache);
            let index = &replay.index[0];
            let native = reference(&expected, index, index.last_hour);
            for _ in 0..2 {
                let before = check::enabled_callbacks();
                let started = check::started();
                let unchecked = match tables {
                    Some(tables) => {
                        replay.seek_selected(index.branch, index.last_hour, Some(tables))
                    }
                    None => replay.seek(index.branch, index.last_hour),
                }
                .unwrap();
                assert_eq!(
                    check::enabled_callbacks(),
                    before,
                    "unchecked seeks must neither construct nor invoke enabled callbacks"
                );
                assert!(check::started() > started, "must exercise block decoding");
                assert_seek(unchecked, &native, tables.unwrap_or(&TableId::ALL));

                let (created, invoked) = check::enabled_callbacks();
                let mut checks = 0;
                let mut decode_checks = 0;
                let checked = replay
                    .seek_selected_with(index.branch, index.last_hour, tables, || {
                        checks += 1;
                        if check::step().is_some() {
                            decode_checks += 1;
                        }
                        Ok(())
                    })
                    .unwrap();
                assert_eq!(check::enabled_callbacks(), (created + 1, invoked + checks));
                assert!(
                    decode_checks > 0,
                    "the live adapter must reach block decoding"
                );
                assert_seek(checked, &native, tables.unwrap_or(&TableId::ALL));
            }
        }
    }
}

#[test]
fn a_never_firing_check_preserves_independently_encoded_frame_bytes() {
    hooks::with(|state| *state = hooks::Hooks::default());
    for layout in LAYOUTS {
        let (replay, expected) = alias_fixture(layout);
        for cache in [false, true] {
            replay.cache_decoded_sections(cache);
            for index in &replay.index {
                for through in [index.first_hour, index.first_hour + 2, index.last_hour] {
                    let native = reference(&expected, index, through);
                    let mut checks = 0;
                    let frames = replay
                        .read_block_selected_through_with(index, None, through, || {
                            checks += 1;
                            Ok(())
                        })
                        .unwrap();
                    assert!(
                        checks > native.len(),
                        "must check inside the decoder, not only on returned frames"
                    );
                    assert_eq!(frames.len(), native.len());
                    assert_eq!(
                        bytes(&frames),
                        bytes(&native),
                        "{layout:?}, cache {cache}, through {through}"
                    );
                    let unchecked = replay
                        .read_block_selected_through(index, None, through, None)
                        .unwrap();
                    assert_eq!(bytes(&unchecked), bytes(&native));
                    let selected = replay
                        .seek_selected_with(index.branch, through, Some(SELECTED), || Ok(()))
                        .unwrap();
                    assert_seek(selected, &native, SELECTED);
                    let recorded = replay
                        .read_block_recorded_selected_with(index, SELECTED, || Ok(()))
                        .unwrap();
                    let full_native = reference(&expected, index, index.last_hour);
                    assert_eq!(recorded.len(), full_native.len());
                    for (actual, expected) in recorded.into_iter().zip(full_native) {
                        let actual = actual.into_owned_frame().unwrap();
                        let mut selected_expected = actual.clone();
                        for &table in SELECTED {
                            selected_expected.tables[table.index()] =
                                expected.tables[table.index()].clone();
                        }
                        assert_eq!(bytes(&[actual]), bytes(&[selected_expected]));
                    }
                }
            }
        }
    }
}

#[test]
fn a_late_source_catch_up_stops_inside_its_own_hour_loop() {
    hooks::with(|state| *state = hooks::Hooks::default());
    let hours: Vec<_> = (0..8).collect();
    let column: Vec<_> = hours
        .iter()
        .enumerate()
        .map(|(at, _)| {
            if at == 0 {
                TableUpdate::Snapshot(TableRows::Countries(Vec::new()))
            } else {
                TableUpdate::Unchanged {
                    baseline: hours[at - 1],
                }
            }
        })
        .collect();
    let flag = AtomicBool::new(false);
    let (arrived, at_gate) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let (error, followed, at_cancel, finished) = std::thread::scope(|scope| {
        let column = &column;
        let hours = &hours;
        let flag = &flag;
        let worker = scope.spawn(move || {
            let mut followed = Followed::default();
            let mut paused = false;
            let result = followed.catch_up(
                TableId::Countries,
                column.as_slice(),
                hours,
                hours.len() - 1,
                &mut Check::new(&mut || {
                    if !paused && check::step() == Some(Step::CatchUp(TableId::Countries, 2)) {
                        paused = true;
                        arrived.send(check::started()).unwrap();
                        released.recv().unwrap();
                    }
                    cancelled(flag)
                }),
            );
            (result.unwrap_err(), followed, check::started())
        });
        let at_cancel = at_gate.recv().unwrap();
        flag.store(true, Ordering::Release);
        release.send(()).unwrap();
        let (error, followed, finished) = worker.join().unwrap();
        (error, followed, at_cancel, finished)
    });
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(finished, at_cancel);
    assert_eq!(
        followed.source_at, 2,
        "six hour steps must remain unexecuted"
    );
    let mut following = Followed::default();
    following
        .catch_up(
            TableId::Countries,
            column.as_slice(),
            &hours,
            hours.len() - 1,
            &mut Check::disabled(),
        )
        .unwrap();
    assert_eq!(following.source_at, hours.len());
    assert_eq!(
        *following.view(),
        TableView::Complete(TableRows::Countries(Vec::new()))
    );
}
