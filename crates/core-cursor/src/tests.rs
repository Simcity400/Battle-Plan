use crate::reference::{InMemory, MemoryPage, Row};
use crate::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::io;
use std::ops::ControlFlow;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

thread_local! {static TRACK:Cell<bool>=const{Cell::new(false)};static ALLOCATIONS:Cell<usize>=const{Cell::new(0)};}
struct Allocator;
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
fn allocation() {
    let _ = TRACK.try_with(|track| {
        if track.get() {
            let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
        }
    });
}
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        allocation();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        allocation();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, bytes: usize) -> *mut u8 {
        allocation();
        unsafe { System.realloc(pointer, layout, bytes) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
}
fn counted(action: impl FnOnce()) -> usize {
    ALLOCATIONS.with(|count| count.set(0));
    TRACK.with(|track| track.set(true));
    action();
    TRACK.with(|track| track.set(false));
    ALLOCATIONS.with(Cell::get)
}
fn address(key: u64) -> Address {
    Address {
        key: key.into(),
        field: 0.into(),
    }
}
fn record(hour: Hour, value: Value) -> Record {
    Record {
        hour,
        value,
        provenance: Provenance {
            supplying_hour: Some(hour),
            ..Default::default()
        },
        break_before: false,
    }
}
fn number(hour: Hour, value: f64) -> Record {
    record(hour, Value::Number(value))
}
fn prepared(first: Hour, last: Hour) -> Coverage {
    Coverage {
        hours: Hours { first, last },
        missing: None,
        accepted_before: first as u64,
    }
}
fn page(
    id: u64,
    first: Hour,
    last: Hour,
    changes: Vec<(Hour, u64, Record)>,
    coverage: Vec<Coverage>,
) -> MemoryPage {
    MemoryPage {
        id,
        hours: Hours { first, last },
        changes: changes
            .into_iter()
            .map(|(hour, key, record)| Change {
                hour,
                address: address(key),
                record,
            })
            .collect(),
        coverage,
    }
}
fn options(memory: &Memory, speed: f64, refresh: f64) -> CursorOptions {
    CursorOptions {
        memory: memory.clone(),
        working_set: memory.limit(),
        speed,
        refresh_hz: refresh,
    }
}
fn value<S: ValueStore>(values: &Values<S>, key: u64) -> RecordRef<'_> {
    let Lookup::Record(record) = values.get(&address(key)) else {
        panic!("selected record absent")
    };
    record
}
pub(crate) fn fixture(memory: &Memory, hours: Hour, keys: usize) -> Arc<InMemory> {
    let baseline = (0..keys)
        .map(|key| Row {
            address: address(key as u64),
            record: number(-1, -1.0),
        })
        .collect();
    let pages = (0..hours)
        .map(|hour| {
            page(
                hour as u64,
                hour,
                hour,
                (0..keys)
                    .map(|key| (hour, key as u64, number(hour, f64::from(hour) + key as f64)))
                    .collect(),
                vec![prepared(hour, hour)],
            )
        })
        .collect();
    Arc::new(InMemory::new(memory, -1, baseline, pages).unwrap())
}

#[test]
fn compact_frames_stream_large_spans_and_later_frames_recover_without_allocations() {
    let memory = Memory::new(32_000_000).unwrap();
    let source = fixture(&memory, 120, 100);
    let mut cursor =
        Cursor::open_with(&source, KeyFilter::All, 0, options(&memory, 1440.0, 15.0)).unwrap();
    cursor.wait_read_ahead().unwrap();
    assert_eq!(
        counted(|| {
            assert!(cursor.advance(96).unwrap().applied);
        }),
        0
    );
    assert_eq!(cursor.values().hour(), 96);
    assert_eq!(value(cursor.values(), 0).value, ValueRef::Number(96.0));
    cursor.wait_read_ahead().unwrap();
    assert!(cursor.advance(110).unwrap().applied);
    assert!(cursor.stats().window_bytes < 120 * 100 * std::mem::size_of::<Change>());
}
#[test]
fn a_late_part_reopens_at_the_clock_and_reports_drops_then_joins() {
    let memory = Memory::new(16_000_000).unwrap();
    let source = fixture(&memory, 110, 10);
    let mut cursor =
        Cursor::open_with(&source, KeyFilter::All, 0, options(&memory, 1.0, 1.0)).unwrap();
    cursor.wait_read_ahead().unwrap();
    assert_eq!(cursor.advance(100).unwrap().status, DeltaStatus::CatchingUp);
    assert_eq!(cursor.values().hour(), 0);
    cursor.wait_read_ahead().unwrap();
    let delta = cursor.advance(100).unwrap();
    assert!(delta.applied && delta.full_refresh);
    assert_eq!(value(cursor.values(), 3).value, ValueRef::Number(103.0));
    assert_eq!(cursor.stats().catch_up_drops, 1);
    assert!(cursor.advance(101).unwrap().applied);
}
#[test]
fn reverse_carries_one_version_through_many_pages_and_round_trips_creation_and_end() {
    let memory = Memory::new(16_000_000).unwrap();
    let pages = (0..20)
        .map(|hour| {
            page(
                hour as u64,
                hour,
                hour,
                if hour == 7 {
                    vec![(hour, 99, number(hour, 7.0))]
                } else if hour == 15 {
                    vec![(hour, 99, record(hour, Value::End))]
                } else {
                    vec![]
                },
                vec![prepared(hour, hour)],
            )
        })
        .collect();
    let source = Arc::new(InMemory::new(&memory, -1, vec![], pages).unwrap());
    let mut cursor =
        Cursor::open_with(&source, KeyFilter::All, 19, options(&memory, -10.0, 1.0)).unwrap();
    cursor.wait_read_ahead().unwrap();
    assert_eq!(source.versions.load(Ordering::Relaxed), 2);
    assert!(cursor.rewind(10).unwrap().applied);
    assert_eq!(value(cursor.values(), 99).value, ValueRef::Number(7.0));
    assert!(cursor.rewind(0).unwrap().applied);
    assert_eq!(value(cursor.values(), 99).value, ValueRef::End);
    cursor.playback(10.0, 1.0).unwrap();
    cursor.wait_read_ahead().unwrap();
    assert!(cursor.advance(10).unwrap().applied);
    assert_eq!(value(cursor.values(), 99).value, ValueRef::Number(7.0));
    assert!(cursor.advance(19).unwrap().applied);
    assert_eq!(value(cursor.values(), 99).value, ValueRef::End);
}
#[test]
fn splits_in_one_hour_restore_the_value_before_all_splits() {
    let memory = Memory::new(16_000_000).unwrap();
    let source = Arc::new(
        InMemory::new(
            &memory,
            -1,
            vec![
                Row {
                    address: address(0),
                    record: number(-1, 0.0),
                },
                Row {
                    address: address(1),
                    record: number(-1, 0.0),
                },
            ],
            vec![
                page(0, 0, 1, vec![(1, 0, number(1, 11.0))], vec![prepared(0, 1)]),
                page(
                    1,
                    1,
                    2,
                    vec![(1, 1, number(1, 12.0)), (2, 0, number(2, 21.0))],
                    vec![prepared(1, 2)],
                ),
            ],
        )
        .unwrap(),
    );
    let mut cursor =
        Cursor::open_with(&source, KeyFilter::All, 2, options(&memory, -10.0, 10.0)).unwrap();
    cursor.wait_read_ahead().unwrap();
    assert!(cursor.rewind(1).unwrap().applied);
    assert_eq!(value(cursor.values(), 0).value, ValueRef::Number(11.0));
    assert!(cursor.rewind(0).unwrap().applied);
    assert_eq!(value(cursor.values(), 0).value, ValueRef::Number(0.0));
    assert_eq!(value(cursor.values(), 1).value, ValueRef::Number(0.0));

    struct SplitEntry {
        info: PageInfo,
        coverage: Vec<Coverage>,
        runs: usize,
    }
    impl crate::window::Entry for SplitEntry {
        fn info(&self) -> PageInfo {
            self.info
        }
        fn bytes(&self) -> usize {
            0
        }
        fn coverage(&self) -> &[Coverage] {
            &self.coverage
        }
        fn coverage_runs(&self) -> usize {
            self.runs
        }
    }
    let split_memory = Memory::new(16_000_000).unwrap();
    let missing_hour_2 = Coverage {
        hours: Hours { first: 2, last: 2 },
        missing: Some(Missing {
            kind: MissingKind::NotRecorded,
            reason: Text::copy_from(&split_memory, Owner::State, "missing").unwrap(),
            hours: Hours { first: 2, last: 2 },
        }),
        accepted_before: 2,
    };
    let left_cov = vec![prepared(0, 1)];
    let right_cov = vec![prepared(1, 1), missing_hour_2.clone()];
    for order in [[0u64, 1], [1, 0]] {
        let mut tree = crate::window::Tree::default();
        for id in order {
            let (hours, coverage) = if id == 0 {
                (Hours { first: 0, last: 1 }, left_cov.clone())
            } else {
                (Hours { first: 1, last: 2 }, right_cov.clone())
            };
            let runs = crate::window::coverage_runs(&coverage);
            tree.insert(
                Arc::new(SplitEntry {
                    info: PageInfo {
                        id,
                        hours,
                        records: 0,
                        coverage: coverage.len(),
                        resident_bytes: 0,
                    },
                    coverage,
                    runs,
                }),
                &split_memory,
            )
            .unwrap();
        }
        assert_eq!(tree.coverage_runs_for(Hours { first: 0, last: 2 }), 2);
        assert_eq!(tree.coverage_runs_for(Hours { first: 1, last: 2 }), 2);
    }
    let split_source = Arc::new(
        InMemory::new(
            &split_memory,
            -1,
            vec![],
            vec![
                page(0, 0, 1, vec![], left_cov),
                page(1, 1, 2, vec![], right_cov),
            ],
        )
        .unwrap(),
    );
    for origin in [0, 1] {
        let mut split_cursor = Cursor::open_with(
            &split_source,
            KeyFilter::All,
            origin,
            options(&split_memory, 1.0, 1.0),
        )
        .unwrap();
        split_cursor.wait_read_ahead().unwrap();
        let delta = split_cursor.advance(2).unwrap();
        assert!(delta.applied);
        assert_eq!(delta.coverage().len(), 2);
        split_cursor.wait_read_ahead().unwrap();
        assert_eq!(
            split_memory.owner_used(Owner::Delta),
            2 * std::mem::size_of::<Coverage>()
        );
    }
}
#[test]
fn filter_selection_and_shared_pages_do_not_read_or_expand_a_second_copy() {
    let memory = Memory::new(32_000_000).unwrap();
    let source = fixture(&memory, 20, 100);
    let keys = KeyFilter::sorted(Arc::from([address(3), address(51)])).unwrap();
    let mut first =
        Cursor::open_with(&source, keys.clone(), 0, options(&memory, 10.0, 10.0)).unwrap();
    first.wait_read_ahead().unwrap();
    let reads = source.reads.load(Ordering::Relaxed);
    let mut second = Cursor::open_with(&source, keys, 0, options(&memory, 10.0, 10.0)).unwrap();
    second.wait_read_ahead().unwrap();
    assert_eq!(source.reads.load(Ordering::Relaxed), reads);
    assert!(first.advance(10).unwrap().applied);
    assert!(second.advance(10).unwrap().applied);
    assert_eq!(first.values().iter().count(), 2);
    assert!(matches!(
        first.values().get(&address(4)),
        Lookup::NotSelected
    ));
    assert!(first.values().store().resident_bytes() < 10_000);
}
#[test]
fn rates_use_accepted_hours_change_on_quiet_frames_and_keep_unknown_anchors() {
    let memory = Memory::new(16_000_000).unwrap();
    let missing = Missing {
        kind: MissingKind::Lost,
        reason: Text::copy_from(&memory, Owner::State, "lost hours").unwrap(),
        hours: Hours { first: 3, last: 4 },
    };
    let unknown = Missing {
        kind: MissingKind::Unknown,
        reason: Text::copy_from(&memory, Owner::State, "unknown prefix").unwrap(),
        hours: Hours {
            first: -100,
            last: -1,
        },
    };
    let rate = |total| {
        record(
            0,
            Value::Accumulating(Accumulation {
                total,
                rate: 2,
                anchor_hour: 0,
                accepted_at_anchor: 1,
                rate_began: 0,
            }),
        )
    };
    let source = Arc::new(
        InMemory::new(
            &memory,
            0,
            vec![
                Row {
                    address: address(0),
                    record: rate(Total::Known(10)),
                },
                Row {
                    address: address(1),
                    record: rate(Total::Missing(unknown.clone())),
                },
            ],
            vec![page(
                0,
                0,
                7,
                vec![(
                    6,
                    0,
                    record(
                        6,
                        Value::Accumulating(Accumulation {
                            total: Total::Known(18),
                            rate: 3,
                            anchor_hour: 6,
                            accepted_at_anchor: 5,
                            rate_began: 6,
                        }),
                    ),
                )],
                vec![
                    prepared(0, 2),
                    Coverage {
                        hours: missing.hours,
                        missing: Some(missing),
                        accepted_before: 3,
                    },
                    Coverage {
                        hours: Hours { first: 5, last: 7 },
                        missing: None,
                        accepted_before: 3,
                    },
                ],
            )],
        )
        .unwrap(),
    );
    let mut cursor =
        Cursor::open_with(&source, KeyFilter::All, 0, options(&memory, 10.0, 10.0)).unwrap();
    cursor.wait_read_ahead().unwrap();
    let delta = cursor.advance(2).unwrap();
    assert!(delta.applied);
    assert_eq!(delta.changes().len(), 2);
    assert_eq!(value(cursor.values(), 0).value, ValueRef::Fixed(14));
    assert!(cursor.advance(4).unwrap().applied);
    assert_eq!(value(cursor.values(), 0).value, ValueRef::Fixed(14));
    assert!(cursor.advance(7).unwrap().applied);
    assert_eq!(value(cursor.values(), 0).value, ValueRef::Fixed(21));
    assert_eq!(
        value(cursor.values(), 1).value,
        ValueRef::Missing(unknown.borrowed())
    );
    cursor.playback(-10.0, 10.0).unwrap();
    cursor.wait_read_ahead().unwrap();
    assert!(cursor.rewind(2).unwrap().applied);
    assert_eq!(value(cursor.values(), 0).value, ValueRef::Fixed(14));
}
#[test]
fn missing_values_keep_each_kind_reason_hours_and_provenance_in_both_directions() {
    let memory = Memory::new(16_000_000).unwrap();
    let mut changes = vec![];
    let mut expected = vec![];
    for (index, kind) in [
        MissingKind::Unknown,
        MissingKind::Refused,
        MissingKind::Lost,
        MissingKind::NotRecorded,
    ]
    .into_iter()
    .enumerate()
    {
        let hour = index as Hour + 1;
        let missing = Missing {
            kind,
            reason: Text::copy_from(&memory, Owner::State, &format!("reason {hour}")).unwrap(),
            hours: Hours {
                first: hour,
                last: hour,
            },
        };
        let mut record = record(hour, Value::Missing(missing));
        record.break_before = true;
        record.provenance.holder = Some(123.into());
        expected.push(record.clone());
        changes.push((hour, 0, record));
    }
    let source = Arc::new(
        InMemory::new(
            &memory,
            -1,
            vec![Row {
                address: address(0),
                record: number(-1, 42.0),
            }],
            vec![page(0, 0, 5, changes, vec![prepared(0, 5)])],
        )
        .unwrap(),
    );
    let mut cursor =
        Cursor::open_with(&source, KeyFilter::All, 0, options(&memory, 10.0, 10.0)).unwrap();
    cursor.wait_read_ahead().unwrap();
    for hour in 1..=4 {
        assert!(cursor.advance(hour).unwrap().applied);
        assert_eq!(
            value(cursor.values(), 0),
            expected[hour as usize - 1].borrowed()
        );
    }
    cursor.playback(-10.0, 10.0).unwrap();
    cursor.wait_read_ahead().unwrap();
    for hour in (1..4).rev() {
        assert!(cursor.rewind(hour).unwrap().applied);
        assert_eq!(
            value(cursor.values(), 0),
            expected[hour as usize - 1].borrowed()
        );
    }
}

#[derive(Default)]
struct Gate {
    state: Mutex<(bool, bool)>,
    changed: Condvar,
}
impl Gate {
    fn enter(&self) {
        let mut state = self.state.lock().unwrap();
        state.0 = true;
        self.changed.notify_all();
        while !state.1 {
            state = self.changed.wait(state).unwrap();
        }
    }
    fn entered(&self) {
        let mut state = self.state.lock().unwrap();
        while !state.0 {
            state = self.changed.wait(state).unwrap();
        }
    }
    fn release(&self) {
        self.state.lock().unwrap().1 = true;
        self.changed.notify_all();
    }
}
struct GateRelease(Arc<Gate>);
impl Drop for GateRelease {
    fn drop(&mut self) {
        self.0.release();
    }
}
struct SlowSource {
    inner: Arc<InMemory>,
    gate: Arc<Gate>,
    block: AtomicUsize,
    backing: usize,
    run_pages: u64,
}
impl LogSource for SlowSource {
    type State = crate::reference::State;
    type Decoder = crate::reference::EncodedDecoder;
    fn version_at(
        &self,
        keys: &KeyFilter,
        hour: Hour,
        scope: Hours,
        memory: &Memory,
    ) -> io::Result<Self::State> {
        self.inner.version_at(keys, hour, scope, memory)
    }
    fn version_before_page(
        &self,
        keys: &KeyFilter,
        page: PageInfo,
        scope: Hours,
        memory: &Memory,
    ) -> io::Result<Self::State> {
        let mut page = page;
        page.resident_bytes -= self.backing;
        self.inner.version_before_page(keys, page, scope, memory)
    }
    fn checkpoint_run(&self, page: PageInfo) -> u64 {
        if self.run_pages == 0 {
            self.inner.checkpoint_run(page)
        } else {
            page.id / self.run_pages
        }
    }
    fn pages(
        &self,
        keys: &KeyFilter,
        hours: Hours,
        visit: &mut dyn FnMut(PageInfo) -> ControlFlow<()>,
    ) -> io::Result<()> {
        self.inner.pages(keys, hours, &mut |mut page| {
            page.resident_bytes += self.backing;
            visit(page)
        })
    }
    fn read_page(&self, page: PageInfo, memory: &Memory) -> io::Result<Page<Self::Decoder>> {
        if page.id as usize == self.block.load(Ordering::Acquire) {
            self.gate.enter();
        }
        let mut original = page;
        original.resident_bytes -= self.backing;
        let mut result = self.inner.read_page(original, memory)?;
        result.info = page;
        if self.backing != 0 {
            result.backing = Some(memory.reserve(Owner::Forward, self.backing)?);
        }
        Ok(result)
    }
    fn coverage(
        &self,
        hours: Hours,
        visit: &mut dyn FnMut(Coverage) -> ControlFlow<()>,
    ) -> io::Result<()> {
        self.inner.coverage(hours, visit)
    }
}
#[test]
fn resident_frames_do_not_contend_with_a_blocked_background_reader() {
    let memory = Memory::new(16_000_000).unwrap();
    let gate = Arc::new(Gate::default());
    let source = Arc::new(SlowSource {
        inner: fixture(&memory, 10, 10),
        gate: gate.clone(),
        block: AtomicUsize::new(usize::MAX),
        backing: 0,
        run_pages: 0,
    });
    let mut cursor =
        Cursor::open_with(&source, KeyFilter::All, 0, options(&memory, 1.0, 10.0)).unwrap();
    cursor.wait_read_ahead().unwrap();
    source.block.store(3, Ordering::Release);
    cursor.playback(2.0, 10.0).unwrap();
    gate.entered();
    let mut applied = false;
    let allocations = counted(|| {
        applied = cursor.advance(1).unwrap().applied;
    });
    gate.release();
    assert!(applied);
    assert_eq!(allocations, 0);
    assert_eq!(cursor.values().hour(), 1);
}
#[test]
fn a_host_can_rebalance_the_default_working_set_and_shared_borrows_remain_charged() {
    let memory = Memory::new(PLAYBACK_BYTES + 1).unwrap();
    assert_eq!(memory.limit(), PLAYBACK_BYTES + 1);
    let bytes = Bytes::copy_from(&memory, Owner::Forward, &[1, 2, 3, 4]).unwrap();
    let slice = bytes.slice(1, 2).unwrap();
    let used = memory.used();
    drop(bytes);
    assert_eq!(memory.used(), used);
    assert_eq!(&*slice, &[2, 3]);
    drop(slice);
    assert_eq!(memory.used(), 0);
}
#[test]
fn publication_keeps_a_pinned_snapshot_whole_and_never_destroys_it_on_the_reader() {
    let published = crate::publish::Published::new();
    assert!(published.publish(Box::new(vec![1, 2, 3])).is_ok());
    let read = published.read();
    assert!(published.publish(Box::new(vec![4, 5])).is_ok());
    assert_eq!(read.get().unwrap(), &vec![1, 2, 3]);
    assert!(published.publish(Box::new(vec![6])).is_err());
    drop(read);
    assert!(published.collect().is_some());
    assert_eq!(published.read().get().unwrap(), &vec![4, 5]);
    let read = published.read();
    assert!(published.clear());
    assert!(published.read().get().is_none());
    assert_eq!(read.get().unwrap(), &vec![4, 5]);
    assert!(published.collect().is_none());
    drop(read);
    assert!(published.collect().is_some());
}
#[test]
fn a_small_cursor_allowance_shortens_pages_instead_of_refusing_playback() {
    let memory = Memory::new(20_000_000).unwrap();
    let source = fixture(&memory, 100, 1000);
    let mut config = options(&memory, 1440.0, 95.0);
    config.working_set = 300_000;
    let mut cursor = Cursor::open_with(&source, KeyFilter::All, 0, config).unwrap();
    cursor.wait_read_ahead().unwrap();
    assert!(cursor.stats().shortened_windows > 0);
    assert!(cursor.stats().window_wall_seconds < 2.0);
    assert!(cursor.advance(1).unwrap().applied);
    assert!(memory.peak() <= memory.limit());
}

#[test]
fn reversal_at_full_memory_releases_the_obsolete_window_and_shortens_inverse_chunks() {
    let memory = Memory::new(4_000_000).unwrap();
    let source = Arc::new(SlowSource {
        inner: fixture(&memory, 40, 10),
        gate: Arc::new(Gate::default()),
        block: AtomicUsize::new(usize::MAX),
        backing: 4096,
        run_pages: 0,
    });
    let fixture_bytes = memory.owner_used(Owner::Forward);
    let mut cursor =
        Cursor::open_with(&source, KeyFilter::All, 16, options(&memory, 10.0, 10.0)).unwrap();
    cursor.wait_read_ahead().unwrap();
    let obsolete_bytes = memory.owner_used(Owner::Forward);
    assert!(obsolete_bytes > fixture_bytes);
    let pressure = memory
        .soft()
        .reserve(Owner::Transient, memory.limit() - memory.used())
        .unwrap();
    assert_eq!(memory.used(), memory.limit());
    cursor.playback(-10.0, 10.0).unwrap();
    cursor.wait_read_ahead().unwrap();
    assert!(cursor.stats().inverse_bytes > 0);
    assert!(cursor.stats().shortened_windows > 0);
    assert_eq!(memory.owner_used(Owner::Forward), fixture_bytes);
    assert_eq!(counted(|| assert!(cursor.rewind(15).unwrap().applied)), 0);
    assert_eq!(value(cursor.values(), 3).value, ValueRef::Number(18.0));
    assert!(memory.peak() <= memory.limit());
    drop(pressure);
}

#[test]
fn scope_switch_at_full_memory_reclaims_its_own_far_edge_without_catching_up() {
    let memory = Memory::new(16_000_000).unwrap();
    let source = Arc::new(SlowSource {
        inner: fixture(&memory, 16, 1000),
        gate: Arc::new(Gate::default()),
        block: AtomicUsize::new(usize::MAX),
        backing: 1_048_576,
        run_pages: 0,
    });
    let mut cursor =
        Cursor::open_with(&source, KeyFilter::All, 0, options(&memory, 2.0, 2.0)).unwrap();
    cursor.wait_read_ahead().unwrap();
    let before = cursor.stats();
    assert_eq!(before.window_hours, 4);
    let headroom = 4096;
    let _pressure = memory
        .soft()
        .reserve(Owner::Transient, memory.limit() - memory.used() - headroom)
        .unwrap();
    assert_eq!(memory.limit() - memory.used(), headroom);
    assert!(
        cursor
            .working_memory()
            .soft()
            .reserve(Owner::State, headroom + 1)
            .is_err()
    );
    cursor.playback(5.0, 5.0).unwrap();
    cursor.wait_read_ahead().unwrap();
    let after = cursor.stats();
    assert_eq!(after.scope_changes, before.scope_changes + 1);
    assert!(after.window_bytes < before.window_bytes);
    assert!(after.shortened_windows > before.shortened_windows);
    let delta = cursor.advance(3).unwrap();
    assert!(delta.applied && delta.scope_switch && !delta.full_refresh);
    assert_eq!(value(cursor.values(), 3).value, ValueRef::Number(6.0));
    cursor.wait_read_ahead().unwrap();
    assert_eq!(cursor.stats().catch_ups, 0);
    assert_eq!(cursor.stats().catch_up_drops, 0);
    assert!(memory.peak() <= memory.limit());
}

#[test]
fn a_resident_span_that_grows_after_a_miss_gets_a_larger_workspace() {
    let memory = Memory::new(16_000_000).unwrap();
    let source = Arc::new(
        InMemory::new(
            &memory,
            -1,
            vec![],
            (0..20)
                .map(|hour| {
                    page(
                        hour as u64,
                        hour,
                        hour,
                        if hour == 7 {
                            vec![(hour, 0, number(hour, 7.0))]
                        } else {
                            vec![]
                        },
                        vec![prepared(hour, hour)],
                    )
                })
                .collect(),
        )
        .unwrap(),
    );
    let mut cursor =
        Cursor::open_with(&source, KeyFilter::All, 19, options(&memory, -10.0, 10.0)).unwrap();
    cursor.wait_read_ahead().unwrap();
    assert!(!cursor.rewind(5).unwrap().applied);
    assert_eq!(cursor.stats().capacity_misses, 1);
    cursor.wait_read_ahead().unwrap();
    assert!(cursor.rewind(4).unwrap().applied);
    assert_eq!(cursor.values().hour(), 4);
}
#[test]
fn oom_workspace_retry_drops_unclaimed_maxima_and_fits_the_new_window() {
    let memory = Memory::new(64_000_000).unwrap();
    let keys = 50_000usize;
    let pairs = 550_000 / (2 * std::mem::size_of::<Coverage>());
    let half = (pairs * 2) as Hour;
    let ended = Record {
        hour: 2,
        value: Value::End,
        provenance: Provenance::default(),
        break_before: false,
    };
    let mut dense_coverage = Vec::with_capacity(pairs * 2);
    let reason = Text::copy_from(&memory, Owner::State, "").unwrap();
    for pair in 0..pairs {
        let even = half + 1 + (pair * 2) as Hour;
        let odd = even + 1;
        dense_coverage.push(Coverage {
            hours: Hours { first: even, last: even },
            missing: None,
            accepted_before: (half + 1) as u64 + pair as u64,
        });
        dense_coverage.push(Coverage {
            hours: Hours { first: odd, last: odd },
            missing: Some(Missing {
                kind: MissingKind::NotRecorded,
                reason: reason.clone(),
                hours: Hours { first: odd, last: odd },
            }),
            accepted_before: (half + 1) as u64 + pair as u64 + 1,
        });
    }
    let last_hour = half * 2;
    let pages = vec![
        page(0, 0, 0, vec![], vec![prepared(0, 0)]),
        page(1, 1, 1, vec![], vec![prepared(1, 1)]),
        page(
            2,
            2,
            2,
            (0..keys).map(|key| (2, key as u64, ended.clone())).collect(),
            vec![prepared(2, 2)],
        ),
        page(3, 3, half, vec![], vec![prepared(3, half)]),
        page(4, half + 1, last_hour, vec![], dense_coverage),
    ];
    let source = Arc::new(InMemory::new(&memory, -1, vec![], pages).unwrap());
    let mut page4 = None;
    source
        .pages(
            &KeyFilter::All,
            Hours {
                first: half + 1,
                last: half + 1,
            },
            &mut |info| {
                page4 = Some(info);
                ControlFlow::Break(())
            },
        )
        .unwrap();
    let page4 = page4.unwrap();
    let open_speed = f64::from(half) / 2.0;
    let mut cursor = Cursor::open_with(
        &source,
        KeyFilter::All,
        0,
        options(&memory, open_speed, open_speed / 2.0),
    )
    .unwrap();
    cursor.wait_read_ahead().unwrap();
    let window_bytes = cursor.stats().window_bytes;
    let coverage_bytes = (pairs * 2 + 1) * std::mem::size_of::<Coverage>();
    let changes_bytes = keys * std::mem::size_of::<usize>();
    assert!(window_bytes + 32_768 < coverage_bytes);
    assert!(coverage_bytes + 32_768 < window_bytes + changes_bytes);
    let headroom = window_bytes + page4.resident_bytes + 32_768;
    let _pressure = memory
        .soft()
        .reserve(Owner::Transient, memory.limit() - memory.used() - headroom)
        .unwrap();
    cursor.playback(f64::from(half), f64::from(half)).unwrap();
    cursor.wait_read_ahead().unwrap();
    assert_eq!(cursor.stats().scope_changes, 0);
    assert_eq!(cursor.stats().shortened_windows, 0);
    assert_eq!(cursor.stats().window_hours, last_hour as u64);
    assert!(cursor.advance(1).unwrap().applied);
    assert_eq!(cursor.stats().capacity_misses, 0);
    assert!(memory.peak() <= memory.limit());
}
#[test]
fn serving_a_short_prefix_of_a_dense_coverage_page_sizes_the_workspace_to_served_runs() {
    let memory = Memory::new(16_000_000).unwrap();
    let pairs = 5_000usize;
    let reason = Text::copy_from(&memory, Owner::State, "").unwrap();
    let mut dense_coverage = Vec::with_capacity(pairs * 2);
    for pair in 0..pairs {
        let even = (pair * 2) as Hour;
        let odd = even + 1;
        dense_coverage.push(Coverage {
            hours: Hours { first: even, last: even },
            missing: None,
            accepted_before: pair as u64,
        });
        dense_coverage.push(Coverage {
            hours: Hours { first: odd, last: odd },
            missing: Some(Missing {
                kind: MissingKind::NotRecorded,
                reason: reason.clone(),
                hours: Hours { first: odd, last: odd },
            }),
            accepted_before: pair as u64 + 1,
        });
    }
    let last_hour = (pairs * 2) as Hour - 1;
    let source = Arc::new(
        InMemory::new(
            &memory,
            -1,
            vec![],
            vec![page(0, 0, last_hour, vec![], dense_coverage)],
        )
        .unwrap(),
    );
    let mut page0 = None;
    source
        .pages(&KeyFilter::All, Hours { first: 0, last: 0 }, &mut |info| {
            page0 = Some(info);
            ControlFlow::Break(())
        })
        .unwrap();
    let page0 = page0.unwrap();
    let full_page_workspace = pairs * 2 * std::mem::size_of::<Coverage>();
    assert!(full_page_workspace > 200_000);
    let headroom = page0.resident_bytes + 256 * 1024 + 32_768;
    let _pressure = memory
        .soft()
        .reserve(Owner::Transient, memory.limit() - memory.used() - headroom)
        .unwrap();
    let mut cursor =
        Cursor::open_with(&source, KeyFilter::All, 0, options(&memory, 1.0, 1.0)).unwrap();
    cursor.wait_read_ahead().unwrap();
    assert_eq!(cursor.stats().shortened_windows, 0);
    assert_eq!(cursor.stats().window_hours, 2);
    assert!(cursor.advance(2).unwrap().applied);
    assert_eq!(cursor.stats().capacity_misses, 0);
    assert!(memory.peak() <= memory.limit());
}
#[test]
fn reverse_chunk_with_an_internal_page_gap_is_rejected_before_publication() {
    let memory = Memory::new(16_000_000).unwrap();
    let source = Arc::new(
        InMemory::new(
            &memory,
            -1,
            vec![],
            vec![
                page(0, 0, 1, vec![], vec![prepared(0, 1)]),
                page(1, 3, 4, vec![], vec![prepared(3, 4)]),
            ],
        )
        .unwrap(),
    );
    let cursor =
        Cursor::open_with(&source, KeyFilter::All, 4, options(&memory, -4.0, 2.0)).unwrap();
    let error = cursor.wait_read_ahead().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(cursor.stats().window_hours, 0);
}
#[test]
fn dictionary_slots_preserve_identities_wider_than_a_machine_word() {
    let memory = Memory::new(8_000_000).unwrap();
    let mut bytes = vec![0x81; 40];
    bytes.push(1);
    let key = Address {
        key: Key::from_varint_in(&memory, &bytes).unwrap(),
        field: 0.into(),
    };
    let source = Arc::new(
        InMemory::new(
            &memory,
            -1,
            vec![],
            vec![MemoryPage {
                id: 0,
                hours: Hours { first: 0, last: 2 },
                changes: vec![Change {
                    hour: 1,
                    address: key.clone(),
                    record: number(1, 3.0),
                }],
                coverage: vec![prepared(0, 2)],
            }],
        )
        .unwrap(),
    );
    let mut cursor =
        Cursor::open_with(&source, KeyFilter::All, 0, options(&memory, 1.0, 1.0)).unwrap();
    cursor.wait_read_ahead().unwrap();
    assert!(cursor.advance(1).unwrap().applied);
    let Lookup::Record(record) = cursor.values().get(&key) else {
        panic!("wide identity lost")
    };
    assert_eq!(record.value, ValueRef::Number(3.0));
}

#[test]
fn clock_parts_advance_independently_and_a_late_seek_part_joins_the_clock() {
    let mut clock = Clock::new(Duration::ZERO, 0.0, 1440.0, false).unwrap();
    let map = clock.register().unwrap();
    let widget = clock.register().unwrap();
    let seek = clock.seek(Duration::from_secs(1), 10_000.0).unwrap();
    assert!(seek.paused);
    let mut seeking = Part::new(seek, 95.0, 0.0).unwrap();
    assert!(!seeking.present(10_001.0, Duration::from_millis(1005), true));
    assert_eq!(seeking.drawn(), None);
    assert_eq!(seeking.ready_generation(), None);
    assert!(seeking.present(10_000.0, Duration::from_millis(1006), true));
    assert_eq!(seeking.ready_generation(), Some(seek.generation));
    assert!(
        clock
            .ready(map, seek.generation, Duration::from_millis(1010))
            .unwrap()
            .is_none()
    );
    assert!(clock.poll(Duration::from_millis(1099)).unwrap().is_none());
    let resumed = clock.poll(Duration::from_millis(1100)).unwrap().unwrap();
    assert!(!resumed.paused);
    let mut fast = Part::new(resumed, 95.0, 0.0).unwrap();
    let mut late = Part::new(resumed, 144.0, -0.25).unwrap();
    let present = Duration::from_millis(1200);
    let hour = fast.hour_at(present);
    assert!(fast.present(hour, present, true));
    assert!(!late.present(10_000.0, Duration::from_millis(1450), true));
    assert_eq!(late.dropped(), 1);
    let joined_hour = late.hour_at(Duration::from_millis(1450));
    assert!((joined_hour - hour).abs() < 1e-8);
    assert!(late.present(joined_hour, Duration::from_millis(1450), true));
    assert!(
        clock
            .ready(widget, seek.generation, Duration::from_millis(1300))
            .unwrap()
            .is_none()
    );
    let saved = clock.anchor();
    for step in 1..100 {
        let time = present + Duration::from_millis(step * 10);
        assert!(fast.present(fast.hour_at(time), time, true));
        late.drop_frame();
    }
    assert_eq!(clock.anchor(), saved);
    assert_eq!(fast.dropped(), 0);
}

#[test]
fn clock_resumes_on_readiness_rejects_old_generations_and_keeps_speed_pause_continuous() {
    let mut clock = Clock::new(Duration::ZERO, 0.0, 10.0, false).unwrap();
    let first = clock.register().unwrap();
    let second = clock.register().unwrap();
    let old = clock.seek(Duration::from_secs(1), 50.0).unwrap();
    let current = clock.seek(Duration::from_secs(2), 100.0).unwrap();
    assert!(
        clock
            .ready(first, old.generation, Duration::from_millis(2010))
            .unwrap()
            .is_none()
    );
    assert!(
        clock
            .ready(second, current.generation, Duration::from_millis(2020))
            .unwrap()
            .is_none()
    );
    let resume = clock
        .ready(first, current.generation, Duration::from_millis(2030))
        .unwrap()
        .unwrap();
    assert_eq!(resume.t0, Duration::from_millis(2030));
    let speed = clock.speed(Duration::from_millis(3030), -20.0).unwrap();
    assert_eq!(speed.h0, 110.0);
    let paused = clock.pause(Duration::from_millis(4030), true).unwrap();
    assert_eq!(paused.h0, 90.0);
    assert_eq!(
        paused.hour_at(PresentTime {
            local: Duration::from_secs(50),
            host_offset_seconds: 0.0
        }),
        90.0
    );
    assert!(
        counted(|| {
            for step in 0..100 {
                let _ = paused.hour_at(PresentTime {
                    local: Duration::from_secs(step),
                    host_offset_seconds: 0.0,
                });
            }
        }) == 0
    );
}

#[test]
fn budgets_follow_the_parts_display_and_present_time_includes_its_period() {
    for refresh in [95.0, 144.0] {
        let budgets = FrameBudgets::from_refresh(refresh).unwrap();
        let t = 1.0 / refresh;
        for (actual, share) in [
            (budgets.widget_slot, 0.03),
            (budgets.widget_advance, 0.01),
            (budgets.widget_render, 0.02),
            (budgets.map_decode, 0.15),
            (budgets.map_render, 0.20),
        ] {
            assert!((actual.as_secs_f64() - t * share).abs() <= 1e-9);
        }
        let present = PresentTime::next(Duration::from_secs(5), -1.0, refresh).unwrap();
        let anchor = Anchor {
            t0: Duration::from_secs(4),
            h0: 0.0,
            speed: 1440.0,
            paused: false,
            generation: 0,
        };
        assert!((anchor.hour_at(present) - 1440.0 * t).abs() < 1e-5);
    }
    for refresh in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(FrameBudgets::from_refresh(refresh).is_err());
    }
}

#[test]
fn late_clock_controls_use_the_resumed_hour_and_delayed_seek_anchors_cannot_pause_it() {
    let mut clock = Clock::new(Duration::ZERO, 0.0, 10.0, false).unwrap();
    clock.register().unwrap();
    clock.seek(Duration::ZERO, 100.0).unwrap();
    let changed = clock.speed(Duration::from_millis(150), 20.0).unwrap();
    assert!(!changed.paused);
    assert_eq!(changed.h0, 100.5);
    assert_eq!(
        changed.hour_at(PresentTime {
            local: Duration::from_millis(200),
            host_offset_seconds: 0.0
        }),
        101.5
    );
    let mut clock = Clock::new(Duration::ZERO, 0.0, 10.0, false).unwrap();
    let id = clock.register().unwrap();
    let seek = clock.seek(Duration::ZERO, 200.0).unwrap();
    let resumed = clock
        .ready(id, seek.generation, Duration::ZERO)
        .unwrap()
        .unwrap();
    let mut part = Part::new(resumed, 95.0, 0.0).unwrap();
    assert!(!part.anchor(seek).unwrap());
    assert_eq!(part.hour_at(Duration::from_secs(1)), 210.0);
    clock.seek(Duration::from_secs(1), 300.0).unwrap();
    let paused = clock.pause(Duration::from_millis(1150), true).unwrap();
    assert_eq!(paused.h0, 300.5);
    assert!(paused.paused);
}

#[test]
fn an_underrun_inside_the_previous_window_recovers_once_and_plays_on_both_ways() {
    for reverse in [false, true] {
        let memory = Memory::new(16_000_000).unwrap();
        let source = fixture(&memory, 20, 10);
        let origin = if reverse { 10 } else { 0 };
        let target = if reverse { 6 } else { 4 };
        let mut cursor = Cursor::open_with(
            &source,
            KeyFilter::All,
            origin,
            options(&memory, if reverse { -1.0 } else { 1.0 }, 1.0),
        )
        .unwrap();
        cursor.wait_read_ahead().unwrap();
        let delta = if reverse {
            cursor.rewind(target)
        } else {
            cursor.advance(target)
        }
        .unwrap();
        assert_eq!(delta.status, DeltaStatus::CatchingUp);
        cursor.wait_read_ahead().unwrap();
        let delta = if reverse {
            cursor.rewind(target)
        } else {
            cursor.advance(target)
        }
        .unwrap();
        assert!(delta.applied && delta.full_refresh);
        let next = if reverse { target - 1 } else { target + 1 };
        assert!(
            if reverse {
                cursor.rewind(next)
            } else {
                cursor.advance(next)
            }
            .unwrap()
            .applied
        );
        assert_eq!(
            value(cursor.values(), 3).value,
            ValueRef::Number(f64::from(next) + 3.0)
        );
        cursor.wait_read_ahead().unwrap();
        assert_eq!(cursor.stats().catch_ups, 1);
        assert_eq!(cursor.stats().catch_up_drops, 1);
    }
}
#[test]
fn reverse_catch_up_into_a_quiet_frame_with_an_active_rate_reserves_its_change_slot() {
    let memory = Memory::new(16_000_000).unwrap();
    let pages = (0..=100)
        .map(|hour| {
            let changes = if hour == 17 {
                vec![(
                    17,
                    0,
                    record(
                        17,
                        Value::Accumulating(Accumulation {
                            total: Total::Known(10),
                            rate: 2,
                            anchor_hour: 17,
                            accepted_at_anchor: 18,
                            rate_began: 17,
                        }),
                    ),
                )]
            } else if hour == 30 {
                vec![(30, 0, number(30, 100.0))]
            } else {
                vec![]
            };
            page(hour as u64, hour, hour, changes, vec![prepared(hour, hour)])
        })
        .collect();
    let source = Arc::new(
        InMemory::new(
            &memory,
            -1,
            vec![Row {
                address: address(0),
                record: number(-1, 0.0),
            }],
            pages,
        )
        .unwrap(),
    );
    let mut cursor =
        Cursor::open_with(&source, KeyFilter::All, 100, options(&memory, -2.0, 2.0)).unwrap();
    cursor.wait_read_ahead().unwrap();
    assert_eq!(cursor.rewind(20).unwrap().status, DeltaStatus::CatchingUp);
    cursor.wait_read_ahead().unwrap();
    let delta = cursor.rewind(20).unwrap();
    assert!(delta.applied && delta.full_refresh);
    assert_eq!(cursor.values().hour(), 20);
    assert_eq!(value(cursor.values(), 0).value, ValueRef::Fixed(16));
    let delta = cursor.rewind(19).unwrap();
    assert!(delta.applied && !delta.full_refresh);
    assert_eq!(delta.changes().len(), 1);
    assert_eq!(value(cursor.values(), 0).value, ValueRef::Fixed(14));
    assert_eq!(cursor.stats().catch_ups, 1);
}
#[test]
fn a_catch_up_joins_before_the_rest_of_its_window_is_read() {
    for reverse in [false, true] {
        let memory = Memory::new(16_000_000).unwrap();
        let gate = Arc::new(Gate::default());
        let source = Arc::new(SlowSource {
            inner: fixture(&memory, 30, 10),
            gate: gate.clone(),
            block: AtomicUsize::new(usize::MAX),
            backing: 0,
            run_pages: 1,
        });
        let origin = if reverse { 20 } else { 0 };
        let target = if reverse { 12 } else { 8 };
        let next = if reverse { 11 } else { 9 };
        let mut cursor = Cursor::open_with(
            &source,
            KeyFilter::All,
            origin,
            options(&memory, if reverse { -2.0 } else { 2.0 }, 2.0),
        )
        .unwrap();
        cursor.wait_read_ahead().unwrap();
        let _release = GateRelease(gate.clone());
        source
            .block
            .store(if reverse { 9 } else { 11 }, Ordering::Release);
        let delta = if reverse {
            cursor.rewind(target)
        } else {
            cursor.advance(target)
        }
        .unwrap();
        assert_eq!(delta.status, DeltaStatus::CatchingUp);
        gate.entered();
        assert_eq!(
            counted(|| {
                let delta = if reverse {
                    cursor.rewind(next)
                } else {
                    cursor.advance(next)
                }
                .unwrap();
                assert!(delta.applied && delta.full_refresh);
            }),
            0
        );
        assert_eq!(cursor.values().hour(), next);
        assert_eq!(
            value(cursor.values(), 3).value,
            ValueRef::Number(f64::from(next) + 3.0)
        );
        assert_eq!(cursor.stats().catch_ups, 1);
        gate.release();
        cursor.wait_read_ahead().unwrap();
    }
}
#[test]
fn reverse_publishes_the_nearest_checkpoint_or_frame_chunk_while_the_next_chunk_is_blocked() {
    let memory = Memory::new(16_000_000).unwrap();
    let gate = Arc::new(Gate::default());
    let source = Arc::new(SlowSource {
        inner: fixture(&memory, 30, 10),
        gate: gate.clone(),
        block: AtomicUsize::new(usize::MAX),
        backing: 0,
        run_pages: 4,
    });
    let mut cursor =
        Cursor::open_with(&source, KeyFilter::All, 10, options(&memory, 5.0, 5.0)).unwrap();
    cursor.wait_read_ahead().unwrap();
    let _release = GateRelease(gate.clone());
    source.block.store(4, Ordering::Release);
    cursor.playback(-5.0, 5.0).unwrap();
    gate.entered();
    assert!(cursor.rewind(9).unwrap().applied);
    assert!(cursor.rewind(8).unwrap().applied);
    assert_eq!(cursor.rewind(7).unwrap().status, DeltaStatus::NotResident);
    assert_eq!(cursor.rewind(6).unwrap().status, DeltaStatus::NotResident);
    assert_eq!(cursor.stats().catch_up_drops, 0);
    gate.release();
    cursor.wait_read_ahead().unwrap();
    assert!(cursor.rewind(6).unwrap().applied);
    assert_eq!(cursor.stats().catch_ups, 0);
}
#[test]
fn a_sources_decoder_backing_stays_charged_beside_the_cursor_page() {
    let mut charged = vec![];
    for backing in [0, 4096] {
        let memory = Memory::new(16_000_000).unwrap();
        let source = Arc::new(SlowSource {
            inner: fixture(&memory, 1, 10),
            gate: Arc::new(Gate::default()),
            block: AtomicUsize::new(usize::MAX),
            backing,
            run_pages: 0,
        });
        let cursor =
            Cursor::open_with(&source, KeyFilter::All, 0, options(&memory, 1.0, 1.0)).unwrap();
        cursor.wait_read_ahead().unwrap();
        charged.push(memory.owner_used(Owner::Forward));
    }
    assert_eq!(charged[1] - charged[0], 4096);
}
#[test]
fn prepaid_page_bytes_cannot_be_taken_between_admission_and_read() {
    let memory = Memory::new(10000).unwrap();
    let credit = memory.reserve(Owner::Transient, 9000).unwrap();
    let prepaid = Memory::prepaid(credit).unwrap();
    assert!(memory.reserve(Owner::State, 1001).is_err());
    let scratch = Buffer::<u8>::new(&prepaid, Owner::Transient, 512).unwrap();
    drop(scratch);
    let bytes = Buffer::<u8>::new(&prepaid, Owner::Forward, 9000).unwrap();
    drop(prepaid);
    assert_eq!(memory.used(), 9000);
    assert_eq!(memory.owner_used(Owner::Forward), 9000);
    drop(bytes);
    assert_eq!(memory.used(), 0);
}
#[test]
fn fixed_point_anchors_keep_units_that_floating_point_cannot_represent() {
    let record = record(
        0,
        Value::Accumulating(Accumulation {
            total: Total::Known(9_007_199_254_740_993),
            rate: 3,
            anchor_hour: 0,
            accepted_at_anchor: 1,
            rate_began: 0,
        }),
    );
    let mut bytes = [0; 128];
    let mut writer = crate::codec::Writer::new(Some(&mut bytes));
    writer.record(record.borrowed(), 0).unwrap();
    let len = writer.len;
    let restored = crate::codec::Reader::new(&bytes[..len]).record(0).unwrap();
    assert_eq!(
        restored.evaluated(4).value,
        ValueRef::Fixed(9_007_199_254_741_002)
    );
    assert_eq!(
        restored.evaluated(0).value,
        ValueRef::Fixed(9_007_199_254_740_990)
    );
}
fn shaped_fixture(
    shape: crate::reference::Shape,
    memory: &Memory,
) -> (Arc<crate::reference::ShapedLog>, std::path::PathBuf) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join("hoi4-core-cursor");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join(format!(
        "unit-{}-{}-{}.checkpoints",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    (
        Arc::new(crate::reference::ShapedLog::create(shape, memory, &path).unwrap()),
        path,
    )
}
#[test]
fn shaped_columns_gallop_spaced_keys_and_resume_at_hour_run_starts() {
    let memory = Memory::new(16_000_000).unwrap();
    let (source, path) = shaped_fixture(
        crate::reference::Shape {
            hours: 100,
            keys: 2000,
            changes_per_tick: 1800,
            tick_hours: 1,
            lifetimes: false,
            live_units: 0,
            churn_per_tick: 0,
        },
        &memory,
    );
    let span = Hours { first: 5, last: 8 };
    let mut info = None;
    source
        .pages(&KeyFilter::All, span, &mut |page| {
            info = Some(page);
            ControlFlow::Break(())
        })
        .unwrap();
    let page = source.read_page(info.unwrap(), &memory).unwrap();
    let mut ids = Buffer::new(&memory, Owner::State, 5).unwrap();
    for id in [0, 51, 510, 1000, 1999] {
        ids.push(id);
    }
    let selection = Selection::Ids(ids);
    let mut expected = vec![];
    page.decoder
        .decode(
            &page.bytes,
            span,
            &Selection::All { len: 2000 },
            &mut |hour, id, record| {
                if let Some(slot) = selection.slot(id) {
                    expected.push((hour, slot, record));
                }
                Ok(())
            },
        )
        .unwrap();
    let mut actual = vec![];
    page.decoder
        .decode(&page.bytes, span, &selection, &mut |hour, slot, record| {
            actual.push((hour, slot, record));
            Ok(())
        })
        .unwrap();
    assert!(!actual.is_empty());
    assert_eq!(actual, expected);
    drop(page);
    drop(source);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn map_fixture_packs_600_000_keys_and_prepared_states_share_their_key_storage() {
    let memory = Memory::new(128_000_000).unwrap();
    let (source, path) = shaped_fixture(
        crate::reference::Shape {
            hours: 168,
            keys: 600_000,
            changes_per_tick: 3000,
            tick_hours: 1,
            lifetimes: true,
            live_units: 540_000,
            churn_per_tick: 10,
        },
        &memory,
    );
    assert!(
        memory.owner_used(Owner::Metadata) < 600_000 * (std::mem::size_of::<Address>() + 4) + 4096
    );
    for id in [0, 127, 128, 16_383, 16_384, 599_999] {
        assert_eq!(source.addresses()[id].key, Key::from(id as u64));
    }
    let state = source
        .version_at(
            &KeyFilter::All,
            100,
            Hours {
                first: 90,
                last: 110,
            },
            &memory,
        )
        .unwrap();
    assert!(memory.owner_used(Owner::State) < state.selection().len() * 64);
    assert!(matches!(state.get(&address(601_000)), Lookup::Record(_)));
    drop(state);
    drop(source);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn resident_window_bytes_are_actual_page_charges_with_the_directory_bound_reported_separately() {
    let memory = Memory::new(16_000_000).unwrap();
    let (source, path) = shaped_fixture(
        crate::reference::Shape {
            hours: 12,
            keys: 2000,
            changes_per_tick: 1800,
            tick_hours: 1,
            lifetimes: false,
            live_units: 0,
            churn_per_tick: 0,
        },
        &memory,
    );
    let cursor = Cursor::open_with(
        &source,
        source.filter(10).unwrap(),
        0,
        options(&memory, 2.0, 2.0),
    )
    .unwrap();
    cursor.wait_read_ahead().unwrap();
    let stats = cursor.stats();
    assert_eq!(stats.window_bytes, memory.owner_used(Owner::Forward));
    assert!(stats.window_bytes < stats.directory_bound_bytes);
    drop(cursor);
    assert_eq!(memory.owner_used(Owner::Forward), 0);
    drop(source);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn map_slots_follow_live_units_and_window_ids_instead_of_all_campaign_addresses() {
    let memory = Memory::new(16_000_000).unwrap();
    let (source, path) = shaped_fixture(
        crate::reference::Shape {
            hours: 10_000,
            keys: 400,
            changes_per_tick: 40,
            tick_hours: 1,
            lifetimes: true,
            live_units: 1,
            churn_per_tick: 1,
        },
        &memory,
    );
    let state = source
        .version_at(
            &KeyFilter::All,
            200,
            Hours {
                first: 180,
                last: 220,
            },
            &memory,
        )
        .unwrap();
    let Lookup::Record(ended) = state.get(&address(579)) else {
        panic!("retired scoped unit omitted")
    };
    assert_eq!(ended.hour, 180);
    assert_eq!(ended.value, ValueRef::End);
    drop(state);
    let mut sizes = vec![];
    for origin in [100, 9000] {
        let mut cursor = Cursor::open_with(
            &source,
            KeyFilter::All,
            origin,
            options(&memory, 10.0, 10.0),
        )
        .unwrap();
        cursor.wait_read_ahead().unwrap();
        sizes.push(cursor.values().store().resident_bytes());
        assert!(cursor.values().iter().count() < 450);
        assert!(matches!(
            cursor.values().get(&address(400)),
            Lookup::NotSelected
        ));
        let mut switches = 0;
        for hour in origin + 1..=origin + 60 {
            let before = cursor
                .values()
                .slots()
                .map(|(slot, a, _)| (a.clone(), slot))
                .collect::<std::collections::BTreeMap<_, _>>();
            assert_eq!(
                counted(|| {
                    let delta = cursor.advance(hour).unwrap();
                    assert!(delta.applied && !delta.full_refresh);
                    assert!(delta.changes().len() <= 40);
                    switches += usize::from(delta.scope_switch);
                }),
                0
            );
            for (slot, a, _) in cursor.values().slots() {
                if let Some(previous) = before.get(a) {
                    assert_eq!(slot, *previous);
                }
            }
            assert!(matches!(
                cursor.values().get(&address((400 + hour) as u64)),
                Lookup::Record(RecordRef {
                    value: ValueRef::Number(_),
                    ..
                })
            ));
            cursor.wait_read_ahead().unwrap();
        }
        assert!(cursor.stats().scope_changes > 0 && switches > 0);
        cursor.playback(-10.0, 10.0).unwrap();
        cursor.wait_read_ahead().unwrap();
        assert_eq!(memory.owner_used(Owner::Forward), 0);
        assert!(cursor.rewind(origin + 59).unwrap().applied);
    }
    assert_eq!(sizes[0], sizes[1]);
    drop(source);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn dense_reverse_replays_chunks_across_individual_block_versions() {
    let memory = Memory::new(64_000_000).unwrap();
    let (source, path) = shaped_fixture(
        crate::reference::Shape {
            hours: 48,
            keys: 51_000,
            changes_per_tick: 45_900,
            tick_hours: 1,
            lifetimes: false,
            live_units: 0,
            churn_per_tick: 0,
        },
        &memory,
    );
    let keys = source.filter(1000).unwrap();
    let mut cursor =
        Cursor::open_with(&source, keys.clone(), 30, options(&memory, -5.0, 1.0)).unwrap();
    cursor.wait_read_ahead().unwrap();
    let reads = source.checkpoint_reads.load(Ordering::Relaxed);
    let pages = cursor.stats().pages_read;
    assert!(reads > 1 && reads < pages);
    for hour in (25..30).rev() {
        assert!(cursor.rewind(hour).unwrap().applied);
        let expected = source
            .version_at(
                &keys,
                hour,
                Hours {
                    first: hour,
                    last: hour,
                },
                &memory,
            )
            .unwrap();
        for (_, address, record) in cursor.values().slots() {
            assert_eq!(expected.get(address), Lookup::Record(record));
        }
        cursor.wait_read_ahead().unwrap();
    }
    drop(cursor);
    drop(source);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn source_selection_handles_are_borrowed_without_frame_conversion() {
    struct TypedKeys {
        ids: Buffer<Key>,
    }
    let memory = Memory::new(1_000_000).unwrap();
    let mut ids = Buffer::new(&memory, Owner::State, 2).unwrap();
    ids.push(Key::from_varint_in(&memory, &[0x80, 0x01]).unwrap());
    ids.push(Key::from_varint_in(&memory, &[0x81, 0x01]).unwrap());
    let selection = Selection::All { len: 2 }
        .with_source(TypedKeys { ids }, &memory)
        .unwrap();
    assert_eq!(
        counted(|| {
            let keys = &selection.source::<TypedKeys>().unwrap().ids;
            assert_eq!(keys[0].varint(), &[0x80, 0x01]);
            assert_eq!(keys[1].varint(), &[0x81, 0x01]);
            assert_eq!(selection.id(1), 1);
        }),
        0
    );
}
