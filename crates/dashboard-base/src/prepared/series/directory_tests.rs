//! A reader looking up many keys reads a batch's directory once, not per key.
use super::tests::Fixture;
use super::*;
use serde_json::Value;
use serde_json::json;

thread_local! {
    /// Small positioned series reads (headers and directory entries) and
    /// whole-directory reads on this thread.
    pub(crate) static SMALL_READS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

const KEYS: u64 = 5000;
/// First seen in the last batch only, so earlier directories lack it.
const LATE_KEY: u64 = 5000;
/// Never written.
const MISSING_KEY: u64 = 9999;

/// Three batches of events: every key in the first, even keys in the second,
/// every fourth key and the late key in the third.
fn three_batches() -> Fixture {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut values = Accumulator::default();
    let mut writer = fixture.writer();
    for batch in 0..3u64 {
        let base = batch as i32 * 100;
        for hour in base..base + 100 {
            for key in (0..KEYS).filter(|key| *key % 100 == (hour - base) as u64) {
                if key % (1 << batch) == 0 {
                    values
                        .event(key, hour, &json!({"key": key, "batch": batch}))
                        .unwrap();
                }
            }
            if batch == 2 && hour == 250 {
                values
                    .event(LATE_KEY, hour, &json!({"late": true}))
                    .unwrap();
            }
        }
        values.flush(&mut writer, "events", 0, &flag).unwrap();
    }
    writer.commit().unwrap();
    fixture
}

fn keys() -> Vec<u64> {
    (0..KEYS).chain([LATE_KEY, MISSING_KEY]).collect()
}

fn fields(dir: Directory) -> (u64, u64, u64, u64, u64, u64, i32) {
    (
        dir.blob.0,
        dir.blob.1,
        dir.run_rows,
        dir.run_count,
        dir.event_base,
        dir.event_rows,
        dir.last_end,
    )
}

#[test]
fn whole_directory_entries_equal_single_key_searches() {
    let fixture = three_batches();
    for batch in 0..3 {
        let mut file = fixture
            .store
            .read(&fixture.key, "events", batch)
            .unwrap()
            .unwrap();
        let table = Table::read(&mut file).unwrap().unwrap();
        for key in keys().into_iter().chain([KEYS, KEYS + 1, u64::MAX]) {
            assert_eq!(
                fields(table.entry(key).unwrap()),
                fields(directory(&mut file, key).unwrap()),
                "batch {batch} key {key}"
            );
        }
    }
}

#[test]
fn latest_rows_of_many_keys_read_each_directory_once_and_equal_single_key_reads() {
    let fixture = three_batches();
    let flag = AtomicBool::new(false);
    let keys = keys();
    for hour in [299, 250, 199, 150, 50, 0, -1] {
        let mut batched =
            Reader::new(&fixture.store, &fixture.key, "events".into(), &flag).unwrap();
        let before = SMALL_READS.get();
        let rows = batched.latest_event_rows(&keys, hour).unwrap();
        let reads = SMALL_READS.get() - before;
        // One search per key costs about log2(keys) + 2 small reads (57,000
        // here); each batch's directory is searched at most 16 times before it
        // is read whole.
        assert!(reads <= 1_000, "{reads} directory reads at hour {hour}");
        let mut single = Reader::new(&fixture.store, &fixture.key, "events".into(), &flag).unwrap();
        for (&key, row) in keys.iter().zip(rows) {
            let expected = single.latest_value(key, hour).unwrap();
            let actual =
                row.map(|(date, bytes)| (date, serde_json::from_slice::<Value>(&bytes).unwrap()));
            assert_eq!(actual, expected, "key {key} at hour {hour}");
        }
    }
}

#[test]
fn a_few_lookups_keep_single_key_searches() {
    let fixture = three_batches();
    let flag = AtomicBool::new(false);
    let mut reader = Reader::new(&fixture.store, &fixture.key, "events".into(), &flag).unwrap();
    for key in 0..8 {
        reader.latest_event(key, 299).unwrap();
    }
    assert!(
        reader.tables.is_empty(),
        "eight lookups read a whole directory"
    );
    reader.latest_event_rows(&keys(), 299).unwrap();
    assert!(
        reader.tables.contains_key(&2),
        "thousands of lookups kept searching the hour's batch key by key"
    );
}

#[test]
fn query_directories_reuse_page_identities_and_drop_unused_or_replaced_pages() {
    let fixture = three_batches();
    let flag = AtomicBool::new(false);
    let selected = keys();
    let read = || {
        Reader::new(&fixture.store, &fixture.key, "events".into(), &flag)
            .unwrap()
            .latest_event_rows(&selected, 299)
            .unwrap()
    };
    let scope = cache_scope(None);
    let expected = read();
    let held = scope.finish();
    let scope = cache_scope(Some(&held));
    let before = SMALL_READS.get();
    assert_eq!(read(), expected);
    assert!(
        SMALL_READS.get() - before <= 6,
        "unchanged directories were searched or read again"
    );
    let held = scope.finish();
    assert!(held.held() > 0);

    // The same ordinal now has different bytes and fewer keys. Neither its
    // old directory nor its decoded events may survive the replacement.
    let other = Fixture::new();
    let mut values = Accumulator::default();
    let mut writer = other.writer();
    for batch in 0..3 {
        for key in 0..5 {
            values
                .event(key, batch * 100, &json!({"replacement": key}))
                .unwrap();
        }
        values.through(batch * 100 + 99);
        values.flush(&mut writer, "events", 0, &flag).unwrap();
    }
    writer.commit().unwrap();
    let mut writer = fixture.writer();
    for batch in 0..3 {
        writer
            .write_at(
                "events",
                batch,
                other
                    .store
                    .read(&other.key, "events", batch)
                    .unwrap()
                    .unwrap(),
            )
            .unwrap();
    }
    writer.commit().unwrap();
    let uncached = read();
    let scope = cache_scope(Some(&held));
    assert_eq!(read(), uncached);
    let current = scope.finish();
    assert!(current.held() > 0);
    let scope = cache_scope(Some(&current));
    assert_eq!(scope.finish().held(), 0, "unused directories survived");
}
