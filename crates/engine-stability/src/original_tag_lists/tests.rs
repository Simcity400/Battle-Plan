use super::*;
use engine_native_memory::memory::SliceMemory;
use std::cell::RefCell;

const STATE: usize = 0x1000;
const HEADERS: usize = 0x2400;
const MEMBERS: usize = 0x3000;

fn put_header(bytes: &mut [u8], address: usize, data: usize, count: u32) {
    bytes[address..address + 8].copy_from_slice(&(data as u64).to_le_bytes());
    bytes[address + 8..address + 12].copy_from_slice(&count.to_le_bytes());
    bytes[address + 12..address + 16].copy_from_slice(&count.to_le_bytes());
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x4000];
    put_header(&mut bytes, STATE + BUCKETS, HEADERS, 3);
    put_header(&mut bytes, HEADERS, MEMBERS, 4);
    put_header(&mut bytes, HEADERS + engine_native_memory::cpdx_array::STRIDE, 0, 0);
    put_header(
        &mut bytes,
        HEADERS + 2 * engine_native_memory::cpdx_array::STRIDE,
        MEMBERS + 0x40,
        2,
    );
    for (address, values) in [
        (MEMBERS, &[0, 7, 7, u32::MAX][..]),
        (MEMBERS + 0x40, &[9, 1][..]),
    ] {
        for (index, value) in values.iter().enumerate() {
            bytes[address + index * 4..address + index * 4 + 4]
                .copy_from_slice(&value.to_le_bytes());
        }
    }
    bytes
}

struct Traced<'a> {
    bytes: &'a [u8],
    calls: RefCell<Vec<(usize, usize)>>,
    fault: Option<usize>,
    changed_header: Option<usize>,
}

impl<'a> Traced<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            calls: RefCell::default(),
            fault: None,
            changed_header: None,
        }
    }
}

impl Memory for Traced<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let same_header_reads = self
            .calls
            .borrow()
            .iter()
            .filter(|&&(at, len)| at == address && len == 16)
            .count();
        self.calls.borrow_mut().push((address, output.len()));
        if self
            .fault
            .is_some_and(|fault| (address..address + output.len()).contains(&fault))
        {
            return false;
        }
        if !(SliceMemory {
            base: 0,
            bytes: self.bytes,
        })
        .copy(address, output)
        {
            return false;
        }
        if self.changed_header == Some(address) && output.len() == 16 && same_header_reads != 0 {
            output[8] ^= 1;
        }
        true
    }
}

fn no_payload(memory: &Traced<'_>) {
    assert!(
        memory
            .calls
            .borrow()
            .iter()
            .all(|&(at, _)| !(MEMBERS..MEMBERS + 0x100).contains(&at))
    );
}

fn controls_cleared(buckets: &[Bucket]) {
    for bucket in buckets {
        assert_eq!(bucket.source, Array::default());
        assert_eq!(bucket.address, 0);
        assert_eq!(bucket.header, [0; 16]);
    }
}

#[test]
fn capture_preserves_every_bucket_raw_word_order_and_repeated_occurrence() {
    let bytes = fixture();
    let memory = Traced::new(&bytes);
    let mut buckets = vec![Bucket::default(); 3];
    let mut members = vec![123; 6];
    let captured = capture(&memory, STATE, &mut buckets, &mut members);
    assert_eq!(captured.len, Ok(3));
    assert_eq!(
        captured.demand,
        Demand {
            buckets: 3,
            members: 6
        }
    );
    assert_eq!(members, [0, 7, 7, u32::MAX, 9, 1]);
    assert_eq!(
        buckets.iter().map(|bucket| (bucket.start, bucket.len)).collect::<Vec<_>>(),
        [(0, 4), (4, 0), (4, 2)]
    );
    controls_cleared(&buckets);
    for payload in [(MEMBERS, 16), (MEMBERS + 0x40, 8)] {
        assert_eq!(
            memory
                .calls
                .borrow()
                .iter()
                .filter(|&&read| read == payload)
                .count(),
            1
        );
    }
}

#[test]
fn payload_fault_withholds_the_whole_field_without_retry() {
    let bytes = fixture();
    let mut memory = Traced::new(&bytes);
    memory.fault = Some(MEMBERS + 0x40);
    let mut buckets = vec![Bucket::default(); 3];
    let mut members = Vec::new();
    let captured = capture(&memory, STATE, &mut buckets, &mut members);
    assert_eq!(captured.len, Err("original-tag member payload unreadable"));
    assert_eq!(
        captured.demand,
        Demand {
            buckets: 3,
            members: 6
        }
    );
    assert_eq!(
        memory
            .calls
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == MEMBERS + 0x40)
            .count(),
        1
    );
    controls_cleared(&buckets);
}

#[test]
fn native_empty_outer_registry_is_available_and_malformed_headers_are_not() {
    let mut bytes = fixture();
    put_header(&mut bytes, STATE + BUCKETS, 0, 0);
    let memory = Traced::new(&bytes);
    assert_eq!(
        capture(&memory, STATE, &mut Vec::new(), &mut Vec::new()),
        Capture {
            len: Ok(0),
            demand: Demand::default()
        }
    );
    assert_eq!(demand(&memory, STATE), Ok(Demand::default()));
    no_payload(&memory);

    for address in [STATE + BUCKETS, HEADERS + engine_native_memory::cpdx_array::STRIDE] {
        let mut bytes = fixture();
        bytes[address + 8..address + 12].copy_from_slice(&u32::MAX.to_le_bytes());
        let memory = Traced::new(&bytes);
        let mut buckets = vec![Bucket::default(); 3];
        let mut members = vec![123; 6];
        let captured = capture(&memory, STATE, &mut buckets, &mut members);
        assert_eq!(captured.len, Err("original-tag array header invalid"));
        assert_eq!(
            captured.demand,
            if address == STATE + BUCKETS {
                Demand::default()
            } else {
                Demand {
                    buckets: 3,
                    members: 4,
                }
            }
        );
        assert!(demand(&memory, STATE).is_err());
        assert_eq!(members, [123; 6]);
        no_payload(&memory);
        controls_cleared(&buckets);
    }
}

#[test]
fn header_only_demand_and_capture_refuse_changed_selectors() {
    let bytes = fixture();
    let memory = Traced::new(&bytes);
    assert_eq!(
        demand(&memory, STATE),
        Ok(Demand {
            buckets: 3,
            members: 6
        })
    );
    no_payload(&memory);
    for address in [
        STATE + BUCKETS,
        HEADERS,
        HEADERS + 2 * engine_native_memory::cpdx_array::STRIDE,
    ] {
        let mut memory = Traced::new(&bytes);
        memory.changed_header = Some(address);
        assert_eq!(
            demand(&memory, STATE),
            Err("original-tag array header changed or unreadable")
        );
        no_payload(&memory);
        let mut memory = Traced::new(&bytes);
        memory.changed_header = Some(address);
        let mut buckets = vec![Bucket::default(); 3];
        let mut members = vec![123; 6];
        let captured = capture(&memory, STATE, &mut buckets, &mut members);
        assert_eq!(
            captured.len,
            Err("original-tag array header changed or unreadable")
        );
        controls_cleared(&buckets);
    }
    assert!(demand(&memory, 0).is_err());
    assert!(demand(&memory, usize::MAX).is_err());
}
