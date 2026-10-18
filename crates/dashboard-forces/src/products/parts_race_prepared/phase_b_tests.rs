use super::test_fixtures::{Fixture, frame};
use super::*;
use dashboard_base::prepared::{
    frame_changes::FrameChangeTracker,
    observer_tests::{self, Work},
};
use replay_data::{rows::TableRows, rows::tables};

fn snapshot<F: Family>(state: &Prepared<F>, fixture: &Fixture) -> Work {
    let native = F::work(&state.scan);
    let catalogue = dashboard_base::catalogue::catalogue(
        &fixture.source,
        fixture.context.content.as_ref().unwrap(),
    )
    .unwrap();
    let loads = if F::NAME == "air" {
        dashboard_base::products::equipment::unit_loads(&catalogue)
    } else {
        catalogue
            .content
            .layers
            .derived::<dashboard_text::catalogue::CachedResult<installed_content_core::unit_leader::SubUnits>>()
            .loads()
    };
    Work {
        content_loads: loads,
        row_visits: native.row_visits,
        history_visits: native.history_visits + state.work.history_visits,
        emitted: state.work.emitted,
        ..Default::default()
    }
}

fn sparse_frame(hour: i32, countries: u32) -> DecodedFrame {
    let mut frame = frame(hour, countries);
    for table in [
        TableId::AirWings,
        TableId::WingEquipment,
        TableId::Ships,
        TableId::ShipModels,
        TableId::TaskForces,
    ] {
        match &mut *frame.tables[table.index()] {
            TableView::Complete(TableRows::AirWings(rows)) => rows.truncate(1),
            TableView::Complete(TableRows::WingEquipment(rows)) => rows.truncate(1),
            TableView::Complete(TableRows::Ships(rows)) => rows.truncate(1),
            TableView::Complete(TableRows::ShipModels(rows)) => rows.truncate(1),
            TableView::Complete(TableRows::TaskForces(rows)) => rows.truncate(1),
            _ => unreachable!(),
        }
    }
    frame
}

fn work_is_flat<F: Family>() {
    let fixture = Fixture::new();
    let measure = |history: usize, hours: i32| -> io::Result<Work> {
        let mut state = Prepared::<F>::default();
        state.configure(&fixture.source, &fixture.context);
        let mut frame = sparse_frame(100, 128);
        let mut tracker = FrameChangeTracker::new();
        for index in 0..history {
            frame.hour = 100 + index as i32;
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                rows[0].tag = format!("retired / {index}").into_bytes();
            }
            state.observe(
                &frame,
                fixture.source.replay_ref(),
                &fixture.source,
                &fixture.context,
                &AtomicBool::new(false),
                &tracker.observe(&frame, fixture.source.replay_ref().format),
            )?;
        }
        let work = observer_tests::unchanged_frames(
            snapshot(&state, &fixture),
            &frame,
            hours,
            fixture.source.replay_ref().format,
            |frame, changes| {
                state.observe(
                    frame,
                    fixture.source.replay_ref(),
                    &fixture.source,
                    &fixture.context,
                    &AtomicBool::new(false),
                    changes,
                )?;
                Ok(snapshot(&state, &fixture))
            },
        )?;
        assert_eq!(
            work,
            Work::default(),
            "unchanged inputs visited native rows or accumulated state"
        );
        frame.hour += hours;
        tracker.observe(&frame, fixture.source.replay_ref().format);
        frame.hour += 1;
        let table = if F::NAME == "air" {
            TableId::AirWings
        } else {
            TableId::Ships
        };
        match &mut *frame.tables[table.index()] {
            TableView::Complete(TableRows::AirWings(rows)) => rows[0].planes += 1,
            TableView::Complete(TableRows::Ships(rows)) => rows[0].strength += 1,
            _ => unreachable!(),
        }
        let before = snapshot(&state, &fixture);
        state.observe(
            &frame,
            fixture.source.replay_ref(),
            &fixture.source,
            &fixture.context,
            &AtomicBool::new(false),
            &tracker.observe(&frame, fixture.source.replay_ref().format),
        )?;
        let after = snapshot(&state, &fixture);
        let changed = Work {
            row_visits: after.row_visits - before.row_visits,
            history_visits: after.history_visits - before.history_visits,
            emitted: after.emitted - before.emitted,
            ..Default::default()
        };
        assert!(
            changed.row_visits <= 1 && changed.history_visits <= 5 && changed.emitted <= 20,
            "{changed:?}"
        );
        // The real native census and the prepared observer retain the same point values.
        let sampled = sample::<F>(&frame, None)?;
        let observed = std::array::from_fn(|index| F::observed(&state.scan, index, frame.hour));
        assert_eq!(sampled, sample_rows::<F>(&state.scan, &observed, None)?);
        assert_eq!(frame.complete::<tables::Countries>()?.len(), 128);
        Ok(changed)
    };
    observer_tests::assert_flat(
        observer_tests::scaled_work(8, |hours| measure(1, hours as i32)).unwrap(),
    );
    observer_tests::assert_flat(
        observer_tests::scaled_work(8, |history| measure(history, 8)).unwrap(),
    );
}

#[test]
fn phase_b_air_work_is_flat_over_hours_history_and_one_changed_row() {
    work_is_flat::<crate::products::air_force::prepared::Air>();
}
#[test]
fn phase_b_navy_work_is_flat_over_hours_history_and_one_changed_row() {
    work_is_flat::<crate::products::navy::prepared::Navy>();
}

fn deferred_parity<F: Family>() {
    use replay_frame::frame::FrameEncoder;
    let mut lazy = F::Scan::default();
    let mut native = F::Scan::default();
    let mut tracker = FrameChangeTracker::new();
    let mut encoder = FrameEncoder::default();
    let flag = AtomicBool::new(false);
    let initial = sparse_frame(100, 3);
    for hour in [100, 101, 102, 103, 104, 105, 106, 107, 108, 111, 112, 113] {
        let mut next = initial.clone();
        next.hour = hour;
        if hour >= 106
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *next.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"renamed / first".to_vec();
        }
        if hour == 107 {
            for &table in F::TABLES {
                *next.tables[table.index()] = TableView::Unavailable {
                    reason: "synthetic refusal".into(),
                };
            }
        }
        let raw = encoder.encode(next.clone(), hour == 100).unwrap();
        F::step(
            &mut native,
            &raw,
            &next,
            FormatVersion::CURRENT,
            &flag,
            None,
        )
        .unwrap();
        let changes = tracker.observe(&next, FormatVersion::CURRENT);
        let raw = updates_changed(&next, F::TABLES, Some(&changes), F::applied(&lazy));
        F::step(
            &mut lazy,
            raw.as_ref(),
            &next,
            FormatVersion::CURRENT,
            &flag,
            Some(&changes),
        )
        .unwrap();
        if hour == 103 || hour == 108 {
            F::finish_hour(&mut lazy);
            lazy = serde_json::from_slice(&serde_json::to_vec(&lazy).unwrap()).unwrap();
            F::resume(
                &mut lazy,
                &updates(&next, F::TABLES),
                &next,
                FormatVersion::CURRENT,
                &flag,
            )
            .unwrap();
            tracker.invalidate();
        }
    }
    F::finish_hour(&mut native);
    F::finish_hour(&mut lazy);
    for index in 0..4 {
        observer_tests::assert_reply_parity(&F::runs(&lazy, index), &F::runs(&native, index))
            .unwrap();
    }
    observer_tests::assert_reply_parity(&F::metadata(&lazy), &F::metadata(&native)).unwrap();
}

#[test]
fn phase_b_air_deferred_runs_match_native_across_checkpoints_resume_append_gaps_and_aliases() {
    deferred_parity::<crate::products::air_force::prepared::Air>();
}
#[test]
fn phase_b_navy_deferred_runs_match_native_across_checkpoints_resume_append_gaps_and_aliases() {
    deferred_parity::<crate::products::navy::prepared::Navy>();
}

fn recorded_deltas<F: Family>() {
    use replay_frame::frame::FrameEncoder;
    use std::sync::Arc;
    let mut scan = F::Scan::default();
    let mut oracle = F::Scan::default();
    let mut encoder = FrameEncoder::default();
    let mut tracker = FrameChangeTracker::new();
    let flag = AtomicBool::new(false);
    let mut next = frame(100, 32);
    for hour in 100..108 {
        next.hour = hour;
        let table = if F::NAME == "air" {
            TableId::AirWings
        } else {
            TableId::Ships
        };
        match hour {
            101 | 105 => match &mut *next.tables[table.index()] {
                TableView::Complete(TableRows::AirWings(rows)) => rows[0].planes += 1,
                TableView::Complete(TableRows::Ships(rows)) => rows[0].strength += 1,
                _ => unreachable!(),
            },
            102 => match &mut *next.tables[table.index()] {
                TableView::Complete(TableRows::AirWings(rows)) => rows[0].mission_mask = 1,
                TableView::Complete(TableRows::Ships(rows)) => rows[0].organisation += 1,
                _ => unreachable!(),
            },
            103 => match &mut *next.tables[table.index()] {
                TableView::Complete(TableRows::AirWings(rows)) => {
                    rows.remove(0);
                }
                TableView::Complete(TableRows::Ships(rows)) => {
                    rows.remove(0);
                }
                _ => unreachable!(),
            },
            104 => match &mut *next.tables[table.index()] {
                TableView::Complete(TableRows::AirWings(rows)) => {
                    let mut added = rows[0].clone();
                    added.id = 1000;
                    rows.push(added);
                }
                TableView::Complete(TableRows::Ships(rows)) => {
                    let mut added = rows[0].clone();
                    added.id = 1000;
                    rows.push(added);
                }
                _ => unreachable!(),
            },
            106 => {
                let companion = if F::NAME == "air" {
                    TableId::WingEquipment
                } else {
                    TableId::ShipModels
                };
                *next.tables[companion.index()] = TableView::Unavailable {
                    reason: "synthetic missing companion".into(),
                };
            }
            107 => {
                next.hour += 1;
                let companion = if F::NAME == "air" {
                    TableId::WingEquipment
                } else {
                    TableId::ShipModels
                };
                next.tables[companion.index()] = frame(100, 32).tables[companion.index()].clone();
            }
            _ => {}
        }
        let raw = Arc::new(
            encoder
                .encode(next.clone(), hour == 100 || hour == 105)
                .unwrap(),
        );
        let changes = tracker.observe_updates(&next, Arc::clone(&raw), FormatVersion::CURRENT);
        let before = F::work(&scan).history_visits;
        step_rows::<F>(
            &mut scan,
            hour != 100,
            &next,
            FormatVersion::CURRENT,
            &flag,
            &changes,
        )
        .unwrap();
        F::step(
            &mut oracle,
            &updates(&next, F::TABLES),
            &next,
            FormatVersion::CURRENT,
            &flag,
            None,
        )
        .unwrap();
        if hour == 101 {
            let adapted = updates_changed(&next, F::TABLES, Some(&changes), Some(100));
            assert!(
                matches!(&adapted, std::borrow::Cow::Borrowed(_)),
                "row deltas became snapshots"
            );
            let recorded = changes.recorded().unwrap();
            for &table in F::TABLES {
                assert!(
                    std::ptr::eq(
                        &adapted.tables[table.index()],
                        &recorded.tables[table.index()]
                    ),
                    "{table:?}: the census must borrow the shared update cell"
                );
            }
            assert_eq!(
                F::work(&scan).history_visits - before,
                1,
                "one changed owner revisited the country census"
            );
        }
        if hour == 102 {
            assert_eq!(
                F::work(&scan).history_visits,
                before,
                "unread fields revisited the census"
            );
        }
        F::finish_hour(&mut scan);
        F::finish_hour(&mut oracle);
        for index in 0..4 {
            observer_tests::assert_reply_parity(&F::runs(&scan, index), &F::runs(&oracle, index))
                .unwrap();
        }
        observer_tests::assert_reply_parity(&F::metadata(&scan), &F::metadata(&oracle)).unwrap();
    }
}

#[test]
fn air_preparation_keeps_sparse_deltas_and_ignores_mission_only_changes() {
    recorded_deltas::<crate::products::air_force::prepared::Air>();
}

#[test]
fn navy_preparation_keeps_sparse_deltas_and_ignores_organisation_only_changes() {
    recorded_deltas::<crate::products::navy::prepared::Navy>();
}
