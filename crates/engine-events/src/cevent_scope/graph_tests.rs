use super::*;
use std::cell::RefCell;

const BASE: usize = 0x1000_0000;
const START: usize = 0x1000;

struct Image {
    bytes: Vec<u8>,
    reads: RefCell<Vec<(usize, usize)>>,
    fault: Option<usize>,
}
impl Image {
    fn new(count: usize) -> Self {
        let mut image = Self {
            bytes: vec![0; count * SIZE],
            reads: RefCell::new(Vec::new()),
            fault: None,
        };
        for n in 0..count {
            image.word(n, 0, BASE + VTABLE_RVA.0);
            for offset in [ROOT, FROM, PREV] {
                image.word(n, offset, START + n * SIZE);
            }
        }
        image
    }
    fn word(&mut self, node: usize, offset: usize, value: usize) {
        let at = node * SIZE + offset;
        self.bytes[at..at + 8].copy_from_slice(&(value as u64).to_le_bytes());
    }
}
impl engine_native_memory::Memory for Image {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, output.len()));
        if self
            .fault
            .is_some_and(|at| address <= at && at < address.saturating_add(output.len()))
        {
            return false;
        }
        let Some(offset) = address.checked_sub(START) else {
            return false;
        };
        let Some(end) = offset.checked_add(output.len()) else {
            return false;
        };
        let Some(source) = self.bytes.get(offset..end) else {
            return false;
        };
        output.copy_from_slice(source);
        true
    }
}

#[test]
fn graph_preserves_shared_bindings_self_links_and_full_reference_words() {
    let mut image = Image::new(3);
    image.word(0, FROM, START + SIZE);
    image.word(0, PREV, START + SIZE);
    image.word(1, ROOT, START);
    image.word(1, FROM, START + 2 * SIZE);
    image.word(2, REFERENCE_FIELDS[0], 0xabcd_0123_7654_3210);
    let mut nodes = [Node::default(); 3];
    let mut index = [IndexSlot::default(); 7];
    let graph = copy_graph(&image, BASE, START, &mut nodes, &mut index);
    assert_eq!(
        graph,
        Graph {
            len: 3,
            required: 3,
            failure: None
        }
    );
    assert_eq!(nodes[0].operands.bindings, [0, 1, 1]);
    assert_eq!(nodes[1].operands.bindings, [0, 2, 1]);
    assert_eq!(nodes[2].operands.bindings, [2, 2, 2]);
    assert_eq!(nodes[2].operands.references[0], 0xabcd_0123_7654_3210);
    assert_eq!(nodes[0].operands.country, 0);
}

#[test]
fn graph_visits_only_the_occurrences_reachable_scopes_and_reuses_storage() {
    let image = Image::new(5_000);
    let mut nodes = [Node::default(); 4];
    let mut index = [IndexSlot::default(); 9];
    for selected in [0, 1_499, 4_999] {
        image.reads.borrow_mut().clear();
        let root = START + selected * SIZE;
        assert_eq!(
            copy_graph(&image, BASE, root, &mut nodes, &mut index).len,
            1
        );
        assert!(
            image
                .reads
                .borrow()
                .iter()
                .all(|&(at, len)| at >= root && at + len <= root + SIZE)
        );
        assert_eq!(nodes[0].operands.bindings, [0; 3]);
    }
}

#[test]
fn deep_scope_graph_uses_no_recursion_or_chosen_depth_limit() {
    let count = 5_000;
    let mut image = Image::new(count);
    for n in 0..count - 1 {
        image.word(n, FROM, START + (n + 1) * SIZE);
    }
    let mut nodes = vec![Node::default(); count];
    let mut index = vec![IndexSlot::default(); count * 2 + 1];
    assert_eq!(
        copy_graph(&image, BASE, START, &mut nodes, &mut index),
        Graph {
            len: count,
            required: count,
            failure: None
        }
    );
    assert_eq!(nodes[count - 1].operands.bindings, [count as u32 - 1; 3]);
}

#[test]
fn shortage_fault_and_unproved_operands_do_not_become_complete_absence() {
    let mut image = Image::new(2);
    image.word(0, FROM, START + SIZE);
    let mut nodes = [Node::default(); 1];
    let mut index = [IndexSlot::default(); 5];
    let graph = copy_graph(&image, BASE, START, &mut nodes, &mut index);
    assert_eq!(graph.len, 0);
    assert_eq!(graph.required, 2);
    assert_eq!(
        graph.failure,
        Some("event scope graph needs prepared storage")
    );
    let mut nodes = [Node::default(); 2];
    image.fault = Some(START + SIZE + STATE_ID);
    let graph = copy_graph(&image, BASE, START, &mut nodes, &mut index);
    assert_eq!(graph.len, 1);
    assert_eq!(graph.failure, Some("event scope state unreadable"));
    image.fault = None;
    image.word(1, TARGET_STORAGE, 0x1234);
    image.word(1, COMBAT_CONTEXT, 0x5678);
    assert_eq!(
        copy_graph(&image, BASE, START, &mut nodes, &mut index).failure,
        None
    );
    assert!(nodes[1].operands.target_storage_unrecorded);
    assert!(nodes[1].operands.combat_context_unrecorded);
}
