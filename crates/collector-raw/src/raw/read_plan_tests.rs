use super::*;
use std::cell::Cell;

struct Script {
    reads: Vec<(usize, Option<Vec<u8>>)>,
    cursor: Cell<usize>,
}

impl Script {
    fn new(reads: Vec<(usize, Option<Vec<u8>>)>) -> Self {
        Self {
            reads,
            cursor: Cell::new(0),
        }
    }

    fn complete(&self) {
        assert_eq!(
            self.cursor.get(),
            self.reads.len(),
            "all expected source observations consumed"
        );
    }
}

impl Memory for Script {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let index = self.cursor.get();
        let (expected, bytes) = self
            .reads
            .get(index)
            .expect("unexpected source read or retry");
        assert_eq!(address, *expected, "source occurrence {index}");
        self.cursor.set(index + 1);
        if let Some(bytes) = bytes {
            assert_eq!(destination.len(), bytes.len());
            destination.copy_from_slice(bytes);
            true
        } else {
            if let Some(first) = destination.first_mut() {
                *first = 0xEE;
            }
            false
        }
    }
}

fn log() -> RawLog<2> {
    let mut log = RawLog::new(16, 128);
    log.initialize_read_plan(16, 16, 128);
    log
}

fn linked(memory: &impl Memory, log: &mut RawLog<2>) {
    let Some(root) = log.take(memory, 0x1000, 8, 3) else {
        return;
    };
    let child = u64::from_le_bytes(log.bytes[root..root + 8].try_into().unwrap()) as usize;
    let _ = log.take(memory, child, 4, 2);
}

fn linked_script(child: usize, value: [u8; 4]) -> Script {
    Script::new(vec![
        (0x1000, Some((child as u64).to_le_bytes().to_vec())),
        (child, Some(value.to_vec())),
    ])
}

#[test]
fn fresh_payload_reuses_plan_and_changed_pointer_resumes_without_repeating_root() {
    let mut log = log();
    let first = linked_script(0x2000, [1; 4]);
    assert!(!log.begin_planned_capture(&first, [1, 2, 3, 4]));
    linked(&first, &mut log);
    log.finish_planned_capture();
    first.complete();
    let shape = log.spans.clone();
    log.prepare_read_plan();
    log.clear();

    let fresh = linked_script(0x2000, [9; 4]);
    assert!(log.begin_planned_capture(&fresh, [1, 2, 3, 4]));
    log.finish_planned_capture();
    fresh.complete();
    assert!(log.planned_capture_succeeded());
    assert_eq!(log.spans, shape);
    assert_eq!(&log.bytes[8..], &[9; 4]);
    assert_eq!(log.raw_bytes, [8, 4]);
    log.prepare_read_plan();
    log.clear();

    let changed = linked_script(0x3000, [7; 4]);
    assert!(!log.begin_planned_capture(&changed, [1, 2, 3, 4]));
    assert!(log.bytes.is_empty());
    linked(&changed, &mut log);
    log.finish_planned_capture();
    changed.complete();
    assert!(!log.planned_capture_failed());
    assert_eq!(log.spans[1].addr, 0x3000);
    assert_eq!(&log.bytes[8..], &[7; 4]);
    log.prepare_read_plan();
    log.clear();
    let rebuilt = linked_script(0x3000, [6; 4]);
    assert!(log.begin_planned_capture(&rebuilt, [1, 2, 3, 4]));
    log.finish_planned_capture();
    rebuilt.complete();
}

#[test]
fn complete_plan_loans_geometry_for_decode_and_returns_it_before_reuse_or_retirement() {
    let mut log = log();
    let staging = log.spans.as_ptr();
    let first = linked_script(0x2000, [1; 4]);
    assert!(!log.begin_planned_capture(&first, [1; 4]));
    linked(&first, &mut log);
    log.finish_planned_capture();
    first.complete();
    log.prepare_read_plan();
    let geometry = log.plan.compiled.as_ref().unwrap().spans.as_ptr();
    log.clear();

    for value in [2, 3] {
        let fresh = linked_script(0x2000, [value; 4]);
        assert!(log.begin_planned_capture(&fresh, [1; 4]));
        log.finish_planned_capture();
        fresh.complete();
        assert_eq!(log.spans.as_ptr(), geometry);
        let recorded = Recorded::build(&log);
        let mut bytes = [0; 4];
        assert!(recorded.copy(0x2000, &mut bytes));
        assert_eq!(bytes, [value; 4]);
        drop(recorded);
        // Both normal recycle and clear without preparation return the loan.
        if value == 2 {
            log.prepare_read_plan();
        }
        log.clear();
        assert_eq!(log.spans.as_ptr(), staging);
        assert_eq!(log.plan.compiled.as_ref().unwrap().spans.as_ptr(), geometry);
    }

    let fresh = linked_script(0x2000, [4; 4]);
    assert!(log.begin_planned_capture(&fresh, [1; 4]));
    log.finish_planned_capture();
    fresh.complete();
    log.invalidate_read_plan();
    log.prepare_read_plan();
    assert!(log.plan.compiled.is_none());
    assert_eq!(log.spans.as_ptr(), staging);
    log.clear();
    let changed = linked_script(0x3000, [5; 4]);
    assert!(!log.begin_planned_capture(&changed, [1; 4]));
    linked(&changed, &mut log);
    log.finish_planned_capture();
    changed.complete();
    assert_eq!(log.spans[1].addr, 0x3000);
}

#[test]
fn failed_occurrence_is_replayed_once_and_partial_bytes_stay_unpublished() {
    let mut log = log();
    let first = Script::new(vec![
        (0x10, Some(vec![1; 4])),
        (0x20, Some(vec![2; 4])),
        (0x30, Some(vec![3; 4])),
    ]);
    assert!(!log.begin_planned_capture(&first, [0; 4]));
    for address in [0x10, 0x20, 0x30] {
        log.take(&first, address, 4, 1);
    }
    log.finish_planned_capture();
    log.prepare_read_plan();
    log.clear();
    let failed = Script::new(vec![
        (0x10, Some(vec![8; 4])),
        (0x20, None),
        (0x30, Some(vec![9; 4])),
    ]);
    assert!(!log.begin_planned_capture(&failed, [0; 4]));
    assert!(log.bytes.is_empty());
    assert_eq!(log.take(&failed, 0x10, 4, 1), Some(0));
    assert_eq!(log.take(&failed, 0x20, 4, 1), None);
    assert_eq!(log.bytes.len(), 4);
    assert_eq!(log.take(&failed, 0x30, 4, 1), Some(4));
    log.finish_planned_capture();
    failed.complete();
    assert_eq!(&log.bytes[..], &[8, 8, 8, 8, 9, 9, 9, 9]);
    assert_eq!(log.raw_bytes, [8, 0]);
    assert_eq!(log.fault, [true, false]);
    assert!(!log.spans[1].ok);
    log.prepare_read_plan();
    assert!(log.plan.compiled.is_none());
}

#[test]
fn duplicate_observations_keep_their_ordinals_without_empty_trace_entries() {
    let mut log = log();
    let walk = |memory: &Script, log: &mut RawLog<2>| {
        assert_eq!(log.take(memory, 0, 0, 3), Some(0));
        for value in [1u8, 2, 1] {
            let start = log.take(memory, 0x10, 1, 1).unwrap();
            let observed = log.bytes.get(start).copied().unwrap();
            assert_eq!(observed, value);
        }
        assert_eq!(log.take(memory, usize::MAX, 0, 2), Some(3));
        assert_eq!(log.bytes[0], 1);
        assert_eq!(log.bytes.get(1..2), Some(&[2][..]));
    };
    let sequence = || {
        Script::new(vec![
            (0x10, Some(vec![1])),
            (0x10, Some(vec![2])),
            (0x10, Some(vec![1])),
        ])
    };
    let first = sequence();
    assert!(!log.begin_planned_capture(&first, [0; 4]));
    walk(&first, &mut log);
    log.finish_planned_capture();
    log.prepare_read_plan();
    assert_eq!(log.plan.compiled.as_ref().unwrap().requests.len(), 3);
    log.clear();
    let fresh = sequence();
    assert!(log.begin_planned_capture(&fresh, [0; 4]));
    log.finish_planned_capture();
    fresh.complete();
    assert_eq!(log.spans.len(), 3);
    assert_eq!(&log.bytes[..], &[1, 2, 1]);
}

#[test]
fn empty_scatter_slots_preserve_changed_controls_and_every_failed_prefix() {
    fn walk(source: &Script, log: &mut RawLog<2>) {
        let Some((starts, 5)) = log.take_scattered(
            source,
            [0, 0x10, usize::MAX, 0x20, 0],
            [0, 1, 0, 1, 0],
            [3, 1, 2, 2, 1],
        ) else {
            return;
        };
        let target = if log.bytes[starts[1]] == 1 {
            0x30
        } else {
            0x40
        };
        assert_eq!(log.take(source, usize::MAX, 0, 3), Some(2));
        log.take(source, target, 1, 2);
    }
    for changed in [false, true] {
        for failure in [None, Some(0), Some(1), Some(2)] {
            let mut prepared = log();
            prepared.initialize_scattered_read_plan(16, 16, 128);
            let baseline = Script::new(vec![
                (0x10, Some(vec![1])),
                (0x20, Some(vec![2])),
                (0x30, Some(vec![3])),
            ]);
            assert!(!prepared.begin_planned_capture(&baseline, [0; 4]));
            walk(&baseline, &mut prepared);
            prepared.finish_planned_capture();
            baseline.complete();
            prepared.prepare_read_plan();
            assert_eq!(prepared.plan.compiled.as_ref().unwrap().requests.len(), 3);
            prepared.clear();
            let mut reads = vec![
                (0x10, Some(vec![if changed { 4 } else { 1 }])),
                (0x20, Some(vec![5])),
                (if changed { 0x40 } else { 0x30 }, Some(vec![6])),
            ];
            if let Some(index) = failure {
                reads[index].1 = None;
                reads.truncate(index + 1);
            }
            let fresh = Script::new(reads.clone());
            let mut direct = log();
            walk(&fresh, &mut direct);
            fresh.complete();
            let fresh = Script::new(reads);
            if !prepared.begin_planned_capture(&fresh, [0; 4]) {
                walk(&fresh, &mut prepared);
            }
            prepared.finish_planned_capture();
            fresh.complete();
            assert!(!prepared.planned_capture_failed());
            assert_eq!(prepared.spans, direct.spans);
            assert_eq!(&prepared.bytes[..], &direct.bytes[..]);
            assert_eq!(prepared.fault, direct.fault);
            assert_eq!(prepared.raw_bytes, direct.raw_bytes);
        }
    }
}

#[test]
fn prefix_descriptor_mismatch_blocks_all_later_reads_and_retires_plan() {
    for (address, len, mask, bulk) in [
        (0x11, 1, 1, false),
        (0x10, 2, 1, false),
        (0x10, 1, 2, false),
        (0x10, 1, 1, true),
    ] {
        let mut log = log();
        let first = Script::new(vec![(0x10, Some(vec![1]))]);
        assert!(!log.begin_planned_capture(&first, [0; 4]));
        log.take(&first, 0x10, 1, 1);
        assert_eq!(log.bytes[0], 1);
        log.finish_planned_capture();
        log.prepare_read_plan();
        log.clear();
        let changed = Script::new(vec![(0x10, Some(vec![2]))]);
        assert!(!log.begin_planned_capture(&changed, [0; 4]));
        let result = if bulk {
            log.take_bulk(&changed, address, len, mask)
        } else {
            log.take(&changed, address, len, mask)
        };
        assert_eq!(result, None);
        assert_eq!(log.take(&changed, 0x10, 1, 1), None);
        log.finish_planned_capture();
        changed.complete();
        assert!(log.planned_capture_failed());
        assert_eq!(log.fault, [true; 2]);
        log.prepare_read_plan();
        assert!(log.plan.compiled.is_none());
    }
}

#[test]
fn planned_prefix_then_payload_overflow_forbids_retry_without_preventing_growth() {
    let mut log = RawLog::<2>::new(4, 8);
    log.initialize_read_plan(4, 4, 8);
    let first = Script::new(vec![(0x10, Some(vec![1])), (0x20, Some(vec![7]))]);
    assert!(!log.begin_planned_capture(&first, [0; 4]));
    let root = log.take(&first, 0x10, 1, 1).unwrap();
    assert_eq!(log.bytes[root], 1);
    assert_eq!(log.take(&first, 0x20, 1, 1), Some(1));
    log.finish_planned_capture();
    log.prepare_read_plan();
    log.clear();

    let changed = Script::new(vec![(0x10, Some(vec![2]))]);
    assert!(!log.begin_planned_capture(&changed, [0; 4]));
    let root = log.take(&changed, 0x10, 1, 1).unwrap();
    assert_eq!(log.bytes[root], 2);
    let overflowing_len = log.bytes.capacity();
    assert_eq!(log.take(&changed, 0x20, overflowing_len, 1), None);
    assert_eq!(log.take(&changed, 0x30, 1, 1), None);
    log.finish_planned_capture();
    changed.complete();
    assert!(log.overflowed);
    assert!(log.planned_capture_requires_no_retry());
    assert!(!log.planned_capture_failed());
    assert_eq!(log.fault, [false; 2]);
    assert!(log.grow_for_next());
    log.prepare_read_plan();
    assert!(log.plan.compiled.is_none());
    log.grow();
    assert!(log.bytes.capacity() > overflowing_len);
    log.clear();
    assert!(!log.planned_capture_requires_no_retry());

    // The same ordinary overflow keeps the existing fallback contract.
    let ordinary = Script::new(vec![(0x10, Some(vec![2]))]);
    assert!(!log.begin_planned_capture(&ordinary, [0; 4]));
    log.take(&ordinary, 0x10, 1, 1);
    let overflowing_len = log.bytes.capacity();
    log.take(&ordinary, 0x20, overflowing_len, 1);
    log.finish_planned_capture();
    ordinary.complete();
    assert!(log.overflowed);
    assert!(!log.planned_capture_requires_no_retry());
    assert!(log.grow_for_next());
}

#[test]
fn trace_exhaustion_grows_only_off_thread_and_never_truncates_the_payload() {
    let mut log = RawLog::<2>::new(16, 128);
    log.initialize_read_plan(1, 0, 0);
    let first = linked_script(0x2000, [1; 4]);
    assert!(!log.begin_planned_capture(&first, [0; 4]));
    linked(&first, &mut log);
    log.finish_planned_capture();
    first.complete();
    assert!(!log.overflowed);
    assert_eq!(log.fault, [false; 2]);
    assert_eq!(log.bytes.len(), 12);
    assert_eq!(log.plan.requests.capacity(), 1);
    log.prepare_read_plan();
    assert!(log.plan.compiled.is_none());
    assert!(log.plan.requests.capacity() >= 2);
    assert!(
        log.bytes
            .observations
            .as_ref()
            .unwrap()
            .borrow()
            .expected
            .capacity()
            >= 8
    );
    log.clear();
    let next = linked_script(0x2000, [2; 4]);
    assert!(!log.begin_planned_capture(&next, [0; 4]));
    linked(&next, &mut log);
    log.finish_planned_capture();
    log.prepare_read_plan();
    assert!(log.plan.compiled.is_some());
}

#[test]
fn untracked_slice_escape_and_non_scalar_acquisition_only_disable_optimization() {
    for operation in 0..3 {
        let mut log = log();
        let first = Script::new(vec![(0x10, Some(vec![1])), (0x20, Some(vec![2]))]);
        assert!(!log.begin_planned_capture(&first, [0; 4]));
        log.take(&first, 0x10, 1, 1);
        if operation == 0 {
            let escaped: &[u8] = &log.bytes;
            assert_eq!(escaped, &[1]);
            log.take(&first, 0x20, 1, 2);
        } else if operation == 1 {
            log.take_scattered(&first, [0x20], [1], [2]);
        } else {
            log.take_scattered_prefetched(&first, [0x20], [1], [2]);
        }
        log.finish_planned_capture();
        first.complete();
        assert!(!log.overflowed);
        assert!(!log.planned_capture_failed());
        assert_eq!(log.fault, [false; 2]);
        assert_eq!(&log.bytes[..], &[1, 2]);
        log.prepare_read_plan();
        assert!(log.plan.compiled.is_none());
    }
}

#[test]
fn prefetched_scatter_obeys_pending_and_consumed_plan_prefix_gates() {
    for prefetched in [false, true] {
        for failed in [false, true] {
            for consume_prefix in [false, true] {
                let mut log = log();
                log.initialize_scattered_read_plan(16, 16, 128);
                let first = Script::new(vec![(0x10, Some(vec![1]))]);
                assert!(!log.begin_planned_capture(&first, [0; 4]));
                assert_eq!(log.take(&first, 0x10, 1, 1), Some(0));
                assert_eq!(log.bytes[0], 1);
                log.finish_planned_capture();
                first.complete();
                log.prepare_read_plan();
                assert!(log.plan.compiled.is_some());
                log.clear();

                let mut reads = vec![(0x10, (!failed).then(|| vec![2]))];
                if consume_prefix {
                    reads.push((0x20, Some(vec![7])));
                }
                let changed = Script::new(reads);
                assert!(!log.begin_planned_capture(&changed, [0; 4]));
                assert_eq!(log.plan.prefix_len, 1);
                if consume_prefix {
                    assert_eq!(log.take(&changed, 0x10, 1, 1), (!failed).then_some(0));
                }
                let result = if prefetched {
                    log.take_scattered_prefetched(&changed, [0x20], [1], [2])
                } else {
                    log.take_scattered(&changed, [0x20], [1], [2])
                };
                assert_eq!(
                    result,
                    consume_prefix.then_some(([usize::from(!failed)], 1))
                );
                assert!(log.plan.retire);
                assert_eq!(
                    log.bytes.observations.as_ref().unwrap().borrow().eligible,
                    consume_prefix && !failed
                );
                assert_eq!(log.planned_capture_failed(), !consume_prefix);
                if consume_prefix {
                    assert_eq!(log.fault, [failed, false]);
                    assert_eq!(log.raw_bytes, [u64::from(!failed), 1]);
                    assert_eq!(log.bytes[log.bytes.len() - 1], 7);
                } else {
                    assert_eq!(log.fault, [true; 2]);
                    assert!(log.spans.is_empty());
                    assert!(log.bytes.is_empty());
                    assert_eq!(log.take(&changed, 0x20, 1, 2), None);
                }
                log.finish_planned_capture();
                changed.complete();
                log.prepare_read_plan();
                assert_eq!(log.plan.compiled.is_some(), consume_prefix && !failed);
            }
        }
    }
}
