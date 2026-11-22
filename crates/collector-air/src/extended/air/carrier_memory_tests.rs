use super::*;
use collector_raw::raw::Recorded;

struct Live<'a> {
    image: SliceMemory<'a>,
    calls: Cell<usize>,
    fail: Cell<Option<usize>>,
}

impl<'a> Live<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            image: SliceMemory {
                base: 0x1000,
                bytes,
            },
            calls: Cell::new(0),
            fail: Cell::new(None),
        }
    }
}

impl Memory for Live<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.calls.set(self.calls.get() + 1);
        if self.fail.get() == Some(address) {
            if let Some(first) = destination.first_mut() {
                *first = 0xee;
            }
            return false;
        }
        self.image.copy(address, destination)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct SourceSnapshot {
    spans: Vec<Span>,
    bytes: Vec<u8>,
    fault: [bool; 3],
    raw_bytes: [u64; 3],
    overflowed: bool,
    has_bulk: bool,
}

fn source_snapshot(log: &RawLog<3>) -> SourceSnapshot {
    SourceSnapshot {
        spans: log.spans.clone(),
        bytes: log.bytes.to_vec(),
        fault: log.fault,
        raw_bytes: log.raw_bytes,
        overflowed: log.overflowed,
        has_bulk: log.has_bulk,
    }
}

#[test]
fn census_seed_copies_owned_prefix_without_acquiring_or_charging_native_bytes() {
    let original_live = Live::new(&[0x29; 32]);
    let mut original = RawLog::<3>::new(8, 32);
    assert_eq!(original.take(&original_live, 0x1000, 8, 7), Some(0));
    assert_eq!(original.take(&original_live, 0x1008, 4, 7), Some(8));
    let before = source_snapshot(&original);
    let mut slots = vec![usize::MAX; 32];
    let mut index = ExactIndex::new(&mut slots);
    let mut carrier = RawLog::<2>::new(8, 32);
    assert_eq!(seed_census(&mut carrier, &original, &mut index), Ok(()));
    assert_eq!(original_live.calls.get(), 2);
    assert_eq!(source_snapshot(&original), before);
    assert_eq!(carrier.bytes.as_ref(), original.bytes.as_ref());
    assert_eq!(carrier.raw_bytes, [0; 2]);
    assert!(carrier.spans.iter().all(|span| span.mask == 3));
    let new_live = Live::new(&[0x77; 32]);
    let carrier = RefCell::new(carrier);
    let index = RefCell::new(index);
    let capture = CaptureMemory::new(&carrier, &new_live, &index);
    let mut bytes = [0; 8];
    assert!(capture.copy(0x1000, &mut bytes));
    assert_eq!(bytes, [0x29; 8]);
    assert_eq!(new_live.calls.get(), 0);
    let log = carrier.borrow();
    let recorded = Recorded::build(&log).unwrap();
    assert!(recorded.copy(0x1000, &mut bytes));
    assert_eq!(bytes, [0x29; 8]);
}

#[test]
fn seeded_original_fault_never_retries_live_and_another_scalar_remains_readable() {
    let mut original = RawLog::<3>::new(4, 16);
    assert_eq!(original.record(0x1000, 4, None, 7), None);
    let before = source_snapshot(&original);
    let mut slots = vec![usize::MAX; 32];
    let mut index = ExactIndex::new(&mut slots);
    let mut carrier = RawLog::<2>::new(8, 32);
    assert_eq!(seed_census(&mut carrier, &original, &mut index), Ok(()));
    let carrier = RefCell::new(carrier);
    let index = RefCell::new(index);
    let live = Live::new(&[0x42; 32]);
    let capture = CaptureMemory::new(&carrier, &live, &index);
    assert!(!capture.copy(0x1000, &mut [0; 4]));
    assert_eq!(live.calls.get(), 0);
    capture.set_mask(2);
    assert!(capture.copy(0x1008, &mut [0; 4]));
    assert_eq!(live.calls.get(), 1);
    assert_eq!(source_snapshot(&original), before);
    let log = carrier.borrow();
    assert_eq!(log.fault, [true; 2]);
    assert_eq!(log.raw_bytes, [0, 4]);
    let recorded = Recorded::build(&log).unwrap();
    assert!(!recorded.copy(0x1000, &mut [0; 4]));
    assert!(recorded.copy(0x1008, &mut [0; 4]));
}

#[test]
fn optional_fault_is_sticky_but_does_not_stop_an_independent_field() {
    let mut original = RawLog::<3>::new(4, 8);
    assert_eq!(original.record(0x1000, 4, Some(&[7; 4]), 7), Some(0));
    let before = source_snapshot(&original);
    let mut slots = vec![usize::MAX; 32];
    let mut index = ExactIndex::new(&mut slots);
    let mut carrier = RawLog::<2>::new(8, 32);
    assert_eq!(seed_census(&mut carrier, &original, &mut index), Ok(()));
    let carrier = RefCell::new(carrier);
    let index = RefCell::new(index);
    let live = Live::new(&[0x42; 32]);
    live.fail.set(Some(0x1008));
    let capture = CaptureMemory::new(&carrier, &live, &index);
    capture.set_mask(1);
    assert!(!capture.copy(0x1008, &mut [0; 4]));
    live.fail.set(None);
    assert!(!capture.copy(0x1008, &mut [0; 4]));
    assert_eq!(live.calls.get(), 1);
    capture.set_mask(2);
    assert!(capture.copy(0x100c, &mut [0; 4]));
    assert_eq!(live.calls.get(), 2);
    let mut mandatory = [0; 4];
    assert!(capture.copy(0x1000, &mut mandatory));
    assert_eq!(mandatory, [7; 4]);
    assert_eq!(source_snapshot(&original), before);
    let log = carrier.borrow();
    assert_eq!(log.fault, [true, false]);
    assert_eq!(log.raw_bytes, [0, 4]);
    assert_eq!(log.bytes.len(), 8);
    let recorded = Recorded::build(&log).unwrap();
    assert!(!recorded.copy(0x1008, &mut [0; 4]));
    assert!(recorded.copy(0x100c, &mut [0; 4]));
}

#[test]
fn exact_failure_overrides_a_successful_containing_bulk_on_capture_and_decode() {
    let image = SliceMemory {
        base: 0x1000,
        bytes: &[0x29; 32],
    };
    let mut original = RawLog::<3>::new(8, 32 + 4);
    assert_eq!(original.take_bulk(&image, 0x1000, 32, 7), Some(0));
    assert_eq!(original.record(0x1008, 4, None, 7), None);
    assert!(!original.overflowed);
    assert_eq!(original.spans.len(), 2);
    assert!(!original.spans[1].ok);
    let mut slots = vec![usize::MAX; 32];
    let mut index = ExactIndex::new(&mut slots);
    let mut carrier = RawLog::<2>::new(8, 32 + 4);
    assert_eq!(seed_census(&mut carrier, &original, &mut index), Ok(()));
    let carrier = RefCell::new(carrier);
    let index = RefCell::new(index);
    let live = Live::new(&[0x77; 32]);
    let capture = CaptureMemory::new(&carrier, &live, &index);
    assert!(!capture.copy(0x1008, &mut [0; 4]));
    let mut contained = [0; 4];
    assert!(capture.copy(0x1010, &mut contained));
    assert_eq!(contained, [0x29; 4]);
    assert_eq!(live.calls.get(), 0);
    let log = carrier.borrow();
    let recorded = Recorded::build(&log).unwrap();
    assert!(!recorded.copy(0x1008, &mut [0; 4]));
    assert!(recorded.copy(0x1010, &mut contained));
    assert_eq!(contained, [0x29; 4]);
}

#[test]
fn equal_successful_census_duplicates_coalesce_and_changed_or_failed_ones_refuse() {
    let mut original = RawLog::<3>::new(8, 32);
    for _ in 0..2 {
        assert!(original.record(0x1000, 4, Some(&[0x29; 4]), 7).is_some());
    }
    let mut slots = vec![usize::MAX; 32];
    let mut index = ExactIndex::new(&mut slots);
    let mut carrier = RawLog::<2>::new(8, 32);
    assert_eq!(seed_census(&mut carrier, &original, &mut index), Ok(()));
    assert_eq!(carrier.spans.len(), 1);
    assert_eq!(carrier.bytes.len(), 4);
    assert_eq!(carrier.raw_bytes, [0; 2]);
    assert!(original.record(0x1000, 4, Some(&[0x30; 4]), 7).is_some());
    let before = source_snapshot(&original);
    assert_eq!(
        seed_census(&mut carrier, &original, &mut index),
        Err("carrier census repeated observation changed")
    );
    assert_eq!(source_snapshot(&original), before);
    original.clear();
    assert_eq!(original.record(0x1000, 4, None, 7), None);
    assert_eq!(original.record(0x1000, 4, None, 7), None);
    assert_eq!(
        seed_census(&mut carrier, &original, &mut index),
        Err("carrier census repeated failed observation")
    );
}

#[test]
fn malformed_owned_span_refuses_without_panicking_or_live_fallback() {
    let mut original = RawLog::<3>::new(4, 8);
    assert!(original.record(0x1000, 4, Some(&[7; 4]), 7).is_some());
    original.spans[0].start = u32::MAX;
    let before = source_snapshot(&original);
    let mut slots = vec![usize::MAX; 32];
    let mut index = ExactIndex::new(&mut slots);
    let mut carrier = RawLog::<2>::new(4, 8);
    assert_eq!(
        seed_census(&mut carrier, &original, &mut index),
        Err("carrier census byte range invalid")
    );
    assert_eq!(source_snapshot(&original), before);
}

#[test]
fn short_optional_reads_and_invalid_addresses_refuse_and_decoder_stays_offline() {
    let original = RawLog::<3>::new(0, 0);
    let mut slots = vec![usize::MAX; 32];
    let mut index = ExactIndex::new(&mut slots);
    let mut carrier = RawLog::<2>::new(16, 32);
    assert_eq!(seed_census(&mut carrier, &original, &mut index), Ok(()));
    let carrier = RefCell::new(carrier);
    let index = RefCell::new(index);
    let live = Live::new(&[0x52; 3]);
    let capture = CaptureMemory::new(&carrier, &live, &index);
    assert!(capture.copy(0x1000, &mut [0; 3]));
    assert!(!capture.copy(0x1000, &mut [0; 4]));
    assert_eq!(live.calls.get(), 2);
    for address in [0, usize::MAX, 0x0000_8000_0000_0000, 0x0000_7fff_ffff_ffff] {
        assert!(!capture.copy(address, &mut [0; 2]));
    }
    assert_eq!(live.calls.get(), 2);
    assert!(capture.copy(usize::MAX, &mut []));
    let log = carrier.borrow();
    let recorded = Recorded::build(&log).unwrap();
    assert!(recorded.copy(0x1000, &mut [0; 3]));
    assert!(!recorded.copy(0x1000, &mut [0; 4]));
    assert!(!recorded.copy(0x1001, &mut [0; 1]));
    assert_eq!(live.calls.get(), 2);
}

#[test]
fn seeded_exact_index_answers_success_and_failure_without_a_native_read() {
    let mut original = RawLog::<3>::new(4, 8);
    assert!(original.record(0x1000, 4, Some(&[7; 4]), 7).is_some());
    assert_eq!(original.record(0x1008, 4, None, 7), None);
    let mut slots = vec![usize::MAX; 4];
    let mut index = ExactIndex::new(&mut slots);
    let mut carrier = RawLog::<2>::new(8, 32);
    assert_eq!(seed_census(&mut carrier, &original, &mut index), Ok(()));
    let carrier = RefCell::new(carrier);
    let index = RefCell::new(index);
    let live = Live::new(&[0x42; 32]);
    let capture = CaptureMemory::new(&carrier, &live, &index);
    let mut bytes = [0; 4];
    assert!(capture.copy(0x1000, &mut bytes));
    assert_eq!(bytes, [7; 4]);
    assert!(!capture.copy(0x1008, &mut bytes));
    assert_eq!(live.calls.get(), 0);
    assert!(!carrier.borrow().overflowed);
    let log = carrier.borrow();
    let recorded = Recorded::build(&log).unwrap();
    assert!(recorded.copy(0x1000, &mut bytes));
    assert!(!recorded.copy(0x1008, &mut bytes));
}

#[test]
fn cached_failed_exact_read_marks_each_consumer_without_rereading_or_recharging() {
    let original = RawLog::<3>::new(0, 0);
    let mut slots = vec![usize::MAX; 8];
    let mut index = ExactIndex::new(&mut slots);
    let mut carrier = RawLog::<2>::new(4, 16);
    assert_eq!(seed_census(&mut carrier, &original, &mut index), Ok(()));
    let carrier = RefCell::new(carrier);
    let index = RefCell::new(index);
    let live = Live::new(&[0x42; 16]);
    live.fail.set(Some(0x1000));
    let capture = CaptureMemory::new(&carrier, &live, &index);
    capture.set_mask(1);
    assert!(!capture.copy(0x1000, &mut [0; 4]));
    assert_eq!(live.calls.get(), 1);
    let (spans, bytes, raw_bytes) = {
        let log = carrier.borrow();
        assert_eq!(log.fault, [true, false]);
        (log.spans.clone(), log.bytes.to_vec(), log.raw_bytes)
    };
    live.fail.set(None);
    capture.set_mask(2);
    assert!(!capture.copy(0x1000, &mut [0; 4]));
    assert_eq!(live.calls.get(), 1);
    let log = carrier.borrow();
    assert_eq!(log.fault, [true; 2]);
    assert_eq!(log.spans, spans);
    assert_eq!(log.bytes.as_ref(), bytes);
    assert_eq!(log.raw_bytes, raw_bytes);
    assert!(!log.overflowed);
    let recorded = Recorded::build(&log).unwrap();
    assert!(!recorded.copy(0x1000, &mut [0; 4]));
}

#[test]
fn cached_bulk_bounds_failure_marks_each_consumer_without_changing_owned_observation() {
    let image = SliceMemory {
        base: 0x1000,
        bytes: &[0x29; 32],
    };
    let mut original = RawLog::<3>::new(4, 32);
    assert!(original.take_bulk(&image, 0x1000, 32, 7).is_some());
    let mut slots = vec![usize::MAX; 8];
    let mut index = ExactIndex::new(&mut slots);
    let mut carrier = RawLog::<2>::new(4, 32);
    assert_eq!(seed_census(&mut carrier, &original, &mut index), Ok(()));
    carrier.spans[0].start = u32::MAX;
    let spans = carrier.spans.clone();
    let bytes = carrier.bytes.to_vec();
    let raw_bytes = carrier.raw_bytes;
    let carrier = RefCell::new(carrier);
    let index = RefCell::new(index);
    let live = Live::new(&[0x42; 32]);
    let capture = CaptureMemory::new(&carrier, &live, &index);
    capture.set_mask(1);
    assert!(!capture.copy(0x1008, &mut [0; 4]));
    assert_eq!(carrier.borrow().fault, [true, false]);
    capture.set_mask(2);
    assert!(!capture.copy(0x1008, &mut [0; 4]));
    assert_eq!(live.calls.get(), 0);
    let log = carrier.borrow();
    assert_eq!(log.fault, [true; 2]);
    assert_eq!(log.spans, spans);
    assert_eq!(log.bytes.as_ref(), bytes);
    assert_eq!(log.raw_bytes, raw_bytes);
    assert!(!log.overflowed);
    let recorded = Recorded::build(&log).unwrap();
    assert!(!recorded.copy(0x1008, &mut [0; 4]));
}
