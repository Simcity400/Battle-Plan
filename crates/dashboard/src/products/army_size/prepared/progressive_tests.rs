use super::*;
use crate::products::parts_race_prepared::progressive_tests::{Fixture, frame, tag};
use crate::test_fixtures::{CAMPAIGN, records};
use serde_json::json;

#[test]
fn army_prepared_enables_detailed_inventory_when_fresh_and_resumed() {
    let fresh = Prepared::default();
    assert!(fresh.state.inventory.is_some());
    let resumed: Prepared = serde_json::from_slice(&serde_json::to_vec(&fresh).unwrap()).unwrap();
    assert!(resumed.state.inventory.is_some());
}

#[test]
#[cfg(feature = "slow-tests")]
fn army_progressive_sampler_matches_observer_every_key_at_5000_countries() {
    let frames = [frame(6, 5000), frame(7, 5000)];
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())]).unwrap();
    let mut prepared = Prepared::default();
    for frame in &frames {
        prepared
            .observe(
                frame,
                &replay,
                &crate::prepared::frame_changes::FrameChangeTracker::new()
                    .observe(frame, replay.format),
            )
            .unwrap();
        let sampled = sample(frame, None).unwrap();
        assert_eq!(sampled.len(), 5000 * 8);
        assert_eq!(
            sampled,
            sample_rows(&prepared.scan, &prepared.state, frame.hour, None).unwrap()
        );
        for ((tag, field), (identity, value)) in &sampled {
            assert!(identity.is_some());
            assert_eq!(
                *value,
                prepared.values[field.parse::<usize>().unwrap()]
                    .state(prepared.tags[tag])
                    .and_then(|state| state.last)
                    .filter(|run| run.last_hour == frame.hour)
                    .map(|run| run.value)
            );
        }
    }
    assert_eq!(
        sample(&frames[0], Some(&[("absent".into(), "0".into())])).unwrap(),
        BTreeMap::from([(("absent".into(), "0".into()), (None, None))])
    );
    assert_eq!(
        sample(&frames[1], Some(&[(tag(0), "6".into())])).unwrap()[&(tag(0), "6".into())].1,
        None,
        "a missing land row remains unknown locally"
    );
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn army_progressive_queries_join_prepared_prefix_and_tail_then_zoom_exactly_without_writing() {
    let fixture = Fixture::new();
    let family = json!({"kind":"army-size", "measure":"strength", "countFor":"owner"});
    let params = |from| json!({"branchId":"1", "family":family, "tags":[tag(0),tag(1)], "from":from, "to":31, "width":32});
    let cold = fixture.query("series-summary", params(0));
    assert!(
        !cold["series"][0]["summary"]["approximate"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        cold["series"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["tag"] == tag(1))
    );
    let after_rename = fixture.query(
        "series-summary",
        json!({"branchId":"1", "family":family,
        "tags":[tag(0)], "from":26,"to":31,"width":6}),
    );
    assert!(
        after_rename["series"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["tag"] == "renamed / first"
                && !row["summary"]["runs"].as_array().unwrap().is_empty()),
        "central branch-start identity evidence keeps a cold zoom attached to its original holder"
    );
    assert!(fixture.store.saved_key().unwrap().is_none());
    let mut prepared = Prepared::default();
    for hour in 0..2 {
        prepared
            .observe(
                &frame(hour, 1),
                fixture.source.replay_ref(),
                &crate::prepared::frame_changes::FrameChangeTracker::new()
                    .observe(&frame(hour, 1), fixture.source.replay_ref().format),
            )
            .unwrap();
        let mut writer = if hour == 0 {
            fixture.store.replace(fixture.key.clone()).unwrap()
        } else {
            fixture.store.extend(&fixture.key).unwrap()
        };
        prepared
            .flush(&mut writer, 1, &AtomicBool::new(false))
            .unwrap();
        fixture.identities(&mut writer, hour);
        writer.commit().unwrap();
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    }
    let ranges = fixture
        .store
        .range_count(&fixture.key, "army-values-1-1")
        .unwrap();
    let rough = fixture.query("series-summary", params(0));
    assert_eq!(rough["coverage"]["lastHour"], 1);
    let exact = fixture.query("series-summary", params(6));
    assert!(
        exact["series"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["summary"]["approximate"] == json!([]))
    );
    let primary = exact["series"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["tag"] == tag(0))
        .unwrap();
    assert!(
        primary["summary"]["runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|run| run["firstHour"] == 9 && run["value"].as_f64() == Some(1.9))
    );
    let aliases = fixture.query(
        "series-summary",
        json!({"branchId":"1", "family":family, "tags":[tag(0)], "from":0,"to":31,"width":32}),
    );
    assert!(
        aliases["series"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["tag"] == "renamed / first")
    );
    assert!(
        !aliases["series"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["tag"] == tag(1))
    );
    let top = fixture.query("series-summary", json!({"branchId":"1", "family":{"top":1,"kind":"army-size","measure":"strength","countFor":"owner"}, "tags":[],"from":0,"to":31,"width":32}));
    assert!(!top["series"].as_array().unwrap().is_empty());
    let navigation = fixture.query(
        "series-next-change",
        json!({"branchId":"1", "family":family,
        "tags":[tag(0)],"key":"ArrowRight","from":8,"firstHour":6,"lastHour":31}),
    );
    assert_eq!(navigation["hour"].as_f64(), Some(9.0));
    let prefix_navigation = fixture.query(
        "series-next-change",
        json!({"branchId":"1", "family":family,
        "tags":[tag(0)],"key":"ArrowRight","from":0,"firstHour":0,"lastHour":1}),
    );
    assert_eq!(prefix_navigation["hour"].as_f64(), Some(1.0));
    let block = fixture.query("army-size", json!({"branchId":"1","tags":[tag(0)],"measure":"battalions","countFor":"owner","byType":true,"window":4}));
    assert!(
        block["series"][0]["mix"]
            .as_array()
            .unwrap()
            .iter()
            .any(|run| run["firstHour"] == 8)
    );
    assert_eq!(
        fixture
            .store
            .range_count(&fixture.key, "army-values-1-1")
            .unwrap(),
        ranges
    );
    for hour in 2..4 {
        prepared
            .observe(
                &frame(hour, 2),
                fixture.source.replay_ref(),
                &crate::prepared::frame_changes::FrameChangeTracker::new()
                    .observe(&frame(hour, 2), fixture.source.replay_ref().format),
            )
            .unwrap();
    }
    let mut writer = fixture.store.extend(&fixture.key).unwrap();
    prepared
        .flush(&mut writer, 1, &AtomicBool::new(false))
        .unwrap();
    fixture.identities(&mut writer, 3);
    writer.commit().unwrap();
    assert_eq!(
        fixture.query("series-summary", params(0))["coverage"]["lastHour"],
        3
    );
}

#[test]
fn phase_b_army_work_is_flat_over_hours_history_and_one_changed_row() {
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    use replay::{frame::TableView, rows::TableRows};
    let measure = |history: usize, hours: i32| -> io::Result<observer_tests::Work> {
        let mut first = frame(100, 128);
        for table in [
            TableId::DivisionCensus,
            TableId::Divisions,
            TableId::DivisionLandInputs,
        ] {
            match &mut *first.tables[table.index()] {
                TableView::Complete(TableRows::DivisionCensus(rows)) => rows.truncate(1),
                TableView::Complete(TableRows::Divisions(rows)) => rows.truncate(1),
                TableView::Complete(TableRows::DivisionLandInputs(rows)) => rows.truncate(1),
                _ => unreachable!(),
            }
        }
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first.clone()])])?;
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
        let work = observer_tests::unchanged_frames(
            state.state.work,
            &first,
            hours,
            replay.format,
            |frame, changes| {
                state.observe(frame, &replay, changes)?;
                Ok(state.state.work)
            },
        )?;
        assert_eq!(work, observer_tests::Work::default());
        first.hour += hours;
        tracker.observe(&first, replay.format);
        first.hour += 1;
        if let TableView::Complete(TableRows::Divisions(rows)) =
            &mut *first.tables[TableId::Divisions.index()]
        {
            rows[0].hit_points += 1;
        }
        let before = state.state.work;
        state.observe(&first, &replay, &tracker.observe(&first, replay.format))?;
        let after = state.state.work;
        let changed = observer_tests::Work {
            row_visits: after.row_visits - before.row_visits,
            history_visits: after.history_visits - before.history_visits,
            emitted: after.emitted - before.emitted,
            ..Default::default()
        };
        assert!(
            changed.row_visits <= 1 && changed.history_visits <= 1 && changed.emitted <= 10,
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
fn phase_b_army_deferred_native_runs_match_eager_after_checkpoints_resume_append_gaps_and_aliases()
{
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    use replay::{
        frame::{FrameDecoder, FrameEncoder, TableView},
        rows::TableRows,
    };
    let initial = frame(100, 3);
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![initial.clone()])]).unwrap();
    let mut lazy = Scan::default();
    let mut native = Scan::default();
    let mut lazy_state = scan_state();
    let mut native_state = scan_state();
    let mut tracker = FrameChangeTracker::new();
    let mut encoder = FrameEncoder::default();
    let mut decoder = FrameDecoder::default();
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
            *next.tables[TableId::Divisions.index()] = TableView::Unavailable {
                reason: "synthetic refusal".into(),
            };
        }
        let raw = encoder.encode(next.clone(), hour == 100).unwrap();
        let next = decoder.advance(&raw).unwrap().clone();
        native_state.step(&mut native, &raw, &next, &replay);
        let changes = tracker.observe(&next, replay.format);
        let raw = super::super::super::parts_race_prepared::updates_changed(
            &next,
            &[
                TableId::Divisions,
                TableId::DivisionTemplateIcons,
                TableId::DivisionLandInputs,
            ],
            Some(&changes),
            lazy_state.applied,
        );
        lazy_state.step_changes(&mut lazy, &raw, &next, &replay, Some(&changes));
        if hour == 103 || hour == 108 {
            lazy.seal();
            lazy = serde_json::from_slice(&serde_json::to_vec(&lazy).unwrap()).unwrap();
            lazy_state = scan_state();
            tracker.invalidate();
        }
    }
    lazy.seal();
    let projection = |scan: &Scan| {
        json!({"unreadable":scan.unreadable,"composition":scan.composition_unreadable,
        "errors":scan.invalid_table_reads,"tracks":scan.tracks.iter().map(|(tag, track)|
            (tag,json!({"runs":track.runs,"segments":track.segments,"mix":track.mix}))).collect::<BTreeMap<_,_>>()})
    };
    observer_tests::assert_reply_parity(&projection(&lazy), &projection(&native)).unwrap();
}
