use super::*;
use crate::products::peace_deals::{
    test_fixtures::{frame, replay},
    tests::{settled_row, with_settlement},
};
use dashboard_base::prepared::series::tests::Fixture;

#[test]
fn peace_observer_work_is_flat_over_unchanged_hours_and_retired_finalizer_history() {
    use dashboard_base::prepared::{
        frame_changes::FrameChangeTracker,
        observer_tests::{self, Work},
    };
    let initial = frame(100, 2, false, &[9]);
    let recording = replay(vec![initial.clone()]);
    observer_tests::assert_flat(
        observer_tests::scaled_work(10, |hours| {
            let mut prepared = Prepared::new(&recording, 1);
            let mut tracker = FrameChangeTracker::new();
            prepared.observe_changed(&initial, &tracker.observe(&initial, recording.format))?;
            observer_tests::unchanged_frames(
                prepared.work,
                &initial,
                hours as i32,
                recording.format,
                |frame, changes| {
                    prepared.observe_changed(frame, changes)?;
                    Ok(prepared.work)
                },
            )
        })
        .unwrap(),
    );
    observer_tests::assert_flat(
        observer_tests::scaled_work(10, |history| {
            let mut prepared = Prepared::new(&recording, 1);
            let mut tracker = FrameChangeTracker::new();
            let mut current = initial.clone();
            for offset in 0..history {
                current.hour = 100 + offset as i32;
                let mut row = settled_row();
                row.sequence = offset as u64 + 1;
                row.hour = Some(current.hour);
                current = with_settlement(current, row);
                prepared.observe_changed(&current, &tracker.observe(&current, recording.format))?;
            }
            let before = prepared.work;
            current.hour += 1;
            if let replay::frame::TableView::Complete(replay::rows::TableRows::States(rows)) =
                &mut *current.tables[TableId::States.index()]
            {
                rows.truncate(1);
                rows[0].owner = 1002;
            }
            prepared.observe_changed(&current, &tracker.observe(&current, recording.format))?;
            assert_eq!(prepared.work.row_visits - before.row_visits, 1);
            Ok(Work {
                row_visits: prepared.work.row_visits - before.row_visits,
                history_visits: prepared.work.history_visits - before.history_visits,
                ..Default::default()
            })
        })
        .unwrap(),
    );
}

#[test]
fn peace_deferred_country_evidence_matches_late_finalizers_after_commits_resume_append_and_a_gap() {
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    let flag = AtomicBool::new(false);
    let mut fixture = Fixture::new();
    let mut current = frame(100, 2, false, &[]);
    let recording = replay(vec![current.clone()]);
    let mut prepared = Prepared::new(&recording, 1);
    let mut tracker = FrameChangeTracker::new();
    let mut frames = Vec::new();
    for hour in 100..=135 {
        if hour == 128 {
            continue;
        }
        current.hour = hour;
        if hour == 120
            && let replay::frame::TableView::Complete(replay::rows::TableRows::Countries(rows)) =
                &mut *current.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"renamed peace holder".to_vec();
        }
        if hour == 125 || hour == 130 {
            let mut row = settled_row();
            row.sequence = if hour == 125 { 1 } else { 2 };
            row.hour = Some(if hour == 125 { 115 } else { 126 });
            row.conference = Some(if hour == 125 { 9 } else { 10 });
            current = with_settlement(current, row);
        }
        prepared
            .observe_changed(&current, &tracker.observe(&current, recording.format))
            .unwrap();
        frames.push(current.clone());
        if [110, 124, 135].contains(&hour) {
            let mut writer = if hour == 135 {
                fixture.append_writer()
            } else {
                fixture.writer()
            };
            prepared.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
            compare(&fixture, &frames);
            prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
            tracker.invalidate();
        }
    }
    let actual = reply(&fixture.store, &fixture.key, 1, 135, false, &flag)
        .unwrap()
        .unwrap();
    let captures: Vec<_> = actual
        .deals
        .iter()
        .filter_map(|deal| deal.captured.as_ref())
        .collect();
    assert_eq!(captures.len(), 2);
    assert!(captures.iter().any(|capture| {
        capture.hour == 115
            && capture
                .actions
                .iter()
                .any(|action| action.giver_tag.as_deref() == Some("country/1"))
    }));
    assert!(captures.iter().any(|capture| {
        capture.hour == 126
            && capture
                .actions
                .iter()
                .any(|action| action.giver_tag.as_deref() == Some("renamed peace holder"))
    }));
}

#[test]
fn peace_snapshot_projections_share_unchanged_maps_and_expire_subjects_at_native_hours() {
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    let mut current = frame(100, 2, false, &[]);
    let recording = replay(vec![current.clone()]);
    if let replay::frame::TableView::Complete(replay::rows::TableRows::Subjects(rows)) =
        &mut *current.tables[TableId::Subjects.index()]
    {
        rows.push(replay::rows::SubjectRow {
            overlord: 1001,
            subject: 1002,
            start_hour: 90,
            end_hour: 103,
            state: b"authored subject".to_vec(),
            ..Default::default()
        });
    }
    let mut prepared = Prepared::new(&recording, 1);
    let mut tracker = FrameChangeTracker::new();
    prepared
        .observe_changed(&current, &tracker.observe(&current, recording.format))
        .unwrap();
    let first = prepared.scan.previous.clone().unwrap();
    assert_eq!(first.subjects.len(), 1);
    current.hour = 101;
    prepared
        .observe_changed(&current, &tracker.observe(&current, recording.format))
        .unwrap();
    let second = prepared.scan.previous.as_ref().unwrap();
    assert!(Arc::ptr_eq(&first.owners, &second.owners));
    assert!(Arc::ptr_eq(&first.wars, &second.wars));
    assert!(Arc::ptr_eq(&first.subjects, &second.subjects));
    let before = prepared.work.row_visits;
    for hour in 102..=103 {
        current.hour = hour;
        prepared
            .observe_changed(&current, &tracker.observe(&current, recording.format))
            .unwrap();
    }
    assert!(prepared.scan.previous.as_ref().unwrap().subjects.is_empty());
    assert_eq!(
        prepared.work.row_visits, before,
        "expiry uses current native operands"
    );
}

/// What a complete reply says of a deal its last read leaves unsettled.
const UNSETTLED: [&str; 2] = [
    "No complete ownership and war read after settlement; outcomes are not recorded.",
    "No end was observed; the conference outcome is not recorded.",
];

/// Compares the published pages with a scan of `frames`, as a complete reply
/// and as a prepared prefix through their last hour; returns how many deals
/// the prefix says are still being prepared.
fn compare(fixture: &Fixture, frames: &[DecodedFrame]) -> usize {
    let reference = scan_branch(&replay(frames.to_vec()), 1, &AtomicBool::new(false)).unwrap();
    let last = frames.last().unwrap().hour;
    let actual = reply(
        &fixture.store,
        &fixture.key,
        1,
        last,
        false,
        &AtomicBool::new(false),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        serde_json::to_value(&actual).unwrap(),
        serde_json::to_value(&reference).unwrap()
    );
    // As a prefix, a deal a later read may still settle is preparing its
    // outcome rather than claiming it was never recorded; all else is equal.
    let prefix = reply(
        &fixture.store,
        &fixture.key,
        1,
        last,
        true,
        &AtomicBool::new(false),
    )
    .unwrap()
    .unwrap();
    let mut expected = serde_json::to_value(&reference).unwrap();
    let mut preparing = 0;
    for deal in expected["deals"].as_array_mut().unwrap() {
        let limitations = deal["limitations"].as_array_mut().unwrap();
        let before = limitations.len();
        limitations.retain(|line| !UNSETTLED.contains(&line.as_str().unwrap()));
        if limitations.len() != before {
            deal["preparing"] = serde_json::json!(["outcome"]);
            preparing += 1;
        }
    }
    assert_eq!(serde_json::to_value(&prefix).unwrap(), expected);
    // Stored native names must survive serialization even though they are not
    // exposed as byte operands in the ordinary reply.
    for deal in &actual.deals {
        if let Some(captured) = &deal.captured {
            for action in &captured.actions {
                let matching = reference
                    .deals
                    .iter()
                    .filter_map(|deal| deal.captured.as_ref())
                    .flat_map(|capture| &capture.actions)
                    .find(|other| other.ordinal == action.ordinal)
                    .unwrap();
                assert_eq!(action.cached_name, matching.cached_name);
            }
        }
    }
    preparing
}

#[test]
fn peace_events_match_two_commits_resume_append_late_capture_gaps_and_identity_changes() {
    let mut fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut frames = vec![frame(100, 3, false, &[9]), frame(101, 3, true, &[])];
    let mut prepared = Prepared::new(&replay(frames.clone()), 1);
    let mut preparing = 0;
    for frame in &frames {
        prepared.observe(frame).unwrap();
        let mut writer = fixture.writer();
        prepared.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        preparing += compare(
            &fixture,
            &frames[..=frames
                .iter()
                .position(|value| value.hour == frame.hour)
                .unwrap()],
        );
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    }
    // The conference open at the first commit is preparing in that prefix.
    assert!(preparing > 0);
    let mut row = settled_row();
    row.actions[0].cached_name = Some(b"literal cached native name".to_vec());
    frames.push(with_settlement(frame(103, 3, true, &[]), row));
    let mut writer = fixture.append_writer();
    prepared.observe(frames.last().unwrap()).unwrap();
    prepared.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    compare(&fixture, &frames);
    let mut changed = frame(104, 3, false, &[]);
    if let replay::frame::TableView::Complete(replay::rows::TableRows::Countries(rows)) =
        &mut *changed.tables[TableId::Countries.index()]
    {
        rows[0].id += 100_000;
        rows.sort_by_key(replay::rows::CountryRow::key);
        rows.dedup_by_key(|row| row.key());
    }
    frames.push(changed);
    prepared.observe(frames.last().unwrap()).unwrap();
    let mut writer = fixture.writer();
    prepared.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    compare(&fixture, &frames);
    assert!(prepared.scan.reply.deals.is_empty());
    assert!(prepared.scan.exact.captures.is_empty());
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn peace_events_preserve_five_thousand_countries_and_finalized_rows_leave_the_checkpoint() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let frames = vec![frame(100, 5_000, false, &[]), frame(101, 5_000, true, &[])];
    let mut prepared = Prepared::new(&replay(frames.clone()), 1);
    for frame in &frames {
        prepared.observe(frame).unwrap();
        let mut writer = fixture.writer();
        prepared.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
    }
    compare(&fixture, &frames);
    assert!(prepared.scan.reply.deals.is_empty());
    assert!(prepared.scan.exact.captures.is_empty());
    let bytes = serde_json::to_vec(&prepared).unwrap().len();
    eprintln!(
        "peace checkpoint after a 5000-country settlement: {bytes} bytes; zero completed deals retained"
    );
    let mut prepared = Prepared::new(&replay(vec![frame(100, 2, false, &[])]), 1);
    let mut before = 0;
    for hour in 100..2_100 {
        prepared.observe(&frame(hour, 2, false, &[])).unwrap();
        if hour == 299 {
            before = serde_json::to_vec(&prepared).unwrap().len();
        }
    }
    let after = serde_json::to_vec(&prepared).unwrap().len();
    eprintln!(
        "peace unchanged checkpoint: {before} bytes after 200 hours, {after} after 2000 hours"
    );
    assert!(after <= before + 128);
}

/// Fresh observers of one recording write the same continuation bytes: each
/// owner's states and each overlord's subjects are listed in order, not hashed.
#[test]
fn peace_fresh_observers_write_the_same_continuation() {
    use replay::{
        frame::TableView,
        rows::{SubjectRow, TableRows},
    };
    let recording = replay(vec![frame(100, 64, false, &[9])]);
    let mut first = frame(100, 64, false, &[9]);
    if let TableView::Complete(TableRows::States(rows)) =
        &mut *first.tables[TableId::States.index()]
    {
        for row in rows.iter_mut() {
            row.owner = 1_001 + row.id % 3;
        }
    }
    if let TableView::Complete(TableRows::Subjects(rows)) =
        &mut *first.tables[TableId::Subjects.index()]
    {
        rows.extend((2..=64).map(|subject| SubjectRow {
            overlord: 1_001,
            subject: 1_000 + subject,
            start_hour: 90,
            state: b"authored subject".to_vec(),
            ..Default::default()
        }));
    }
    let mut second = first.clone();
    second.hour = 101;
    let continuations: Vec<_> = (0..2)
        .map(|_| {
            let mut prepared = Prepared::new(&recording, 1);
            for frame in [&first, &second] {
                prepared.observe(frame).unwrap();
            }
            let previous = prepared.scan.previous.as_ref().unwrap();
            assert_eq!(previous.states_by_owner.len(), 3);
            assert_eq!(previous.subject_targets["country/1"].len(), 63);
            assert!(
                previous
                    .states_by_owner
                    .values()
                    .all(|states| states.windows(2).all(|pair| pair[0] < pair[1]))
            );
            assert!(
                previous
                    .subject_targets
                    .values()
                    .all(|subjects| subjects.windows(2).all(|pair| pair[0] < pair[1]))
            );
            serde_json::to_vec(&(&prepared.scan, &prepared.ordinals)).unwrap()
        })
        .collect();
    assert_eq!(continuations[0], continuations[1]);
}
