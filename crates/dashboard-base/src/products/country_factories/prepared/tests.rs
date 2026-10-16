use super::*;
use crate::test_fixtures::scale_frame;
#[cfg(feature = "slow-tests")]
use crate::test_fixtures::{CAMPAIGN, records};
use replay::{frame::TableView, rows::TableRows};

fn frame(hour: i32, count: u32) -> DecodedFrame {
    let mut frame = scale_frame(1, hour, count);
    *frame.tables[TableId::CountryFactories.index()] =
        TableView::Complete(TableRows::CountryFactories(
            (0..count)
                .map(|index| CountryFactoriesRow {
                    country: 100_000 + index,
                    military_total_raw: (i64::from(index) + i64::from(hour)) * 100_000 + 99_999,
                    civilian_total_raw: -299_999,
                    consumer_goods: (index % 5) as i32,
                    ..Default::default()
                })
                .collect(),
        ));
    frame
}

#[test]
#[cfg(feature = "slow-tests")]
fn progressive_factory_sampler_matches_observer_on_every_key_at_5000_countries() {
    let frames = [frame(0, 5000), frame(1, 5000)];
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())]).unwrap();
    let mut observer = Prepared::default();
    for frame in &frames {
        let samples = sample(frame, None).unwrap();
        assert_eq!(samples.len(), 5000 * fields().len());
        observer
            .observe(
                frame,
                &replay,
                &crate::prepared::frame_changes::FrameChangeTracker::new()
                    .observe(frame, replay.format),
            )
            .unwrap();
        for ((tag, field), (identity, amount)) in &samples {
            assert_eq!(
                *identity,
                observer.scan.tracks[tag].last.map(|(identity, _)| identity)
            );
            let key = observer.keys[&series::field_key(tag, field)];
            assert_eq!(
                observer.parts.state(key).unwrap().last.unwrap().value,
                amount.unwrap()
            );
        }
        for (tag, track) in &observer.scan.tracks {
            let key = observer.keys[&series::field_key(tag, "")];
            for index in 0..21 {
                assert_eq!(
                    observer.values[index]
                        .state(key)
                        .unwrap()
                        .last
                        .unwrap()
                        .value,
                    i128::from(count_for(
                        &track.runs.last().unwrap().values,
                        &rank_selection(index)
                    ))
                );
            }
        }
    }
}

#[test]
fn progressive_factory_sampler_keeps_absent_and_ambiguous_rows_unknown() {
    let mut frame = frame(0, 2);
    if let TableView::Complete(TableRows::CountryFactories(rows)) =
        &mut *frame.tables[TableId::CountryFactories.index()]
    {
        rows.push(rows[0].clone());
    }
    let tag = String::from_utf8(crate::test_fixtures::scale_tag(0)).unwrap();
    let requested = vec![
        (tag.clone(), "0".into()),
        ("tail absent".into(), "0".into()),
    ];
    let samples = sample(&frame, Some(&requested)).unwrap();
    assert_eq!(samples.len(), 2);
    assert_eq!(samples[&requested[0]], (Some((0, 100_000)), None));
    assert_eq!(samples[&requested[1]], (None, None));
    assert!(sample(&frame, Some(&[(tag, "31".into())])).is_err());
}

#[test]
fn phase_b_unchanged_hours_and_retained_holders_are_flat_and_one_row_is_bounded() {
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    let measure = |history: usize, hours: i32| -> io::Result<observer_tests::Work> {
        let mut first = frame(100, 128);
        if let TableView::Complete(TableRows::CountryFactories(rows)) =
            &mut *first.tables[TableId::CountryFactories.index()]
        {
            rows.truncate(1);
        }
        let replay = Replay::from_records(
            crate::test_fixtures::CAMPAIGN.into(),
            &[crate::test_fixtures::records(1, vec![first.clone()])],
        )?;
        let mut state = Prepared::default();
        let mut tracker = FrameChangeTracker::new();
        for index in 0..history {
            first.hour = 100 + index as i32;
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *first.tables[TableId::Countries.index()]
            {
                rows[0].tag = format!("retired / {index}").into_bytes();
            }
            state.observe(&first, &replay, &tracker.observe(&first, replay.format))?;
        }
        let delta = observer_tests::unchanged_frames(
            state.scan.work,
            &first,
            hours,
            replay.format,
            |frame, changes| {
                state.observe(frame, &replay, changes)?;
                Ok(state.scan.work)
            },
        )?;
        assert_eq!(delta, observer_tests::Work::default());
        first.hour += hours;
        tracker.observe(&first, replay.format);
        first.hour += 1;
        if let TableView::Complete(TableRows::CountryFactories(rows)) =
            &mut *first.tables[TableId::CountryFactories.index()]
        {
            rows[0].military_total_raw += 100_000_000;
        }
        let before = state.scan.work;
        state.observe(&first, &replay, &tracker.observe(&first, replay.format))?;
        let after = state.scan.work;
        let changed = observer_tests::Work {
            row_visits: after.row_visits - before.row_visits,
            history_visits: after.history_visits - before.history_visits,
            emitted: after.emitted - before.emitted,
            ..Default::default()
        };
        assert!(
            changed.row_visits <= 1 && changed.history_visits <= 1 && changed.emitted <= 60,
            "{changed:?}"
        );
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
fn phase_b_deferred_native_runs_match_eager_after_two_checkpoints_resume_append_gaps_and_unknowns()
{
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    let mut lazy = Scan::default();
    let mut oracle = Scan::default();
    let mut tracker = FrameChangeTracker::new();
    let initial = frame(100, 3);
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
            *next.tables[TableId::CountryFactories.index()] = TableView::Unavailable {
                reason: "synthetic refusal".into(),
            };
        }
        oracle.observe(&next, replay::format::FormatVersion::CURRENT);
        lazy.observe_changes(
            &next,
            replay::format::FormatVersion::CURRENT,
            Some(&tracker.observe(&next, replay::format::FormatVersion::CURRENT)),
        );
        if hour == 103 || hour == 108 {
            lazy.seal();
            lazy = serde_json::from_slice(&serde_json::to_vec(&lazy).unwrap()).unwrap();
            tracker.invalidate();
        }
    }
    observer_tests::assert_reply_parity(&lazy.finish(), &oracle.finish()).unwrap();
}
