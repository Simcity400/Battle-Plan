use super::*;
use replay_data::{rows::TableRows, rows::WeatherProvinceRow};
use std::cell::Cell;
use std::time::Duration;

fn frame(hour: i32, weather: &SharedView, other: &SharedView) -> DecodedFrame {
    let mut tables: [SharedView; TABLE_COUNT] = std::array::from_fn(|_| {
        SharedView::from(TableView::Unavailable {
            reason: "not recorded".into(),
        })
    });
    tables[TableId::WeatherProvinces.index()] = weather.clone();
    tables[TableId::Countries.index()] = other.clone();
    DecodedFrame {
        branch: 1,
        hour,
        tables,
        metrics: Metrics::default(),
    }
}

fn weather(rows: usize) -> SharedView {
    SharedView::from(TableView::Complete(TableRows::WeatherProvinces(
        (0..rows)
            .map(|id| WeatherProvinceRow {
                id: id as u32,
                ..Default::default()
            })
            .collect(),
    )))
}

#[test]
fn a_keyed_layer_builds_once_while_its_tables_are_shared() {
    let layer = Keyed::<usize>::new(&[TableId::WeatherProvinces]);
    let shared = weather(3);
    let (a, b) = (weather(1), weather(2));
    let builds = Cell::new(0);
    let get = |frame: &DecodedFrame| {
        layer
            .get_or_build(frame, (), |projected| {
                builds.set(builds.get() + 1);
                let rows = match &*projected.tables[TableId::WeatherProvinces.index()] {
                    TableView::Complete(rows) => rows.len(),
                    _ => 0,
                };
                Ok::<_, ()>(Arc::new(rows))
            })
            .unwrap()
    };
    let first = get(&frame(10, &shared, &a));
    // Another table changed, the keyed one did not: the same value.
    let second = get(&frame(11, &shared, &b));
    assert!(Arc::ptr_eq(&first, &second));
    // Equal rows in a new allocation are a miss, not a guess.
    let third = get(&frame(12, &weather(3), &b));
    assert!(!Arc::ptr_eq(&first, &third));
    assert_eq!(*third, 3);
    assert_eq!(builds.get(), 2);
}

#[test]
fn the_extra_key_separates_values_of_the_same_tables() {
    let layer = Keyed::<i32, i32>::new(&[TableId::WeatherProvinces]);
    let shared = weather(1);
    let at = |hour: i32| {
        layer
            .get_or_build(&frame(hour, &shared, &shared), hour, |projected| {
                Ok::<_, ()>(Arc::new(projected.hour))
            })
            .unwrap()
    };
    assert_eq!(*at(7), 7);
    assert_eq!(*at(8), 8);
    assert!(Arc::ptr_eq(&at(7), &at(7)));
}

#[test]
fn a_projected_frame_holds_only_the_listed_tables() {
    let shared = weather(2);
    let mut full = frame(5, &shared, &weather(1));
    full.metrics.total_ns = 9;
    let projected = project(&full, &[TableId::WeatherProvinces]);
    assert_eq!((projected.branch, projected.hour), (1, 5));
    assert_eq!(projected.metrics, Metrics::default());
    assert!(SharedView::ptr_eq(
        &projected.tables[TableId::WeatherProvinces.index()],
        &shared
    ));
    assert!(matches!(
        &*projected.tables[TableId::Countries.index()],
        TableView::Unavailable { reason } if reason == NOT_AN_INPUT
    ));
}

#[test]
fn a_settled_layer_keeps_one_value_while_its_content_is_unchanged() {
    let layer = Settled::<usize>::new(&[TableId::WeatherProvinces]);
    let other = weather(1);
    let builds = Cell::new(0);
    let get = |weather: &SharedView| {
        layer
            .get_or_build(&frame(10, weather, &other), |projected| {
                builds.set(builds.get() + 1);
                // The unlisted table is not visible to the build.
                assert!(matches!(
                    &*projected.tables[TableId::Countries.index()],
                    TableView::Unavailable { reason } if reason == NOT_AN_INPUT
                ));
                match &*projected.tables[TableId::WeatherProvinces.index()] {
                    TableView::Complete(rows) => Ok::<_, ()>(rows.len()),
                    _ => Err(()),
                }
            })
            .unwrap()
    };
    let shared = weather(3);
    let first = get(&shared);
    // The same allocation hits without building.
    assert!(Arc::ptr_eq(&first, &get(&shared)));
    assert_eq!(builds.get(), 1);
    // A new allocation builds, and settles on the equal value.
    let equal = weather(3);
    assert!(Arc::ptr_eq(&first, &get(&equal)));
    assert_eq!(builds.get(), 2);
    // A changed value is a new one.
    let changed = get(&weather(2));
    assert_eq!(*changed, 2);
    assert!(!Arc::ptr_eq(&first, &changed));
    // Each allocation got its own entry: both earlier ones still hit.
    assert!(Arc::ptr_eq(&first, &get(&shared)));
    assert!(Arc::ptr_eq(&first, &get(&equal)));
    assert_eq!(builds.get(), 3);
}

/// A settled layer of `WeatherProvinces` row counts, and how many builds
/// it made.
fn counting() -> (Settled<usize>, std::sync::atomic::AtomicU32) {
    (
        Settled::new(&[TableId::WeatherProvinces]),
        std::sync::atomic::AtomicU32::new(0),
    )
}

fn settle(
    layer: &Settled<usize>,
    builds: &std::sync::atomic::AtomicU32,
    weather: &SharedView,
) -> Arc<usize> {
    let other = self::weather(0);
    layer
        .get_or_build(&frame(1, weather, &other), |projected| {
            builds.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            match &*projected.tables[TableId::WeatherProvinces.index()] {
                TableView::Complete(rows) => Ok::<_, ()>(rows.len()),
                _ => Err(()),
            }
        })
        .unwrap()
}

#[test]
fn a_settled_layer_settles_back_onto_an_older_value() {
    let (layer, builds) = counting();
    let three = settle(&layer, &builds, &weather(3));
    let two = settle(&layer, &builds, &weather(2));
    assert!(!Arc::ptr_eq(&three, &two));
    // The latest value is two rows; a new allocation of three rows still
    // settles on the older value.
    assert!(Arc::ptr_eq(&three, &settle(&layer, &builds, &weather(3))));
    assert_eq!(builds.load(std::sync::atomic::Ordering::SeqCst), 3);
}

#[test]
fn a_settled_layer_keeps_the_latest_value_and_those_held() {
    let (layer, builds) = counting();
    let count = || builds.load(std::sync::atomic::Ordering::SeqCst);
    let (a, b, c) = (weather(1), weather(2), weather(3));
    let held = settle(&layer, &builds, &a);
    settle(&layer, &builds, &b);
    settle(&layer, &builds, &c);
    assert_eq!(count(), 3);
    // `c`'s value is the latest: it hits though nothing else holds it.
    settle(&layer, &builds, &c);
    assert_eq!(count(), 3);
    // `a`'s value is held: it hits.
    assert!(Arc::ptr_eq(&held, &settle(&layer, &builds, &a)));
    assert_eq!(count(), 3);
    // Nothing holds `b`'s value: it builds again.
    settle(&layer, &builds, &b);
    assert_eq!(count(), 4);
}

// regression: 9501a84f48 Build Settled on Recent so one hour's tables build once
#[test]
fn concurrent_callers_of_one_settled_hour_share_one_build() {
    let (layer, builds) = counting();
    let (shared, other) = (weather(3), weather(0));
    let started = std::sync::Barrier::new(4);
    let values: Vec<_> = std::thread::scope(|scope| {
        let threads: Vec<_> = (0..4)
            .map(|_| {
                scope.spawn(|| {
                    started.wait();
                    // The build outlasts the other callers' arrival, so
                    // they wait on it rather than hit a finished value.
                    layer
                        .get_or_build(&frame(1, &shared, &other), |_| {
                            builds.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            std::thread::sleep(Duration::from_millis(50));
                            Ok::<_, ()>(3)
                        })
                        .unwrap()
                })
            })
            .collect();
        threads.into_iter().map(|t| t.join().unwrap()).collect()
    });
    assert_eq!(builds.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(values.iter().all(|value| Arc::ptr_eq(value, &values[0])));
}

#[test]
fn distinct_values_hold_each_allocation_once_most_recent_first() {
    let (layer, builds) = counting();
    let one = settle(&layer, &builds, &weather(1));
    let two = settle(&layer, &builds, &weather(2));
    // A second allocation of one row is a second entry for the same value.
    settle(&layer, &builds, &weather(1));
    let values = layer.recent.distinct_values();
    assert_eq!(values.len(), 2);
    assert!(Arc::ptr_eq(&values[0], &one));
    assert!(Arc::ptr_eq(&values[1], &two));
}

#[test]
fn a_settled_layer_does_not_keep_failed_builds() {
    let layer = Settled::<usize>::new(&[TableId::WeatherProvinces]);
    let shared = weather(1);
    let other = weather(1);
    let hour = frame(1, &shared, &other);
    assert!(
        layer
            .get_or_build(&hour, |_| Err::<usize, _>("failed"))
            .is_err()
    );
    assert_eq!(*layer.get_or_build(&hour, |_| Ok::<_, ()>(5)).unwrap(), 5);
}

#[test]
fn recent_values_live_while_held_and_failures_are_not_kept() {
    let recent = Recent::<u32, u32>::new();
    for key in 0..3 {
        recent
            .get_or_insert_with(key, || Ok::<_, ()>(Arc::new(key)))
            .unwrap();
    }
    // Only key 2, the latest, is held: key 0 builds again.
    let mut built = false;
    recent
        .get_or_insert_with(0, || {
            built = true;
            Ok::<_, ()>(Arc::new(0))
        })
        .unwrap();
    assert!(built);
    assert!(
        recent
            .get_or_insert_with(9, || Err::<Arc<u32>, _>("failed"))
            .is_err()
    );
    let mut rebuilt = false;
    recent
        .get_or_insert_with(9, || {
            rebuilt = true;
            Ok::<_, ()>(Arc::new(9))
        })
        .unwrap();
    assert!(rebuilt);
}

#[test]
fn concurrent_callers_of_one_key_share_one_build() {
    let recent = Arc::new(Recent::<u32, u32>::new());
    let builds = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let started = Arc::new(std::sync::Barrier::new(4));
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let (recent, builds, started) = (
                Arc::clone(&recent),
                Arc::clone(&builds),
                Arc::clone(&started),
            );
            std::thread::spawn(move || {
                started.wait();
                recent
                    .get_or_insert_with(1, || {
                        builds.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(50));
                        Ok::<_, ()>(Arc::new(42))
                    })
                    .unwrap()
            })
        })
        .collect();
    let values: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(builds.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(values.iter().all(|value| Arc::ptr_eq(value, &values[0])));
}

/// A caller that never waits, asking for a key another caller is building (here its own build,
/// where a caller that waited would wait forever), builds its own copy; the first build is kept.
#[test]
fn a_caller_that_never_waits_builds_its_own_copy_while_another_builds_the_key() {
    let recent = Recent::<u32, u32>::new();
    let first = recent
        .get_or_insert_without_waiting(1, || {
            let own = recent
                .get_or_insert_without_waiting(1, || Ok::<_, ()>(Arc::new(7)))
                .unwrap();
            assert_eq!(*own, 7, "built for itself");
            Ok::<_, ()>(Arc::new(42))
        })
        .unwrap();
    assert_eq!(*first, 42);
    let again = recent
        .get_or_insert_without_waiting(1, || Ok::<_, ()>(Arc::new(0)))
        .unwrap();
    assert!(Arc::ptr_eq(&first, &again), "the first build is the one kept");
}
