use super::*;
use crate::prepared::series::tests::Fixture;

#[test]
fn projected_values_reuse_only_matching_pages_types_and_parameters() {
    let calls = std::cell::Cell::new(0);
    let read = |page, through| {
        reuse_value(
            vec![[page, 2, 3]],
            &through,
            || {
                calls.set(calls.get() + 1);
                Ok(vec![page as u8; through])
            },
        )
        .unwrap()
    };
    let scope = cache_scope(None);
    let first = read(1, 8);
    let held = scope.finish();
    let scope = cache_scope(Some(&held));
    assert!(Arc::ptr_eq(&first, &read(1, 8)));
    assert_eq!(calls.get(), 1);
    assert_eq!(*read(2, 8), vec![2; 8]);
    assert_eq!(*read(1, 4), vec![1; 4]);
    assert_eq!(calls.get(), 3);
    let current = scope.finish();
    assert_eq!(current.values.len(), 3);
    let scope = cache_scope(Some(&current));
    assert!(Arc::ptr_eq(&first, &read(1, 8)));
    assert_eq!(
        scope.finish().values.len(),
        1,
        "unused projections survived"
    );
}

fn amount(hour: i32) -> Option<i128> {
    (hour % 29 != 7).then_some(i128::from(hour / 9 - 4))
}
fn tail(first: i32) -> Vec<Run> {
    (first..128)
        .filter_map(|hour| amount(hour).map(|value| Run::new(hour, hour, value, hour % 17 == 0)))
        .collect()
}
fn append(fixture: &Fixture, values: &mut Accumulator, first: i32) {
    for hour in first..first + 32 {
        values.through(hour);
        values.observe(0, hour, amount(hour), hour % 17 == 0);
    }
    let mut writer = fixture.writer();
    values
        .flush(&mut writer, "test", 0, &AtomicBool::new(false))
        .unwrap();
    writer.commit().unwrap();
}
fn summary(
    fixture: &Fixture,
    view: (f64, f64, u32),
    tail: &[Run],
    within: Option<&[crate::products::country_groups::HourSpan]>,
) -> ChartSummary {
    let flag = AtomicBool::new(false);
    Reader::new(&fixture.store, &fixture.key, "test".into(), &flag)
        .unwrap()
        .summary_with_tail(0, (0, 127), view, 0, tail, within)
        .unwrap()
}
fn equal(actual: &ChartSummary, expected: &ChartSummary) {
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

#[test]
fn moving_prefixes_fold_only_new_buckets_and_membership_stays_independent() {
    let fixture = Fixture::new();
    let mut values = Accumulator::default();
    let view = (0.0, 127.0, 16);
    let mut held = SummaryCache::default();
    for first in [0, 32, 64, 96] {
        append(&fixture, &mut values, first);
        let tail = tail(first + 32);
        let before = BUCKET_FOLDS.get();
        let scope = cache_scope(Some(&held));
        let actual = summary(&fixture, view, &tail, None);
        assert_eq!(BUCKET_FOLDS.get() - before, 4, "re-read the settled prefix");
        // A different membership selection reuses only numeric buckets. Its
        // clipping and breaks still come from the new request.
        let spans = [crate::products::country_groups::HourSpan {
            first_hour: 5,
            last_hour: 83,
        }];
        let masked = summary(&fixture, view, &tail, Some(&spans));
        held = scope.finish();
        equal(&actual, &summary(&fixture, view, &tail, None));
        equal(&masked, &summary(&fixture, view, &tail, Some(&spans)));
    }
    assert_eq!(
        held.folds.len(),
        1,
        "only the current request's folds survive"
    );
}

#[test]
fn replacement_pages_invalidate_old_buckets_and_old_readers_keep_their_view() {
    let fixture = Fixture::new();
    let mut values = Accumulator::default();
    for first in [0, 32, 64, 96] {
        append(&fixture, &mut values, first);
    }
    let view = (0.0, 127.0, 16);
    let flag = AtomicBool::new(false);
    let scope = cache_scope(None);
    let original = summary(&fixture, view, &[], None);
    let held = scope.finish();
    let mut old = Reader::new(&fixture.store, &fixture.key, "test".into(), &flag).unwrap();
    let other = Fixture::new();
    let mut replacement = Accumulator::default();
    for hour in 0..32 {
        replacement.through(hour);
        replacement.observe(
            0,
            hour,
            amount(hour).map(|value| value - 101),
            hour % 17 == 0,
        );
    }
    let mut writer = other.writer();
    replacement.flush(&mut writer, "test", 0, &flag).unwrap();
    writer.commit().unwrap();
    let mut writer = fixture.writer();
    writer
        .write_at(
            "test",
            0,
            other.store.read(&other.key, "test", 0).unwrap().unwrap(),
        )
        .unwrap();
    writer.commit().unwrap();
    let scope = cache_scope(Some(&held));
    let before = BUCKET_FOLDS.get();
    let actual = summary(&fixture, view, &[], None);
    assert_eq!(BUCKET_FOLDS.get() - before, 16);
    assert_ne!(actual.buckets, original.buckets);
    drop(scope);
    equal(&actual, &summary(&fixture, view, &[], None));
    equal(&old.summary(0, (0, 127), view, 0).unwrap(), &original);
}

#[test]
fn a_new_grid_or_fine_zoom_preserves_the_uncached_geometry_and_run_bounds() {
    let fixture = Fixture::new();
    let mut values = Accumulator::default();
    for first in [0, 32] {
        append(&fixture, &mut values, first);
    }
    let sampled = tail(64);
    let mut held = SummaryCache::default();
    for view in [(0.0, 127.0, 16), (3.25, 99.75, 11), (0.0, 127.0, 800)] {
        let scope = cache_scope(Some(&held));
        let actual = summary(&fixture, view, &sampled, None);
        held = scope.finish();
        equal(&actual, &summary(&fixture, view, &sampled, None));
    }
}

#[test]
fn narrow_windows_do_not_scan_unrelated_page_identities() {
    let fixture = Fixture::new();
    let mut values = Accumulator::default();
    for first in [0, 32, 64, 96] {
        append(&fixture, &mut values, first);
    }
    let flag = AtomicBool::new(false);
    let scope = cache_scope(None);
    let mut reader = Reader::new(&fixture.store, &fixture.key, "test".into(), &flag).unwrap();
    let actual = reader.summary(0, (0, 127), (100.0, 127.0, 2), 0).unwrap();
    assert!(
        reader.publication.is_none(),
        "a narrow window scanned every range identity"
    );
    assert!(scope.finish().folds.is_empty());
    equal(&actual, &summary(&fixture, (100.0, 127.0, 2), &[], None));
}

#[test]
fn state_runs_and_events_reuse_unchanged_pages_and_replacements_miss() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut values = Accumulator::default();
    let mut held = SummaryCache::default();
    let read = || {
        let mut reader = Reader::new(&fixture.store, &fixture.key, "test".into(), &flag).unwrap();
        serde_json::json!({
            "values": reader.values_at(&[0], 31).unwrap(),
            "runs": reader.runs(0, 0, 127).unwrap(),
            "events": reader.events(&[0], 0, 127, 0, 100).unwrap().rows.into_iter()
                .map(|row| (row.hour, row.ordinal, row.row)).collect::<Vec<_>>(),
            "latest": reader.latest_event(0, 31).unwrap(),
        })
    };
    for first in [0, 32, 64] {
        for hour in first..first + 32 {
            values.through(hour);
            values.observe(0, hour, amount(hour), hour % 17 == 0);
            if hour % 8 == 0 {
                values
                    .event(
                        0,
                        hour,
                        &serde_json::json!({"native":hour, "value":amount(hour)}),
                    )
                    .unwrap();
            }
        }
        let mut writer = fixture.writer();
        values.flush(&mut writer, "test", 0, &flag).unwrap();
        writer.commit().unwrap();
        let before = BLOB_DECODES.get();
        let scope = cache_scope(Some(&held));
        let actual = read();
        assert_eq!(
            BLOB_DECODES.get() - before,
            1,
            "decoded a settled batch again"
        );
        held = scope.finish();
        assert_eq!(
            actual,
            read(),
            "reuse changed the published values or event order"
        );
    }
    let other = Fixture::new();
    let mut replacement = Accumulator::default();
    for hour in 0..32 {
        replacement.through(hour);
        replacement.observe(0, hour, Some(999), false);
        if hour % 8 == 0 {
            replacement
                .event(0, hour, &serde_json::json!({"replacement":hour}))
                .unwrap();
        }
    }
    let mut writer = other.writer();
    replacement.flush(&mut writer, "test", 0, &flag).unwrap();
    writer.commit().unwrap();
    let original = read();
    let mut writer = fixture.writer();
    writer
        .write_at(
            "test",
            0,
            other.store.read(&other.key, "test", 0).unwrap().unwrap(),
        )
        .unwrap();
    writer.commit().unwrap();
    let before = BLOB_DECODES.get();
    let scope = cache_scope(Some(&held));
    let actual = read();
    assert_eq!(
        BLOB_DECODES.get() - before,
        1,
        "a replaced page reused old decoded bytes"
    );
    assert_ne!(actual, original);
    drop(scope);
    assert_eq!(actual, read());
}

#[test]
fn catalogue_and_reply_text_reuse_their_complete_page_identities() {
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let keys = BTreeMap::from([("synthetic".to_owned(), 7)]);
    write_catalogue(&mut writer, "test", 1, &keys, &vec!["one"]).unwrap();
    write_reply(&mut writer, "reply", 0, &serde_json::json!({"value":1})).unwrap();
    writer.commit().unwrap();
    let read = || {
        (
            read_catalogue::<Vec<String>>(&fixture.store, &fixture.key, "test", 1).unwrap(),
            read_reply::<Value>(&fixture.store, &fixture.key, "reply", 0).unwrap(),
        )
    };
    let scope = cache_scope(None);
    let first = read();
    let held = scope.finish();
    assert_eq!(held.texts.len(), 2);
    let scope = cache_scope(Some(&held));
    assert_eq!(read(), first);
    assert!(
        read_reply::<Value>(
            &fixture.store,
            &fixture.key,
            &catalogue_section("test", 1),
            0
        )
        .is_err(),
        "a cached catalogue must not turn an invalid reply encoding into a valid reply"
    );
    let again = scope.finish();
    for (id, text) in &held.texts {
        assert!(Arc::ptr_eq(text, &again.texts[id]));
    }
    let mut writer = fixture.writer();
    write_catalogue(&mut writer, "test", 1, &keys, &vec!["two"]).unwrap();
    write_reply(&mut writer, "reply", 0, &serde_json::json!({"value":2})).unwrap();
    writer.commit().unwrap();
    let scope = cache_scope(Some(&again));
    let replaced = read();
    assert_ne!(replaced, first);
    let latest = scope.finish();
    assert_eq!(
        latest.texts.len(),
        2,
        "unused page versions survived the current request"
    );
    assert_eq!(read(), replaced);
}

#[test]
fn a_summary_read_holds_the_summaries_it_decodes_in_its_own_scope() {
    let fixture = Fixture::new();
    let mut values = Accumulator::default();
    for first in [0, 32, 64, 96] {
        append(&fixture, &mut values, first);
    }
    let flag = AtomicBool::new(false);
    let scope = cache_scope(None);
    Reader::new(&fixture.store, &fixture.key, "test".into(), &flag)
        .unwrap()
        .runs(0, 0, 127)
        .unwrap();
    let runs = scope.finish();
    assert!(!runs.blobs.is_empty());
    assert!(
        runs.summaries.is_empty(),
        "a runs read decodes no summaries"
    );
    let held = runs.held();
    // Reusing those blobs, a summary read keeps what it decodes with its own reply.
    let scope = cache_scope(Some(&runs));
    summary(&fixture, (0.0, 127.0, 4), &[], None);
    assert!(!scope.finish().summaries.is_empty());
    assert_eq!(runs.held(), held);
}
