use super::*;
use crate::{memory::SliceMemory, modifier_keys};

struct Fixture(Vec<u8>);

impl Memory for Fixture {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if address == modifier_definition::TABLE_RVA && output.len() == 8 {
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
    fn pointers(&mut self, address: usize, values: &[usize]) {
        let data = self.alloc(values.len() * 8);
        for (index, &value) in values.iter().enumerate() {
            self.word(data + index * 8, value as u64);
        }
        self.array(address, data, values.len());
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
    requests: &'a mut [Request],
    roots: &'a mut [Root],
    nodes: &'a mut [RawNode],
    text: &'a mut [u8],
    stack: &'a mut [Frame],
) -> Buffers<'a> {
    let request_len = requests.len();
    Buffers {
        requests,
        request_len,
        request_required: request_len,
        roots,
        root_len: 0,
        root_required: 0,
        nodes,
        node_len: 0,
        node_required: 0,
        text,
        text_len: 0,
        text_required: 0,
        stack,
        stack_required: 0,
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
    let mut requests = [
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
    let mut roots = [Root::default(); 2];
    let mut nodes = [RawNode::default(); 10];
    let mut text = [0; 256];
    let mut stack = [Frame::default(); 3];
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
    let mut requests = [
        Request {
            accumulator: bad,
            key,
        },
        Request {
            accumulator: good,
            key,
        },
    ];
    let mut roots = [Root::default(); 2];
    let mut nodes = [RawNode::default(); 2];
    let mut text = [0; 16];
    let mut stack = [Frame::default(); 2];
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
fn attrition_five_thousand_armies_share_each_requested_tree_once() {
    let mut fixture = Fixture::new();
    let key = modifier_keys::ATTRITION;
    let root = fixture.node(b"shared", &[(key, 100)], &[]);
    let mut requests = vec![
        Request {
            accumulator: root,
            key
        };
        5_000 * crate::carmy_attrition::SOURCE_COUNT
    ];
    let mut roots = [Root::default(); 1];
    let mut nodes = [RawNode::default(); 1];
    let mut text = [0; 6];
    let mut stack = [Frame::default(); 1];
    let mut buffers = buffers(&mut requests, &mut roots, &mut nodes, &mut text, &mut stack);
    capture(&fixture, 0, &mut buffers).unwrap();
    assert_eq!(
        (
            buffers.request_required,
            buffers.request_len,
            buffers.root_len,
            buffers.node_required
        ),
        (50_000, 1, 1, 1)
    );
    assert_eq!(&buffers.text[..buffers.text_len], b"shared");
}

#[test]
fn attrition_short_node_and_text_arenas_count_complete_demand_and_keep_small_root() {
    let mut fixture = Fixture::new();
    let key = modifier_keys::ATTRITION;
    let leaf = fixture.node(&[b'x'; 80], &[(key, 3)], &[]);
    let large = fixture.node(b"large", &[], &[leaf, leaf, leaf]);
    let good = fixture.node(b"ok", &[(key, 2)], &[]);
    let mut requests = [
        Request {
            accumulator: large,
            key,
        },
        Request {
            accumulator: good,
            key,
        },
    ];
    let mut roots = [Root::default(); 2];
    let mut nodes = [RawNode::default(); 2];
    let mut text = [0; 8];
    let mut stack = [Frame::default(); 2];
    let mut buffers = buffers(&mut requests, &mut roots, &mut nodes, &mut text, &mut stack);
    capture(&fixture, 0, &mut buffers).unwrap();
    assert_eq!(buffers.roots[0].failure, Some(TEXT_CAPACITY));
    assert_eq!(buffers.roots[1].failure, None);
    assert_eq!(
        (
            buffers.node_required,
            buffers.text_required,
            buffers.stack_required
        ),
        (5, 242, 2)
    );
    assert_eq!((buffers.node_len, buffers.text_len), (1, 2));
}

#[test]
#[cfg(windows)]
fn attrition_short_depth_counts_all_descendants_and_detects_cycles() {
    let mut fixture = Fixture::new();
    let key = modifier_keys::ATTRITION;
    let mut root = fixture.node(b"leaf", &[(key, 1)], &[]);
    for _ in 0..24 {
        root = fixture.node(b"", &[], &[root]);
    }
    let mut requests = [Request {
        accumulator: root,
        key,
    }];
    let mut roots = [Root::default(); 1];
    let mut nodes = [RawNode::default(); 25];
    let mut text = [0; 4];
    let mut stack = [Frame::default(); 1];
    let mut buffers = buffers(&mut requests, &mut roots, &mut nodes, &mut text, &mut stack);
    capture(&fixture, 0, &mut buffers).unwrap();
    assert_eq!(buffers.roots[0].failure, Some(DEPTH_CAPACITY));
    assert_eq!(
        (
            buffers.node_required,
            buffers.text_required,
            buffers.stack_required
        ),
        (25, 4, 25)
    );
}

#[test]
fn attrition_root_or_request_shortage_never_certifies_a_partial_identity_census() {
    let mut fixture = Fixture::new();
    let key = modifier_keys::ATTRITION;
    let first = fixture.node(b"", &[], &[]);
    let second = fixture.node(b"", &[], &[]);
    let mut requests = [
        Request {
            accumulator: first,
            key,
        },
        Request {
            accumulator: second,
            key,
        },
    ];
    let mut roots = [Root::default(); 1];
    let mut nodes = [RawNode::default(); 2];
    let mut text = [];
    let mut stack = [Frame::default(); 1];
    let mut buffers = buffers(&mut requests, &mut roots, &mut nodes, &mut text, &mut stack);
    assert_eq!(capture(&fixture, 0, &mut buffers), Err(ROOT_CAPACITY));
    assert_eq!((buffers.root_required, buffers.node_required), (2, 2));
    buffers.request_len = 1;
    buffers.request_required = 2;
    assert_eq!(capture(&fixture, 0, &mut buffers), Err(REQUEST_CAPACITY));
}

#[test]
#[cfg(windows)]
fn attrition_live_census_sizing_keeps_cached_name_and_origins_when_tree_is_unreadable() {
    use crate::{
        carmy, ccountry, cgame_state, cname_group_member, cprovince, cunit, cunit_supply_inputs,
    };
    let mut fixture = Fixture::new();
    let state = fixture.alloc(0x500);
    let country = fixture.alloc(0x2000);
    let province = fixture.alloc(0x200);
    let army = fixture.alloc(0x800);
    let member = fixture.alloc(0xb0);
    fixture.word(country, ccountry::VTABLE_RVA as u64);
    fixture.put(country + ccountry::ID, &1_u32.to_le_bytes());
    fixture.pointers(state + cgame_state::COUNTRIES, &[0, country]);
    let links = fixture.alloc(8);
    fixture.put(links + 4, &1_u32.to_le_bytes());
    fixture.array(state + cgame_state::COUNTRY_LINKS, links, 2);
    fixture.word(province, cprovince::VTABLE_RVA as u64);
    fixture.put(province + cprovince::ID, &1_u32.to_le_bytes());
    fixture.pointers(state + cgame_state::PROVINCES, &[0, province]);
    fixture.word(army, carmy::VTABLE_RVA as u64);
    fixture.word(
        army + carmy::SECONDARY_BASE,
        carmy::SECONDARY_VTABLE_RVA as u64,
    );
    fixture.word(army + cunit::HANDLE, (1_u64 << 32) | 51);
    fixture.put(army + cunit::ROSTER_OWNER, &1_u32.to_le_bytes());
    fixture.put(army + cunit::LOGICAL_OWNER, &1_u32.to_le_bytes());
    fixture.word(army + cunit::LOCATION, province as u64);
    fixture.word(
        army + crate::carmy_combat_inputs::NAME_GROUP_MEMBER,
        member as u64,
    );
    fixture.pointers(country + ccountry::DIVISIONS, &[army]);
    fixture.word(member, cname_group_member::VTABLE_RVA as u64);
    fixture.put(member + cname_group_member::CACHED_NAME, b"exact name");
    fixture.word(member + cname_group_member::CACHED_NAME + 16, 10);
    fixture.word(member + cname_group_member::CACHED_NAME + 24, 15);
    let origins = fixture.alloc(2 * cunit_supply_inputs::ORIGIN_STRIDE);
    fixture.array(army + cunit_supply_inputs::ORIGINS, origins, 2);
    // Scalars read an empty root; its independent formatter child is null.
    let broken = fixture.alloc(8);
    fixture.array(country + ccountry::MODIFIER_ACCUMULATOR + 0x18, broken, 1);
    let demand = demand(&fixture, 0, state).unwrap();
    assert_eq!(
        (demand.rows, demand.cached_name_text, demand.supply_origins),
        (1, 10, 2)
    );
    assert_eq!(
        (demand.requests, demand.roots, demand.nodes, demand.depth),
        (1, 1, 1, 1)
    );
}
