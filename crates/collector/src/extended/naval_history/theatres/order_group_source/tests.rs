use super::*;
use engine_native::{carmy_group, memory::SliceMemory};

const GROUP: usize = 0x30000;
const FLAG: usize = GROUP + corders_group::IS_ARMY_GROUP;
const MOTOR: usize = GROUP + corders_group::MOTORIZATION_LEVEL;
const HEADER: usize = GROUP + carmy_group::MEMBER_DATA;

fn take<const N: usize>(log: &mut RawLog<N>, at: usize, bytes: &[u8], len: usize) {
    let memory = SliceMemory { base: at, bytes };
    let _ = log.take(&memory, at, len, OOB_MASK);
}

fn staged(own_flag: Option<u8>) -> NavalJob {
    let mut job = NavalJob {
        log: RawLog::new(16, 256),
        ..Default::default()
    };
    take(&mut job.log, FLAG, &[1], 1);
    match own_flag {
        Some(flag) => take(&mut job.log, FLAG, &[flag], 1),
        None => take(&mut job.log, FLAG, &[], 1),
    }
    take(&mut job.log, MOTOR, &[0], 1);
    let mut header = [0; 16];
    header[12..16].copy_from_slice(&7_i32.to_le_bytes());
    take(&mut job.log, HEADER, &header, 16);
    take(&mut job.log, FLAG, &[1], 1);
    job
}

#[test]
fn own_flag_success_or_fault_is_independent_of_parent_occurrences_and_cursors() {
    for own in [Some(0), Some(1), None] {
        let job = staged(own);
        let source = OrderGroupSource::new(&job).unwrap();
        let mut byte = [9];
        assert!(source.recorded.copy(FLAG, &mut byte));
        assert_eq!(byte, [1]);
        assert_eq!(source.copy(FLAG, &mut byte), own.is_some());
        if let Some(own) = own {
            assert_eq!(byte, [own]);
        }
        assert_eq!(source.recorded.copy(FLAG, &mut byte), own.is_some());
        if let Some(own) = own {
            assert_eq!(byte, [own]);
        }
        assert!(source.recorded.copy(FLAG, &mut byte));
        assert_eq!(byte, [1]);
        assert_eq!(source.copy(FLAG, &mut byte), own.is_some());
    }
}

#[test]
fn range_prefix_suffix_and_empty_cannot_supply_a_missing_own_batch() {
    let job = staged(None);
    for range in [0..1, 2..4, 4..5, 0..0] {
        let source = OrderGroupSource {
            log: &job.log,
            recorded: Recorded::build_range(&job.log, range).unwrap(),
        };
        assert!(!source.copy(FLAG, &mut [0]));
    }
    let source = OrderGroupSource {
        log: &job.log,
        recorded: Recorded::build_range(&job.log, 1..4).unwrap(),
    };
    assert!(!source.copy(FLAG, &mut [0]));
    let mut header = [0; 16];
    assert!(source.copy(HEADER, &mut header));
    assert_eq!(&header[12..16], &7_i32.to_le_bytes());
}

#[test]
fn ambiguous_anchor_or_wrong_predecessor_refuses_without_a_scan() {
    let mut job = staged(Some(1));
    take(&mut job.log, MOTOR, &[0], 1);
    let source = OrderGroupSource::new(&job).unwrap();
    assert!(!source.copy(FLAG, &mut [0]));
    for _ in 0..3 {
        assert!(source.recorded.copy(MOTOR, &mut [0]));
    }
    assert!(source.recorded.unique_observation(MOTOR, 1).is_none());
    assert!(!source.copy(FLAG, &mut [0]));
    drop(source);
    let mut job = staged(Some(1));
    job.log.spans[1].mask = 1;
    let source = OrderGroupSource::new(&job).unwrap();
    assert!(!source.copy(FLAG, &mut [0]));
}

#[test]
fn consumed_repeated_header_never_becomes_a_unique_observation() {
    let mut job = staged(Some(1));
    take(&mut job.log, HEADER, &[0; 16], 16);
    let source = OrderGroupSource::new(&job).unwrap();
    assert!(!source.copy(HEADER, &mut [0; 16]));
    for _ in 0..3 {
        assert!(source.recorded.copy(HEADER, &mut [0; 16]));
    }
    assert!(source.recorded.unique_observation(HEADER, 16).is_none());
    assert!(!source.copy(HEADER, &mut [0; 16]));
}

#[test]
fn exact_header_failure_is_not_healed_by_readable_count_or_bulk_bytes() {
    let mut job = staged(Some(1));
    job.log.spans[3].ok = false;
    take(
        &mut job.log,
        GROUP + carmy_group::MEMBER_COUNT,
        &99_i32.to_le_bytes(),
        4,
    );
    let mut healthy = [0; 16];
    healthy[12..16].copy_from_slice(&99_i32.to_le_bytes());
    let base = HEADER - 1;
    let mut enclosing = [0; 18];
    enclosing[1..17].copy_from_slice(&healthy);
    let memory = SliceMemory {
        base,
        bytes: &enclosing,
    };
    assert!(job.log.take_bulk(&memory, base, 18, OOB_MASK).is_some());
    let source = OrderGroupSource::new(&job).unwrap();
    assert!(!source.copy(HEADER, &mut [0; 16]));
    assert!(!source.copy(GROUP + carmy_group::MEMBER_COUNT, &mut [0; 4]));
    assert!(source.copy(FLAG, &mut [0]));
}

#[test]
fn failed_motorization_anchor_still_selects_the_successful_own_flag() {
    let mut job = staged(Some(0));
    job.log.spans[2].ok = false;
    let source = OrderGroupSource::new(&job).unwrap();
    let mut byte = [9];
    assert!(source.copy(FLAG, &mut byte));
    assert_eq!(byte, [0]);
}
