use super::*;
use engine_native_memory::memory::SliceMemory;
use crate::modifier_keys;

struct Fixture(Vec<u8>);

impl Memory for Fixture {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if address == modifier_definition::TABLE_RVA.0 && output.len() == 8 {
            output.copy_from_slice(&0x3000_0000_u64.to_le_bytes());
            return true;
        }
        if (0x3000_0000..0x3004_0000).contains(&address) {
            output.fill(0);
            return true;
        }
        SliceMemory {
            base: 0,
            bytes: &self.0,
        }
        .copy(address, output)
    }
}

impl Fixture {
    fn new() -> Self {
        Self(vec![0; 0x2000])
    }
    fn alloc(&mut self, size: usize) -> usize {
        let first = (self.0.len() + 7) & !7;
        self.0.resize(first + size, 0);
        first
    }
    fn put(&mut self, address: usize, bytes: &[u8]) {
        self.0[address..address + bytes.len()].copy_from_slice(bytes);
    }
    fn word(&mut self, address: usize, value: u64) {
        self.put(address, &value.to_le_bytes());
    }
    fn array(&mut self, address: usize, data: usize, count: usize) {
        self.word(address, data as u64);
        self.word(address + 8, (count as u64) * 0x1_0000_0001);
    }
    fn node(&mut self, name: &[u8], values: &[(u32, i64)], children: &[usize]) -> usize {
        let address = self.alloc(0xb0);
        let entries = self.alloc(values.len() * 16);
        self.array(address, entries, values.len());
        for (index, &(key, value)) in values.iter().enumerate() {
            self.put(entries + index * 16, &key.to_le_bytes());
            self.word(entries + index * 16 + 8, value as u64);
        }
        let data = self.alloc(children.len() * 8);
        self.array(address + 0x18, data, children.len());
        for (index, child) in children.iter().enumerate() {
            self.word(data + index * 8, (*child - 0x10) as u64);
        }
        self.word(address + 0x48 + 16, name.len() as u64);
        if name.len() < 16 {
            self.put(address + 0x48, name);
            self.word(address + 0x48 + 24, 15);
        } else {
            let data = self.alloc(name.len() + 1);
            self.put(data, name);
            self.word(address + 0x48, data as u64);
            self.word(address + 0x48 + 24, name.len() as u64);
        }
        address
    }
}

fn buffers<'a>(
    requests: &'a mut Vec<Request>,
    roots: &'a mut Vec<Root>,
    nodes: &'a mut Vec<RawNode>,
    text: &'a mut Vec<u8>,
    stack: &'a mut Vec<Frame>,
) -> Buffers<'a> {
    let request_len = requests.len();
    Buffers {
        requests: engine_native_memory::walk::Buffer::Growing(requests),
        request_len,
        request_required: request_len,
        roots: engine_native_memory::walk::Buffer::Growing(roots),
        root_len: 0,
        root_required: 0,
        nodes: engine_native_memory::walk::Buffer::Growing(nodes),
        node_len: 0,
        node_required: 0,
        text: engine_native_memory::walk::Buffer::Growing(text),
        text_len: 0,
        text_required: 0,
        stack: engine_native_memory::walk::Buffer::Growing(stack),
        stack_required: 0,
        repeats: None,
    }
}

#[test]
fn cold_worker_formatter_arenas_keep_native_labels_and_later_tree_growth() {
    let mut roots = Vec::new();
    let mut nodes = Vec::new();
    let mut text = Vec::new();
    let mut stack = Vec::new();
    for count in [3_usize, 7] {
        let mut fixture = Fixture::new();
        let key = modifier_keys::ATTRITION;
        let label = vec![b'M'; 300];
        let children = (0..count)
            .map(|_| fixture.node(&label, &[(key, 100_000)], &[]))
            .collect::<Vec<_>>();
        let root = fixture.node(b"", &[], &children);
        let mut requests = vec![Request {
            accumulator: root,
            key,
        }];
        let mut captured = Buffers {
            requests: engine_native_memory::walk::Buffer::Growing(&mut requests),
            request_len: 1,
            request_required: 1,
            roots: engine_native_memory::walk::Buffer::Growing(&mut roots),
            root_len: 0,
            root_required: 0,
            nodes: engine_native_memory::walk::Buffer::Growing(&mut nodes),
            node_len: 0,
            node_required: 0,
            text: engine_native_memory::walk::Buffer::Growing(&mut text),
            text_len: 0,
            text_required: 0,
            stack: engine_native_memory::walk::Buffer::Growing(&mut stack),
            stack_required: 0,
            repeats: None,
        };
        capture(&fixture, 0, &mut captured).unwrap();
        assert_eq!(
            (captured.root_len, captured.node_len, captured.text_len),
            (1, count + 1, count * label.len())
        );
        assert!(
            captured.roots[..1]
                .iter()
                .all(|root| root.failure.is_none())
        );
        assert_eq!(
            captured.nodes[count].name,
            [(count - 1) * label.len(), label.len()]
        );
        assert!(
            captured.text[..captured.text_len]
                .iter()
                .all(|&byte| byte == b'M')
        );
        let mut requests = vec![Request {
            accumulator: root,
            key,
        }];
        let mut enough_roots = vec![Root::default(); 1];
        let mut enough_nodes = vec![RawNode::default(); count + 1];
        let mut enough_text = vec![0; count * label.len()];
        let mut enough_stack = vec![Frame::default(); 2];
        let mut expected = buffers(
            &mut requests,
            &mut enough_roots,
            &mut enough_nodes,
            &mut enough_text,
            &mut enough_stack,
        );
        capture(&fixture, 0, &mut expected).unwrap();
        assert_eq!(
            &captured.nodes[..captured.node_len],
            &expected.nodes[..expected.node_len]
        );
        assert_eq!(
            &captured.text[..captured.text_len],
            &expected.text[..expected.text_len]
        );
    }
}

#[test]
fn attrition_trees_deduplicate_pairs_and_keep_aliases_interior_labels_and_zero_nodes() {
    let mut fixture = Fixture::new();
    let keys = [modifier_keys::ATTRITION, modifier_keys::HEAT_ATTRITION];
    let leaf = fixture.node(b"literal label", &[(keys[0], -700), (keys[1], 300)], &[]);
    let zero = fixture.node(b"unused", &[(keys[0], 0), (keys[1], 0)], &[]);
    let branch = fixture.node(
        b"interior cutoff",
        &[(keys[0], 100), (keys[1], 400)],
        &[leaf, zero, leaf],
    );
    let root = fixture.node(b"root", &[], &[branch]);
    let mut requests = vec![
        Request {
            accumulator: root,
            key: keys[1],
        },
        Request {
            accumulator: root,
            key: keys[0],
        },
        Request {
            accumulator: root,
            key: keys[0],
        },
    ];
    let mut roots = vec![Root::default(); 2];
    let mut nodes = vec![RawNode::default(); 10];
    let mut text = vec![0; 256];
    let mut stack = vec![Frame::default(); 3];
    let mut buffers = buffers(&mut requests, &mut roots, &mut nodes, &mut text, &mut stack);
    capture(&fixture, 0, &mut buffers).unwrap();
    assert_eq!(
        (
            buffers.request_required,
            buffers.request_len,
            buffers.root_len
        ),
        (3, 2, 2)
    );
    assert_eq!(
        (
            buffers.node_len,
            buffers.node_required,
            buffers.stack_required
        ),
        (10, 10, 3)
    );
    assert!(buffers.roots.iter().all(|root| root.failure.is_none()));
    assert_eq!(buffers.roots[0].nodes, [0, 5]);
    assert_eq!(
        buffers.nodes[..5]
            .iter()
            .map(|node| (node.children, node.value))
            .collect::<Vec<_>>(),
        [(1, 0), (3, 100), (0, -700), (0, 0), (0, -700)]
    );
    let [first, length] = buffers.nodes[1].name;
    assert_eq!(&buffers.text[first..first + length], b"interior cutoff");
    assert_eq!(buffers.nodes[3].name[1], 0);
}

#[test]
fn attrition_failed_tree_releases_storage_and_preserves_other_root() {
    let mut fixture = Fixture::new();
    let key = modifier_keys::ATTRITION;
    let bad = fixture.node(b"bad", &[(key, 1)], &[]);
    let cycle = fixture.alloc(8);
    fixture.word(cycle, (bad - 0x10) as u64);
    fixture.array(bad + 0x18, cycle, 1);
    let good = fixture.node(b"good", &[(key, 2)], &[]);
    let mut requests = vec![
        Request {
            accumulator: bad,
            key,
        },
        Request {
            accumulator: good,
            key,
        },
    ];
    let mut roots = vec![Root::default(); 2];
    let mut nodes = vec![RawNode::default(); 2];
    let mut text = vec![0; 16];
    let mut stack = vec![Frame::default(); 2];
    let mut buffers = buffers(&mut requests, &mut roots, &mut nodes, &mut text, &mut stack);
    capture(&fixture, 0, &mut buffers).unwrap();
    assert_eq!(
        buffers.roots[0].failure,
        Some("attrition modifier null node or ancestor cycle")
    );
    assert_eq!(buffers.roots[0].nodes, [0, 0]);
    assert_eq!(buffers.roots[1].failure, None);
    assert_eq!(buffers.roots[1].nodes, [0, 1]);
    assert_eq!(buffers.nodes[0].value, 2);
    assert_eq!(&buffers.text[..buffers.text_len], b"good");
    assert_eq!(buffers.node_required, 2);
    assert_eq!(buffers.text_required, 7);
}

#[test]
fn attrition_armies_share_each_requested_tree_once() {
    let mut fixture = Fixture::new();
    let key = modifier_keys::ATTRITION;
    let root = fixture.node(b"shared", &[(key, 100)], &[]);
    let mut requests = vec![
        Request {
            accumulator: root,
            key
        };
        3 * crate::carmy_attrition::SOURCE_COUNT
    ];
    let mut roots = vec![Root::default(); 1];
    let mut nodes = vec![RawNode::default(); 1];
    let mut text = vec![0; 6];
    let mut stack = vec![Frame::default(); 1];
    let mut buffers = buffers(&mut requests, &mut roots, &mut nodes, &mut text, &mut stack);
    capture(&fixture, 0, &mut buffers).unwrap();
    assert_eq!(
        (
            buffers.request_required,
            buffers.request_len,
            buffers.root_len,
            buffers.node_required
        ),
        (3 * crate::carmy_attrition::SOURCE_COUNT, 1, 1, 1)
    );
    assert_eq!(&buffers.text[..buffers.text_len], b"shared");
}

#[test]
fn attrition_request_shortage_never_certifies_a_partial_identity_census() {
    let mut fixture = Fixture::new();
    let key = modifier_keys::ATTRITION;
    let first = fixture.node(b"", &[], &[]);
    let second = fixture.node(b"", &[], &[]);
    let mut requests = vec![
        Request {
            accumulator: first,
            key,
        },
        Request {
            accumulator: second,
            key,
        },
    ];
    let mut roots = Vec::new();
    let mut nodes = Vec::new();
    let mut text = Vec::new();
    let mut stack = Vec::new();
    let mut buffers = buffers(&mut requests, &mut roots, &mut nodes, &mut text, &mut stack);
    buffers.request_len = 1;
    buffers.request_required = 2;
    assert_eq!(capture(&fixture, 0, &mut buffers), Err(REQUEST_CAPACITY));
}

struct Logged<'a> {
    fixture: &'a Fixture,
    reads: std::cell::RefCell<Vec<(usize, usize)>>,
    faults: std::cell::RefCell<Vec<usize>>,
    persistent: &'a [usize],
}

impl Memory for Logged<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, output.len()));
        if self.persistent.contains(&address) {
            return false;
        }
        let mut faults = self.faults.borrow_mut();
        if let Some(at) = faults.iter().position(|&fault| fault == address) {
            faults.remove(at);
            return false;
        }
        self.fixture.copy(address, output)
    }
}

#[derive(Debug, PartialEq)]
struct Trees {
    roots: Vec<Root>,
    nodes: Vec<RawNode>,
    text: Vec<u8>,
    demand: [usize; 5],
}

fn logged_capture(
    fixture: &Fixture,
    requests: &[Request],
    faults: &[usize],
    persistent: &[usize],
    repeats: Option<&mut RepeatCache>,
) -> (Trees, Vec<(usize, usize)>) {
    let memory = Logged {
        fixture,
        reads: std::cell::RefCell::new(Vec::new()),
        faults: std::cell::RefCell::new(faults.to_vec()),
        persistent,
    };
    let mut requests = requests.to_vec();
    let (mut roots, mut nodes, mut text, mut stack) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let len = requests.len();
    let mut buffers = Buffers {
        requests: engine_native_memory::walk::Buffer::Growing(&mut requests),
        request_len: len,
        request_required: len,
        roots: engine_native_memory::walk::Buffer::Growing(&mut roots),
        root_len: 0,
        root_required: 0,
        nodes: engine_native_memory::walk::Buffer::Growing(&mut nodes),
        node_len: 0,
        node_required: 0,
        text: engine_native_memory::walk::Buffer::Growing(&mut text),
        text_len: 0,
        text_required: 0,
        stack: engine_native_memory::walk::Buffer::Growing(&mut stack),
        stack_required: 0,
        repeats,
    };
    capture(&memory, 0, &mut buffers).unwrap();
    let lens = (buffers.root_len, buffers.node_len, buffers.text_len);
    let demand = [
        buffers.request_required,
        buffers.root_required,
        buffers.node_required,
        buffers.text_required,
        buffers.stack_required,
    ];
    let trees = Trees {
        roots: roots[..lens.0].to_vec(),
        nodes: nodes[..lens.1].to_vec(),
        text: text[..lens.2].to_vec(),
        demand,
    };
    (trees, memory.reads.into_inner())
}

fn omits_only_successful_repeats(
    reads: &[(usize, usize)],
    every: &[(usize, usize)],
    one_shot: &[usize],
) -> bool {
    let mut faults = one_shot.to_vec();
    let mut succeeded: Vec<(usize, usize)> = Vec::new();
    let mut next = 0;
    for &read in every {
        if reads.get(next) == Some(&read) {
            next += 1;
        } else if !succeeded.contains(&read) {
            return false;
        }
        if let Some(at) = faults.iter().position(|&fault| fault == read.0) {
            faults.remove(at);
        } else if !succeeded.contains(&read) {
            succeeded.push(read);
        }
    }
    next == reads.len()
}

fn without_repeats(reads: &[(usize, usize)], one_shot: &[usize]) -> Vec<(usize, usize)> {
    let mut faults = one_shot.to_vec();
    let mut kept: Vec<(usize, usize)> = Vec::new();
    let mut succeeded: Vec<(usize, usize)> = Vec::new();
    for &read in reads {
        if succeeded.contains(&read) {
            continue;
        }
        kept.push(read);
        if let Some(at) = faults.iter().position(|&fault| fault == read.0) {
            faults.remove(at);
        } else {
            succeeded.push(read);
        }
    }
    kept
}

fn repeated_tree(fixture: &mut Fixture) -> ([u32; 3], usize, usize, [usize; 2]) {
    let keys = [
        modifier_keys::ATTRITION,
        modifier_keys::HEAT_ATTRITION,
        modifier_keys::REFIT_SPEED,
    ];
    let long = fixture.node(
        b"a label held on the heap",
        &[(keys[0], 5), (keys[2], 7)],
        &[],
    );
    let short = fixture.node(b"short", &[(keys[1], 3)], &[]);
    let branch = fixture.node(
        b"branch",
        &[(keys[0], 1), (keys[1], 2), (keys[2], 0)],
        &[long, short, long],
    );
    let root = fixture.node(b"", &[], &[branch]);
    let other = fixture.node(b"other", &[(keys[1], 4)], &[short]);
    (keys, root, other, [long, short])
}

#[test]
fn an_accumulators_keys_repeat_no_read_and_build_the_same_trees() {
    let mut fixture = Fixture::new();
    let (keys, root, _, _) = repeated_tree(&mut fixture);
    let requests = keys.map(|key| Request {
        accumulator: root,
        key,
    });
    let (trees, every) = logged_capture(&fixture, &requests, &[], &[], None);
    let mut cache = RepeatCache::default();
    let (cached, reads) = logged_capture(&fixture, &requests, &[], &[], Some(&mut cache));
    assert!(trees.roots.iter().all(|root| root.failure.is_none()));
    assert_eq!(cached, trees);
    assert_eq!(reads, without_repeats(&every, &[]));
    assert!(reads.len() * 2 < every.len());
}

#[test]
fn a_wide_tree_grows_the_cache_and_repeats_no_read() {
    let mut fixture = Fixture::new();
    let keys = [modifier_keys::ATTRITION, modifier_keys::HEAT_ATTRITION];
    let leaves: Vec<_> = (0..200_i64)
        .map(|index| {
            let label = format!("leaf {index} with a label held on the heap");
            fixture.node(
                label.as_bytes(),
                &[(keys[(index % 2) as usize], index + 1)],
                &[],
            )
        })
        .collect();
    let root = fixture.node(b"root", &[(keys[0], 1)], &leaves);
    let requests = keys.map(|key| Request {
        accumulator: root,
        key,
    });
    let (trees, every) = logged_capture(&fixture, &requests, &[], &[], None);
    let mut cache = RepeatCache::default();
    let (cached, reads) = logged_capture(&fixture, &requests, &[], &[], Some(&mut cache));
    assert_eq!(cached, trees);
    assert_eq!(reads, without_repeats(&every, &[]));
    assert!(cache.allocated_bytes() > 0);
}

#[test]
fn repeats_are_kept_only_within_one_accumulator_and_one_capture() {
    let mut fixture = Fixture::new();
    let (keys, root, other, [_, short]) = repeated_tree(&mut fixture);
    let requests: Vec<_> = [root, other]
        .into_iter()
        .flat_map(|accumulator| keys.map(|key| Request { accumulator, key }))
        .collect();
    let (trees, _) = logged_capture(&fixture, &requests, &[], &[], None);
    let mut cache = RepeatCache::default();
    for _ in 0..2 {
        let (cached, reads) = logged_capture(&fixture, &requests, &[], &[], Some(&mut cache));
        assert_eq!(cached, trees);
        let header = (short + 0x18, 16);
        assert_eq!(reads.iter().filter(|&&read| read == header).count(), 2);
    }
}

#[test]
fn a_failed_read_is_made_again_by_the_next_key() {
    let mut fixture = Fixture::new();
    let (keys, root, _, [long, _]) = repeated_tree(&mut fixture);
    let requests = keys.map(|key| Request {
        accumulator: root,
        key,
    });
    let entries = u64::from_le_bytes(fixture.0[long..long + 8].try_into().unwrap()) as usize;
    let label =
        u64::from_le_bytes(fixture.0[long + 0x48..long + 0x50].try_into().unwrap()) as usize;
    let label_len = b"a label held on the heap".len();
    for fault in [long + 0x18, long, entries, label + label_len, label] {
        let (trees, every) = logged_capture(&fixture, &requests, &[fault], &[], None);
        assert!(trees.roots[0].failure.is_some(), "{fault:#x}");
        assert!(trees.roots[1..].iter().all(|root| root.failure.is_none()));
        let mut cache = RepeatCache::default();
        let (cached, reads) = logged_capture(&fixture, &requests, &[fault], &[], Some(&mut cache));
        assert_eq!(cached, trees, "{fault:#x}");
        assert!(
            omits_only_successful_repeats(&reads, &every, &[fault]),
            "{fault:#x}"
        );
        assert!(reads.len() < every.len());
        assert!(reads.iter().filter(|read| read.0 == fault).count() >= 2);
    }
    let fault = long + 0x18;
    let (trees, every) = logged_capture(&fixture, &requests, &[], &[fault], None);
    assert!(trees.roots.iter().all(|root| root.failure.is_some()));
    let mut cache = RepeatCache::default();
    let (cached, reads) = logged_capture(&fixture, &requests, &[], &[fault], Some(&mut cache));
    assert_eq!(cached, trees);
    assert_eq!(reads, without_repeats(&every, &[fault; 3]));
    assert!(omits_only_successful_repeats(&reads, &every, &[fault; 3]));
}
