use super::*;
use engine_native_memory::memory::SliceMemory;
use std::cell::RefCell;

struct Names {
    bytes: Vec<u8>,
    reads: RefCell<Vec<(usize, usize)>>,
    refuse: Option<usize>,
}
impl Memory for Names {
    fn copy(&self, at: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((at, output.len()));
        if self.refuse == Some(at) {
            output.fill(0xee);
            return false;
        }
        SliceMemory {
            base: 0,
            bytes: &self.bytes,
        }
        .copy(at, output)
    }
}
impl Names {
    fn new(literal: &[u8], key: &[u8]) -> Self {
        let mut result = Self {
            bytes: vec![0; 0x500],
            reads: RefCell::default(),
            refuse: None,
        };
        result.text(0x100 + NAME, literal, 0x300);
        result.text(0x100 + NAME_KEY, key, 0x400);
        result
    }
    fn text(&mut self, at: usize, text: &[u8], heap: usize) {
        let (data, capacity) = if text.len() > 15 {
            self.bytes[at..at + 8].copy_from_slice(&(heap as u64).to_le_bytes());
            (heap, text.len() as u64)
        } else {
            (at, 15)
        };
        self.bytes[data..data + text.len()].copy_from_slice(text);
        self.bytes[at + 16..at + 24].copy_from_slice(&(text.len() as u64).to_le_bytes());
        self.bytes[at + 24..at + 32].copy_from_slice(&capacity.to_le_bytes());
    }
}

#[test]
fn literal_bytes_preserve_nul_and_non_utf8_without_demanding_key_or_country() {
    let mut memory = Names::new(&[b'A', 0, 255], b"unused");
    memory.refuse = Some(0x100 + NAME_KEY);
    let mut arena = vec![0; 3];
    let (mut used, mut required) = (0, 0);
    let name = display_name(&memory, 0x100, &mut arena, &mut used, &mut required).unwrap();
    assert_eq!((used, required), (3, 3));
    assert_eq!(arena, [b'A', 0, 255]);
    assert_eq!(name.fallback_key, None);
    assert!(
        !memory
            .reads
            .borrow()
            .iter()
            .any(|&(at, _)| (0x100 + NAME_KEY..0x100 + NAME_KEY + 32).contains(&at))
    );
}

#[test]
fn empty_literal_demands_key_and_known_empty_key_needs_no_payload_space() {
    let memory = Names::new(b"", b"KEY");
    let mut arena = vec![0; 3];
    let (mut used, mut required) = (0, 0);
    let name = display_name(&memory, 0x100, &mut arena, &mut used, &mut required).unwrap();
    assert_eq!(name.literal.len, 0);
    assert_eq!(name.fallback_key, Some(Span { start: 0, len: 3 }));
    assert_eq!(arena, *b"KEY");
    let memory = Names::new(b"", b"");
    let (mut used, mut required) = (0, 0);
    let name = display_name(&memory, 0x100, &mut Vec::new(), &mut used, &mut required).unwrap();
    assert_eq!(name.fallback_key, Some(Span::default()));
    assert_eq!((used, required), (0, 0));
}

#[test]
fn long_names_grow_to_the_exact_demand_and_never_publish_truncation() {
    let text = b"template literal longer than inline storage";
    let memory = Names::new(text, b"unused");
    let mut arena = vec![0; 2];
    let (mut used, mut required) = (0, 0);
    let name = display_name(&memory, 0x100, &mut arena, &mut used, &mut required).unwrap();
    assert_eq!(name.literal.len, text.len());
    assert_eq!(required, text.len());
    assert_eq!(&arena[..text.len()], text);
}

#[test]
fn failed_payload_keeps_the_committed_prefix_and_no_new_span() {
    let mut memory = Names::new(b"long template literal from heap", b"unused");
    memory.refuse = Some(0x300);
    let mut arena = vec![0; 64];
    arena[..3].copy_from_slice(b"old");
    let (mut used, mut required) = (3, 0);
    assert!(display_name(&memory, 0x100, &mut arena, &mut used, &mut required).is_err());
    assert_eq!(used, 3);
    assert_eq!(&arena[..3], b"old");
    assert_eq!(required, b"long template literal from heap".len());
}
