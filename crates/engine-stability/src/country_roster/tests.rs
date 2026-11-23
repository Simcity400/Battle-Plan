use super::*;
use engine_native_intelligence::country_memory::Cache;
use engine_native_memory::memory::SliceMemory;
use std::cell::RefCell;

const STATE: usize = 0x1000;
const POINTERS: usize = 0x2000;
const LINKS: usize = 0x2100;
const ZERO: usize = 0x2800;
const FIRST: usize = 0x3000;
const SECOND: usize = 0x3100;

fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
    bytes[address..address + value.len()].copy_from_slice(value);
}

fn array(bytes: &mut [u8], address: usize, data: usize, count: u32) {
    put(bytes, address, &(data as u64).to_le_bytes());
    put(bytes, address + 8, &count.to_le_bytes());
    put(bytes, address + 12, &count.to_le_bytes());
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x4000];
    array(&mut bytes, STATE + cgame_state::COUNTRIES, POINTERS, 4);
    array(&mut bytes, STATE + cgame_state::COUNTRY_LINKS, LINKS, 10);
    for (slot, pointer) in [ZERO, FIRST, FIRST, SECOND].into_iter().enumerate() {
        put(
            &mut bytes,
            POINTERS + slot * 8,
            &(pointer as u64).to_le_bytes(),
        );
    }
    for (pointer, id) in [(ZERO, 0_u32), (FIRST, 7), (SECOND, 9)] {
        put(
            &mut bytes,
            pointer,
            &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, pointer + ccountry::ID, &id.to_le_bytes());
    }
    for (raw, slot) in [(1, 1_u32), (2, 1), (7, 1), (3, 3), (9, 3)] {
        put(&mut bytes, LINKS + raw * 4, &slot.to_le_bytes());
    }
    bytes
}

struct Traced<'a> {
    bytes: &'a [u8],
    reads: RefCell<Vec<(usize, usize)>>,
    fault: Option<usize>,
    changed_header: bool,
}

impl<'a> Traced<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            reads: RefCell::default(),
            fault: None,
            changed_header: false,
        }
    }
}

impl Memory for Traced<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let already_read = self.reads.borrow().contains(&(address, output.len()));
        self.reads.borrow_mut().push((address, output.len()));
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
        if self.changed_header && address == STATE + cgame_state::COUNTRIES && already_read {
            output[8] ^= 1;
        }
        true
    }
}

#[test]
fn physical_roster_preserves_current_raw_ids_alias_order_and_cached_census() {
    let bytes = fixture();
    let memory = Traced::new(&bytes);
    let mut output = vec![123; 4];
    assert_eq!(
        capture(&memory, 0, STATE, &mut output),
        Capture {
            len: Ok(4),
            required: 4
        }
    );
    assert_eq!(output, [0, 7, 7, 9]);

    let mut cache = Cache::try_with_capacity(256).unwrap();
    cache.prepare(&memory, 0, STATE);
    memory.reads.borrow_mut().clear();
    let cached = cache.view(&memory);
    assert_eq!(capture(&cached, 0, STATE, &mut output).len, Ok(4));
    assert_eq!(output, [0, 7, 7, 9]);
    assert!(memory.reads.borrow().iter().all(|&(address, _)| {
        ![FIRST, FIRST + ccountry::ID, SECOND, SECOND + ccountry::ID].contains(&address)
    }));
}

#[test]
fn repeated_reserved_pointer_is_zero_but_null_or_unproved_zero_is_refused() {
    let mut bytes = fixture();
    put(&mut bytes, POINTERS + 16, &(ZERO as u64).to_le_bytes());
    let memory = Traced::new(&bytes);
    let mut output = vec![123; 4];
    assert_eq!(capture(&memory, 0, STATE, &mut output).len, Ok(4));
    assert_eq!(output, [0, 7, 0, 9]);
    put(&mut bytes, POINTERS + 16, &(SECOND as u64).to_le_bytes());
    put(&mut bytes, LINKS + 2 * 4, &0_u32.to_le_bytes());
    assert!(
        capture(&Traced::new(&bytes), 0, STATE, &mut output)
            .len
            .is_err()
    );
    put(&mut bytes, POINTERS + 16, &0_u64.to_le_bytes());
    assert_eq!(
        capture(&Traced::new(&bytes), 0, STATE, &mut output).len,
        Err("country-array roster pointer unreadable or null")
    );
    for (address, value) in [(ZERO, 0_u64), (ZERO + ccountry::ID, 1)] {
        let mut bytes = fixture();
        if address == ZERO {
            put(&mut bytes, address, &value.to_le_bytes());
        } else {
            put(&mut bytes, address, &(value as u32).to_le_bytes());
        }
        assert!(
            capture(&Traced::new(&bytes), 0, STATE, &mut output)
                .len
                .is_err()
        );
    }
}

#[test]
fn sizing_reads_only_the_proved_array_header_and_short_storage_grows() {
    let bytes = fixture();
    let memory = Traced::new(&bytes);
    assert_eq!(demand(&memory, 0, STATE), Ok(4));
    assert!(
        memory
            .reads
            .borrow()
            .iter()
            .all(|&(address, len)| { address == STATE + cgame_state::COUNTRIES && len == 16 })
    );
    let mut output = vec![123; 3];
    assert_eq!(
        capture(&memory, 0, STATE, &mut output),
        Capture {
            len: Ok(4),
            required: 4
        }
    );
}

#[test]
fn fault_and_selector_change_withhold_the_field_without_retry() {
    let bytes = fixture();
    let mut memory = Traced::new(&bytes);
    memory.fault = Some(FIRST + ccountry::ID);
    let mut output = Vec::new();
    let captured = capture(&memory, 0, STATE, &mut output);
    assert_eq!(captured.len, Err("country id unreadable"));
    assert_eq!(captured.required, 4);
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|&&(address, _)| address == FIRST + ccountry::ID)
            .count(),
        1
    );

    let mut memory = Traced::new(&bytes);
    memory.changed_header = true;
    assert!(capture(&memory, 0, STATE, &mut output).len.is_err());
    let mut memory = Traced::new(&bytes);
    memory.changed_header = true;
    assert_eq!(
        demand(&memory, 0, STATE),
        Err("country-array header changed or unreadable")
    );
    for state in [0, usize::MAX] {
        assert!(demand(&memory, 0, state).is_err());
    }
}

#[test]
fn known_empty_and_reserved_only_arrays_need_no_positive_links_or_guessing() {
    let mut bytes = fixture();
    array(&mut bytes, STATE + cgame_state::COUNTRY_LINKS, 0, 0);
    for count in [0, 1] {
        array(&mut bytes, STATE + cgame_state::COUNTRIES, POINTERS, count);
        let mut output = vec![123; 1];
        let memory = Traced::new(&bytes);
        assert_eq!(
            capture(&memory, 0, STATE, &mut output).len,
            Ok(count as usize)
        );
        if count == 1 {
            assert_eq!(output, [0]);
        }
        assert_eq!(demand(&memory, 0, STATE), Ok(count as usize));
        assert!(
            memory
                .reads
                .borrow()
                .iter()
                .all(|&(address, _)| address != STATE + cgame_state::COUNTRY_LINKS)
        );
    }
}
