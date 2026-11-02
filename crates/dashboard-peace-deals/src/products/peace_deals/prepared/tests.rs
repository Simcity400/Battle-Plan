use super::*;
use crate::products::peace_deals::{
    test_fixtures::{frame, replay},
    tests::{settled_row, with_settlement},
};
use dashboard_base::prepared::frame_changes::FrameChangeTracker;

/// Whether `frame` read the conference roster's slot.
fn read(recording: &Replay, frame: &DecodedFrame) -> bool {
    slot_read(
        recording.format,
        &frame.metrics,
        TableId::PeaceConferences,
        frame.hour,
    )
}

/// `frame` observed with every table changed, its events published as the emit writes them.
fn observe(
    prepared: &mut Prepared,
    recording: &Replay,
    frame: &DecodedFrame,
    published: &mut Vec<Vec<u8>>,
) {
    let mut events = Vec::new();
    let changes = FrameChanges {
        rows_changed: [true; replay_data::rows::TABLE_COUNT],
        table_read: [true; replay_data::rows::TABLE_COUNT],
        slot_read: [true; replay_data::rows::TABLE_COUNT],
        updates: None,
    };
    prepared
        .observe_changed(frame, &changes, read(recording, frame), &mut events)
        .unwrap();
    published.extend(events.iter().map(|event| serde_json::to_vec(event).unwrap()));
}

/// The events `published` holds, as a reader takes them up.
fn events(published: &[Vec<u8>]) -> Vec<Event> {
    published
        .iter()
        .map(|bytes| serde_json::from_slice(bytes).unwrap())
        .collect()
}

#[test]
fn peace_deferred_country_evidence_matches_late_finalizers_after_cuts_resume_and_a_gap() {
    let mut current = frame(100, 2, false, &[]);
    let recording = replay(vec![current.clone()]);
    let mut prepared = Prepared::new(&recording, 1).unwrap();
    let mut tracker = FrameChangeTracker::new();
    let mut frames = Vec::new();
    let mut published = Vec::new();
    let mut continuation = Vec::new();
    for hour in 100..=135 {
        if hour == 128 {
            continue;
        }
        current.hour = hour;
        if hour == 120
            && let replay_frame::frame::TableView::Complete(
                replay_data::rows::TableRows::Countries(rows),
            ) = &mut *current.tables[TableId::Countries.index()]
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
        let mut events = Vec::new();
        let roster = read(&recording, &current);
        prepared
            .observe_changed(
                &current,
                &tracker.observe(&current, recording.format),
                roster,
                &mut events,
            )
            .unwrap();
        published.extend(events.iter().map(|event| serde_json::to_vec(event).unwrap()));
        frames.push(current.clone());
        if [110, 124, 135].contains(&hour) {
            continuation = prepared.continuation().unwrap();
            compare(&continuation, &published, &frames);
            prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
            tracker.invalidate();
        }
    }
    let actual = reply(&continuation, events(&published), 135, false).unwrap();
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
    let mut current = frame(100, 2, false, &[]);
    let recording = replay(vec![current.clone()]);
    if let replay_frame::frame::TableView::Complete(replay_data::rows::TableRows::Subjects(rows)) =
        &mut *current.tables[TableId::Subjects.index()]
    {
        rows.push(replay_data::rows::SubjectRow {
            overlord: 1001,
            subject: 1002,
            start_hour: 90,
            end_hour: 103,
            state: b"authored subject".to_vec(),
            ..Default::default()
        });
    }
    let mut prepared = Prepared::new(&recording, 1).unwrap();
    let mut tracker = FrameChangeTracker::new();
    let mut events = Vec::new();
    let mut step = |prepared: &mut Prepared, current: &DecodedFrame| {
        let roster = read(&recording, current);
        prepared
            .observe_changed(
                current,
                &tracker.observe(current, recording.format),
                roster,
                &mut events,
            )
            .unwrap();
    };
    step(&mut prepared, &current);
    let first = prepared.scan.previous.clone().unwrap();
    assert_eq!(first.subjects.len(), 1);
    current.hour = 101;
    step(&mut prepared, &current);
    let second = prepared.scan.previous.clone().unwrap();
    assert!(Arc::ptr_eq(&first.owners, &second.owners));
    assert!(Arc::ptr_eq(&first.wars, &second.wars));
    assert!(Arc::ptr_eq(&first.subjects, &second.subjects));
    let visits =
        |prepared: &Prepared| prepared.scan.snapshot_inputs.row_visits + prepared.scan.exact.row_visits;
    let before = visits(&prepared);
    for hour in 102..=103 {
        current.hour = hour;
        step(&mut prepared, &current);
    }
    assert!(prepared.scan.previous.as_ref().unwrap().subjects.is_empty());
    assert_eq!(
        visits(&prepared),
        before,
        "expiry uses current native operands"
    );
}

/// What a complete reply says of a deal its last read leaves unsettled.
const UNSETTLED: [&str; 2] = [
    "No complete ownership and war read after settlement; outcomes are not recorded.",
    "No end was observed; the conference outcome is not recorded.",
];

/// Compares the reply from `continuation` and the events `published` before it with a scan
/// of `frames`, as a complete reply and as a prepared prefix through their last hour; returns
/// how many deals the prefix says are still being prepared.
fn compare(continuation: &[u8], published: &[Vec<u8>], frames: &[DecodedFrame]) -> usize {
    let reference = scan_branch(&replay(frames.to_vec()), 1, &AtomicBool::new(false)).unwrap();
    let last = frames.last().unwrap().hour;
    let actual = reply(continuation, events(published), last, false).unwrap();
    assert_eq!(
        serde_json::to_value(&actual).unwrap(),
        serde_json::to_value(&reference).unwrap()
    );
    // As a prefix, a deal a later read may still settle is preparing its
    // outcome rather than claiming it was never recorded; all else is equal.
    let prefix = reply(continuation, events(published), last, true).unwrap();
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
fn peace_events_match_cuts_resume_late_capture_gaps_and_identity_changes() {
    let mut frames = vec![frame(100, 3, false, &[9]), frame(101, 3, true, &[])];
    let recording = replay(frames.clone());
    let mut prepared = Prepared::new(&recording, 1).unwrap();
    let mut published = Vec::new();
    let mut preparing = 0;
    for frame in &frames {
        observe(&mut prepared, &recording, frame, &mut published);
        let continuation = prepared.continuation().unwrap();
        preparing += compare(
            &continuation,
            &published,
            &frames[..=frames
                .iter()
                .position(|value| value.hour == frame.hour)
                .unwrap()],
        );
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    }
    // The conference open at the first cut is preparing in that prefix.
    assert!(preparing > 0);
    let mut row = settled_row();
    row.actions[0].cached_name = Some(b"literal cached native name".to_vec());
    frames.push(with_settlement(frame(103, 3, true, &[]), row));
    observe(&mut prepared, &recording, frames.last().unwrap(), &mut published);
    compare(&prepared.continuation().unwrap(), &published, &frames);
    let mut changed = frame(104, 3, false, &[]);
    if let replay_frame::frame::TableView::Complete(replay_data::rows::TableRows::Countries(rows)) =
        &mut *changed.tables[TableId::Countries.index()]
    {
        rows[0].id += 100_000;
        rows.sort_by_key(replay_data::rows::CountryRow::key);
        rows.dedup_by_key(|row| row.key());
    }
    frames.push(changed);
    observe(&mut prepared, &recording, frames.last().unwrap(), &mut published);
    compare(&prepared.continuation().unwrap(), &published, &frames);
    assert!(prepared.scan.reply.deals.is_empty());
    assert!(prepared.scan.exact.captures.is_empty());
}

/// Fresh observers of one recording write the same continuation bytes: each
/// owner's states and each overlord's subjects are listed in order, not hashed.
#[test]
fn peace_fresh_observers_write_the_same_continuation() {
    use replay_data::{rows::SubjectRow, rows::TableRows};
    use replay_frame::frame::TableView;
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
            let mut prepared = Prepared::new(&recording, 1).unwrap();
            let mut published = Vec::new();
            for frame in [&first, &second] {
                observe(&mut prepared, &recording, frame, &mut published);
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
            prepared.continuation().unwrap()
        })
        .collect();
    assert_eq!(continuations[0], continuations[1]);
}
