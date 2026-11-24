use super::*;
use engine_native_memory::memory::SliceMemory;
use std::cell::RefCell;

const POINTER: usize = 0x1000;
const HANDLE: u64 = (7_u64 << 32) | 51;
const HEAP: usize = 0x1400;

fn fixture(text: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; 0x400 + text.len() + 1];
    bytes[..8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[SECONDARY_BASE..SECONDARY_BASE + 8]
        .copy_from_slice(&(SECONDARY_VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[cunit::HANDLE..cunit::HANDLE + 8].copy_from_slice(&HANDLE.to_le_bytes());
    bytes[NAME + 16..NAME + 24].copy_from_slice(&(text.len() as u64).to_le_bytes());
    let capacity = text.len().max(msvc_string::INLINE_CAPACITY as usize);
    bytes[NAME + 24..NAME + 32].copy_from_slice(&(capacity as u64).to_le_bytes());
    if text.len() <= msvc_string::INLINE_CAPACITY as usize {
        bytes[NAME..NAME + text.len()].copy_from_slice(text);
    } else {
        bytes[NAME..NAME + 8].copy_from_slice(&(HEAP as u64).to_le_bytes());
        bytes[0x400..0x400 + text.len()].copy_from_slice(text);
    }
    bytes
}

fn acquire(bytes: &[u8], capacity: usize) -> (Result<Option<Span>, &'static str>, Vec<u8>, usize) {
    let mut owned = vec![0; capacity];
    let mut names = Names {
        bytes: engine_native_memory::walk::Buffer::Growing(&mut owned),
        len: 0,
        required: 0,
    };
    let result = cached_name(
        &SliceMemory {
            base: POINTER,
            bytes,
        },
        0,
        POINTER,
        HANDLE,
        &mut names,
    );
    let required = names.required;
    assert_eq!(names.len, 0, "the reader never commits an unadmitted row");
    (result, owned, required)
}

#[test]
fn cached_task_force_name_retains_empty_inline_heap_and_raw_nul_bytes() {
    for text in [
        &b""[..],
        &b"Guard\0suffix"[..],
        &b"A retained name longer than inline storage"[..],
        &[0xff][..],
    ] {
        let bytes = fixture(text);
        let (result, owned, required) = acquire(&bytes, text.len());
        assert_eq!(result, Ok(Some((0, text.len()))));
        assert_eq!(owned, text);
        assert_eq!(required, text.len());
        assert_eq!(
            cached_name_demand(
                &SliceMemory {
                    base: POINTER,
                    bytes: &bytes
                },
                0,
                POINTER,
                HANDLE
            ),
            Ok(text.len())
        );
    }
}

struct Traced<'a> {
    memory: SliceMemory<'a>,
    reads: RefCell<Vec<(usize, usize)>>,
}
impl Memory for Traced<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, output.len()));
        self.memory.copy(address, output)
    }
}

#[test]
fn cached_task_force_name_demand_reads_only_authenticated_header_and_boundaries() {
    let text = b"The heap name is longer than a short admitted arena";
    let bytes = fixture(text);
    let memory = Traced {
        memory: SliceMemory {
            base: POINTER,
            bytes: &bytes,
        },
        reads: RefCell::default(),
    };
    assert_eq!(
        cached_name_demand(&memory, 0, POINTER, HANDLE),
        Ok(text.len())
    );
    assert!(
        !memory
            .reads
            .borrow()
            .iter()
            .any(|&(address, len)| address == HEAP && len == text.len())
    );
    assert_eq!(
        acquire(&bytes, 3),
        (Ok(Some((0, text.len()))), text.to_vec(), text.len())
    );
    let mut bad = bytes.clone();
    bad[0x400 + text.len()] = 1;
    assert_eq!(acquire(&bad, 3).0, Err(ERRORS[3]));
    assert_eq!(
        acquire(&bad, 3).2,
        0,
        "torn strings cannot request arena growth"
    );
}

#[test]
fn cached_task_force_name_refuses_changed_identity_before_touching_string_fields() {
    let bytes = fixture(b"Stored");
    for offset in [0, SECONDARY_BASE, cunit::HANDLE] {
        let mut bad = bytes.clone();
        bad[offset] ^= 1;
        let memory = Traced {
            memory: SliceMemory {
                base: POINTER,
                bytes: &bad,
            },
            reads: RefCell::default(),
        };
        assert_eq!(
            cached_name_demand(&memory, 0, POINTER, HANDLE),
            Err("task-force cached name identity changed or invalid")
        );
        assert_eq!(*memory.reads.borrow(), [(POINTER, 32)]);
    }
    assert_eq!(
        acquire(&bytes[..31], 16).0,
        Err("task-force cached name identity unreadable")
    );
    assert_eq!(
        acquire(&bytes[..NAME + 31], 16).0,
        Err("task-force cached name header unreadable")
    );
    let mut bad = fixture(b"This requires heap storage");
    bad[NAME..NAME + 8].fill(0);
    assert_eq!(acquire(&bad, 64).0, Err(ERRORS[1]));
    bad = bytes;
    bad[NAME + 24..NAME + 32].copy_from_slice(&1_u64.to_le_bytes());
    assert_eq!(acquire(&bad, 64).0, Err(ERRORS[0]));
}
