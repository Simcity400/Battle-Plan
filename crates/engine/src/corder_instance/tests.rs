use super::*;
use crate::{creference_registry as registry, reference_bucket as bucket};
use std::collections::BTreeMap;

const BASE: usize = 0x1000_0000;
const TABLE: usize = 0x10000;
const BUCKETS: usize = 0x11000;
const GROUP: usize = 0x20000;
const ROOTS: usize = 0x21000;
const FIRST: usize = 0x30000;
const SECOND: usize = 0x31000;
const CHILD: usize = 0x32000;
const CHILDREN_A: usize = 0x40000;
const CHILDREN_B: usize = 0x40100;
const PATH: usize = 0x41000;
const TARGETS: usize = 0x42000;
const HANDLE: u64 = (u32::MAX as u64) << 32 | corders_group::REFERENCE_TYPE as u64;

#[derive(Default)]
struct MemoryImage(
    BTreeMap<usize, Vec<u8>>,
    std::cell::RefCell<Vec<(usize, usize)>>,
);

impl Memory for MemoryImage {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.1.borrow_mut().push((address, output.len()));
        let Some((&base, bytes)) = self.0.range(..=address).next_back() else {
            return false;
        };
        let offset = address - base;
        let Some(end) = offset.checked_add(output.len()) else {
            return false;
        };
        let Some(source) = bytes.get(offset..end) else {
            return false;
        };
        output.copy_from_slice(source);
        true
    }
}

impl MemoryImage {
    fn bytes(&mut self, address: usize, value: &[u8]) {
        if let Some((&base, bytes)) = self.0.range_mut(..=address).next_back() {
            let offset = address - base;
            if let Some(out) = bytes.get_mut(offset..offset + value.len()) {
                out.copy_from_slice(value);
                return;
            }
        }
        self.0.insert(address, value.to_vec());
    }
    fn u64(&mut self, address: usize, value: u64) {
        self.bytes(address, &value.to_le_bytes());
    }
    fn u32(&mut self, address: usize, value: u32) {
        self.bytes(address, &value.to_le_bytes());
    }
    fn array(&mut self, address: usize, data: usize, count: u32) {
        self.u64(address, data as u64);
        self.u32(address + 8, count);
        self.u32(address + 12, count);
    }
    fn order(&mut self, address: usize, instance: u32, kind: i32) {
        self.bytes(address, &[0; 0x400]);
        self.u64(address, (BASE + VTABLE_RVA) as u64);
        self.u64(address + OWNER, GROUP as u64);
        self.u32(address + ID, instance);
        self.u32(address + TYPE, kind as u32);
    }
}

fn fixture() -> MemoryImage {
    let mut image = MemoryImage::default();
    image.u64(
        BASE + registry::LOW_ROOTS_RVA + corders_group::REFERENCE_TYPE as usize * 8,
        TABLE as u64,
    );
    image.bytes(TABLE, &[0; 32]);
    image.u64(TABLE + registry::ENTRIES, BUCKETS as u64);
    image.u32(TABLE + registry::COUNT, 1);
    image.u32(TABLE + registry::MASK, 7);
    image.bytes(TABLE + registry::OVERFLOW_DISTANCE, &[5]);
    image.bytes(BUCKETS, &[0; 14 * bucket::STRIDE]);
    image.bytes(BUCKETS + bucket::DISTANCE, &[1]);
    image.u64(BUCKETS + bucket::TYPE, HANDLE);
    image.u64(BUCKETS + bucket::OBJECT, GROUP as u64);
    image.bytes(
        BUCKETS + 13 * bucket::STRIDE + bucket::DISTANCE,
        &[bucket::TERMINAL_DISTANCE],
    );
    image.bytes(GROUP, &[0; 0x240]);
    image.u64(GROUP, (BASE + corders_group::VTABLE_RVA) as u64);
    image.u64(GROUP + corders_group::HANDLE, HANDLE);
    image.u32(GROUP + corders_group::COUNTRY, 0xffff_fffe);
    // An empty inline group name: capacity 15, length 0.
    image.u64(GROUP + corders_group::NAME + 0x18, 15);
    image.array(GROUP + corders_group::PRIMARY_ORDER_DATA, ROOTS, 2);
    image.bytes(ROOTS, &[0; 16]);
    image.u64(ROOTS, FIRST as u64);
    image.u64(ROOTS + 8, SECOND as u64);
    image.order(FIRST, 0, TYPE_LINE);
    image.order(SECOND, u32::MAX, TYPE_AREA_DEFENSE);
    image.order(CHILD, 0x8000_0000, i32::MIN);
    image.array(FIRST + SUCCESSOR_DATA, CHILDREN_A, 1);
    image.u64(CHILDREN_A, CHILD as u64);
    image.array(SECOND + SUCCESSOR_DATA, CHILDREN_B, 1);
    image.u64(CHILDREN_B, CHILD as u64);
    image.array(FIRST + PATH_DATA, PATH, 3);
    image.bytes(PATH, &[0; 12]);
    image.u32(PATH, u32::MAX);
    image.u32(PATH + 4, 9);
    image.u32(PATH + 8, 9);
    image.array(FIRST + TARGET_STATE_DATA, TARGETS, 2);
    image.bytes(TARGETS, &[0; 8]);
    image.u32(TARGETS, 0x8000_0000);
    image.u32(TARGETS + 4, 7);
    image.bytes(FIRST + ACTIVE, &[0xff]);
    image.u32(FIRST + PARADROP_TARGET, u32::MAX);
    image
}

fn capture(image: &MemoryImage, rows: &mut Vec<Row>, geometry: &mut Vec<u32>) -> Walk {
    walk(
        image,
        BASE,
        rows,
        geometry,
        &mut [corders_group::Root::default(); 8],
        &mut vec![Visit::default(); 16],
    )
}

#[test]
fn native_geometry_counts_grow_only_after_readable_batches() {
    for offset in [PATH_DATA, TARGET_STATE_DATA] {
        let mut image = fixture();
        image.array(FIRST + offset, 0x9000_0000, 32_768);
        let mut geometry = Vec::new();
        let result = capture(&image, &mut Vec::new(), &mut geometry);
        assert_eq!(
            result.outcome,
            Outcome::Unavailable {
                reason: "army order geometry unreadable"
            }
        );
        assert_eq!(geometry.len(), if offset == PATH_DATA { 0 } else { 3 });
        assert!(geometry.capacity() <= 4);
        assert!(result.required_geometry <= geometry.len());
    }
}

#[test]
fn assigned_member_count_does_not_allocate_before_authentication() {
    let mut image = fixture();
    image.u64(BASE + registry::INVALID_REFERENCE_RVA, u64::MAX);
    let mut groups = [corders_group::Root::default(); 8];
    let mut visits = Vec::new();
    let source = walk(
        &image,
        BASE,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut groups,
        &mut visits,
    );
    image.array(FIRST + ASSIGNED_MEMBER_DATA, 0x9000_0000, 32_768);
    let mut assigned = Vec::new();
    let result = walk_plans(
        &image,
        BASE,
        &groups,
        &visits,
        source.outcome,
        &mut Vec::new(),
        &mut assigned,
        &mut Vec::new(),
        &mut Vec::new(),
    );
    assert_eq!(
        result.outcome,
        Outcome::Unavailable {
            reason: "plan member null or unreadable"
        }
    );
    assert_eq!(assigned.capacity(), 0);
    assert_eq!(result.required_assigned, 0);
}

#[test]
fn named_arrows_record_their_name_strings_and_lines_record_none() {
    let mut image = fixture();
    image.u64(BASE + registry::INVALID_REFERENCE_RVA, u64::MAX);
    // SECOND becomes a move order: inline prefix and name, a heap unique text.
    image.u32(SECOND + TYPE, TYPE_MOVE as u32);
    for offset in NAME_STRINGS {
        image.u64(SECOND + offset + 0x18, 15);
    }
    image.bytes(SECOND + NAME_STRINGS[0], b"o_operation\0");
    image.u64(SECOND + NAME_STRINGS[0] + 0x10, 11);
    image.bytes(SECOND + NAME_STRINGS[1], b"o_x\0");
    image.u64(SECOND + NAME_STRINGS[1] + 0x10, 3);
    let unique = b"South Sudetenland\0";
    image.bytes(0x70000, unique);
    image.u64(SECOND + NAME_STRINGS[4], 0x70000);
    image.u64(SECOND + NAME_STRINGS[4] + 0x10, 17);
    image.u64(SECOND + NAME_STRINGS[4] + 0x18, 31);
    let mut groups = [corders_group::Root::default(); 8];
    let mut visits = vec![Visit::default(); 16];
    let source = walk(
        &image,
        BASE,
        &mut vec![Row::default(); 8],
        &mut vec![0; 16],
        &mut groups,
        &mut visits,
    );
    let (mut plans, mut names) = (Vec::new(), Vec::new());
    let result = walk_plans(
        &image,
        BASE,
        &groups[..1],
        &visits,
        source.outcome,
        &mut plans,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut names,
    );
    assert_eq!(result.outcome, Outcome::Rows { len: 3 });
    let text = |row: &PlanRow, index: usize| {
        let (start, len) = row.names[index];
        names[start..start + len].to_vec()
    };
    let named = plans.iter().find(|row| row.instance == u32::MAX).unwrap();
    assert_eq!(text(named, 0), b"o_operation");
    assert_eq!(text(named, 1), b"o_x");
    assert_eq!(text(named, 2), b"");
    assert_eq!(text(named, 4), b"South Sudetenland");
    assert_eq!(result.names, 11 + 3 + 17);
    let line = plans.iter().find(|row| row.instance == 0).unwrap();
    assert_eq!(line.names, [(0, 0); 6]);
    // An unreadable name is a failure, never an empty string.
    image.u64(SECOND + NAME_STRINGS[4], 0x71000);
    let failed = walk_plans(
        &image,
        BASE,
        &groups[..1],
        &visits,
        source.outcome,
        &mut plans,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut Vec::new(),
    );
    assert!(matches!(failed.outcome, Outcome::Unavailable { .. }));
}

#[test]
fn plans_preserve_raw_bytes_scoped_ids_members_and_grow_without_rereading() {
    let mut image = fixture();
    image.u64(BASE + registry::INVALID_REFERENCE_RVA, u64::MAX);
    image.u64(FIRST + FRONT_BINDING, 17_u64 << 32 | 66);
    image.u32(FIRST + FRONT_SECTION, 9);
    image.u32(FIRST + SEGMENT_START, u32::MAX);
    image.u32(FIRST + SEGMENT_END, 7);
    image.bytes(FIRST + MOVE_SPECIAL, &[255, 128]);
    image.bytes(FIRST + FALLBACK, &[3, 4]);
    image.bytes(FIRST + HAS_CHILD, &[2]);
    image.u64(FIRST + OWNER_OVERRIDE, GROUP as u64);
    const UNIT: usize = 0x60000;
    image.bytes(UNIT, &[0; 0x100]);
    image.u64(UNIT, (BASE + crate::carmy::VTABLE_RVA) as u64);
    image.u64(
        UNIT + crate::carmy::SECONDARY_BASE,
        (BASE + crate::carmy::SECONDARY_VTABLE_RVA) as u64,
    );
    image.u64(UNIT + crate::cunit::HANDLE, 0xffff_fffe_0000_0033);
    image.array(FIRST + ASSIGNED_MEMBER_DATA, 0x50000, 2);
    image.u64(0x50000, (UNIT + crate::cunit::SCHEDULED_MEMBER) as u64);
    image.u64(0x50008, (UNIT + crate::cunit::SCHEDULED_MEMBER) as u64);
    let mut groups = [corders_group::Root::default(); 8];
    let mut visits = vec![Visit::default(); 16];
    let source = walk(
        &image,
        BASE,
        &mut vec![Row::default(); 8],
        &mut vec![0; 16],
        &mut groups,
        &mut visits,
    );
    let mut plans = vec![PlanRow::default(); 8];
    let mut assigned = vec![0; 4];
    image.1.borrow_mut().clear();
    let result = walk_plans(
        &image,
        BASE,
        &groups[..1],
        &visits,
        source.outcome,
        &mut plans,
        &mut assigned,
        &mut Vec::new(),
        &mut Vec::new(),
    );
    assert_eq!(result.outcome, Outcome::Rows { len: 3 });
    assert_eq!(
        (
            plans[0].front_valid,
            plans[0].section,
            plans[0].segment_start,
            plans[0].segment_end
        ),
        (1, 9, -1, 7)
    );
    assert_eq!(
        (
            plans[0].blitz,
            plans[0].withdraw,
            plans[0].fallback,
            plans[0].virtual_order,
            plans[0].has_child
        ),
        (255, 128, 3, 4, 2)
    );
    assert_eq!(plans[0].owner_override, HANDLE);
    assert_eq!(
        &assigned[plans[0].assigned_start..plans[0].assigned_start + 2],
        &[0xffff_fffe_0000_0033; 2]
    );
    // Each order's fixed spans are one gathered record, read once; no
    // field inside them is read again on its own.
    for pointer in [FIRST, SECOND, CHILD] {
        for (offset, len) in PLAN_SPANS {
            assert_eq!(
                image
                    .1
                    .borrow()
                    .iter()
                    .filter(|p| **p == (pointer + offset, len))
                    .count(),
                1
            );
        }
        assert!(image.1.borrow().iter().all(|&(address, len)| {
            PLAN_SPANS.contains(&(address.wrapping_sub(pointer), len))
                || address + len <= pointer
                || address >= pointer + OWNER_OVERRIDE + 8
        }));
    }
    let reads = image.1.borrow().clone();
    image.1.borrow_mut().clear();
    let mut grown_plans = Vec::new();
    let mut grown_assigned = Vec::new();
    let grown = walk_plans(
        &image,
        BASE,
        &groups[..1],
        &visits,
        source.outcome,
        &mut grown_plans,
        &mut grown_assigned,
        &mut Vec::new(),
        &mut Vec::new(),
    );
    assert_eq!(grown.outcome, Outcome::Rows { len: 3 });
    assert_eq!(grown.required_assigned, 2);
    assert_eq!(grown_plans, plans[..3]);
    assert_eq!(grown_assigned, assigned[..2]);
    assert_eq!(*image.1.borrow(), reads);
    image.u64(UNIT, 0);
    image.1.borrow_mut().clear();
    assert!(matches!(
        walk_plans(
            &image,
            BASE,
            &groups[..1],
            &visits,
            source.outcome,
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
        )
        .outcome,
        Outcome::Unavailable { .. }
    ));
    assert_eq!(image.1.borrow().iter().filter(|p| p.0 == UNIT).count(), 1);
    assert!(matches!(
        walk_plans(
            &image,
            BASE,
            &groups[..1],
            &visits,
            Outcome::Truncated {
                len: 2,
                unread_rows: 1
            },
            &mut plans,
            &mut assigned,
            &mut Vec::new(),
            &mut Vec::new(),
        )
        .outcome,
        Outcome::Unavailable { .. }
    ));
}

/// Both plan walks hint ahead of themselves. Every direct read of a census
/// order or group lands inside a span hinted before it, and the hints
/// change neither the rows nor the reads: the same walk over memory that
/// drops its hints makes the identical reads.
#[test]
fn plan_walks_hint_every_direct_read_ahead_and_change_no_read_or_row() {
    struct Hinted<'a> {
        image: &'a MemoryImage,
        keep: bool,
        events: std::cell::RefCell<Vec<(bool, usize, usize)>>,
    }
    impl Memory for Hinted<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.events
                .borrow_mut()
                .push((false, address, output.len()));
            self.image.copy(address, output)
        }
        fn prefetch(&self, address: usize, len: usize) {
            if self.keep {
                self.events.borrow_mut().push((true, address, len));
            }
        }
    }
    let mut image = fixture();
    image.u64(BASE + registry::INVALID_REFERENCE_RVA, u64::MAX);
    // More successors than the hint distance, so the walk also hints
    // while it reads; each is a named move with empty inline strings.
    const MORE: usize = 0x80000;
    const LIST: usize = 0x45000;
    let extra = 2 * PLAN_BATCH;
    image.bytes(LIST, &vec![0; (extra + 1) * 8]);
    image.array(FIRST + SUCCESSOR_DATA, LIST, extra as u32 + 1);
    image.u64(LIST, CHILD as u64);
    let mut orders = vec![FIRST, SECOND, CHILD];
    for index in 0..extra {
        let order = MORE + index * 0x1000;
        image.order(order, 100 + index as u32, TYPE_MOVE);
        for offset in NAME_STRINGS {
            image.u64(order + offset + crate::msvc_string::CAPACITY, 15);
        }
        image.u64(LIST + (index + 1) * 8, order as u64);
        orders.push(order);
    }
    let mut groups = [corders_group::Root::default(); 8];
    let mut visits = vec![Visit::default(); 16];
    let census = walk(
        &image,
        BASE,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut groups,
        &mut visits,
    );
    assert_eq!(census.outcome, Outcome::Rows { len: 3 + extra });
    let covered = |events: &[(bool, usize, usize)], objects: &[(usize, usize)]| {
        let mut hinted_while_reading = false;
        for (at, &(hint, address, len)) in events.iter().enumerate() {
            if hint {
                hinted_while_reading |= events[..at].iter().any(|event| !event.0);
                continue;
            }
            if !objects
                .iter()
                .any(|&(object, size)| (object..object + size).contains(&address))
            {
                continue;
            }
            assert!(
                events[..at].iter().any(|&(hint, start, span)| hint
                    && start <= address
                    && address + len <= start + span),
                "read {address:#x}+{len} was not hinted first"
            );
        }
        hinted_while_reading
    };
    let run_orders = |keep| {
        let memory = Hinted {
            image: &image,
            keep,
            events: Default::default(),
        };
        let (mut rows, mut assigned, mut links, mut names) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let walk = walk_plans(
            &memory,
            BASE,
            &groups[..1],
            &visits,
            census.outcome,
            &mut rows,
            &mut assigned,
            &mut links,
            &mut names,
        );
        assert_eq!(walk.outcome, Outcome::Rows { len: 3 + extra });
        (rows, links, memory.events.into_inner())
    };
    let (rows, links, events) = run_orders(true);
    let (plain_rows, plain_links, plain_events) = run_orders(false);
    assert_eq!((rows, links), (plain_rows, plain_links));
    let reads: Vec<_> = events.iter().filter(|event| !event.0).copied().collect();
    assert_eq!(reads, plain_events);
    let mut objects: Vec<_> = orders.iter().map(|&order| (order, 0x400)).collect();
    objects.push((GROUP, corders_group::COUNTRY + 4));
    assert!(covered(&events, &objects));

    let run_groups = |keep| {
        let memory = Hinted {
            image: &image,
            keep,
            events: Default::default(),
        };
        let mut rows = [corders_group::PlanRow::default(); 1];
        let mut root_ids = Vec::new();
        let walk = corders_group::walk_plans(
            &memory,
            BASE,
            &groups[..1],
            census.root_outcome,
            &visits,
            census.outcome,
            &mut rows,
            &mut root_ids,
            &mut Vec::new(),
        );
        assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
        (rows, root_ids, memory.events.into_inner())
    };
    let (rows, root_orders, events) = run_groups(true);
    let (plain_rows, plain_orders, plain_events) = run_groups(false);
    assert_eq!((rows, root_orders), (plain_rows, plain_orders));
    let reads: Vec<_> = events.iter().filter(|event| !event.0).copied().collect();
    assert_eq!(reads, plain_events);
    covered(&events, &[(GROUP, corders_group::SIZE)]);
}

/// The link vectors and heap names an order's record points at are hinted
/// a batch ahead: before the first of them is read for the previous batch.
/// A successor vector the census listed is not read, so it is not hinted.
#[test]
fn plan_dependents_are_hinted_one_batch_before_their_reads() {
    struct Hinted<'a> {
        image: &'a MemoryImage,
        events: std::cell::RefCell<Vec<(bool, usize, usize)>>,
    }
    impl Memory for Hinted<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.events
                .borrow_mut()
                .push((false, address, output.len()));
            self.image.copy(address, output)
        }
        fn prefetch(&self, address: usize, len: usize) {
            self.events.borrow_mut().push((true, address, len));
        }
    }
    let mut image = fixture();
    image.u64(BASE + registry::INVALID_REFERENCE_RVA, u64::MAX);
    const MORE: usize = 0x80000;
    const LIST: usize = 0x45000;
    const TEXTS: usize = 0x200000;
    const PREDECESSORS: usize = 0x300000;
    let extra = 3 * PLAN_BATCH;
    image.bytes(LIST, &vec![0; (extra + 1) * 8]);
    image.array(FIRST + SUCCESSOR_DATA, LIST, extra as u32 + 1);
    image.u64(LIST, CHILD as u64);
    let mut dependents = Vec::new();
    for index in 0..extra {
        let order = MORE + index * 0x1000;
        image.order(order, 100 + index as u32, TYPE_MOVE);
        for offset in NAME_STRINGS {
            image.u64(order + offset + crate::msvc_string::CAPACITY, 15);
        }
        let text = TEXTS + index * 0x100;
        let unique = order + NAME_STRINGS[4];
        image.bytes(text, b"Operation Barbarossa\0");
        image.u64(unique, text as u64);
        image.u64(unique + crate::msvc_string::LENGTH, 20);
        image.u64(unique + crate::msvc_string::CAPACITY, 31);
        let predecessors = PREDECESSORS + index * 0x100;
        image.array(order + PREDECESSOR_DATA, predecessors, 1);
        image.u64(predecessors, FIRST as u64);
        image.u64(LIST + (index + 1) * 8, order as u64);
        dependents.push((order, [text, predecessors]));
    }
    let mut groups = [corders_group::Root::default(); 8];
    let mut visits = vec![Visit::default(); 64];
    let mut lists = SuccessorLists::default();
    let census = walk_listing_successors(
        &image,
        BASE,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut groups,
        &mut visits,
        &mut lists,
    );
    assert_eq!(census.outcome, Outcome::Rows { len: 3 + extra });
    let memory = Hinted {
        image: &image,
        events: Default::default(),
    };
    let walk = walk_plans_ordered(
        &memory,
        BASE,
        &groups[..1],
        &visits,
        &lists,
        census.outcome,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut Vec::new(),
        &mut Vec::new(),
        &mut Vec::new(),
    );
    assert_eq!(walk.outcome, Outcome::Rows { len: 3 + extra });
    let events = memory.events.into_inner();
    let first = |hint: bool, address: usize| {
        events
            .iter()
            .position(|&(kind, start, len)| {
                kind == hint && start <= address && address < start + len.max(1)
            })
            .unwrap_or_else(|| panic!("no {hint} event at {address:#x}"))
    };
    // Census order decides the batches.
    let batch_of = |pointer: usize| {
        visits
            .iter()
            .filter(|visit| visit.pointer != 0)
            .position(|visit| visit.pointer == pointer)
            .unwrap()
            / PLAN_BATCH
    };
    let batches = dependents
        .iter()
        .map(|&(order, _)| batch_of(order))
        .max()
        .unwrap()
        + 1;
    let mut first_reads = vec![usize::MAX; batches];
    for &(order, addresses) in &dependents {
        for address in addresses {
            let batch = batch_of(order);
            first_reads[batch] = first_reads[batch].min(first(false, address));
        }
    }
    let mut checked = 0;
    for &(order, addresses) in &dependents {
        let batch = batch_of(order);
        let Some(&previous) = batch.checked_sub(1).and_then(|b| first_reads.get(b)) else {
            continue;
        };
        if previous == usize::MAX {
            continue;
        }
        for address in addresses {
            assert!(
                first(true, address) < previous,
                "dependent {address:#x} of batch {batch} hinted after batch {} read",
                batch - 1
            );
            checked += 1;
        }
    }
    assert!(checked >= 2 * PLAN_BATCH);
    // FIRST's successors came from the census list: neither read nor hinted.
    assert!(!events.iter().any(|&(_, address, _)| address == LIST));
}

#[test]
fn group_plans_keep_the_native_root_order_as_census_ids() {
    let mut image = fixture();
    let mut roots = [corders_group::Root::default(); 8];
    let mut visits = vec![Visit::default(); 16];
    let census = walk(
        &image,
        BASE,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut roots,
        &mut visits,
    );
    let plans = |image: &MemoryImage, orders: &mut Vec<u32>| {
        let mut rows = [corders_group::PlanRow::default(); 1];
        let walk = corders_group::walk_plans(
            image,
            BASE,
            &roots[..1],
            census.root_outcome,
            &visits,
            census.outcome,
            &mut rows,
            orders,
            &mut Vec::new(),
        );
        (walk, rows[0])
    };
    let mut orders = Vec::new();
    let (walk, row) = plans(&image, &mut orders);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    // Array order, not instance order: the second root's id is u32::MAX.
    assert_eq!(
        &orders[row.primary_start..row.primary_start + row.primary_len],
        &[0, u32::MAX]
    );
    assert_eq!(row.fallback_len, 0);
    // A root outside the group's own census is refused, not dropped.
    image.u64(ROOTS + 8, 0x7777_0000);
    let (walk, _) = plans(&image, &mut Vec::new());
    assert!(matches!(walk.outcome, Outcome::Unavailable { .. }));
}

#[test]
fn plan_graphs_preserve_native_order_and_reject_references_outside_the_census() {
    let mut image = fixture();
    image.u64(BASE + registry::INVALID_REFERENCE_RVA, u64::MAX);
    image.array(CHILD + PREDECESSOR_DATA, 0x44000, 3);
    image.bytes(0x44000, &[0; 24]);
    image.u64(0x44000, SECOND as u64);
    image.u64(0x44008, FIRST as u64);
    image.u64(0x44010, FIRST as u64);
    image.array(FIRST + SUCCESSOR_DATA, CHILDREN_A, 2);
    image.bytes(CHILDREN_A, &[0; 16]);
    image.u64(CHILDREN_A, CHILD as u64);
    image.u64(CHILDREN_A + 8, CHILD as u64);
    let mut groups = [corders_group::Root::default(); 8];
    let mut visits = Vec::new();
    let source = walk(
        &image,
        BASE,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut groups,
        &mut visits,
    );
    assert_eq!(source.outcome, Outcome::Rows { len: 3 });
    let mut plans = Vec::new();
    let mut links = Vec::new();
    let result = walk_plans(
        &image,
        BASE,
        &groups[..1],
        &visits,
        source.outcome,
        &mut plans,
        &mut Vec::new(),
        &mut links,
        &mut Vec::new(),
    );
    assert_eq!(result.outcome, Outcome::Rows { len: 3 });
    let child = plans
        .iter()
        .find(|row| row.instance == 0x8000_0000)
        .unwrap();
    assert_eq!(
        &links[child.predecessors_start..child.predecessors_start + child.predecessors_len],
        &[u32::MAX, 0, 0]
    );
    let first = plans.iter().find(|row| row.instance == 0).unwrap();
    assert_eq!(
        &links[first.successors_start..first.successors_start + first.successors_len],
        &[0x8000_0000; 2]
    );
    let mut group_plans = [corders_group::PlanRow::default(); 1];
    let mut roots = Vec::new();
    let result = corders_group::walk_plans(
        &image,
        BASE,
        &groups[..1],
        source.root_outcome,
        &visits,
        source.outcome,
        &mut group_plans,
        &mut roots,
        &mut Vec::new(),
    );
    assert_eq!(result.outcome, Outcome::Rows { len: 1 });
    assert_eq!(roots, [0, u32::MAX]);
    assert_eq!(
        (group_plans[0].primary_len, group_plans[0].fallback_len),
        (2, 0)
    );
    image.u64(0x44000, 0x9999);
    assert_eq!(
        walk_plans(
            &image,
            BASE,
            &groups[..1],
            &visits,
            source.outcome,
            &mut plans,
            &mut Vec::new(),
            &mut links,
            &mut Vec::new()
        )
        .outcome,
        Outcome::Unavailable {
            reason: "plan graph reference absent from owning census"
        }
    );
}

/// The census of one pass, listing successors or not, and the plan walks
/// over it: the order rows with their link and name arenas, the group
/// rows with their root ids, and every read the plan walks made.
struct Planned {
    orders: Outcome,
    rows: Vec<PlanRow>,
    links: Vec<u32>,
    names: Vec<u8>,
    groups: Outcome,
    group_rows: Vec<corders_group::PlanRow>,
    roots: Vec<u32>,
    reads: Vec<(usize, usize)>,
}

/// Runs the census on `census_image` and the plan walks on `image`, which
/// is the same memory unless a test changes it between the two.
fn planned(census_image: &MemoryImage, image: &MemoryImage, listing: bool) -> Planned {
    planned_with(census_image, image, listing, true)
}

/// [`planned`], keeping the plan records a listing census gathered only
/// with `records`: without them the walks read every order themselves.
fn planned_with(
    census_image: &MemoryImage,
    image: &MemoryImage,
    listing: bool,
    records: bool,
) -> Planned {
    let mut groups = [corders_group::Root::default(); 8];
    let mut visits = vec![Visit::default(); 16];
    let mut lists = SuccessorLists::default();
    let census = if listing {
        let census = walk_listing_successors(
            census_image,
            BASE,
            &mut Vec::new(),
            &mut Vec::new(),
            &mut groups,
            &mut visits,
            &mut lists,
        );
        if !records {
            lists.plans.discard();
        }
        census
    } else {
        walk(
            census_image,
            BASE,
            &mut Vec::new(),
            &mut Vec::new(),
            &mut groups,
            &mut visits,
        )
    };
    image.1.borrow_mut().clear();
    let (mut rows, mut links, mut names) = (Vec::new(), Vec::new(), Vec::new());
    let orders = walk_plans_ordered(
        image,
        BASE,
        &groups[..1],
        &visits,
        &lists,
        census.outcome,
        &mut rows,
        &mut Vec::new(),
        &mut links,
        &mut names,
        &mut Vec::new(),
    );
    let mut group_rows = vec![corders_group::PlanRow::default(); 1];
    let mut roots = Vec::new();
    let group = corders_group::walk_plans(
        image,
        BASE,
        &groups[..1],
        census.root_outcome,
        &visits,
        census.outcome,
        &mut group_rows,
        &mut roots,
        &mut Vec::new(),
    );
    rows.truncate(orders.outcome.len());
    links.truncate(orders.links);
    names.truncate(orders.names);
    roots.truncate(group.orders);
    Planned {
        orders: orders.outcome,
        rows,
        links,
        names,
        groups: group.outcome,
        group_rows,
        roots,
        reads: image.1.borrow().clone(),
    }
}

impl Planned {
    fn assert_same_rows(&self, other: &Self, case: &str) {
        assert_eq!(
            (self.orders, &self.rows, &self.links, &self.names),
            (other.orders, &other.rows, &other.links, &other.names),
            "orders: {case}"
        );
        assert_eq!(
            (self.groups, &self.group_rows, &self.roots),
            (other.groups, &other.group_rows, &other.roots),
            "groups: {case}"
        );
    }
}

fn copy_image(image: &MemoryImage) -> MemoryImage {
    MemoryImage(image.0.clone(), Default::default())
}

/// Successor lists the census resolved in the same pass give the rows and
/// arenas of reading each vector again, and replace exactly those reads.
/// A vector the census did not walk (reached only through a virtual edge,
/// or a fallback root's) is read as before, and so is one whose header no
/// longer matches the census's; a failed census lists nothing.
#[test]
fn census_successor_lists_give_the_rows_of_rereading_them() {
    const VIRTUAL: usize = 0x33000;
    const FALLBACK: usize = 0x35000;
    const FALLBACK_ROOTS: usize = 0x46000;
    const VIRTUAL_LIST: usize = 0x46100;
    const VIRTUAL_SUCCESSORS: usize = 0x46200;
    const FALLBACK_SUCCESSORS: usize = 0x46300;
    let mut image = fixture();
    image.u64(BASE + registry::INVALID_REFERENCE_RVA, u64::MAX);
    // FIRST succeeds into CHILD and SECOND, SECOND into CHILD: the listed
    // vectors.
    image.array(FIRST + SUCCESSOR_DATA, CHILDREN_A, 2);
    image.bytes(CHILDREN_A, &[0; 16]);
    image.u64(CHILDREN_A, CHILD as u64);
    image.u64(CHILDREN_A + 8, SECOND as u64);
    // VIRTUAL is reached only through FIRST's virtual edge, so the census
    // never walks its ordinary successors; FALLBACK is a fallback root,
    // entered without any graph walk.
    image.order(VIRTUAL, 17, TYPE_LINE);
    image.array(FIRST + VIRTUAL_SUCCESSOR_DATA, VIRTUAL_LIST, 1);
    image.u64(VIRTUAL_LIST, VIRTUAL as u64);
    image.array(VIRTUAL + SUCCESSOR_DATA, VIRTUAL_SUCCESSORS, 2);
    image.bytes(VIRTUAL_SUCCESSORS, &[0; 16]);
    image.u64(VIRTUAL_SUCCESSORS, CHILD as u64);
    image.u64(VIRTUAL_SUCCESSORS + 8, FIRST as u64);
    image.order(FALLBACK, 19, TYPE_MOVE);
    for offset in NAME_STRINGS {
        image.u64(FALLBACK + offset + crate::msvc_string::CAPACITY, 15);
    }
    image.array(
        GROUP + corders_group::SECONDARY_ORDER_DATA,
        FALLBACK_ROOTS,
        1,
    );
    image.u64(FALLBACK_ROOTS, FALLBACK as u64);
    image.array(FALLBACK + SUCCESSOR_DATA, FALLBACK_SUCCESSORS, 1);
    image.u64(FALLBACK_SUCCESSORS, SECOND as u64);
    // Predecessors are never listed.
    image.array(CHILD + PREDECESSOR_DATA, 0x44000, 2);
    image.bytes(0x44000, &[0; 16]);
    image.u64(0x44000, SECOND as u64);
    image.u64(0x44008, FIRST as u64);

    let read = planned(&image, &image, false);
    let listed = planned(&image, &image, true);
    assert_eq!(read.orders, Outcome::Rows { len: 5 });
    assert_eq!(read.groups, Outcome::Rows { len: 1 });
    listed.assert_same_rows(&read, "same memory");
    let first = listed.rows.iter().find(|row| row.instance == 0).unwrap();
    assert_eq!(
        &listed.links[first.successors_start..][..first.successors_len],
        &[0x8000_0000, u32::MAX]
    );
    // The listed vectors are not read again; every other vector is, once.
    let reads_of = |planned: &Planned, list: usize| {
        planned
            .reads
            .iter()
            .filter(|&&(address, _)| address == list)
            .count()
    };
    for list in [CHILDREN_A, CHILDREN_B] {
        assert_eq!((reads_of(&read, list), reads_of(&listed, list)), (1, 0));
    }
    for list in [VIRTUAL_SUCCESSORS, FALLBACK_SUCCESSORS, 0x44000] {
        assert_eq!((reads_of(&read, list), reads_of(&listed, list)), (1, 1));
    }

    // A header the census did not list it from is read as before: a moved
    // vector, a shorter count, and an invalid header. The plans read
    // `changed` themselves here, without the census's plan records,
    // which hold the census's own headers of the same pass.
    const MOVED: usize = 0x46400;
    for change in 0..3 {
        let mut changed = copy_image(&image);
        match change {
            0 => {
                changed.array(FIRST + SUCCESSOR_DATA, MOVED, 2);
                changed.bytes(MOVED, &[0; 16]);
                changed.u64(MOVED, SECOND as u64);
                changed.u64(MOVED + 8, CHILD as u64);
            }
            1 => changed.array(FIRST + SUCCESSOR_DATA, CHILDREN_A, 1),
            _ => changed.u32(FIRST + SUCCESSOR_DATA + 8, 0),
        }
        let reused = planned_with(&image, &changed, true, false);
        let reread = planned(&image, &changed, false);
        reused.assert_same_rows(&reread, &format!("header change {change}"));
        if change < 2 {
            let fresh = planned(&changed, &changed, false);
            reused.assert_same_rows(&fresh, &format!("header change {change}, fresh census"));
        } else {
            assert_eq!(
                reused.orders,
                Outcome::Unavailable {
                    reason: "plan graph header invalid"
                }
            );
        }
    }

    // A census that fails lists nothing, and plans refuse it as before.
    let mut cyclic = copy_image(&image);
    cyclic.array(CHILD + SUCCESSOR_DATA, 0x46500, 1);
    cyclic.u64(0x46500, FIRST as u64);
    let mut lists = SuccessorLists::default();
    let census = walk_listing_successors(
        &cyclic,
        BASE,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut [corders_group::Root::default(); 8],
        &mut vec![Visit::default(); 16],
        &mut lists,
    );
    assert!(matches!(census.outcome, Outcome::Unavailable { .. }));
    assert!(lists.words.is_empty() && lists.pending.is_empty());
    planned(&cyclic, &cyclic, true)
        .assert_same_rows(&planned(&cyclic, &cyclic, false), "failed census");

    // A listing census is the plain census: rows, geometry and visits.
    let mut plain_rows = vec![Row::default(); 8];
    let mut plain_geometry = vec![0; 16];
    let mut plain_visits = vec![Visit::default(); 16];
    let plain = walk(
        &image,
        BASE,
        &mut plain_rows,
        &mut plain_geometry,
        &mut [corders_group::Root::default(); 8],
        &mut plain_visits,
    );
    let mut rows = vec![Row::default(); 8];
    let mut geometry = vec![0; 16];
    let mut visits = vec![Visit::default(); 16];
    let listing = walk_listing_successors(
        &image,
        BASE,
        &mut rows,
        &mut geometry,
        &mut [corders_group::Root::default(); 8],
        &mut visits,
        &mut SuccessorLists::default(),
    );
    assert_eq!(listing, plain);
    assert_eq!((rows, geometry), (plain_rows, plain_geometry));
    let key = |visits: &[Visit]| {
        visits
            .iter()
            .map(|visit| (visit.pointer, visit.owner, visit.instance))
            .collect::<Vec<_>>()
    };
    assert_eq!(key(&visits), key(&plain_visits));
}

/// A vector whose entries were not all pushed is never filed, and nothing
/// else is listed for the rest of that census.
#[test]
fn successor_lists_file_only_complete_vectors() {
    let mut lists = SuccessorLists::default();
    lists.begin();
    lists.push(7);
    lists.push(8);
    let tag = lists.finish(0x1000, 2);
    assert_eq!(tag, 1);
    let visit = Visit {
        successors: tag,
        ..Visit::default()
    };
    let array = Array {
        data: 0x1000,
        count: 2,
    };
    assert_eq!(lists.ids(&visit, array), Some(&[7, 8][..]));
    assert_eq!(lists.ids(&visit, Array { count: 1, ..array }), None);
    assert_eq!(
        lists.ids(
            &visit,
            Array {
                data: 0x2000,
                ..array
            }
        ),
        None
    );
    assert_eq!(lists.ids(&Visit::default(), array), None);
    lists.push(9);
    assert_eq!(lists.finish(0x3000, 2), 0);
    lists.push(10);
    assert_eq!(lists.finish(0x4000, 1), 0);
    assert_eq!(lists.ids(&visit, array), Some(&[7, 8][..]));
}

/// The group plans take each root order's id from the census entry that
/// authenticated it, and the ids equal the ones its `+ID` field holds. A
/// root outside the census still fails before any id is used.
#[test]
fn group_root_ids_are_the_census_ids_of_their_orders() {
    let image = fixture();
    let planned = planned(&image, &image, true);
    assert_eq!(planned.groups, Outcome::Rows { len: 1 });
    let ids: Vec<u32> = [FIRST, SECOND]
        .iter()
        .map(|&order| plan_field(&image, order, ID).unwrap())
        .collect();
    assert_eq!(planned.roots, ids);
    let mut visits = vec![Visit::default(); 16];
    let mut groups = [corders_group::Root::default(); 8];
    walk(
        &image,
        BASE,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut groups,
        &mut visits,
    );
    assert_eq!(census_instance(&visits, SECOND, 0), Ok(u32::MAX));
    for (pointer, owner) in [(0x7777_0000, 0), (SECOND, 1), (0, 0)] {
        assert_eq!(
            census_instance(&visits, pointer, owner),
            Err("root order absent from its group's census")
        );
    }
}

#[test]
fn native_composite_ids_full_widths_and_ordered_geometry_survive_shared_successors() {
    let image = fixture();
    let mut rows = vec![Row::default(); 8];
    let mut geometry = vec![0; 16];
    let report = capture(&image, &mut rows, &mut geometry);
    assert_eq!(report.outcome, Outcome::Rows { len: 3 });
    assert_eq!(
        (
            report.geometry_len,
            report.required_geometry,
            report.required_groups
        ),
        (5, 5, 1)
    );
    assert_eq!(
        (rows[0].owner_group, rows[0].instance, rows[0].country),
        (HANDLE, 0, 0xffff_fffe)
    );
    assert_eq!(rows[0].key(), (u128::from(HANDLE) << 32));
    assert_eq!((rows[0].active, rows[0].paradrop_target), (255, u32::MAX));
    assert_eq!(
        &geometry[rows[0].path_start..rows[0].path_start + rows[0].path_len],
        &[u32::MAX, 9, 9]
    );
    assert_eq!(
        &geometry[rows[0].target_start..rows[0].target_start + rows[0].target_len],
        &[0x8000_0000, 7]
    );
    assert_eq!(
        (rows[1].instance, rows[1].order_type),
        (0x8000_0000, i32::MIN)
    );
    assert_eq!(rows[2].instance, u32::MAX);
    assert_eq!(rows[1].path_len + rows[1].target_len, 0);
}

#[test]
fn row_and_geometry_growth_preserves_all_payloads_and_original_reads() {
    let image = fixture();
    let mut expected_rows = vec![Row::default(); 8];
    let mut expected_geometry = vec![0; 16];
    let expected = capture(&image, &mut expected_rows, &mut expected_geometry);
    let reads = image.1.borrow().clone();
    for (rows, words) in [(0, 0), (1, 16), (8, 4)] {
        image.1.borrow_mut().clear();
        let mut rows = vec![Row::default(); rows];
        let mut geometry = vec![0; words];
        let report = capture(&image, &mut rows, &mut geometry);
        assert_eq!(report, expected);
        assert_eq!(&rows[..3], &expected_rows[..3]);
        assert_eq!(&geometry[..5], &expected_geometry[..5]);
        assert_eq!(*image.1.borrow(), reads);
    }
}

#[test]
fn virtual_graphs_capture_missing_nodes_once_and_keep_edge_traversals_separate() {
    let mut image = fixture();
    const VIRTUAL: usize = 0x33000;
    const VIRTUAL_CHILD: usize = 0x34000;
    image.order(VIRTUAL, 17, TYPE_LINE);
    image.order(VIRTUAL_CHILD, 18, TYPE_LINE);
    image.array(FIRST + VIRTUAL_SUCCESSOR_DATA, 0x43000, 1);
    image.u64(0x43000, VIRTUAL as u64);
    image.array(SECOND + VIRTUAL_SUCCESSOR_DATA, 0x43100, 1);
    image.u64(0x43100, VIRTUAL as u64);
    image.array(VIRTUAL + VIRTUAL_SUCCESSOR_DATA, 0x43200, 1);
    image.u64(0x43200, CHILD as u64);
    image.array(CHILD + VIRTUAL_SUCCESSOR_DATA, 0x43300, 1);
    image.u64(0x43300, VIRTUAL_CHILD as u64);
    // This mixed-edge loop is not a cycle in either native gather graph.
    // Gathering virtual successors must not follow VIRTUAL's normal edge.
    image.array(VIRTUAL + SUCCESSOR_DATA, 0x43400, 1);
    image.u64(0x43400, FIRST as u64);
    image.array(VIRTUAL_CHILD + PATH_DATA, 0x43500, 2);
    image.bytes(0x43500, &[0; 8]);
    image.u32(0x43500, 9);
    image.u32(0x43504, 9);

    let mut rows = vec![Row::default(); 8];
    let mut geometry = vec![0; 16];
    let report = capture(&image, &mut rows, &mut geometry);
    assert_eq!(report.outcome, Outcome::Rows { len: 5 });
    assert_eq!(report.required_geometry, 7);
    assert_eq!(
        rows[..5].iter().map(|row| row.instance).collect::<Vec<_>>(),
        vec![0, 17, 18, 0x8000_0000, u32::MAX]
    );
    assert_eq!(
        &geometry[rows[2].path_start..rows[2].path_start + rows[2].path_len],
        &[9, 9]
    );
    assert_eq!(
        capture(&image, &mut vec![Row::default(); 2], &mut geometry).outcome,
        Outcome::Rows { len: 5 }
    );
    let short_geometry = capture(&image, &mut rows, &mut vec![0; 2]);
    assert_eq!(short_geometry.outcome, Outcome::Rows { len: 5 });
    assert_eq!(short_geometry.required_geometry, 7);

    image.array(VIRTUAL_CHILD + VIRTUAL_SUCCESSOR_DATA, 0x43600, 1);
    image.u64(0x43600, FIRST as u64);
    assert_eq!(
        capture(&image, &mut rows, &mut geometry).outcome,
        Outcome::Unavailable {
            reason: "army order virtual successor cycle"
        }
    );
}

#[test]
fn fallback_rosters_are_direct_rows_without_recursive_successor_gathering() {
    let mut image = fixture();
    image.array(GROUP + corders_group::PRIMARY_ORDER_DATA, 0, 0);
    image.array(GROUP + corders_group::SECONDARY_ORDER_DATA, ROOTS, 1);
    image.array(GROUP + corders_group::TERTIARY_ORDER_DATA, ROOTS + 8, 1);
    // CHILD is referenced by both fallbacks, but the group serializer emits
    // only the fallback objects themselves, not their descendant graphs.
    let mut rows = vec![Row::default(); 8];
    assert_eq!(
        capture(&image, &mut rows, &mut vec![0; 16]).outcome,
        Outcome::Rows { len: 2 }
    );
    assert_eq!((rows[0].instance, rows[1].instance), (0, u32::MAX));
}

#[test]
fn group_scratch_exhaustion_is_explicit_and_traversal_scratch_grows_in_place() {
    let image = fixture();
    let mut rows = vec![Row::default(); 8];
    let mut visits = vec![Visit::default(); 2];
    let report = walk(
        &image,
        BASE,
        &mut rows,
        &mut vec![0; 16],
        &mut [],
        &mut vec![Visit::default(); 16],
    );
    assert_eq!(
        report.outcome,
        Outcome::Unavailable {
            reason: "orders group scratch capacity exhausted"
        }
    );
    assert_eq!(report.required_groups, 1);
    assert_eq!(
        report.root_outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
    let report = walk(
        &image,
        BASE,
        &mut rows,
        &mut vec![0; 16],
        &mut [corders_group::Root::default(); 8],
        &mut visits,
    );
    assert_eq!(report.outcome, Outcome::Rows { len: 3 });
    assert_eq!(visits.len(), 4);
    assert_eq!(report.minimum_visits, 0);
    assert_eq!(report.root_outcome, Outcome::Rows { len: 1 });
    assert_eq!(report.visited, 3);
    let recovered = walk(
        &image,
        BASE,
        &mut rows,
        &mut vec![0; 16],
        &mut [corders_group::Root::default(); 1],
        &mut vec![Visit::default(); 4],
    );
    assert_eq!(recovered.outcome, Outcome::Rows { len: 3 });
    assert_eq!(recovered.minimum_visits, 0);
    assert_eq!(recovered.visited, 3);
}

#[test]
fn traversal_growth_preserves_nested_stacks_cycles_and_native_failure_prefixes() {
    const NODES: usize = 0x50000;
    const EDGES: usize = 0x70000;
    const DEPTH: usize = 10;
    for failure in [
        None,
        Some("cycle"),
        Some("virtual cycle"),
        Some("owner"),
        Some("fault"),
    ] {
        let mut image = fixture();
        image.array(FIRST + SUCCESSOR_DATA, EDGES, 2);
        image.array(FIRST + VIRTUAL_SUCCESSOR_DATA, EDGES, 2);
        image.bytes(EDGES, &[0; 16]);
        image.u64(EDGES, NODES as u64);
        image.u64(EDGES + 8, CHILD as u64);
        for index in 0..DEPTH {
            let pointer = NODES + index * 0x1000;
            let edges = EDGES + (index + 1) * 0x40;
            image.order(pointer, index as u32 + 1, TYPE_LINE);
            image.array(pointer + SUCCESSOR_DATA, edges, 2);
            image.array(pointer + VIRTUAL_SUCCESSOR_DATA, edges, 2);
            image.bytes(edges, &[0; 16]);
            image.u64(
                edges,
                if index + 1 == DEPTH {
                    CHILD
                } else {
                    pointer + 0x1000
                } as u64,
            );
            image.u64(edges + 8, CHILD as u64);
        }
        let tail = NODES + (DEPTH - 1) * 0x1000;
        match failure {
            Some("cycle") | Some("virtual cycle") => {
                let offset = if failure == Some("cycle") {
                    SUCCESSOR_DATA
                } else {
                    VIRTUAL_SUCCESSOR_DATA
                };
                image.array(tail + offset, EDGES + 0x1000, 1);
                image.u64(EDGES + 0x1000, FIRST as u64);
            }
            Some("owner") => image.u64(tail + OWNER, GROUP as u64 + 8),
            Some("fault") => image.0.get_mut(&tail).unwrap().truncate(OWNER),
            _ => {}
        }
        image.1.borrow_mut().clear();
        let mut expected_rows = Vec::new();
        let mut expected_geometry = Vec::new();
        let expected = walk(
            &image,
            BASE,
            &mut expected_rows,
            &mut expected_geometry,
            &mut [corders_group::Root::default(); 8],
            &mut vec![Visit::default(); 32],
        );
        let reads = image.1.borrow().clone();
        assert_eq!(
            expected.outcome,
            match failure {
                None => Outcome::Rows { len: DEPTH + 3 },
                Some("cycle") => Outcome::Unavailable {
                    reason: "army order successor cycle",
                },
                Some("virtual cycle") => Outcome::Unavailable {
                    reason: "army order virtual successor cycle",
                },
                Some("fault") => Outcome::Unavailable {
                    reason: "army order owner unreadable",
                },
                _ => Outcome::Unavailable {
                    reason: "army order owner backlink mismatch",
                },
            }
        );
        for initial in [0, 1, 2, 4, 8] {
            let mut visits = vec![Visit::default(); initial];
            // Reuse also checks that active ancestors left by an earlier
            // failed walk do not leak into the next observation.
            for _ in 0..2 {
                image.1.borrow_mut().clear();
                let mut rows = Vec::new();
                let mut geometry = Vec::new();
                let actual = walk(
                    &image,
                    BASE,
                    &mut rows,
                    &mut geometry,
                    &mut [corders_group::Root::default(); 8],
                    &mut visits,
                );
                assert_eq!(actual, expected, "initial={initial}, failure={failure:?}");
                assert_eq!(rows, expected_rows);
                assert_eq!(geometry, expected_geometry);
                assert_eq!(*image.1.borrow(), reads);
                assert!(visits.len() > initial);
            }
        }
    }
}

#[test]
fn group_scratch_demand_counts_the_full_registry_and_does_not_invent_omitted_orders() {
    let mut image = fixture();
    image.u32(TABLE + registry::COUNT, 3);
    // Both omitted groups are valid but have no orders. Their registry
    // count cannot be substituted for an omitted order-row count.
    for index in 1..3 {
        let pointer = GROUP + index * 0x2000;
        let handle = ((index as u64) << 32) | corders_group::REFERENCE_TYPE as u64;
        let slot = BUCKETS + index * bucket::STRIDE;
        image.bytes(slot + bucket::DISTANCE, &[1]);
        image.u64(slot + bucket::TYPE, handle);
        image.u64(slot + bucket::OBJECT, pointer as u64);
        image.bytes(pointer, &[0; 0x240]);
        image.u64(pointer, (BASE + corders_group::VTABLE_RVA) as u64);
        image.u64(pointer + corders_group::HANDLE, handle);
        image.u32(pointer + corders_group::COUNTRY, 19);
    }
    let mut rows = vec![Row::default(); 8];
    let mut groups = [corders_group::Root::default(); 1];
    assert_eq!(
        corders_group::walk_roots(&image, BASE, &mut groups),
        Outcome::Truncated {
            len: 1,
            unread_rows: 2
        }
    );
    let short = walk(
        &image,
        BASE,
        &mut rows,
        &mut vec![0; 16],
        &mut groups,
        &mut vec![Visit::default(); 16],
    );
    assert_eq!(short.required_groups, 3);
    assert_eq!(
        short.outcome,
        Outcome::Unavailable {
            reason: "orders group scratch capacity exhausted"
        }
    );
    let recovered = walk(
        &image,
        BASE,
        &mut rows,
        &mut vec![0; 16],
        &mut [corders_group::Root::default(); 3],
        &mut vec![Visit::default(); 16],
    );
    assert_eq!(recovered.required_groups, 3);
    assert_eq!(recovered.outcome, Outcome::Rows { len: 3 });
    assert_eq!(recovered.required_geometry, 5);
}

#[test]
fn cycles_foreign_owners_and_duplicate_references_are_unavailable() {
    let mut image = fixture();
    image.array(CHILD + SUCCESSOR_DATA, 0x43000, 1);
    image.u64(0x43000, FIRST as u64);
    assert_eq!(
        capture(&image, &mut vec![Row::default(); 8], &mut vec![0; 16]).outcome,
        Outcome::Unavailable {
            reason: "army order successor cycle"
        }
    );
    let mut image = fixture();
    image.u64(CHILD + OWNER, GROUP as u64 + 8);
    // The bad backlink retains precedence over the later missing ID field.
    image.0.get_mut(&CHILD).unwrap().truncate(OWNER + 8);
    assert_eq!(
        capture(&image, &mut vec![Row::default(); 8], &mut vec![0; 16]).outcome,
        Outcome::Unavailable {
            reason: "army order owner backlink mismatch"
        }
    );
    let mut image = fixture();
    image.u32(CHILD + ID, 0);
    assert_eq!(
        capture(&image, &mut vec![Row::default(); 8], &mut vec![0; 16]).outcome,
        Outcome::Unavailable {
            reason: "duplicate native army order reference"
        }
    );
}

#[test]
fn malformed_registry_vectors_and_unreadable_geometry_are_not_empty_success() {
    let mut image = fixture();
    image.bytes(TABLE + registry::OVERFLOW_DISTANCE, &[1]);
    assert_eq!(
        capture(&image, &mut vec![Row::default(); 8], &mut vec![0; 16]).outcome,
        Outcome::Unavailable {
            reason: "orders registry header invalid"
        }
    );
    let mut image = fixture();
    image.u32(TABLE + registry::MASK, 3);
    image.bytes(TABLE + registry::OVERFLOW_DISTANCE, &[4]);
    assert_eq!(
        capture(&image, &mut vec![Row::default(); 8], &mut vec![0; 16]).outcome,
        Outcome::Unavailable {
            reason: "orders registry header invalid"
        }
    );
    let mut image = fixture();
    image.u32(TABLE + registry::COUNT, 2);
    assert_eq!(
        capture(&image, &mut vec![Row::default(); 8], &mut vec![0; 16]).outcome,
        Outcome::Unavailable {
            reason: "orders registry occupancy mismatch"
        }
    );
    let mut image = fixture();
    image.u32(FIRST + PATH_CAPACITY, 2);
    assert_eq!(
        capture(&image, &mut vec![Row::default(); 8], &mut vec![0; 16]).outcome,
        Outcome::Unavailable {
            reason: "army order vector unreadable or invalid"
        }
    );
    let mut image = fixture();
    image.0.remove(&PATH);
    assert_eq!(
        capture(&image, &mut Vec::new(), &mut Vec::new()).outcome,
        Outcome::Unavailable {
            reason: "army order geometry unreadable"
        }
    );
    assert_eq!(image.1.borrow().iter().filter(|p| p.0 == PATH).count(), 1);
}

#[test]
fn empty_native_order_lists_produce_a_complete_empty_table() {
    let mut image = fixture();
    image.array(GROUP + corders_group::PRIMARY_ORDER_DATA, 0, 0);
    assert_eq!(
        capture(&image, &mut Vec::new(), &mut Vec::new()).outcome,
        Outcome::Rows { len: 0 }
    );
    let static_empty = BASE + registry::STATIC_EMPTY_RVA;
    image.u64(TABLE + registry::ENTRIES, static_empty as u64);
    image.u32(TABLE + registry::COUNT, 0);
    image.u32(TABLE + registry::MASK, 0);
    image.bytes(TABLE + registry::OVERFLOW_DISTANCE, &[0]);
    image.bytes(static_empty, &[0; 2 * bucket::STRIDE]);
    image.bytes(
        static_empty + bucket::STRIDE + bucket::DISTANCE,
        &[bucket::TERMINAL_DISTANCE],
    );
    assert_eq!(
        capture(&image, &mut Vec::new(), &mut Vec::new()).outcome,
        Outcome::Rows { len: 0 }
    );
}

#[test]
fn group_withdrawing_byte_uses_authenticated_identities() {
    let mut image = fixture();
    image.bytes(GROUP + corders_group::WITHDRAWING, &[255]);
    let mut roots = [corders_group::Root::default(); 2];
    let source = corders_group::walk_roots(&image, BASE, &mut roots);
    assert_eq!(source, Outcome::Rows { len: 1 });
    let mut rows = [corders_group::CounterInputsRow::default(); 1];
    let census = crate::read_census::ReadCensus::new(&image);
    assert_eq!(
        corders_group::walk_counter_inputs(&census, BASE, &roots, source, &mut rows),
        Outcome::Rows { len: 1 }
    );
    println!(
        "orders-group counter inputs for 1 group after its registry census: {} copies, {} bytes",
        census.calls(),
        census.bytes()
    );
    assert_eq!(
        rows[0],
        corders_group::CounterInputsRow {
            group: HANDLE,
            withdrawing: 255
        }
    );
    assert_eq!(
        corders_group::walk_counter_inputs(&image, BASE, &roots, source, &mut []),
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
}

/// Unreadable spans and per-field widths for the gathered-read equivalence
/// test. `per_field` refuses every record gather and every merged copy
/// (joined string payloads, slot chunks), so each object takes the
/// original one-field-at-a-time reads.
struct Holed<'a> {
    image: &'a MemoryImage,
    hole: Option<(usize, usize)>,
    per_field: bool,
    payloads: &'a [usize],
}

impl Memory for Holed<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if let Some((start, len)) = self.hole
            && address < start + len
            && start < address + output.len()
        {
            return false;
        }
        if self.per_field
            && ![1, 4, 8, 16, 32].contains(&output.len())
            && !self.payloads.contains(&output.len())
        {
            return false;
        }
        self.image.copy(address, output)
    }

    fn copy_records(
        &self,
        sources: crate::memory::RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        if self.per_field {
            return 0;
        }
        let Some(count) = layout.count(sources, destination.len()) else {
            return 0;
        };
        let mut completed = 0;
        for row in 0..sources.len() {
            for field in layout.fields() {
                let Some(address) = sources
                    .pointer(row)
                    .filter(|&pointer| pointer != 0)
                    .and_then(|pointer| pointer.checked_add(field.source_offset))
                else {
                    return completed;
                };
                let at = row * layout.stride() + field.destination_offset;
                if !self.copy(address, &mut destination[at..at + field.len]) {
                    return completed;
                }
                completed += 1;
            }
        }
        debug_assert_eq!(completed, count);
        completed
    }
}

const PLAN_UNIT: usize = 0x60000;
const PLAN_MEMBERS: usize = 0x50000;
const PLAN_UNIQUE: usize = 0x70000;
const PLAN_GROUP_NAME: usize = 0x70100;

/// The fixture with every plan field in use: three assigned members, an
/// owner override, a named move with inline keys and a heap unique text,
/// and a coloured group with a heap name. Returns the image and the
/// lengths of its two heap payloads.
fn plan_fixture() -> (MemoryImage, [usize; 2]) {
    let mut image = fixture();
    image.u64(BASE + registry::INVALID_REFERENCE_RVA, u64::MAX);
    const UNIT: usize = PLAN_UNIT;
    const MEMBERS: usize = PLAN_MEMBERS;
    const UNIQUE: usize = PLAN_UNIQUE;
    const GROUP_NAME: usize = PLAN_GROUP_NAME;
    image.bytes(UNIT, &[0; 0x100]);
    image.u64(UNIT, (BASE + crate::carmy::VTABLE_RVA) as u64);
    image.u64(
        UNIT + crate::carmy::SECONDARY_BASE,
        (BASE + crate::carmy::SECONDARY_VTABLE_RVA) as u64,
    );
    image.u64(UNIT + crate::cunit::HANDLE, 0xffff_fffe_0000_0033);
    image.array(FIRST + ASSIGNED_MEMBER_DATA, MEMBERS, 3);
    image.bytes(MEMBERS, &[0; 24]);
    for slot in 0..3 {
        image.u64(
            MEMBERS + slot * 8,
            (UNIT + crate::cunit::SCHEDULED_MEMBER) as u64,
        );
    }
    image.u64(FIRST + OWNER_OVERRIDE, GROUP as u64);
    image.u32(SECOND + TYPE, TYPE_MOVE as u32);
    for offset in NAME_STRINGS {
        image.u64(SECOND + offset + crate::msvc_string::CAPACITY, 15);
    }
    image.bytes(SECOND + NAME_STRINGS[0], b"o_operation\0");
    image.u64(SECOND + NAME_STRINGS[0] + crate::msvc_string::LENGTH, 11);
    let unique = b"South Sudetenland";
    image.bytes(UNIQUE, b"South Sudetenland\0");
    image.u64(SECOND + NAME_STRINGS[4], UNIQUE as u64);
    image.u64(SECOND + NAME_STRINGS[4] + crate::msvc_string::LENGTH, 17);
    image.u64(SECOND + NAME_STRINGS[4] + crate::msvc_string::CAPACITY, 31);
    let group_name = b"Heeresgruppe Mitte, 2. Armee";
    image.bytes(GROUP_NAME, b"Heeresgruppe Mitte, 2. Armee\0");
    image.u64(GROUP + corders_group::NAME, GROUP_NAME as u64);
    image.u64(
        GROUP + corders_group::NAME + crate::msvc_string::LENGTH,
        group_name.len() as u64,
    );
    image.u64(
        GROUP + corders_group::NAME + crate::msvc_string::CAPACITY,
        31,
    );
    image.u32(GROUP + corders_group::COLOUR, 0x7fc0_1234);
    (image, [unique.len(), group_name.len()])
}

/// Gathered records give the per-field walk's rows, arenas and first
/// failure, for a hole at every eight bytes of every object the walks
/// read and for a changed byte at every field they decode.
#[test]
fn gathered_reads_match_per_field_reads_rows_and_failure_precedence() {
    const UNIT: usize = PLAN_UNIT;
    const MEMBERS: usize = PLAN_MEMBERS;
    const UNIQUE: usize = PLAN_UNIQUE;
    const GROUP_NAME: usize = PLAN_GROUP_NAME;
    let (image, payloads) = plan_fixture();
    let unique = b"South Sudetenland";
    let group_name = b"Heeresgruppe Mitte, 2. Armee";
    let mut groups = [corders_group::Root::default(); 8];
    let mut visits = vec![Visit::default(); 16];
    let census = walk(
        &image,
        BASE,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut groups,
        &mut visits,
    );
    assert_eq!(census.outcome, Outcome::Rows { len: 3 });

    #[derive(Debug, PartialEq)]
    struct Observed {
        orders: Outcome,
        groups: Outcome,
        counts: [usize; 6],
        rows: Vec<PlanRow>,
        group_rows: Vec<corders_group::PlanRow>,
        assigned: Vec<u64>,
        links: Vec<u32>,
        names: Vec<u8>,
        roots: Vec<u32>,
        group_names: Vec<u8>,
    }
    let run = |image: &MemoryImage, hole, per_field| {
        let memory = Holed {
            image,
            hole,
            per_field,
            payloads: &payloads,
        };
        let (mut rows, mut assigned, mut links, mut names) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let orders = walk_plans(
            &memory,
            BASE,
            &groups[..1],
            &visits,
            census.outcome,
            &mut rows,
            &mut assigned,
            &mut links,
            &mut names,
        );
        let mut group_rows = vec![corders_group::PlanRow::default(); 1];
        let (mut roots, mut group_names) = (Vec::new(), Vec::new());
        let group = corders_group::walk_plans(
            &memory,
            BASE,
            &groups[..1],
            census.root_outcome,
            &visits,
            census.outcome,
            &mut group_rows,
            &mut roots,
            &mut group_names,
        );
        let len = match orders.outcome {
            Outcome::Rows { len } => len,
            _ => 0,
        };
        rows.truncate(len);
        assigned.truncate(orders.assigned);
        links.truncate(orders.links);
        names.truncate(orders.names);
        roots.truncate(group.orders);
        group_names.truncate(group.names);
        if !matches!(group.outcome, Outcome::Rows { .. }) {
            group_rows.clear();
        }
        Observed {
            orders: orders.outcome,
            groups: group.outcome,
            counts: [
                orders.assigned,
                orders.required_assigned,
                orders.links,
                orders.names,
                group.orders,
                group.names,
            ],
            rows,
            group_rows,
            assigned,
            links,
            names,
            roots,
            group_names,
        }
    };
    let clean = run(&image, None, false);
    assert_eq!(clean.orders, Outcome::Rows { len: 3 });
    assert_eq!(clean.groups, Outcome::Rows { len: 1 });
    assert_eq!(clean.assigned, [0xffff_fffe_0000_0033; 3]);
    assert!(clean.names.ends_with(unique));
    assert_eq!(clean.group_names, group_name);
    assert_eq!(clean, run(&image, None, true));

    // Every eight bytes of every object and payload the walks read.
    let objects = [
        (FIRST, 0x380),
        (SECOND, 0x380),
        (CHILD, 0x380),
        (GROUP, corders_group::SIZE),
        (UNIT, 0x20),
        (MEMBERS, 24),
        (ROOTS, 16),
        (CHILDREN_A, 8),
        (CHILDREN_B, 8),
        (UNIQUE, 24),
        (GROUP_NAME, 32),
    ];
    let mut failures = std::collections::BTreeSet::new();
    for (object, size) in objects {
        for at in (object..object + size).step_by(8) {
            for len in [1, 8] {
                let gathered = run(&image, Some((at, len)), false);
                assert_eq!(
                    gathered,
                    run(&image, Some((at, len)), true),
                    "hole {at:#x}+{len}"
                );
                for outcome in [gathered.orders, gathered.groups] {
                    if let Outcome::Unavailable { reason } = outcome {
                        failures.insert(reason);
                    }
                }
            }
        }
    }
    // A changed byte at every decoded field.
    for (object, size) in objects {
        for at in object..object + size {
            let mut byte = [0];
            if !image.copy(at, &mut byte) {
                continue;
            }
            let mut changed = MemoryImage(image.0.clone(), Default::default());
            changed.bytes(at, &[byte[0] ^ 0x81]);
            let gathered = run(&changed, None, false);
            assert_eq!(gathered, run(&changed, None, true), "byte {at:#x}");
            for outcome in [gathered.orders, gathered.groups] {
                if let Outcome::Unavailable { reason } = outcome {
                    failures.insert(reason);
                }
            }
        }
    }
    // The sweep reached the failures of every stage of both walks.
    for reason in [
        "plan order identity mismatch",
        "plan group vtable mismatch",
        "plan group census identity changed",
        "plan order census identity changed",
        "plan graph header invalid",
        "plan graph pointers unreadable",
        "plan graph reference absent from owning census",
        "string header invalid",
        "string terminator unreadable or invalid",
        "string bytes unreadable",
        "plan assigned member header invalid",
        "plan member null or unreadable",
        "plan assigned division identity mismatch",
        "group plan root array invalid",
        "group plan root pointer unreadable",
        "root order absent from its group's census",
    ] {
        assert!(
            failures.contains(reason),
            "{reason} not reached: {failures:?}"
        );
    }
}

/// The plan records a listing census gathers while it holds each order
/// give the rows, arenas and first failure of the plan walks gathering
/// every order themselves, the census and the walks reading the same
/// memory as in one pass: clean, with a hole at every eight bytes of every
/// object either reads, and with a changed byte at every field they
/// decode. The census's own report, rows and geometry are unchanged, and
/// with complete records the order walk reads none of the orders' own
/// bytes. Wherever the registry census authenticated the roots, the group
/// identity check the order walk no longer repeats passes on that memory.
#[test]
fn census_plan_records_give_the_plan_walks_own_rows_and_failures() {
    let (image, payloads) = plan_fixture();
    #[derive(Debug, PartialEq)]
    struct Observed {
        census: Walk,
        census_rows: Vec<Row>,
        geometry: Vec<u32>,
        orders: Outcome,
        groups: Outcome,
        counts: [usize; 6],
        rows: Vec<PlanRow>,
        group_rows: Vec<corders_group::PlanRow>,
        assigned: Vec<u64>,
        links: Vec<u32>,
        names: Vec<u8>,
        roots: Vec<u32>,
        group_names: Vec<u8>,
    }
    // `records`: a listing census that gathers the plan records, or the
    // plain census, whose plan walk gathers every order itself.
    let run = |image: &MemoryImage, hole, records: bool| {
        let memory = Holed {
            image,
            hole,
            per_field: false,
            payloads: &payloads,
        };
        let mut groups = [corders_group::Root::default(); 8];
        let mut visits = vec![Visit::default(); 16];
        let mut lists = SuccessorLists::default();
        let (mut census_rows, mut geometry) = (Vec::new(), Vec::new());
        let census = if records {
            walk_listing_successors(
                &memory,
                BASE,
                &mut census_rows,
                &mut geometry,
                &mut groups,
                &mut visits,
                &mut lists,
            )
        } else {
            walk(
                &memory,
                BASE,
                &mut census_rows,
                &mut geometry,
                &mut groups,
                &mut visits,
            )
        };
        census_rows.truncate(census.outcome.len());
        geometry.truncate(census.geometry_len);
        let roots = &groups[..census.root_outcome.len().min(groups.len())];
        // The order walk no longer re-reads its group's identity: in the
        // pass that authenticated the roots, that check always passes.
        if matches!(census.root_outcome, Outcome::Rows { .. }) {
            for root in roots {
                assert_eq!(corders_group::plan_identity(&memory, BASE, *root), Ok(()));
            }
        }
        image.1.borrow_mut().clear();
        let (mut rows, mut assigned, mut links, mut names) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let orders = walk_plans_ordered(
            &memory,
            BASE,
            roots,
            &visits,
            &lists,
            census.outcome,
            &mut rows,
            &mut assigned,
            &mut links,
            &mut names,
            &mut Vec::new(),
        );
        let reads = image.1.borrow().clone();
        let mut group_rows = vec![corders_group::PlanRow::default(); roots.len()];
        let (mut root_ids, mut group_names) = (Vec::new(), Vec::new());
        let group = corders_group::walk_plans(
            &memory,
            BASE,
            roots,
            census.root_outcome,
            &visits,
            census.outcome,
            &mut group_rows,
            &mut root_ids,
            &mut group_names,
        );
        rows.truncate(orders.outcome.len());
        assigned.truncate(orders.assigned);
        links.truncate(orders.links);
        names.truncate(orders.names);
        root_ids.truncate(group.orders);
        group_names.truncate(group.names);
        if !matches!(group.outcome, Outcome::Rows { .. }) {
            group_rows.clear();
        }
        let observed = Observed {
            census,
            census_rows,
            geometry,
            orders: orders.outcome,
            groups: group.outcome,
            counts: [
                orders.assigned,
                orders.required_assigned,
                orders.links,
                orders.names,
                group.orders,
                group.names,
            ],
            rows,
            group_rows,
            assigned,
            links,
            names,
            roots: root_ids,
            group_names,
        };
        (observed, reads)
    };
    let (clean, reads) = run(&image, None, true);
    assert_eq!(clean.orders, Outcome::Rows { len: 3 });
    assert_eq!(clean.groups, Outcome::Rows { len: 1 });
    assert_eq!(clean.assigned.len(), 3);
    assert!(clean.names.ends_with(b"South Sudetenland"));
    assert_eq!(clean, run(&image, None, false).0);
    for order in [FIRST, SECOND, CHILD] {
        assert!(
            reads
                .iter()
                .all(|&(address, len)| address + len <= order || address >= order + 0x400),
            "order {order:#x} read again: {reads:x?}"
        );
    }

    let objects = [
        (FIRST, 0x380),
        (SECOND, 0x380),
        (CHILD, 0x380),
        (GROUP, corders_group::SIZE),
        (PLAN_UNIT, 0x20),
        (PLAN_MEMBERS, 24),
        (ROOTS, 16),
        (CHILDREN_A, 8),
        (CHILDREN_B, 8),
        (PLAN_UNIQUE, 24),
        (PLAN_GROUP_NAME, 32),
    ];
    let mut failures = std::collections::BTreeSet::new();
    for (object, size) in objects {
        for at in (object..object + size).step_by(8) {
            for len in [1, 8] {
                let (recorded, _) = run(&image, Some((at, len)), true);
                assert_eq!(
                    recorded,
                    run(&image, Some((at, len)), false).0,
                    "hole {at:#x}+{len}"
                );
                for outcome in [recorded.census.outcome, recorded.orders, recorded.groups] {
                    if let Outcome::Unavailable { reason } = outcome {
                        failures.insert(reason);
                    }
                }
            }
        }
    }
    for (object, size) in objects {
        for at in object..object + size {
            let mut byte = [0];
            if !image.copy(at, &mut byte) {
                continue;
            }
            let mut changed = MemoryImage(image.0.clone(), Default::default());
            changed.bytes(at, &[byte[0] ^ 0x81]);
            let (recorded, _) = run(&changed, None, true);
            assert_eq!(recorded, run(&changed, None, false).0, "byte {at:#x}");
            for outcome in [recorded.census.outcome, recorded.orders, recorded.groups] {
                if let Outcome::Unavailable { reason } = outcome {
                    failures.insert(reason);
                }
            }
        }
    }
    // The sweep reached failures of the census and of every stage of the
    // order walk that reads a recorded field. (A hole in a link vector
    // fails the census, which walks the same vectors, first.)
    for reason in [
        "army order vtable mismatch",
        "army order owner backlink mismatch",
        "plan graph header invalid",
        "string header invalid",
        "string terminator unreadable or invalid",
        "string bytes unreadable",
        "plan assigned member header invalid",
        "plan member null or unreadable",
        "plan assigned division identity mismatch",
    ] {
        assert!(
            failures.contains(reason),
            "{reason} not reached: {failures:?}"
        );
    }
}

/// A census over the roots this pass's registry walk already filled gives
/// the census that walks the registry itself: the same walk report, rows,
/// geometry, visits and plan rows, for a complete, a truncated and an
/// unavailable registry census.
#[test]
fn a_census_over_walked_roots_is_the_census_that_walks_them() {
    let complete = fixture();
    let mut unreadable = fixture();
    unreadable.0.remove(&BUCKETS);
    for (image, capacity) in [(&complete, 8), (&complete, 0), (&unreadable, 8)] {
        let own = {
            let (mut rows, mut geometry) = (Vec::new(), Vec::new());
            let mut groups = vec![corders_group::Root::default(); capacity];
            let mut visits = vec![Visit::default(); 16];
            let mut lists = SuccessorLists::default();
            let walk = walk_listing_successors(
                image,
                BASE,
                &mut rows,
                &mut geometry,
                &mut groups,
                &mut visits,
                &mut lists,
            );
            (walk, rows, geometry, groups, visits, lists)
        };
        let mut groups = vec![corders_group::Root::default(); capacity];
        let root_outcome = corders_group::walk_roots(image, BASE, &mut groups);
        let (mut rows, mut geometry) = (Vec::new(), Vec::new());
        let mut visits = vec![Visit::default(); 16];
        let mut lists = SuccessorLists::default();
        let walk = walk_listing_successors_of(
            image,
            BASE,
            &mut rows,
            &mut geometry,
            &groups,
            root_outcome,
            &mut visits,
            &mut lists,
        );
        assert_eq!(walk, own.0, "capacity {capacity}");
        assert_eq!(
            format!("{:?}", (&rows, &geometry, &groups, &visits)),
            format!("{:?}", (&own.1, &own.2, &own.3, &own.4))
        );
        let plans = |lists: &SuccessorLists| {
            let (mut output, mut assigned, mut links, mut names) =
                (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            let plan = walk_plans_ordered(
                image,
                BASE,
                &groups[..walk.root_outcome.len().min(groups.len())],
                &visits,
                lists,
                walk.outcome,
                &mut output,
                &mut assigned,
                &mut links,
                &mut names,
                &mut Vec::new(),
            );
            output.truncate(plan.outcome.len());
            links.truncate(plan.links);
            (plan.outcome, output, links)
        };
        assert_eq!(plans(&lists), plans(&own.5), "capacity {capacity}");
    }
    // A census longer than the roots it is given refuses to walk them.
    let walk = walk_listing_successors_of(
        &complete,
        BASE,
        &mut Vec::new(),
        &mut Vec::new(),
        &[],
        Outcome::Rows { len: 1 },
        &mut vec![Visit::default(); 16],
        &mut SuccessorLists::default(),
    );
    assert_eq!(
        walk.outcome,
        Outcome::Unavailable {
            reason: "orders group census exceeds its roots"
        }
    );
}

/// The name record follows the gathered type. For every order type the
/// gathered walk gives the per-field walk's rows, arenas and first
/// failure, clean, with a hole at every eight bytes of the name strings
/// and of a heap text, and with a changed byte at every name-string and
/// type byte. An order whose type reads no names never reads its
/// name-string headers; a named order reads them in one copy.
#[test]
fn type_driven_name_records_match_per_field_reads_for_every_order_type() {
    use crate::msvc_string::{CAPACITY, LENGTH};
    const UNIQUE: usize = 0x70000;
    let orders = [FIRST, SECOND, CHILD];
    let payloads = [20];
    for kind in [
        0,
        TYPE_MOVE,
        TYPE_LINE,
        TYPE_INVASION,
        TYPE_PARADROP,
        TYPE_AREA_DEFENSE,
        6,
        -1,
    ] {
        let mut image = fixture();
        image.u64(BASE + registry::INVALID_REFERENCE_RVA, u64::MAX);
        image.bytes(UNIQUE, b"Operation Barbarossa\0");
        for order in orders {
            image.u32(order + TYPE, kind as u32);
            for offset in NAME_STRINGS {
                image.bytes(order + offset, b"o_key\0");
                image.u64(order + offset + LENGTH, 5);
                image.u64(order + offset + CAPACITY, 15);
            }
        }
        image.u64(SECOND + NAME_STRINGS[4], UNIQUE as u64);
        image.u64(SECOND + NAME_STRINGS[4] + LENGTH, 20);
        image.u64(SECOND + NAME_STRINGS[4] + CAPACITY, 31);
        let mut groups = [corders_group::Root::default(); 8];
        let mut visits = vec![Visit::default(); 16];
        let census = walk(
            &image,
            BASE,
            &mut Vec::new(),
            &mut Vec::new(),
            &mut groups,
            &mut visits,
        );
        assert_eq!(census.outcome, Outcome::Rows { len: 3 });
        let run = |image: &MemoryImage, hole, per_field| {
            image.1.borrow_mut().clear();
            let memory = Holed {
                image,
                hole,
                per_field,
                payloads: &payloads,
            };
            let (mut rows, mut assigned, mut links, mut names) =
                (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            let walk = walk_plans(
                &memory,
                BASE,
                &groups[..1],
                &visits,
                census.outcome,
                &mut rows,
                &mut assigned,
                &mut links,
                &mut names,
            );
            rows.truncate(walk.outcome.len());
            names.truncate(walk.names);
            links.truncate(walk.links);
            assigned.truncate(walk.assigned);
            (walk.outcome, rows, names, links, assigned)
        };
        let name_reads = |image: &MemoryImage| {
            image
                .1
                .borrow()
                .iter()
                .filter(|&&(address, len)| {
                    orders
                        .iter()
                        .any(|&order| address < order + NAMES.1 && order + NAMES.0 < address + len)
                })
                .copied()
                .collect::<Vec<_>>()
        };
        let clean = run(&image, None, false);
        assert_eq!(clean.0, Outcome::Rows { len: 3 }, "type {kind}");
        let reads = name_reads(&image);
        if named_type(kind) {
            let text = b"Operation Barbarossa";
            assert!(
                clean.2.windows(text.len()).any(|w| w == text),
                "type {kind}"
            );
            let mut reads = reads;
            reads.sort_unstable();
            assert_eq!(
                reads,
                orders.map(|order| (order + NAMES.0, NAMES.1 - NAMES.0)),
                "type {kind}"
            );
        } else {
            assert!(clean.2.is_empty() && reads.is_empty(), "type {kind}");
        }
        assert_eq!(clean, run(&image, None, true), "type {kind}");
        let spans = orders
            .iter()
            .map(|&order| (order + NAMES.0, NAMES.1 - NAMES.0))
            .chain([(UNIQUE, 24)]);
        for (object, size) in spans.clone() {
            for at in (object..object + size).step_by(8) {
                for len in [1, 8] {
                    assert_eq!(
                        run(&image, Some((at, len)), false),
                        run(&image, Some((at, len)), true),
                        "type {kind} hole {at:#x}+{len}"
                    );
                }
            }
        }
        let types = orders.iter().map(|&order| (order + TYPE, 4));
        for (object, size) in spans.chain(types) {
            for at in object..object + size {
                let mut byte = [0];
                if !image.copy(at, &mut byte) {
                    continue;
                }
                let mut changed = MemoryImage(image.0.clone(), Default::default());
                changed.bytes(at, &[byte[0] ^ 0x81]);
                assert_eq!(
                    run(&changed, None, false),
                    run(&changed, None, true),
                    "type {kind} byte {at:#x}"
                );
            }
        }
    }
}

/// The masked slot is the remainder's slot for every power-of-two table,
/// through collisions, runs that wrap past the end, and full tables; other
/// lengths keep the remainder.
#[test]
fn visit_slots_are_the_remainder_slots_of_every_table() {
    fn remainder_slot(visits: &[Visit], pointer: usize) -> usize {
        if visits.is_empty() {
            return 0;
        }
        let mut hash = pointer as u64;
        hash ^= hash >> 30;
        hash = hash.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        hash ^= hash >> 27;
        let mut index = hash as usize % visits.len();
        for _ in 0..visits.len() {
            if visits[index].pointer == 0 || visits[index].pointer == pointer {
                break;
            }
            index = (index + 1) % visits.len();
        }
        index
    }
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state as usize) & 0x7fff_ffff_fff0
    };
    for len in [0, 1, 2, 3, 4, 5, 8, 12, 16, 64, 100, 256, 1024] {
        let mut visits = vec![Visit::default(); len];
        // Fill to full, checking every probe on the way, then probe more
        // pointers than the table holds against the full table.
        let mut inserted = Vec::new();
        for _ in 0..len + 8 {
            let pointer = next().max(1);
            let slot = visit_index(&visits, pointer);
            assert_eq!(slot, remainder_slot(&visits, pointer), "len {len}");
            if let Some(entry) = visits.get_mut(slot)
                && entry.pointer == 0
            {
                entry.pointer = pointer;
                inserted.push(pointer);
            }
        }
        for &pointer in &inserted {
            let slot = visit_index(&visits, pointer);
            assert_eq!(slot, remainder_slot(&visits, pointer));
            assert_eq!(visits[slot].pointer, pointer);
        }
    }
}

/// Sorting `(key, row)` pairs and permuting gives the rows a whole-row
/// sort gives, for every cycle shape.
#[test]
fn pair_sort_permutation_matches_a_whole_row_sort() {
    let mut state = 0x9e37_79b9_u64;
    for len in [0, 1, 2, 3, 17, 64, 257] {
        let rows: Vec<PlanRow> = (0..len)
            .map(|index| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                PlanRow {
                    owner_group: state % 7,
                    instance: (state >> 8) as u32 ^ index,
                    section: index,
                    ..Default::default()
                }
            })
            .collect();
        let mut expected = rows.clone();
        expected.sort_by_key(|row| row.key());
        let mut order: Vec<(u128, u32)> = rows
            .iter()
            .enumerate()
            .map(|(row, plan)| (plan.key(), row as u32))
            .collect();
        order.sort_unstable_by_key(|&(key, _)| key);
        let mut actual = rows.clone();
        permute(&mut actual, &mut order);
        assert_eq!(
            actual.iter().map(|row| row.key()).collect::<Vec<_>>(),
            expected.iter().map(|row| row.key()).collect::<Vec<_>>()
        );
        let mut sections: Vec<_> = actual.iter().map(|row| row.section).collect();
        sections.sort_unstable();
        assert_eq!(sections, (0..len).collect::<Vec<_>>());
    }
}
