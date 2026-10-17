use super::*;
use crate::prepared::series::tests::Fixture;

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
    assert_eq!(held.0.len(), 1, "only the current request's folds survive");
    assert!(held.bytes() > 0);
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
    assert!(scope.finish().0.is_empty());
    equal(&actual, &summary(&fixture, (100.0, 127.0, 2), &[], None));
}
