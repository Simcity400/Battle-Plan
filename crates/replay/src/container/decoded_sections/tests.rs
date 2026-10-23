use super::*;
use crate::container::{
    grouped::read_selected_updates,
    tests::{CAMPAIGN, records},
};
use crate::rows::{CountryRow, TableRows};

fn key(block: usize) -> Key {
    Key {
        format: 1,
        branch: 1,
        first: block as i32,
        last: block as i32,
        offset: block as u64,
        length: 1,
        raw_length: 1,
        table: 0,
        block,
    }
}

fn at(block: usize, table: usize) -> Key {
    Key {
        table,
        ..key(block)
    }
}

fn rows(countries: usize) -> Vec<TableUpdate> {
    vec![TableUpdate::Snapshot(TableRows::Countries(
        (0..countries)
            .map(|id| CountryRow {
                id: id as u32,
                ..Default::default()
            })
            .collect(),
    ))]
}

fn put(cache: &Cache, block: usize, table: usize, countries: usize) {
    cache
        .read(at(block, table), 1, false, || Ok(rows(countries)))
        .unwrap();
}

fn reader(
    cache: &Cache,
    next: usize,
    tables: &[usize],
    last: Option<usize>,
    name: &'static str,
) -> SectionConsumer {
    let mut read = Box::new([false; TABLE_COUNT]);
    for &table in tables {
        read[table] = true;
    }
    cache.register(Frontier {
        next,
        ahead: last.unwrap_or(next),
        last,
        tables: Some(read),
        name,
    })
}

fn assert_consistent(cache: &Cache) {
    let state = cache.lock();
    let (mut bytes, mut obsolete_bytes) = (0, 0);
    let (mut lru, mut obsolete, mut retained) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    let mut counts = BTreeMap::new();
    for (key, entry) in &state.entries {
        let Some(value) = &entry.value else {
            assert!(entry.reading, "only a read under way keeps an empty entry");
            continue;
        };
        bytes += value.bytes;
        retained.insert((key.table, key.block, *key));
        assert_eq!(
            entry.obsolete,
            !state.needed(key.table, key.block),
            "the need of table {} block {}",
            key.table,
            key.block
        );
        if entry.obsolete {
            obsolete_bytes += value.bytes;
            obsolete.insert((entry.used, *key));
            *counts.entry(key.table).or_insert(0) += 1;
        } else {
            lru.insert((entry.used, *key));
        }
    }
    assert_eq!(state.bytes, bytes);
    assert_eq!(state.obsolete_bytes, obsolete_bytes);
    assert!(
        state.lru == lru,
        "the needed order holds every needed column"
    );
    assert!(
        state.obsolete == obsolete,
        "the obsolete order holds the rest"
    );
    assert!(state.retained == retained, "the index holds every column");
    let tables: BTreeMap<usize, usize> = state
        .obsolete_tables
        .iter()
        .filter(|&(_, &count)| count != 0)
        .map(|(&table, &count)| (table, count))
        .collect();
    assert_eq!(tables, counts);
    assert_eq!(state.allowance, state.largest.values().sum::<usize>());
}

fn kept(cache: &Cache) -> Vec<(usize, usize)> {
    let mut kept: Vec<(usize, usize)> = cache
        .lock()
        .entries
        .iter()
        .filter(|(_, entry)| entry.value.is_some())
        .map(|(key, _)| (key.table, key.block))
        .collect();
    kept.sort_unstable();
    kept
}

#[test]
fn ready_columns_share_prefixes_and_keep_the_cache_lru_consistent() {
    let cache = Cache::default();
    let mut column = rows(2);
    column.push(TableUpdate::Unchanged { baseline: 0 });
    let column = Updates::new(column).unwrap();
    assert!(cache.ready(key(0), 1, true).is_none());
    cache.enable(true);
    let _reader = cache.consumer(0);
    cache
        .read_updates(key(0), 1, true, || Ok(column.prefix(1)))
        .unwrap();
    assert!(cache.ready(key(0), 1, false).is_none());
    assert!(cache.ready(key(0), 2, true).is_none());
    let keyframe = cache.ready(key(0), 1, true).unwrap();
    assert!(Arc::ptr_eq(keyframe.get(0), column.get(0)));
    cache
        .read_updates(key(0), 2, false, || Ok(column.clone()))
        .unwrap();
    let previous = cache.lock().clock;
    let prefix = cache.ready(key(0), 1, false).unwrap();
    assert_eq!(prefix.len(), 1);
    assert!(Arc::ptr_eq(prefix.get(0), column.get(0)));
    let state = cache.lock();
    assert_eq!(state.clock, previous + 1);
    assert_eq!(state.lru.len(), 1);
    assert!(state.lru.contains(&(state.clock, key(0))));
    assert_eq!(state.entries[&key(0)].used, state.clock);
    drop(state);
    let cached = cache
        .read_updates(key(0), 2, false, || panic!("a ready column must be reused"))
        .unwrap();
    assert!(Arc::ptr_eq(cached.get(1), column.get(1)));
    cache.enable(false);
    assert!(cache.ready(key(0), 1, false).is_none());
}

#[test]
fn measured_country_sizes_and_live_block_span_set_the_budget() {
    let mut previous = 0;
    for countries in [1, 1_500, 5_000] {
        let cache = Cache::default();
        cache.enable(true);
        let behind = cache.consumer(0);
        let ahead = cache.consumer(4);
        // The reader behind schedules every block to the one ahead.
        behind.reading(4);
        for block in 0..5 {
            cache
                .read(key(block), 1, false, || Ok(rows(countries)))
                .unwrap();
        }
        let largest = cache.lock().largest[&0];
        assert!(largest > previous, "country growth must grow the bound");
        previous = largest;
        assert_eq!(cache.lock().budget(), largest * 5);
        assert_eq!(cache.lock().bytes, largest * 5);
        behind.at(3);
        assert_eq!(cache.lock().budget(), largest * 2);
        assert_eq!(
            cache.lock().entries.len(),
            2,
            "retire LRU entries as the live span shrinks"
        );
        drop(ahead);
        behind.at(5);
        assert_eq!(cache.lock().entries.len(), 1);
        assert_consistent(&cache);
        cache.enable(false);
        assert!(cache.lock().entries.is_empty());
        assert_consistent(&cache);
    }
}

#[test]
fn finished_query_releases_its_read_ahead_span_while_preparation_stays_behind() {
    let cache = Cache::default();
    cache.enable(true);
    let preparation = cache.consumer(0);
    let query = cache.consumer(2);
    query.reading(7);
    for block in 0..8 {
        cache
            .read(key(block), 1, false, || Ok(rows(1_500)))
            .unwrap();
    }
    let largest = cache.lock().largest[&0];
    assert_eq!(cache.lock().budget(), largest * 8);
    assert_eq!(cache.lock().entries.len(), 8);
    drop(query);
    assert_eq!(cache.lock().budget(), largest);
    assert_eq!(cache.lock().entries.len(), 1);
    preparation.reading(2);
    assert_eq!(cache.lock().budget(), largest * 3);
    preparation.at(2);
    assert_eq!(cache.lock().budget(), largest);
    assert_consistent(&cache);
}

#[test]
fn finished_query_cannot_evict_columns_still_in_the_preparation_span() {
    for countries in [128, 1_500, 5_000] {
        let cache = Cache::default();
        cache.enable(true);
        let preparation = cache.consumer(0);
        preparation.reading(1);
        let query = cache.consumer(4);
        let mut decoded = 0;
        for block in [0, 1, 4] {
            cache
                .read(key(block), 1, false, || {
                    decoded += 1;
                    Ok(rows(countries))
                })
                .unwrap();
        }
        drop(query);
        for block in [0, 1] {
            cache
                .read(key(block), 1, false, || {
                    decoded += 1;
                    Ok(rows(countries))
                })
                .unwrap();
        }
        assert_eq!(
            decoded, 3,
            "live columns survive a finished query at {countries} countries"
        );
        let state = cache.lock();
        assert_eq!(state.entries.len(), 2);
        assert_eq!(state.bytes, state.budget());
    }
}

#[test]
fn finishing_a_663_block_query_retires_its_columns_without_rescanning_the_rest() {
    let tables = TableId::ALL.len();
    let cache = Cache::default();
    cache.enable(true);
    let preparation = cache.consumer(0);
    preparation.reading(330);
    let query = cache.consumer(331);
    query.reading(662);
    for block in 0..663 {
        for table in 0..tables {
            cache
                .read(at(block, table), 1, false, || {
                    Ok(vec![TableUpdate::Unavailable {
                        reason: "synthetic column".into(),
                    }])
                })
                .unwrap();
        }
    }
    let before = {
        let state = cache.lock();
        assert_eq!(state.entries.len(), 663 * tables);
        state.inspections
    };
    drop(query);
    {
        let state = cache.lock();
        assert_eq!(
            state.inspections - before,
            2 * 332 * tables,
            "each of the query's columns reclassified once and evicted once"
        );
        assert_eq!(state.entries.len(), 331 * tables);
        assert!(state.entries.keys().all(|key| key.block <= 330));
        assert_eq!(state.bytes, state.budget());
    }
    // Disable uses the existing needed-order sweep before its LRU fallback.
    let before = cache.lock().inspections;
    cache.enable(false);
    let state = cache.lock();
    assert_eq!(state.inspections - before, 2 * 331 * tables);
    assert!(state.entries.is_empty());
    assert!(state.lru.is_empty() && state.obsolete.is_empty());
    assert_eq!(state.bytes, 0);
}

#[test]
fn hits_share_cells_and_keyframe_admission_never_supplies_a_table_read() {
    let cache = Cache::default();
    cache.enable(true);
    let first = cache.read(key(0), 1, true, || Ok(rows(2))).unwrap();
    let second = cache
        .read(key(0), 1, true, || panic!("keyframe decoded twice"))
        .unwrap();
    assert!(Arc::ptr_eq(first.get(0), second.get(0)));
    let mut called = false;
    cache
        .read(key(0), 1, false, || {
            called = true;
            Ok(rows(2))
        })
        .unwrap();
    assert!(
        called,
        "a table read must admit its later section even in a one-frame block"
    );
    assert_consistent(&cache);
}

#[test]
fn failed_and_panicked_extensions_preserve_the_earlier_prefix_and_release_the_slot() {
    let cache = Cache::default();
    cache.enable(true);
    let first = cache.read(key(0), 1, false, || Ok(rows(2))).unwrap();
    assert!(
        cache
            .read(key(0), 2, false, || Err(io::Error::other(
                "synthetic refusal"
            )))
            .is_err()
    );
    assert!(
        std::panic::catch_unwind(|| {
            let _ = cache.read(key(0), 2, false, || panic!("synthetic panic"));
        })
        .is_err()
    );
    let second = cache
        .read(key(0), 1, false, || panic!("prefix lost"))
        .unwrap();
    assert!(Arc::ptr_eq(first.get(0), second.get(0)));
    assert!(!cache.lock().entries[&key(0)].reading);
    assert_consistent(&cache);
}

#[test]
fn columns_retire_once_every_reader_of_their_table_has_passed() {
    for countries in [1_500, 5_000] {
        let cache = Cache::default();
        cache.enable(true);
        let every = cache.consumer(0);
        let narrow = reader(&cache, 0, &[0], None, "narrow");
        let other = reader(&cache, 0, &[1], None, "other");
        // Schedule every fixture block so the span budget does not constrain retention.
        every.reading(9);
        narrow.reading(9);
        other.reading(9);
        for block in 0..5 {
            put(&cache, block, 0, countries);
            put(&cache, block, 1, countries);
        }
        narrow.at(3);
        assert_eq!(
            cache.lock().obsolete_bytes,
            0,
            "the undeclared reader reads them"
        );
        assert_consistent(&cache);
        drop(every);
        let largest = cache.lock().largest[&0];
        assert_consistent(&cache);
        assert_eq!(
            kept(&cache),
            [
                (0, 1),
                (0, 2),
                (0, 3),
                (0, 4),
                (1, 0),
                (1, 1),
                (1, 2),
                (1, 3),
                (1, 4)
            ],
            "table 0's passed blocks retire past one block's set: two tables' largest"
        );
        assert_eq!(cache.lock().obsolete_bytes, 2 * largest);
        drop(narrow);
        assert_consistent(&cache);
        assert_eq!(
            kept(&cache),
            [(0, 3), (0, 4), (1, 0), (1, 1), (1, 2), (1, 3), (1, 4)]
        );
        drop(other);
        assert_consistent(&cache);
        assert_eq!(kept(&cache), [(0, 4), (1, 4)], "no reader: one block's set");
    }
}

#[test]
fn a_reader_moving_back_needs_its_columns_again_and_reads_evicted_ones_afresh() {
    let cache = Cache::default();
    cache.enable(true);
    let narrow = reader(&cache, 0, &[0], None, "narrow");
    narrow.reading(9);
    for block in 0..4 {
        put(&cache, block, 0, 1_500);
    }
    narrow.at(3);
    assert_consistent(&cache);
    assert_eq!(
        kept(&cache),
        [(0, 2), (0, 3)],
        "one table's largest is kept"
    );
    narrow.at(1);
    assert_consistent(&cache);
    assert!(!cache.lock().entries[&at(2, 0)].obsolete);
    let mut decoded = Vec::new();
    for block in 1..4 {
        cache
            .read(at(block, 0), 1, false, || {
                decoded.push(block);
                Ok(rows(1_500))
            })
            .unwrap();
    }
    assert_eq!(decoded, [1], "only the evicted block is decoded again");
    assert_consistent(&cache);
}

#[test]
fn advancing_readers_reclassify_only_the_blocks_they_pass() {
    let blocks = 663;
    let cache = Cache::default();
    cache.enable(true);
    let walk = reader(&cache, 0, &[0, 1], None, "walk");
    walk.reading(blocks);
    let query = cache.consumer(blocks);
    let start = cache.lock().inspections;
    for block in 0..blocks {
        put(&cache, block, 0, 1);
        put(&cache, block, 1, 1);
        walk.at(block + 1);
    }
    // Each advance reclassifies two columns, then evicts the previous obsolete pair.
    assert_eq!(cache.lock().inspections - start, 4 * blocks - 2);
    let before = cache.lock().inspections;
    for _ in 0..10 {
        walk.at(blocks);
    }
    assert_eq!(cache.lock().inspections, before, "an unchanged reader");
    drop(query);
    assert_eq!(
        cache.lock().inspections,
        before,
        "the walk still reads everything the query did"
    );
    assert_consistent(&cache);
}

#[test]
fn a_column_retired_while_its_extension_is_decoded_keeps_its_entry() {
    let cache = Cache::default();
    cache.enable(true);
    let narrow = reader(&cache, 0, &[0], None, "narrow");
    narrow.reading(9);
    put(&cache, 0, 0, 1_500);
    let two_hours = || {
        let mut updates = rows(1_500);
        updates.extend(rows(1_500));
        updates
    };
    let extended = cache
        .read(at(0, 0), 2, false, || {
            // The older value is evicted while its extension still owns the entry.
            put(&cache, 1, 0, 1_500);
            narrow.at(2);
            assert_consistent(&cache);
            let state = cache.lock();
            let entry = &state.entries[&at(0, 0)];
            assert!(entry.reading && entry.value.is_none());
            drop(state);
            Ok(two_hours())
        })
        .unwrap();
    assert_eq!(extended.len(), 2);
    assert_consistent(&cache);
    assert!(
        !cache
            .lock()
            .entries
            .get(&at(0, 0))
            .is_some_and(|entry| entry.reading)
    );
}

#[test]
fn a_declared_read_includes_the_sources_and_layout_gaps_it_decodes() {
    let version = crate::format::FormatVersion::CURRENT.version();
    let read = widen(version, &[TableId::Divisions]);
    for table in [
        TableId::Divisions,
        TableId::ProvinceUnitOrder,
        TableId::DivisionStatus,
        TableId::DivisionPanelStats,
        TableId::SimulationFeatures,
        TableId::Campaign,
    ] {
        assert!(read[table.index()], "{table:?}");
    }
    let clock = widen(version, &[TableId::Clock]);
    for table in TableId::ALL {
        let change = &HISTORY[table.index()];
        let gap = change.present(version) && change.table_gap(table, version).is_some();
        if gap {
            assert!(read[table.index()] && clock[table.index()], "{table:?}");
        }
    }
    assert!(!clock[TableId::Divisions.index()] && !clock[TableId::DivisionStatus.index()]);
}

#[derive(Debug, PartialEq)]
struct Snapshot {
    needed: Vec<(u64, usize, usize)>,
    obsolete: Vec<(u64, usize, usize)>,
    retained: Vec<(usize, usize)>,
    bytes: usize,
    obsolete_bytes: usize,
    obsolete_tables: BTreeMap<usize, usize>,
    allowance: usize,
}

fn snapshot(cache: &Cache) -> Snapshot {
    let state = cache.lock();
    let order = |order: &BTreeSet<(u64, Key)>| -> Vec<(u64, usize, usize)> {
        order
            .iter()
            .map(|&(used, key)| (used, key.table, key.block))
            .collect()
    };
    Snapshot {
        needed: order(&state.lru),
        obsolete: order(&state.obsolete),
        retained: state
            .retained
            .iter()
            .map(|&(table, block, _)| (table, block))
            .collect(),
        bytes: state.bytes,
        obsolete_bytes: state.obsolete_bytes,
        obsolete_tables: state.obsolete_tables.clone(),
        allowance: state.allowance,
    }
}

#[test]
fn ready_hits_keep_each_column_in_its_order_as_read_hits_do() {
    let run = |hit: &dyn Fn(&Cache, Key)| {
        let cache = Cache::default();
        cache.enable(true);
        let narrow = reader(&cache, 0, &[0], None, "narrow");
        let other = reader(&cache, 0, &[1], None, "other");
        let keep = reader(&cache, 0, &[2], None, "keep");
        for consumer in [&narrow, &other, &keep] {
            consumer.reading(9);
        }
        put(&cache, 0, 0, 1_500);
        put(&cache, 1, 0, 1_500);
        put(&cache, 5, 1, 1);
        put(&cache, 6, 1, 1);
        put(&cache, 5, 2, 1_500);
        // Both of table 0's columns retire, within an allowance of two large columns and a small one.
        narrow.at(2);
        assert_consistent(&cache);
        assert_eq!(kept(&cache), [(0, 0), (0, 1), (1, 5), (1, 6), (2, 5)]);
        // The older retired column and the older needed one are used again.
        hit(&cache, at(0, 0));
        hit(&cache, at(5, 1));
        let before = snapshot(&cache);
        assert!(
            cache.ready(at(3, 0), 1, false).is_none(),
            "an absent column"
        );
        assert!(cache.ready(at(1, 0), 2, false).is_none(), "a short column");
        assert_eq!(snapshot(&cache), before, "a miss changes nothing");
        // Retiring a third column forces eviction from the obsolete order.
        keep.at(6);
        assert_consistent(&cache);
        assert_eq!(kept(&cache), [(0, 0), (1, 5), (1, 6), (2, 5)]);
        snapshot(&cache)
    };
    let by_ready = run(&|cache, key| assert!(cache.ready(key, 1, false).is_some()));
    let by_read = run(&|cache, key| {
        cache
            .read(key, 1, false, || panic!("a hit decodes nothing"))
            .unwrap();
    });
    assert_eq!(by_ready, by_read);
    let columns = |order: &[(u64, usize, usize)]| -> Vec<(usize, usize)> {
        order
            .iter()
            .map(|&(_, table, block)| (table, block))
            .collect()
    };
    assert_eq!(
        columns(&by_ready.needed),
        [(1, 6), (1, 5)],
        "the needed column hit is the later"
    );
    assert_eq!(
        columns(&by_ready.obsolete),
        [(2, 5), (0, 0)],
        "the retired column hit outlives the older one"
    );
}

#[test]
fn the_decode_log_holds_every_threads_misses_by_extent_mode_and_outcome() {
    let cache = Cache::default();
    cache.enable(true);
    cache.log_decodes();
    cache.read(at(0, 0), 1, true, || Ok(rows(2))).unwrap();
    cache
        .read(at(0, 0), 1, true, || panic!("a hit decodes nothing"))
        .unwrap();
    // A table read cannot use a keyframe admission.
    cache.read(at(0, 0), 1, false, || Ok(rows(2))).unwrap();
    assert!(
        cache
            .read(at(0, 0), 2, false, || Err(io::Error::other("refused")))
            .is_err()
    );
    assert!(
        std::panic::catch_unwind(|| {
            let _ = cache.read(at(0, 0), 2, false, || panic!("synthetic panic"));
        })
        .is_err()
    );
    std::thread::scope(|scope| {
        scope.spawn(|| {
            cache.read(at(1, 3), 1, false, || Ok(rows(2))).unwrap();
        });
    });
    let decode = |block, table, hours, keyframe_only, returned| SectionDecode {
        block,
        table,
        hours,
        keyframe_only,
        returned,
    };
    assert_eq!(
        cache.decodes(),
        [
            decode(0, 0, 1, true, true),
            decode(0, 0, 1, false, true),
            decode(0, 0, 2, false, false),
            decode(0, 0, 2, false, false),
            decode(1, 3, 1, false, true),
        ]
    );
}

fn retained(block: usize, table: usize, bytes: usize, obsolete: bool) -> (Key, Entry) {
    let value = Column {
        updates: Updates::new(rows(1)).unwrap(),
        bytes,
        table_read: true,
    };
    (
        at(block, table),
        Entry {
            value: Some(Arc::new(value)),
            reading: false,
            priority: 0,
            used: 0,
            obsolete,
        },
    )
}

#[test]
fn census_classifies_columns_by_need_and_walk_window_without_pruning() {
    let entries: BTreeMap<Key, Entry> = (0..6)
        .flat_map(|block| {
            [
                retained(block, 0, 10 + block, block < 2),
                retained(block, 1, 100, block < 2),
            ]
        })
        .chain([(key(9), Entry::default())])
        .collect();
    let mut census = SectionCacheCensus::EMPTY;
    classify(&entries, Some((2, 3)), &mut census);
    // Blocks 0-1 are behind, 2-3 within, 4-5 beyond; the empty entry adds no charge.
    assert_eq!(census.obsolete[0], 10 + 11);
    assert_eq!(census.needed[0], 12 + 13 + 14 + 15);
    assert_eq!(census.behind[0], 10 + 11);
    assert_eq!(census.window[0], 12 + 13);
    assert_eq!(census.beyond[0], 14 + 15);
    assert_eq!(
        (census.obsolete[1], census.needed[1]),
        (200, 400),
        "a second table classifies apart"
    );
    assert_eq!(
        (census.behind[1], census.window[1], census.beyond[1]),
        (200, 200, 200)
    );
    // Without a walk, nothing is placed against it.
    let mut census = SectionCacheCensus::EMPTY;
    classify(&entries, None, &mut census);
    assert_eq!(census.obsolete[0], 10 + 11);
    assert_eq!(census.window, [0; TABLE_COUNT]);
    assert_eq!(census.behind, [0; TABLE_COUNT]);
    assert_eq!(census.beyond, [0; TABLE_COUNT]);
}

#[test]
fn census_reads_one_snapshot_and_changes_nothing() {
    for countries in [1_500, 5_000] {
        let cache = Cache::default();
        cache.enable(true);
        assert_eq!(cache.census(None), SectionCacheCensus::EMPTY);
        let consumer = cache.consumer(1);
        consumer.reading(3);
        let narrow = reader(&cache, 2, &[1], None, "narrow");
        for block in 0..4 {
            put(&cache, block, 0, countries);
        }
        let census = cache.census(Some((2, 2)));
        let state = cache.lock();
        assert_eq!(census.bytes, state.bytes);
        assert_eq!(census.budget, state.budget());
        assert_eq!(census.frontier, Some((1, 3)));
        assert_eq!(census.readers, 2);
        assert_eq!(census.largest[0], state.largest[&0]);
        assert_eq!(census.retaining[0], Some("reader"));
        assert_eq!(
            census.retaining[1],
            Some("reader"),
            "the undeclared reader's next is the least"
        );
        assert_eq!(
            census.needed.iter().sum::<usize>() + census.obsolete.iter().sum::<usize>(),
            state.bytes,
            "every retained column is needed or obsolete"
        );
        assert_eq!(
            census.behind.iter().sum::<usize>()
                + census.window.iter().sum::<usize>()
                + census.beyond.iter().sum::<usize>(),
            state.bytes,
            "every retained column is before, within or after a window"
        );
        drop(state);
        assert_eq!(
            cache.census(Some((2, 2))),
            census,
            "a census changes nothing"
        );
        drop(consumer);
        assert_eq!(
            cache.census(None).retaining[1],
            Some("narrow"),
            "with the undeclared reader gone, the declared one retains its table"
        );
        drop(narrow);
    }
}

#[test]
fn replay_accounting_excludes_caller_held_updates_after_retention_is_disabled() {
    let replay = recording(1);
    let table = TableId::Countries;
    let held = replay
        .decoded_sections
        .read(Key::new(&replay, &replay.index[0], table), 1, false, || {
            Ok(rows(1_500))
        })
        .unwrap();
    let charge = ENTRY_BYTES + held.bytes();
    let obsolete = replay.decoded_section_census(Some((0, 0)));
    assert_eq!(replay.decoded_section_cache_bytes(), charge);
    assert_eq!(obsolete.bytes, charge);
    assert_eq!(obsolete.budget, charge);
    assert_eq!(obsolete.obsolete[table.index()], charge);
    assert_eq!(obsolete.needed, [0; TABLE_COUNT]);
    assert_eq!(obsolete.window[table.index()], charge);
    assert_eq!(obsolete.readers, 0);
    assert_eq!(obsolete.frontier, None);
    let reader = replay.section_reader(0, &[table], "accounting");
    let needed = replay.decoded_section_census(Some((0, 0)));
    assert_eq!(needed.needed[table.index()], charge);
    assert_eq!(needed.obsolete, [0; TABLE_COUNT]);
    assert_eq!(needed.retaining[table.index()], Some("accounting"));
    assert_eq!(needed.frontier, Some((0, 0)));
    assert_eq!(needed.readers, 1);
    replay.cache_decoded_sections(false);
    assert_eq!(replay.decoded_section_cache_bytes(), 0);
    assert_eq!(replay.decoded_section_census(None).bytes, 0);
    assert_eq!(held.len(), 1);
    assert!(matches!(
        &**held.get(0),
        TableUpdate::Snapshot(TableRows::Countries(rows)) if rows.len() == 1_500
    ));
    drop(reader);
}

fn recording(blocks: usize) -> Replay {
    let hours: Vec<i32> = (0..blocks).map(|block| 1 + 720 * block as i32).collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, &hours)]).unwrap();
    assert_eq!(replay.index.len(), blocks, "a block for each hour");
    replay.cache_decoded_sections(true);
    replay
}

fn fill(replay: &Replay, block: usize, table: TableId, countries: usize) {
    let key = Key::new(replay, &replay.index[block], table);
    replay
        .decoded_sections
        .read(key, 1, false, || Ok(rows(countries)))
        .unwrap();
}

struct Reads {
    blocks: (usize, usize),
    tables: Option<Box<[bool; TABLE_COUNT]>>,
}

impl Reads {
    // Reference ranges use explicit fixture endpoints rather than Frontier::range.
    fn window(replay: &Replay, next: usize, last: usize, tables: &[TableId]) -> Self {
        Self {
            blocks: (next, last + 1),
            tables: Some(widen(replay.format.version(), tables)),
        }
    }

    fn block(replay: &Replay, block: usize, tables: Option<&[TableId]>) -> Self {
        Self {
            blocks: (block, block + 1),
            tables: tables.map(|tables| widen(replay.format.version(), tables)),
        }
    }

    fn reads(&self, table: usize, block: usize) -> bool {
        (self.blocks.0..self.blocks.1).contains(&block)
            && self.tables.as_ref().is_none_or(|tables| tables[table])
    }
}

fn assert_needs(replay: &Replay, readers: &[&Reads]) {
    assert_consistent(&replay.decoded_sections);
    let state = replay.decoded_sections.lock();
    for (key, entry) in &state.entries {
        if entry.value.is_some() {
            let needed = readers
                .iter()
                .any(|reader| reader.reads(key.table, key.block));
            assert_eq!(
                entry.obsolete, !needed,
                "the need of table {} block {}",
                key.table, key.block
            );
        }
    }
    assert!(state.obsolete_bytes <= state.allowance);
}

fn columns(columns: &[(TableId, usize)]) -> Vec<(usize, usize)> {
    let mut columns: Vec<(usize, usize)> = columns
        .iter()
        .map(|&(table, block)| (table.index(), block))
        .collect();
    columns.sort_unstable();
    columns
}

#[test]
fn a_block_reader_needs_its_block_alone_wherever_it_moves_and_whoever_else_reads() {
    let blocks = 24;
    let replay = recording(blocks);
    let cache = &replay.decoded_sections;
    let (countries, clock) = (TableId::Countries, TableId::Clock);
    assert!(!widen(replay.format.version(), &[countries])[clock.index()]);
    let paged = |block| Reads::block(&replay, block, Some(&[countries][..]));
    let walked = |next| Reads::window(&replay, next, blocks, &[countries]);
    let pages = replay.section_block_reader(0, Some(&[countries][..]), "pages");
    let walk = replay.section_reader(0, &[countries], "walk");
    // A far reach keeps the span budget out of the way.
    walk.reading(blocks);
    for block in 0..blocks {
        fill(&replay, block, countries, 1_500);
        walk.at(block + 1);
        assert_needs(&replay, &[&paged(0), &walked(block + 1)]);
        let mut held = vec![(countries, 0)];
        if block > 0 {
            held.push((countries, block));
        }
        assert_eq!(kept(cache), columns(&held), "past block {block}");
    }
    // Forward, then back: the block it left is no longer its need.
    pages.at(5);
    assert_needs(&replay, &[&paged(5), &walked(blocks)]);
    assert_eq!(kept(cache), columns(&[(countries, 23)]));
    fill(&replay, 5, countries, 1_500);
    pages.at(2);
    assert_needs(&replay, &[&paged(2), &walked(blocks)]);
    assert_eq!(kept(cache), columns(&[(countries, 5)]));
    fill(&replay, 2, countries, 1_500);
    // Another reader of block 2 keeps it needed as the first moves on.
    let other = replay.section_block_reader(2, Some(&[countries][..]), "other");
    pages.at(3);
    assert_needs(&replay, &[&paged(3), &paged(2), &walked(blocks)]);
    assert_eq!(kept(cache), columns(&[(countries, 2), (countries, 5)]));
    drop(other);
    assert_needs(&replay, &[&paged(3), &walked(blocks)]);
    assert_eq!(kept(cache), columns(&[(countries, 2)]));
    // An earlier reader keeps columns through its scheduled endpoint.
    let early = replay.section_reader(1, &[countries], "early");
    early.reading(blocks);
    assert_needs(&replay, &[&paged(3), &walked(1), &walked(blocks)]);
    fill(&replay, 3, countries, 1_500);
    drop(pages);
    assert_needs(&replay, &[&walked(1), &walked(blocks)]);
    assert_eq!(kept(cache), columns(&[(countries, 2), (countries, 3)]));
    early.at(4);
    assert_needs(&replay, &[&walked(4), &walked(blocks)]);
    assert_eq!(kept(cache), columns(&[(countries, 3)]));
    // The later reader cannot retain the earlier block on the block reader's behalf.
    let late = replay.section_reader(10, &[countries], "late");
    late.reading(blocks);
    drop(early);
    let alone = replay.section_block_reader(8, Some(&[countries][..]), "pages");
    fill(&replay, 8, countries, 1_500);
    fill(&replay, 12, countries, 1_500);
    assert_needs(&replay, &[&paged(8), &walked(10), &walked(blocks)]);
    assert_eq!(
        kept(cache),
        columns(&[(countries, 3), (countries, 8), (countries, 12)])
    );
    drop(alone);
    assert_needs(&replay, &[&walked(10), &walked(blocks)]);
    assert_eq!(kept(cache), columns(&[(countries, 8), (countries, 12)]));
    // An undeclared block reader needs every table at its block.
    let read = replay.section_block_reader(12, None, "read");
    fill(&replay, 12, clock, 1);
    let every = Reads::block(&replay, 12, None);
    assert_needs(&replay, &[&every, &walked(10), &walked(blocks)]);
    drop(read);
    assert_needs(&replay, &[&walked(10), &walked(blocks)]);
    assert_eq!(
        kept(cache),
        columns(&[(countries, 8), (countries, 12), (clock, 12)])
    );
    drop((late, walk));
    assert_needs(&replay, &[]);
}

#[test]
fn a_declaration_moves_need_between_tables_over_the_columns_retained() {
    let replay = recording(4);
    let cache = &replay.decoded_sections;
    let version = replay.format.version();
    let (countries, clock, divisions, status, campaign) = (
        TableId::Countries,
        TableId::Clock,
        TableId::Divisions,
        TableId::DivisionStatus,
        TableId::Campaign,
    );
    let reads = |tables: &[TableId], table: TableId| widen(version, tables)[table.index()];
    for table in [clock, status] {
        assert!(!reads(&[countries], table), "{table:?}");
    }
    for table in [countries, status] {
        assert!(!reads(&[clock], table), "{table:?}");
    }
    for table in [countries, clock, status] {
        assert!(!reads(&[], table), "{table:?}");
    }
    assert!(reads(&[], campaign));
    assert!(reads(&[clock, divisions], status) && !reads(&[clock, divisions], countries));
    let declared = |next, tables: &[TableId]| Reads::window(&replay, next, 9, tables);
    let family = replay.section_reader(0, &[countries], "family");
    // A far reach keeps the span budget out of the way.
    family.reading(9);
    for (table, block, size) in [
        (countries, 1, 1_500),
        (countries, 2, 1_500),
        (clock, 1, 1),
        (clock, 2, 1),
        (status, 2, 1),
        (campaign, 1, 1),
    ] {
        fill(&replay, block, table, size);
    }
    assert_needs(&replay, &[&declared(0, &[countries])]);
    // Clock in place of Countries: of what retires, one block's set stays.
    family.declare(&replay, &[clock]);
    assert_needs(&replay, &[&declared(0, &[clock])]);
    let after = columns(&[
        (countries, 2),
        (clock, 1),
        (clock, 2),
        (status, 2),
        (campaign, 1),
    ]);
    assert_eq!(kept(cache), after);
    // The same declaration again changes nothing and inspects nothing.
    let (before, inspected) = (snapshot(cache), cache.lock().inspections);
    family.declare(&replay, &[clock]);
    assert_eq!(snapshot(cache), before);
    assert_eq!(cache.lock().inspections, inspected);
    // Divisions brings the sources its private aliases read.
    family.declare(&replay, &[clock, divisions]);
    assert_needs(&replay, &[&declared(0, &[clock, divisions])]);
    let source = Key::new(&replay, &replay.index[2], status);
    assert!(!cache.lock().entries[&source].obsolete);
    // Forward, then back: the declared tables' columns are needed again.
    family.at(2);
    assert_needs(&replay, &[&declared(2, &[clock, divisions])]);
    family.at(1);
    assert_needs(&replay, &[&declared(1, &[clock, divisions])]);
    assert_eq!(kept(cache), after);
    // Declaring nothing still reads Campaign.
    family.declare(&replay, &[]);
    assert_needs(&replay, &[&declared(1, &[])]);
    assert_eq!(kept(cache), after);
    // An extension under way as a declaration retires its column.
    family.declare(&replay, &[clock]);
    assert_needs(&replay, &[&declared(1, &[clock])]);
    let key = Key::new(&replay, &replay.index[1], clock);
    let extended = cache
        .read(key, 2, false, || {
            family.declare(&replay, &[countries]);
            assert_needs(&replay, &[&declared(1, &[countries])]);
            assert!(cache.lock().entries[&key].reading);
            let mut updates = rows(1);
            updates.extend(rows(1));
            Ok(updates)
        })
        .unwrap();
    assert_eq!(extended.len(), 2);
    assert_needs(&replay, &[&declared(1, &[countries])]);
    let state = cache.lock();
    assert!(!state.entries[&key].reading && state.entries[&key].obsolete);
    drop(state);
    assert_eq!(kept(cache), after);
}

#[test]
fn a_grouped_reads_own_reader_holds_its_block_while_another_reader_passes_it() {
    let replay = recording(4);
    let divisions = TableId::Divisions;
    let walk = replay.section_reader(0, &[divisions], "walk");
    // A far reach keeps the span budget out of the way.
    walk.reading(9);
    fill(&replay, 1, divisions, 1_500);
    let index = &replay.index[1];
    let held = Key::new(&replay, index, divisions);
    let block = replay.block_bytes(index).unwrap();
    let mut checked = false;
    read_selected_updates(&replay, index, &block, &[TableId::Countries], &mut || {
        if !checked {
            checked = true;
            for later in [2, 3] {
                fill(&replay, later, divisions, 1_500);
            }
            walk.at(4);
            let read = Reads::block(&replay, 1, None);
            assert_needs(
                &replay,
                &[&read, &Reads::window(&replay, 4, 9, &[divisions])],
            );
            let state = replay.decoded_sections.lock();
            assert!(
                !state.entries[&held].obsolete,
                "the read's block is its need"
            );
        }
        Ok(())
    })
    .unwrap();
    assert!(checked, "the read checks before each table");
    assert_needs(&replay, &[&Reads::window(&replay, 4, 9, &[divisions])]);
    assert!(
        replay
            .decoded_sections
            .lock()
            .entries
            .get(&held)
            .is_none_or(|entry| entry.obsolete)
    );
}

#[test]
fn the_wire_log_holds_each_decode_a_read_runs_apart_from_the_caches_admissions() {
    use crate::container::aliases::private::hooks;
    use WireRoute::{Direct, Keyframe, Original};
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2, 3])]).unwrap();
    replay.cache_decoded_sections(true);
    replay.log_section_decodes();
    let index = &replay.index[0];
    let table = TableId::WorldTension;
    let wire = || -> Vec<(usize, usize, WireRoute)> {
        replay
            .wire_decodes()
            .into_iter()
            .filter(|decode| decode.table == table.index())
            .map(|decode| (decode.block, decode.hours, decode.route))
            .collect()
    };
    let admitted = || -> Vec<(usize, bool, bool)> {
        replay
            .section_decodes()
            .into_iter()
            .filter(|decode| decode.table == table.index())
            .map(|decode| (decode.hours, decode.keyframe_only, decode.returned))
            .collect()
    };
    replay.read_keyframe_selected(index, &[table]).unwrap();
    replay
        .read_block_recorded_through(index, &[table], 2)
        .unwrap();
    replay
        .read_block_recorded_selected(index, &[table])
        .unwrap();
    replay
        .read_block_recorded_selected(index, &[table])
        .unwrap();
    assert_eq!(wire(), [(0, 1, Keyframe), (0, 2, Direct), (0, 3, Direct)]);
    assert_eq!(
        admitted(),
        [(1, true, true), (2, false, true), (3, false, true)]
    );
    // Refused on both routes, then failing to allocate on the first.
    replay.cache_decoded_sections(false);
    replay.cache_decoded_sections(true);
    hooks::with(|hooks| hooks.fail_raw = vec![(table, hooks::Failure::Text)]);
    assert!(
        replay
            .read_block_recorded_selected(index, &[table])
            .is_err()
    );
    hooks::with(|hooks| hooks.fail_raw = vec![(table, hooks::Failure::Bare)]);
    assert!(
        replay
            .read_block_recorded_selected(index, &[table])
            .is_err()
    );
    hooks::with(|hooks| {
        hooks.fail_raw.clear();
    });
    assert_eq!(
        wire()[3..],
        [(0, 3, Direct), (0, 3, Original), (0, 3, Direct)]
    );
    assert_eq!(admitted()[3..], [(3, false, false), (3, false, false)]);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            replay
                .read_block_recorded_selected(index, &[table])
                .unwrap()
        });
    });
    assert_eq!(wire()[6..], [(0, 3, Direct)], "another thread's decode");
    assert_eq!(admitted()[5..], [(3, false, true)]);
}

#[test]
fn a_reader_needs_its_window_through_the_furthest_block_it_scheduled() {
    let replay = recording(8);
    let cache = &replay.decoded_sections;
    let table = TableId::Countries;
    let window = |next: usize, last: usize| Reads {
        blocks: (next, last + 1),
        tables: Some(widen(replay.format.version(), &[table])),
    };
    let slow = replay.section_reader(0, &[table], "slow");
    let fast = replay.section_reader(4, &[table], "fast");
    fast.reading(7);
    for block in 4..8 {
        fill(&replay, block, table, 1_500);
    }
    assert_needs(&replay, &[&window(0, 0), &window(4, 7)]);
    slow.reading(5);
    assert_needs(&replay, &[&window(0, 5), &window(4, 7)]);
    fast.at(8);
    assert_needs(&replay, &[&window(0, 5), &window(8, 8)]);
    assert_eq!(kept(cache), columns(&[(table, 4), (table, 5), (table, 7)]));
    slow.at(5);
    assert_needs(&replay, &[&window(5, 5), &window(8, 8)]);
    assert_eq!(kept(cache), columns(&[(table, 5), (table, 7)]));
    drop((slow, fast));
    assert_needs(&replay, &[]);
}

#[test]
fn a_far_claim_makes_every_retained_column_it_covers_needed() {
    let replay = recording(6);
    let cache = &replay.decoded_sections;
    let (large, small) = (TableId::Countries, TableId::Clock);
    let window = |table: TableId, next: usize, last: usize| Reads {
        blocks: (next, last + 1),
        tables: Some(widen(replay.format.version(), &[table])),
    };
    let keep = replay.section_reader(5, &[large], "keep");
    fill(&replay, 5, large, 1_500);
    let reader = replay.section_reader(0, &[small], "reader");
    for block in 2..5 {
        fill(&replay, block, small, 1);
    }
    assert_needs(&replay, &[&window(large, 5, 5), &window(small, 0, 0)]);
    assert_eq!(
        kept(cache),
        columns(&[(large, 5), (small, 2), (small, 3), (small, 4)])
    );
    reader.reading(4);
    assert_needs(&replay, &[&window(large, 5, 5), &window(small, 0, 4)]);
    let inspected = cache.lock().inspections;
    reader.reading(3);
    reader.reading(4);
    assert_eq!(cache.lock().inspections, inspected, "nothing changed");
    drop((keep, reader));
}

#[test]
fn a_reader_moved_back_keeps_only_the_blocks_it_still_has_scheduled() {
    let blocks = 48;
    let replay = recording(blocks);
    let cache = &replay.decoded_sections;
    let table = TableId::Countries;
    let window = |next: usize, last: usize| Reads {
        blocks: (next, last + 1),
        tables: Some(widen(replay.format.version(), &[table])),
    };
    let single = replay.section_block_reader(47, Some(&[table][..]), "single");
    let ordinary = replay.section_reader(0, &[table], "ordinary");
    ordinary.reading(47);
    for block in 0..blocks {
        fill(&replay, block, table, 1_500);
    }
    assert_needs(&replay, &[&window(47, 47), &window(0, 47)]);
    assert_eq!(kept(cache).len(), blocks);
    single.at(0);
    assert_needs(&replay, &[&window(0, 0), &window(0, 47)]);
    ordinary.at(47);
    assert_needs(&replay, &[&window(0, 0), &window(47, 47)]);
    assert_eq!(
        kept(cache),
        columns(&[(table, 0), (table, 46), (table, 47)])
    );
    ordinary.at(0);
    assert_needs(&replay, &[&window(0, 0), &window(0, 0)]);
    assert_eq!(kept(cache), columns(&[(table, 0)]));
    ordinary.reading(5);
    ordinary.at(2);
    assert_needs(&replay, &[&window(0, 0), &window(2, 5)]);
    fill(&replay, 4, table, 1_500);
    assert_needs(&replay, &[&window(0, 0), &window(2, 5)]);
    assert_eq!(kept(cache), columns(&[(table, 0), (table, 4)]));
    ordinary.at(10);
    assert_needs(&replay, &[&window(0, 0), &window(10, 10)]);
    single.at(9);
    single.at(3);
    assert_needs(&replay, &[&window(3, 3), &window(10, 10)]);
    assert_eq!(kept(cache), columns(&[(table, 4)]));
    ordinary.reading(14);
    ordinary.at(12);
    ordinary.at(11);
    assert_needs(&replay, &[&window(3, 3), &window(11, 14)]);
    assert_eq!(kept(cache), columns(&[(table, 4)]));
}

#[test]
fn retention_stays_flat_as_the_gap_grows() {
    let table = TableId::Countries;
    for countries in [1_500, 5_000] {
        let mut measured_peaks = Vec::new();
        for blocks in [12, 24, 48] {
            let replay = recording(blocks);
            let cache = &replay.decoded_sections;
            let reads = |next: usize, last: usize| Reads {
                blocks: (next, last + 1),
                tables: Some(widen(replay.format.version(), &[table])),
            };
            let expected = |held: &[usize]| -> Vec<(usize, usize)> {
                let held: BTreeSet<usize> = held.iter().copied().collect();
                held.into_iter()
                    .map(|block| (table.index(), block))
                    .collect()
            };
            let held = replay.section_reader(0, &[table], "held");
            let runner = replay.section_reader(0, &[table], "runner");
            fill(&replay, 0, table, countries);
            let mut peak = 0;
            for block in 1..blocks {
                runner.reading(block);
                fill(&replay, block, table, countries);
                peak = peak.max(cache.lock().bytes);
                assert_needs(&replay, &[&reads(0, 0), &reads(block - 1, block)]);
                let read = [0, block.saturating_sub(2), block - 1, block];
                assert_eq!(kept(cache), expected(&read), "read {block} of {blocks}");
                runner.at(block);
                peak = peak.max(cache.lock().bytes);
                assert_needs(&replay, &[&reads(0, 0), &reads(block, block)]);
                let advanced = [0, block - 1, block];
                assert_eq!(kept(cache), expected(&advanced), "at {block} of {blocks}");
            }
            let largest = cache.lock().largest[&table.index()];
            assert_eq!(peak, 4 * largest, "{blocks} blocks, {countries} countries");
            measured_peaks.push(peak);
            drop((held, runner));
            assert_needs(&replay, &[]);
            assert_eq!(kept(cache), columns(&[(table, blocks - 1)]));
        }
        assert_eq!(measured_peaks, vec![measured_peaks[0]; 3]);
    }
}

#[test]
fn a_reader_scheduling_over_a_faster_ones_blocks_hits_all_it_decoded() {
    let blocks = 12;
    let table = TableId::Countries;
    for k in [2, 3, 4] {
        let replay = recording(blocks);
        let cache = &replay.decoded_sections;
        let slow = replay.section_reader(0, &[table], "slow");
        slow.reading(k);
        let fast = replay.section_reader(0, &[table], "fast");
        replay.log_section_decodes();
        let mut peak = 0;
        for block in 0..blocks {
            fast.reading(block);
            fast.at(block);
            fill(&replay, block, table, 1_500);
            peak = peak.max(cache.lock().bytes);
            if block >= k {
                slow.at(block - k);
                slow.reading(block);
                fill(&replay, block - k, table, 1_500);
                peak = peak.max(cache.lock().bytes);
            }
            assert_consistent(cache);
        }
        let decoded: Vec<usize> = replay
            .section_decodes()
            .iter()
            .map(|decode| decode.block)
            .collect();
        assert_eq!(
            decoded,
            (0..blocks).collect::<Vec<_>>(),
            "k {k}: each block once"
        );
        let largest = cache.lock().largest[&table.index()];
        assert_eq!(peak, (k + 2) * largest, "k {k}");
        drop((slow, fast));
    }
}

#[test]
fn a_slower_reader_decodes_again_only_what_its_window_did_not_keep() {
    let blocks = 6;
    let replay = recording(blocks);
    let reference = recording(blocks);
    reference.cache_decoded_sections(false);
    let table = TableId::WorldTension;
    let read = |replay: &Replay, block: usize| {
        replay
            .read_block_selected(&replay.index[block], &[table])
            .unwrap()
    };
    let slow = replay.section_reader(0, &[table], "slow");
    slow.reading(2);
    let fast = replay.section_reader(0, &[table], "fast");
    fast.reading(blocks - 1);
    for block in 0..blocks {
        fast.at(block);
        assert_eq!(read(&replay, block), read(&reference, block));
    }
    fast.at(blocks);
    replay.log_section_decodes();
    let mut again = Vec::new();
    for block in 1..blocks {
        slow.at(block);
        let held = replay
            .decoded_sections
            .lock()
            .retained
            .iter()
            .any(|&(kept_table, kept_block, _)| kept_table == table.index() && kept_block == block);
        let before = replay.wire_decodes().len();
        assert_eq!(
            read(&replay, block),
            read(&reference, block),
            "block {block}"
        );
        let decoded = replay.wire_decodes()[before..]
            .iter()
            .any(|decode| decode.table == table.index() && decode.block == block);
        assert_eq!(decoded, !held, "block {block}: decoded again when not kept");
        if block <= 2 {
            assert!(held, "block {block} was in its window");
        }
        if decoded {
            again.push(block);
        }
    }
    assert!(again.contains(&3), "decoded again: {again:?}");
    drop((slow, fast));
}

fn wait_for_waiters(cache: &Cache, count: usize) {
    let mut state = cache.lock();
    while state.waiters < count {
        state = cache.0.waiting.wait(state).unwrap();
    }
}

fn normal_priority() {
    #[cfg(windows)]
    set_priority(windows_sys::Win32::System::Threading::THREAD_PRIORITY_NORMAL);
}

#[cfg(windows)]
fn set_priority(priority: i32) {
    use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority};
    assert_ne!(
        unsafe { SetThreadPriority(GetCurrentThread(), priority) },
        0
    );
    assert_eq!(thread_priority(), priority);
}

fn assert_same_updates(left: &Updates, right: &Updates) {
    assert_eq!(left.len(), right.len());
    for hour in 0..left.len() {
        assert_eq!(left.get(hour), right.get(hour));
    }
}

#[test]
fn equal_priority_waits_for_the_owner_and_reuses_exactly_one_decode() {
    use std::sync::mpsc;
    let cache = Cache::default();
    cache.enable(true);
    cache.log_decodes();
    std::thread::scope(|scope| {
        let cache = &cache;
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let owner = scope.spawn(move || {
            normal_priority();
            cache
                .read(key(0), 1, false, || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(rows(2))
                })
                .unwrap()
        });
        entered_rx.recv().unwrap();
        let waiter = scope.spawn(move || {
            normal_priority();
            cache
                .read(key(0), 1, false, || panic!("equal priority decoded again"))
                .unwrap()
        });
        wait_for_waiters(cache, 1);
        assert!(!waiter.is_finished());
        assert!(cache.decodes().is_empty());
        release_tx.send(()).unwrap();
        let first = owner.join().unwrap();
        let second = waiter.join().unwrap();
        assert_same_updates(&first, &second);
        assert!(Arc::ptr_eq(first.get(0), second.get(0)));
    });
    assert_eq!(cache.decodes().len(), 1);
    assert!(cache.decodes()[0].returned);
    assert_eq!(cache.lock().waiters, 0);
    assert_consistent(&cache);
}

#[test]
#[cfg(windows)]
fn normal_priority_decodes_while_the_lowest_priority_owner_stays_parked() {
    use std::sync::mpsc;
    use windows_sys::Win32::System::Threading::THREAD_PRIORITY_LOWEST;
    let cache = Cache::default();
    cache.enable(true);
    cache.log_decodes();
    std::thread::scope(|scope| {
        let cache = &cache;
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let owner = scope.spawn(move || {
            set_priority(THREAD_PRIORITY_LOWEST);
            cache
                .read(key(0), 1, false, || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(rows(2))
                })
                .unwrap()
        });
        entered_rx.recv().unwrap();
        let waiter = scope.spawn(move || {
            set_priority(THREAD_PRIORITY_LOWEST);
            cache
                .read(key(0), 1, false, || {
                    panic!("published bypass was not reused")
                })
                .unwrap()
        });
        wait_for_waiters(cache, 1);
        let bypass = scope
            .spawn(move || {
                normal_priority();
                cache
                    .read(key(0), 1, false, || {
                        assert!(cache.lock().entries[&key(0)].reading);
                        assert_eq!(
                            cache.lock().entries[&key(0)].priority,
                            THREAD_PRIORITY_LOWEST
                        );
                        Ok(rows(2))
                    })
                    .unwrap()
            })
            .join()
            .unwrap();
        assert!(!owner.is_finished());
        assert!(cache.lock().entries[&key(0)].reading);
        assert_eq!(cache.decodes().len(), 1);
        assert!(cache.decodes()[0].returned);
        let retained = cache.ready(key(0), 1, false).unwrap();
        assert!(Arc::ptr_eq(retained.get(0), bypass.get(0)));
        let waited = waiter.join().unwrap();
        assert!(Arc::ptr_eq(waited.get(0), bypass.get(0)));
        assert_consistent(cache);
        release_tx.send(()).unwrap();
        let original = owner.join().unwrap();
        assert_same_updates(&bypass, &original);
        let retained = cache.ready(key(0), 1, false).unwrap();
        assert!(Arc::ptr_eq(retained.get(0), original.get(0)));
    });
    assert_eq!(cache.decodes().len(), 2);
    assert!(cache.decodes().iter().all(|decode| decode.returned));
    assert_consistent(&cache);
}

#[test]
#[cfg(windows)]
fn lower_priority_waits_for_a_normal_priority_owner() {
    use std::sync::mpsc;
    use windows_sys::Win32::System::Threading::THREAD_PRIORITY_LOWEST;
    let cache = Cache::default();
    cache.enable(true);
    cache.log_decodes();
    std::thread::scope(|scope| {
        let cache = &cache;
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let owner = scope.spawn(move || {
            normal_priority();
            cache
                .read(key(0), 1, false, || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(rows(2))
                })
                .unwrap()
        });
        entered_rx.recv().unwrap();
        let waiter = scope.spawn(move || {
            set_priority(THREAD_PRIORITY_LOWEST);
            cache
                .read(key(0), 1, false, || panic!("lower priority decoded again"))
                .unwrap()
        });
        wait_for_waiters(cache, 1);
        assert!(!waiter.is_finished());
        release_tx.send(()).unwrap();
        let first = owner.join().unwrap();
        let second = waiter.join().unwrap();
        assert!(Arc::ptr_eq(first.get(0), second.get(0)));
    });
    assert_eq!(cache.decodes().len(), 1);
    assert_consistent(&cache);
}

#[test]
#[cfg(windows)]
fn bypass_errors_and_panics_leave_the_owner_and_its_waiter_alone() {
    use std::sync::mpsc;
    use windows_sys::Win32::System::Threading::THREAD_PRIORITY_LOWEST;
    let cache = Cache::default();
    cache.enable(true);
    cache.log_decodes();
    std::thread::scope(|scope| {
        let cache = &cache;
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let owner = scope.spawn(move || {
            set_priority(THREAD_PRIORITY_LOWEST);
            cache
                .read(key(0), 1, false, || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(rows(2))
                })
                .unwrap()
        });
        entered_rx.recv().unwrap();
        let waiter = scope.spawn(move || {
            set_priority(THREAD_PRIORITY_LOWEST);
            cache
                .read(key(0), 1, false, || {
                    panic!("owner slot was released by a bypass")
                })
                .unwrap()
        });
        wait_for_waiters(cache, 1);
        scope
            .spawn(move || {
                normal_priority();
                let error = cache
                    .read(key(0), 1, false, || Err(io::Error::other("bypass refusal")))
                    .unwrap_err();
                assert_eq!(error.to_string(), "bypass refusal");
                assert!(
                    std::panic::catch_unwind(|| {
                        let _ = cache.read(key(0), 1, false, || panic!("bypass panic"));
                    })
                    .is_err()
                );
            })
            .join()
            .unwrap();
        wait_for_waiters(cache, 1);
        assert!(!owner.is_finished() && !waiter.is_finished());
        assert!(cache.lock().entries[&key(0)].reading);
        assert_eq!(cache.decodes().len(), 2);
        assert!(cache.decodes().iter().all(|decode| !decode.returned));
        release_tx.send(()).unwrap();
        let original = owner.join().unwrap();
        let waited = waiter.join().unwrap();
        assert!(Arc::ptr_eq(original.get(0), waited.get(0)));
    });
    assert_eq!(cache.decodes().len(), 3);
    assert!(cache.decodes()[2].returned);
    assert_consistent(&cache);
}

#[test]
#[cfg(windows)]
fn a_late_bypass_keeps_a_covering_publication_but_replaces_a_short_or_keyframe_one() {
    use std::sync::mpsc;
    use windows_sys::Win32::System::Threading::THREAD_PRIORITY_LOWEST;
    for (owner_hours, owner_keyframe, bypass_hours, bypass_keyframe, keep) in [
        (2, false, 1, false, true),
        (1, true, 1, true, true),
        (1, false, 2, false, false),
        (1, true, 1, false, false),
    ] {
        let cache = Cache::default();
        cache.enable(true);
        cache.log_decodes();
        std::thread::scope(|scope| {
            let cache = &cache;
            let (owner_tx, owner_rx) = mpsc::channel();
            let (release_owner_tx, release_owner_rx) = mpsc::channel();
            let (bypass_tx, bypass_rx) = mpsc::channel();
            let (release_bypass_tx, release_bypass_rx) = mpsc::channel();
            let owner = scope.spawn(move || {
                set_priority(THREAD_PRIORITY_LOWEST);
                cache
                    .read(key(0), owner_hours, owner_keyframe, || {
                        owner_tx.send(()).unwrap();
                        release_owner_rx.recv().unwrap();
                        Ok((0..owner_hours).flat_map(|_| rows(2)).collect())
                    })
                    .unwrap()
            });
            owner_rx.recv().unwrap();
            let bypass = scope.spawn(move || {
                normal_priority();
                cache
                    .read(key(0), bypass_hours, bypass_keyframe, || {
                        bypass_tx.send(()).unwrap();
                        release_bypass_rx.recv().unwrap();
                        Ok((0..bypass_hours).flat_map(|_| rows(2)).collect())
                    })
                    .unwrap()
            });
            bypass_rx.recv().unwrap();
            release_owner_tx.send(()).unwrap();
            let original = owner.join().unwrap();
            assert!(!cache.lock().entries[&key(0)].reading);
            release_bypass_tx.send(()).unwrap();
            let independent = bypass.join().unwrap();
            let retained = cache.ready(key(0), bypass_hours, bypass_keyframe).unwrap();
            let expected = if keep { &original } else { &independent };
            assert!(Arc::ptr_eq(retained.get(0), expected.get(0)));
            assert_eq!(retained.get(0), original.get(0));
            assert_eq!(cache.decodes().len(), 2);
        });
        assert_consistent(&cache);
    }
}

#[test]
#[cfg(windows)]
fn a_bypass_can_publish_after_its_owner_fails_or_the_cache_is_reenabled() {
    use std::sync::mpsc;
    use windows_sys::Win32::System::Threading::THREAD_PRIORITY_LOWEST;
    for enabled in [true, false] {
        let cache = Cache::default();
        cache.enable(true);
        cache.log_decodes();
        std::thread::scope(|scope| {
            let cache = &cache;
            let (owner_tx, owner_rx) = mpsc::channel();
            let (release_owner_tx, release_owner_rx) = mpsc::channel();
            let (bypass_tx, bypass_rx) = mpsc::channel();
            let (release_bypass_tx, release_bypass_rx) = mpsc::channel();
            let owner = scope.spawn(move || {
                set_priority(THREAD_PRIORITY_LOWEST);
                cache.read(key(0), 1, false, || {
                    owner_tx.send(()).unwrap();
                    release_owner_rx.recv().unwrap();
                    Err(io::Error::other("owner refusal"))
                })
            });
            owner_rx.recv().unwrap();
            let bypass = scope.spawn(move || {
                normal_priority();
                cache
                    .read(key(0), 1, false, || {
                        bypass_tx.send(()).unwrap();
                        release_bypass_rx.recv().unwrap();
                        Ok(rows(2))
                    })
                    .unwrap()
            });
            bypass_rx.recv().unwrap();
            release_owner_tx.send(()).unwrap();
            assert_eq!(
                owner.join().unwrap().unwrap_err().to_string(),
                "owner refusal"
            );
            assert!(cache.lock().entries.is_empty());
            cache.enable(false);
            cache.enable(enabled);
            release_bypass_tx.send(()).unwrap();
            let updates = bypass.join().unwrap();
            assert_eq!(cache.ready(key(0), 1, false).is_some(), enabled);
            if enabled {
                assert!(Arc::ptr_eq(
                    cache.ready(key(0), 1, false).unwrap().get(0),
                    updates.get(0)
                ));
            } else {
                assert_eq!(cache.lock().bytes, 0);
                assert!(cache.lock().entries.is_empty());
            }
        });
        let log = cache.decodes();
        assert_eq!(log.len(), 2);
        assert!(!log[0].returned && log[1].returned);
        assert_consistent(&cache);
    }
}

#[test]
#[cfg(windows)]
fn a_bypass_can_publish_after_its_successful_owner_is_pruned() {
    use std::sync::mpsc;
    use windows_sys::Win32::System::Threading::THREAD_PRIORITY_LOWEST;
    let cache = Cache::default();
    cache.enable(true);
    cache.log_decodes();
    let expected = Updates::new(rows(3).into_iter().chain(rows(4)).collect()).unwrap();
    std::thread::scope(|scope| {
        let cache = &cache;
        let expected = &expected;
        let (owner_tx, owner_rx) = mpsc::channel();
        let (release_owner_tx, release_owner_rx) = mpsc::channel();
        let (bypass_tx, bypass_rx) = mpsc::channel();
        let (release_bypass_tx, release_bypass_rx) = mpsc::channel();
        let owner = scope.spawn(move || {
            set_priority(THREAD_PRIORITY_LOWEST);
            cache
                .read(key(1), 1, false, || {
                    owner_tx.send(()).unwrap();
                    release_owner_rx.recv().unwrap();
                    Ok(rows(2))
                })
                .unwrap()
        });
        owner_rx.recv().unwrap();
        let bypass = scope.spawn(move || {
            normal_priority();
            cache
                .read_updates(key(1), expected.len(), false, || {
                    bypass_tx.send(()).unwrap();
                    release_bypass_rx.recv().unwrap();
                    Ok(expected.clone())
                })
                .unwrap()
        });
        bypass_rx.recv().unwrap();
        assert!(!owner.is_finished() && !bypass.is_finished());
        release_owner_tx.send(()).unwrap();
        let original = owner.join().unwrap();
        assert!(original.len() < expected.len());
        let state = cache.lock();
        let entry = &state.entries[&key(1)];
        assert!(!entry.reading);
        assert!(Arc::ptr_eq(
            entry.value.as_ref().unwrap().updates.get(0),
            original.get(0)
        ));
        let owner_bytes = state.bytes;
        assert_eq!(state.allowance, owner_bytes);
        drop(state);
        assert_consistent(cache);

        put(cache, 0, 0, 2);
        assert!(!bypass.is_finished());
        assert!(!cache.lock().entries.contains_key(&key(1)));
        assert_eq!(kept(cache), vec![(0, 0)]);
        assert_eq!(cache.lock().bytes, owner_bytes);
        assert_eq!(cache.lock().allowance, owner_bytes);
        assert_consistent(cache);

        release_bypass_tx.send(()).unwrap();
        let independent = bypass.join().unwrap();
        assert_same_updates(&independent, expected);
        let retained = cache.ready(key(1), expected.len(), false).unwrap();
        for hour in 0..expected.len() {
            assert!(Arc::ptr_eq(independent.get(hour), expected.get(hour)));
            assert!(Arc::ptr_eq(retained.get(hour), expected.get(hour)));
        }
        assert_eq!(kept(cache), vec![(0, 1)]);
        assert!(!cache.lock().entries[&key(1)].reading);
        assert_eq!(cache.lock().bytes, ENTRY_BYTES + expected.bytes());
        assert_eq!(cache.lock().allowance, ENTRY_BYTES + expected.bytes());
    });
    let log = cache.decodes();
    assert_eq!(log.len(), 3);
    assert!(log.iter().all(|decode| decode.returned));
    assert_consistent(&cache);
}
