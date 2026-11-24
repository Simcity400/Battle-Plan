use super::*;
use engine_native_parts::{creference_registry as registry, reference_bucket as bucket};
use std::collections::BTreeMap;

mod batched_orders {
    use super::*;
    use std::cell::Cell;

    struct Source<'a> {
        image: &'a MemoryImage,
        fault: Option<usize>,
        once: bool,
        spent: Cell<bool>,
    }

    impl Memory for Source<'_> {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            if self
                .fault
                .is_some_and(|fault| at <= fault && fault < at + out.len())
                && (!self.once || !self.spent.replace(true))
            {
                return false;
            }
            self.image.copy(at, out)
        }
    }

    struct Reads<'a> {
        source: Source<'a>,
        serial: bool,
        census: Cell<usize>,
    }

    impl Memory for Reads<'_> {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            if self.serial && (0x2000_0000..0x3000_0000).contains(&at) && out.len() > 8 {
                return false;
            }
            self.source.copy(at, out)
        }
        fn copy_records(
            &self,
            sources: RecordSources<'_>,
            layout: &RecordLayout<'_>,
            out: &mut [u8],
        ) -> usize {
            if layout.fields() == CENSUS_FIELDS {
                if self.serial {
                    return 0;
                }
                self.census.set(self.census.get() + 1);
            }
            self.source.copy_records(sources, layout, out)
        }
    }

    fn world(countries: usize, orders: usize) -> (MemoryImage, Vec<corders_group::Root>) {
        let mut image = MemoryImage::default();
        image.u64(BASE + registry::INVALID_REFERENCE_RVA.0, u64::MAX);
        let mut groups = Vec::new();
        for country in 0..countries {
            let group = 0x1000_0000 + country * 0x1000;
            let roster = 0x2000_0000 + country * 0x1000;
            image.bytes(group, &[0; 0x240]);
            image.bytes(roster, &vec![0; orders * 8]);
            image.array(
                group + corders_group::PRIMARY_ORDER_DATA,
                roster,
                orders as u32,
            );
            groups.push(corders_group::Root {
                pointer: group,
                handle: ((country as u64 + 1) << 32) | corders_group::REFERENCE_TYPE as u64,
                country: country as u32 + 1,
            });
            for order in 0..orders {
                let pointer = 0x3000_0000 + country * 0x100000 + order * 0x1000;
                image.order(pointer, order as u32 + 1, TYPE_LINE);
                image.u64(pointer + OWNER, group as u64);
                image.u64(roster + order * 8, pointer as u64);
            }
        }
        (image, groups)
    }

    pub(super) fn capture(
        image: &MemoryImage,
        groups: &[corders_group::Root],
        serial: bool,
        fault: Option<usize>,
        once: bool,
    ) -> (PartWalk, Census, usize) {
        let memory = Reads {
            source: Source {
                image,
                fault,
                once,
                spent: Cell::new(false),
            },
            serial,
            census: Cell::new(0),
        };
        let mut storage = CensusStorage::default();
        let part = walk_listing_successors_part(
            &memory,
            BASE,
            &mut storage.rows,
            &mut storage.geometry,
            groups,
            Outcome::Rows { len: groups.len() },
            0..groups.len(),
            &mut storage.visits,
            &mut storage.lists,
        );
        (part, Census::of(part.walk, &storage), memory.census.get())
    }

    #[test]
    fn persistent_fault_keeps_the_serial_entry_failure_and_prefix() {
        let (image, groups) = world(3, 32);
        for offset in [
            0,
            OWNER,
            ID,
            TYPE,
            ACTIVE,
            PARADROP_TARGET,
            PATH_DATA,
            TARGET_STATE_DATA,
        ] {
            let at = 0x3000_0000 + 20 * 0x1000 + offset;
            let batched = capture(&image, &groups, false, Some(at), false);
            let serial = capture(&image, &groups, true, Some(at), false);
            assert_eq!(batched.0, serial.0, "offset {offset:#x}");
            assert_eq!(batched.1, serial.1, "offset {offset:#x}");
            assert_eq!(
                batched.0.failure.unwrap().entering,
                Some(0x3000_0000 + 20 * 0x1000)
            );
        }
    }

    #[test]
    fn one_shot_fault_withholds_no_more_and_counts_the_fallback() {
        let (image, groups) = world(1, 32);
        let at = 0x3000_0000 + 20 * 0x1000 + PATH_DATA;
        let serial = capture(&image, &groups, true, Some(at), true);
        let before = engine_native_memory::batch_fallback::count(engine_native_memory::batch_fallback::Batch::OrderCensus);
        let batched = capture(&image, &groups, false, Some(at), true);
        assert_eq!(
            engine_native_memory::batch_fallback::count(engine_native_memory::batch_fallback::Batch::OrderCensus),
            before + 1
        );
        assert!(batched.0.walk.visited >= serial.0.walk.visited);
        assert_eq!(batched.0.walk.outcome, Outcome::Rows { len: 32 });
        assert_eq!(batched.1, capture(&image, &groups, true, None, false).1);
    }

    #[test]
    fn aliases_null_slots_descendants_and_first_failure_equal_serial() {
        for case in 0..5 {
            let (mut image, groups) = world(1, 32);
            let first = 0x3000_0000;
            let roster = 0x2000_0000;
            match case {
                0 => image.u64(roster + 8, 0),
                1 => image.u64(roster + 8, first as u64),
                2 => image.array(first + SUCCESSOR_DATA, roster + 8, 1),
                3 => image.u64(first, 0),
                _ => {}
            }
            let fault = match case {
                3 => Some(first + 4 * 0x1000),
                4 => Some(roster + 20 * 8),
                _ => None,
            };
            let batch = capture(&image, &groups, false, fault, false);
            let serial = capture(&image, &groups, true, fault, false);
            assert_eq!(batch.0, serial.0, "case {case}: entry and retained prefix");
            assert_eq!(batch.1, serial.1, "case {case}: rows, geometry and visits");
        }
    }

    #[test]
    fn aliased_roster_one_shot_recovers_in_one_counted_fallback() {
        let (mut image, groups) = world(1, 32);
        let at = 0x2000_0000 + 8;
        image.array(0x3000_0000 + SUCCESSOR_DATA, at, 1);
        let before = engine_native_memory::batch_fallback::count(engine_native_memory::batch_fallback::Batch::OrderCensus);
        let batch = capture(&image, &groups, false, Some(at), true);
        assert_eq!(
            engine_native_memory::batch_fallback::count(engine_native_memory::batch_fallback::Batch::OrderCensus),
            before + 1
        );
        let serial = capture(&image, &groups, true, Some(at), true);
        assert!(batch.0.walk.visited >= serial.0.walk.visited);
        assert_eq!(batch.1, capture(&image, &groups, true, None, false).1);
    }

}

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
        self.u64(address, (BASE + VTABLE_RVA.0) as u64);
        self.u64(address + OWNER, GROUP as u64);
        self.u32(address + ID, instance);
        self.u32(address + TYPE, kind as u32);
    }
}

fn fixture() -> MemoryImage {
    let mut image = MemoryImage::default();
    image.u64(
        BASE + registry::LOW_ROOTS_RVA.0 + corders_group::REFERENCE_TYPE as usize * 8,
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
    image.u64(GROUP, (BASE + corders_group::VTABLE_RVA.0) as u64);
    image.u64(GROUP + corders_group::HANDLE, HANDLE);
    image.u32(GROUP + corders_group::COUNTRY, 0xffff_fffe);
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
        &mut vec![corders_group::Root::default(); 8],
        &mut vec![Visit::default(); 16],
    )
}

#[test]
fn census_sequences_refuse_ordinals_past_the_stored_width() {
    let last = u32::MAX as usize;
    assert_eq!(super::census_sequence(5, 7), Ok(12));
    assert_eq!(super::census_sequence(last, 0), Ok(u32::MAX));
    assert_eq!(super::census_sequence(last - 1, 1), Ok(u32::MAX));
    let overflow = Err("army order census sequence overflows");
    assert_eq!(super::census_sequence(last + 1, 0), overflow);
    assert_eq!(super::census_sequence(last, 1), overflow);
    assert_eq!(super::census_sequence(1, u32::MAX), overflow);
}

#[test]
fn invasion_allocation_authenticates_the_subscriber_and_reuses_the_plan_prefix() {
    let mut image = MemoryImage::default();
    image.order(FIRST, 1, TYPE_INVASION);
    image.u64(FIRST + 8, (BASE + INVASION_SUBSCRIBER_VTABLE_RVA.0) as u64);
    for count in [0_i32, -3, i32::MIN, i32::MAX] {
        image.u32(FIRST + INVASION_TRANSPORTS, count as u32);
        let mut bytes = [0_u8; PLAN_STRIDE];
        let mut complete = [false];
        gather_records(&image, &[FIRST], &PLAN_LAYOUT, &mut bytes, &mut complete);
        assert_eq!(complete, [true]);
        let record = Gathered::row(&PLAN_LAYOUT, &PLAN_MAP, &bytes, &complete, 0);
        image.1.borrow_mut().clear();
        assert_eq!(
            invasion_transports(&image, BASE, FIRST, record),
            Some(count)
        );
        assert!(
            image.1.borrow().is_empty(),
            "count is already in the gathered prefix"
        );
        assert_eq!(
            invasion_transports(&image, BASE, FIRST, Gathered::NONE),
            Some(count)
        );
        assert_eq!(
            *image.1.borrow(),
            vec![(FIRST + 8, 8), (FIRST + INVASION_TRANSPORTS, 4)]
        );
    }
    image.u64(FIRST + 8, 0);
    image.1.borrow_mut().clear();
    assert_eq!(
        invasion_transports(&image, BASE, FIRST, Gathered::NONE),
        None
    );
    assert_eq!(*image.1.borrow(), vec![(FIRST + 8, 8)]);
    image.u64(FIRST + 8, (BASE + INVASION_SUBSCRIBER_VTABLE_RVA.0) as u64);
    image
        .0
        .get_mut(&FIRST)
        .unwrap()
        .truncate(INVASION_TRANSPORTS);
    image.1.borrow_mut().clear();
    assert_eq!(
        invasion_transports(&image, BASE, FIRST, Gathered::NONE),
        None
    );
    assert_eq!(
        *image.1.borrow(),
        vec![(FIRST + 8, 8), (FIRST + INVASION_TRANSPORTS, 4)]
    );
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
    image.u64(BASE + registry::INVALID_REFERENCE_RVA.0, u64::MAX);
    let mut groups = vec![corders_group::Root::default(); 8];
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
    image.u64(BASE + registry::INVALID_REFERENCE_RVA.0, u64::MAX);
    image.u32(SECOND + TYPE, TYPE_MOVE as u32);
    for offset in NAME_STRINGS {
        image.u64(SECOND + offset + 0x18, 15);
    }
    image.bytes(SECOND + NAME_STRINGS[0], b"fixture_key\0");
    image.u64(SECOND + NAME_STRINGS[0] + 0x10, 11);
    image.bytes(SECOND + NAME_STRINGS[1], b"o_x\0");
    image.u64(SECOND + NAME_STRINGS[1] + 0x10, 3);
    let unique = b"Synthetic Heights\0";
    image.bytes(0x70000, unique);
    image.u64(SECOND + NAME_STRINGS[4], 0x70000);
    image.u64(SECOND + NAME_STRINGS[4] + 0x10, 17);
    image.u64(SECOND + NAME_STRINGS[4] + 0x18, 31);
    let mut groups = vec![corders_group::Root::default(); 8];
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
    assert_eq!(text(named, 0), b"fixture_key");
    assert_eq!(text(named, 1), b"o_x");
    assert_eq!(text(named, 2), b"");
    assert_eq!(text(named, 4), b"Synthetic Heights");
    assert_eq!(result.names, 11 + 3 + 17);
    let line = plans.iter().find(|row| row.instance == 0).unwrap();
    assert_eq!(line.names, [(0, 0); 6]);
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
    image.u64(BASE + registry::INVALID_REFERENCE_RVA.0, u64::MAX);
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
    image.u64(UNIT, (BASE + crate::carmy::VTABLE_RVA.0) as u64);
    image.u64(
        UNIT + crate::carmy::SECONDARY_BASE,
        (BASE + crate::carmy::SECONDARY_VTABLE_RVA.0) as u64,
    );
    image.u64(UNIT + crate::cunit::HANDLE, 0xffff_fffe_0000_0033);
    image.array(FIRST + ASSIGNED_MEMBER_DATA, 0x50000, 2);
    image.u64(0x50000, (UNIT + crate::cunit::SCHEDULED_MEMBER) as u64);
    image.u64(0x50008, (UNIT + crate::cunit::SCHEDULED_MEMBER) as u64);
    let mut groups = vec![corders_group::Root::default(); 8];
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

#[test]
fn group_plans_keep_the_native_root_order_as_census_ids() {
    let mut image = fixture();
    let mut roots = vec![corders_group::Root::default(); 8];
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
    assert_eq!(
        &orders[row.primary_start..row.primary_start + row.primary_len],
        &[0, u32::MAX]
    );
    assert_eq!(row.fallback_len, 0);
    image.u64(ROOTS + 8, 0x7777_0000);
    let (walk, _) = plans(&image, &mut Vec::new());
    assert!(matches!(walk.outcome, Outcome::Unavailable { .. }));
}

#[test]
fn plan_graphs_preserve_native_order_and_reject_references_outside_the_census() {
    let mut image = fixture();
    image.u64(BASE + registry::INVALID_REFERENCE_RVA.0, u64::MAX);
    image.array(CHILD + PREDECESSOR_DATA, 0x44000, 3);
    image.bytes(0x44000, &[0; 24]);
    image.u64(0x44000, SECOND as u64);
    image.u64(0x44008, FIRST as u64);
    image.u64(0x44010, FIRST as u64);
    image.array(FIRST + SUCCESSOR_DATA, CHILDREN_A, 2);
    image.bytes(CHILDREN_A, &[0; 16]);
    image.u64(CHILDREN_A, CHILD as u64);
    image.u64(CHILDREN_A + 8, CHILD as u64);
    let mut groups = vec![corders_group::Root::default(); 8];
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

fn planned(census_image: &MemoryImage, image: &MemoryImage, listing: bool) -> Planned {
    planned_with(census_image, image, listing, true)
}

fn planned_with(
    census_image: &MemoryImage,
    image: &MemoryImage,
    listing: bool,
    records: bool,
) -> Planned {
    let mut groups = vec![corders_group::Root::default(); 8];
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
        &mut PlanOrder::default(),
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

type ResolvedRow = (PlanRow, Vec<u32>, Vec<u32>, Vec<Vec<u8>>);

impl Planned {
    fn resolved(&self) -> Vec<ResolvedRow> {
        let links = |start: usize, len: usize| self.links[start..start + len].to_vec();
        self.rows
            .iter()
            .map(|row| {
                let bare = PlanRow {
                    assigned_start: 0,
                    predecessors_start: 0,
                    successors_start: 0,
                    names: [(0, 0); 6],
                    ..*row
                };
                let names = row
                    .names
                    .iter()
                    .map(|&(start, len)| self.names[start..start + len].to_vec())
                    .collect();
                (
                    bare,
                    links(row.predecessors_start, row.predecessors_len),
                    links(row.successors_start, row.successors_len),
                    names,
                )
            })
            .collect()
    }

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

#[test]
fn census_successor_lists_give_the_rows_of_rereading_them() {
    const VIRTUAL: usize = 0x33000;
    const FALLBACK: usize = 0x35000;
    const FALLBACK_ROOTS: usize = 0x46000;
    const VIRTUAL_LIST: usize = 0x46100;
    const VIRTUAL_SUCCESSORS: usize = 0x46200;
    const FALLBACK_SUCCESSORS: usize = 0x46300;
    let mut image = fixture();
    image.u64(BASE + registry::INVALID_REFERENCE_RVA.0, u64::MAX);
    image.array(FIRST + SUCCESSOR_DATA, CHILDREN_A, 2);
    image.bytes(CHILDREN_A, &[0; 16]);
    image.u64(CHILDREN_A, CHILD as u64);
    image.u64(CHILDREN_A + 8, SECOND as u64);
    image.order(VIRTUAL, 17, TYPE_LINE);
    image.array(FIRST + VIRTUAL_SUCCESSOR_DATA, VIRTUAL_LIST, 1);
    image.u64(VIRTUAL_LIST, VIRTUAL as u64);
    image.array(VIRTUAL + SUCCESSOR_DATA, VIRTUAL_SUCCESSORS, 2);
    image.bytes(VIRTUAL_SUCCESSORS, &[0; 16]);
    image.u64(VIRTUAL_SUCCESSORS, CHILD as u64);
    image.u64(VIRTUAL_SUCCESSORS + 8, FIRST as u64);
    image.order(FALLBACK, 19, TYPE_MOVE);
    for offset in NAME_STRINGS {
        image.u64(FALLBACK + offset + engine_native_memory::msvc_string::CAPACITY, 15);
    }
    image.array(
        GROUP + corders_group::SECONDARY_ORDER_DATA,
        FALLBACK_ROOTS,
        1,
    );
    image.u64(FALLBACK_ROOTS, FALLBACK as u64);
    image.array(FALLBACK + SUCCESSOR_DATA, FALLBACK_SUCCESSORS, 1);
    image.u64(FALLBACK_SUCCESSORS, SECOND as u64);
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
            let case = format!("header change {change}, fresh census");
            assert_eq!(
                (reused.orders, reused.resolved()),
                (fresh.orders, fresh.resolved()),
                "orders: {case}"
            );
            assert_eq!(
                (reused.groups, &reused.group_rows, &reused.roots),
                (fresh.groups, &fresh.group_rows, &fresh.roots),
                "groups: {case}"
            );
        } else {
            assert_eq!(
                reused.orders,
                Outcome::Unavailable {
                    reason: "plan graph header invalid"
                }
            );
        }
    }

    let mut cyclic = copy_image(&image);
    cyclic.array(CHILD + SUCCESSOR_DATA, 0x46500, 1);
    cyclic.u64(0x46500, FIRST as u64);
    let mut lists = SuccessorLists::default();
    let census = walk_listing_successors(
        &cyclic,
        BASE,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut vec![corders_group::Root::default(); 8],
        &mut vec![Visit::default(); 16],
        &mut lists,
    );
    assert!(matches!(census.outcome, Outcome::Unavailable { .. }));
    assert!(lists.words.is_empty() && lists.pending.is_empty());
    planned(&cyclic, &cyclic, true)
        .assert_same_rows(&planned(&cyclic, &cyclic, false), "failed census");

    let mut plain_rows = vec![Row::default(); 8];
    let mut plain_geometry = vec![0; 16];
    let mut plain_visits = vec![Visit::default(); 16];
    let plain = walk(
        &image,
        BASE,
        &mut plain_rows,
        &mut plain_geometry,
        &mut vec![corders_group::Root::default(); 8],
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
        &mut vec![corders_group::Root::default(); 8],
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
    let mut groups = vec![corders_group::Root::default(); 8];
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
    let mut rows = vec![Row::default(); 8];
    assert_eq!(
        capture(&image, &mut rows, &mut vec![0; 16]).outcome,
        Outcome::Rows { len: 2 }
    );
    assert_eq!((rows[0].instance, rows[1].instance), (0, u32::MAX));
}

#[test]
fn group_and_traversal_scratch_grow_in_place() {
    let image = fixture();
    let mut rows = vec![Row::default(); 8];
    let mut visits = vec![Visit::default(); 2];
    let report = walk(
        &image,
        BASE,
        &mut rows,
        &mut vec![0; 16],
        &mut Vec::new(),
        &mut visits,
    );
    assert_eq!(report.outcome, Outcome::Rows { len: 3 });
    assert_eq!(report.required_groups, 1);
    assert_eq!(report.root_outcome, Outcome::Rows { len: 1 });
    assert_eq!(visits.len(), 4);
    assert_eq!(report.minimum_visits, 0);
    assert_eq!(report.visited, 3);
    let recovered = walk(
        &image,
        BASE,
        &mut rows,
        &mut vec![0; 16],
        &mut vec![corders_group::Root::default(); 1],
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
            &mut vec![corders_group::Root::default(); 8],
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
            for _ in 0..2 {
                image.1.borrow_mut().clear();
                let mut rows = Vec::new();
                let mut geometry = Vec::new();
                let actual = walk(
                    &image,
                    BASE,
                    &mut rows,
                    &mut geometry,
                    &mut vec![corders_group::Root::default(); 8],
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
fn group_scratch_grows_to_the_full_registry_and_does_not_invent_omitted_orders() {
    let mut image = fixture();
    image.u32(TABLE + registry::COUNT, 3);
    for index in 1..3 {
        let pointer = GROUP + index * 0x2000;
        let handle = ((index as u64) << 32) | corders_group::REFERENCE_TYPE as u64;
        let slot = BUCKETS + index * bucket::STRIDE;
        image.bytes(slot + bucket::DISTANCE, &[1]);
        image.u64(slot + bucket::TYPE, handle);
        image.u64(slot + bucket::OBJECT, pointer as u64);
        image.bytes(pointer, &[0; 0x240]);
        image.u64(pointer, (BASE + corders_group::VTABLE_RVA.0) as u64);
        image.u64(pointer + corders_group::HANDLE, handle);
        image.u32(pointer + corders_group::COUNTRY, 19);
    }
    let mut rows = vec![Row::default(); 8];
    let mut groups = vec![corders_group::Root::default(); 1];
    assert_eq!(
        corders_group::walk_roots(&image, BASE, &mut groups),
        Outcome::Rows { len: 3 }
    );
    let grown = walk(
        &image,
        BASE,
        &mut rows,
        &mut vec![0; 16],
        &mut vec![corders_group::Root::default(); 1],
        &mut vec![Visit::default(); 16],
    );
    assert_eq!(grown.required_groups, 3);
    assert_eq!(grown.outcome, Outcome::Rows { len: 3 });
    assert_eq!(grown.required_geometry, 5);
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
    let static_empty = BASE + registry::STATIC_EMPTY_RVA.0;
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
    let mut roots = vec![corders_group::Root::default(); 2];
    let source = corders_group::walk_roots(&image, BASE, &mut roots);
    assert_eq!(source, Outcome::Rows { len: 1 });
    let mut rows = [corders_group::CounterInputsRow::default(); 1];
    assert_eq!(
        corders_group::walk_counter_inputs(&image, BASE, &roots, source, &mut rows),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(
        rows[0],
        corders_group::CounterInputsRow {
            group: HANDLE,
            withdrawing: 255,
            hq_distance: Some(0),
            leader_enroute_days: Some(0),
            border_war_presence: Some(0),
            invasion_source_distance: Some(0),
            hq_nearest_front: Some(0),
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

#[test]
fn group_hq_distance_keeps_raw_signed_words_and_isolated_failures() {
    let mut image = fixture();
    image.bytes(GROUP + corders_group::WITHDRAWING, &[0xfe]);
    let mut roots = vec![corders_group::Root::default(); 2];
    let source = corders_group::walk_roots(&image, BASE, &mut roots);
    let mut rows = [corders_group::CounterInputsRow::default(); 1];
    for raw in [i32::MIN, -7, -1, 0, 1, i32::MAX] {
        image.u32(GROUP + corders_group::HQ_DISTANCE, raw as u32);
        image.u32(GROUP + corders_group::INVASION_SOURCE_DISTANCE, raw as u32);
        image.u32(GROUP + corders_group::HQ_NEAREST_FRONT, raw as u32);
        image.1.borrow_mut().clear();
        assert_eq!(
            corders_group::walk_counter_inputs(&image, BASE, &roots, source, &mut rows),
            Outcome::Rows { len: 1 }
        );
        assert_eq!(rows[0].hq_distance, Some(raw));
        assert_eq!(rows[0].invasion_source_distance, Some(raw));
        assert_eq!(rows[0].hq_nearest_front, Some(raw));
        assert!(image.1.borrow().iter().any(|&(at, len)| {
            at <= GROUP + corders_group::HQ_DISTANCE
                && GROUP + corders_group::HQ_DISTANCE + 4 <= at + len
        }));
        let gathered = rows;
        for (hole, per_field) in [
            (None, true),
            (Some((GROUP + 0x20, 8)), false),
            (Some((GROUP + corders_group::HQ_DISTANCE - 4, 4)), false),
        ] {
            let holed = Holed {
                image: &image,
                hole,
                per_field,
                payloads: &[],
            };
            assert_eq!(
                corders_group::walk_counter_inputs(&holed, BASE, &roots, source, &mut rows),
                Outcome::Rows { len: 1 }
            );
            assert_eq!(rows, gathered, "{hole:x?} {per_field}");
        }
        let holed = Holed {
            image: &image,
            hole: Some((GROUP + corders_group::HQ_DISTANCE, 4)),
            per_field: false,
            payloads: &[],
        };
        assert_eq!(
            corders_group::walk_counter_inputs(&holed, BASE, &roots, source, &mut rows),
            Outcome::Rows { len: 1 }
        );
        assert_eq!((rows[0].group, rows[0].withdrawing), (HANDLE, 0xfe));
        assert_eq!(rows[0].hq_distance, None);
        assert_eq!(rows[0].invasion_source_distance, Some(raw));
        assert_eq!(rows[0].hq_nearest_front, Some(raw));
        let holed = Holed {
            image: &image,
            hole: Some((GROUP + corders_group::INVASION_SOURCE_DISTANCE, 4)),
            per_field: false,
            payloads: &[],
        };
        assert_eq!(
            corders_group::walk_counter_inputs(&holed, BASE, &roots, source, &mut rows),
            Outcome::Rows { len: 1 }
        );
        assert_eq!((rows[0].group, rows[0].withdrawing), (HANDLE, 0xfe));
        assert_eq!(rows[0].hq_distance, Some(raw));
        assert_eq!(rows[0].invasion_source_distance, None);
        assert_eq!(rows[0].hq_nearest_front, Some(raw));
        assert_eq!(rows[0].border_war_presence, Some(0));
        assert_eq!(rows[0].leader_enroute_days, Some(0));
        let holed = Holed {
            image: &image,
            hole: Some((GROUP + corders_group::HQ_NEAREST_FRONT, 4)),
            per_field: false,
            payloads: &[],
        };
        assert_eq!(
            corders_group::walk_counter_inputs(&holed, BASE, &roots, source, &mut rows),
            Outcome::Rows { len: 1 }
        );
        assert_eq!(
            rows,
            [corders_group::CounterInputsRow {
                hq_nearest_front: None,
                ..gathered[0]
            }]
        );
    }
}

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
        sources: engine_native_memory::memory::RecordSources<'_>,
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

fn plan_fixture() -> (MemoryImage, [usize; 2]) {
    let mut image = fixture();
    image.u64(BASE + registry::INVALID_REFERENCE_RVA.0, u64::MAX);
    const UNIT: usize = PLAN_UNIT;
    const MEMBERS: usize = PLAN_MEMBERS;
    const UNIQUE: usize = PLAN_UNIQUE;
    const GROUP_NAME: usize = PLAN_GROUP_NAME;
    image.bytes(UNIT, &[0; 0x100]);
    image.u64(UNIT, (BASE + crate::carmy::VTABLE_RVA.0) as u64);
    image.u64(
        UNIT + crate::carmy::SECONDARY_BASE,
        (BASE + crate::carmy::SECONDARY_VTABLE_RVA.0) as u64,
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
        image.u64(SECOND + offset + engine_native_memory::msvc_string::CAPACITY, 15);
    }
    image.bytes(SECOND + NAME_STRINGS[0], b"fixture_key\0");
    image.u64(SECOND + NAME_STRINGS[0] + engine_native_memory::msvc_string::LENGTH, 11);
    let unique = b"Synthetic Heights";
    image.bytes(UNIQUE, b"Synthetic Heights\0");
    image.u64(SECOND + NAME_STRINGS[4], UNIQUE as u64);
    image.u64(SECOND + NAME_STRINGS[4] + engine_native_memory::msvc_string::LENGTH, 17);
    image.u64(SECOND + NAME_STRINGS[4] + engine_native_memory::msvc_string::CAPACITY, 31);
    let group_name = b"Synthetic Army Group Epsilon";
    image.bytes(GROUP_NAME, b"Synthetic Army Group Epsilon\0");
    image.u64(GROUP + corders_group::NAME, GROUP_NAME as u64);
    image.u64(
        GROUP + corders_group::NAME + engine_native_memory::msvc_string::LENGTH,
        group_name.len() as u64,
    );
    image.u64(
        GROUP + corders_group::NAME + engine_native_memory::msvc_string::CAPACITY,
        31,
    );
    image.u32(GROUP + corders_group::COLOUR, 0x7fc0_1234);
    (image, [unique.len(), group_name.len()])
}

#[test]
fn original_assigned_host_lane_has_no_reads_and_a_short_lane_grows() {
    let (image, _) = plan_fixture();
    let mut groups = vec![corders_group::Root::default(); 8];
    let mut visits = vec![Visit::default(); 16];
    let census = walk(
        &image,
        BASE,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut groups,
        &mut visits,
    );
    let run = |mut hosts: Option<&mut AssignedSources>| {
        image.1.borrow_mut().clear();
        let (mut rows, mut assigned, mut links, mut names) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let report = walk_plans_ordered_with_sources(
            &image,
            BASE,
            &groups[..census.root_outcome.len()],
            &visits,
            &SuccessorLists::default(),
            census.outcome,
            &mut rows,
            &mut assigned,
            &mut links,
            &mut names,
            &mut PlanOrder::default(),
            hosts.as_deref_mut(),
        );
        (
            report.outcome,
            rows,
            assigned,
            links,
            names,
            image.1.borrow().clone(),
        )
    };
    let baseline = run(None);
    let mut complete = AssignedSources::try_with_capacity(3).unwrap();
    assert_eq!(run(Some(&mut complete)), baseline);
    assert_eq!(complete.pointers(), &[PLAN_UNIT; 3]);
    assert_eq!(complete.outcome(), Outcome::Rows { len: 3 });
    let mut short = AssignedSources::try_with_capacity(1).unwrap();
    assert_eq!(run(Some(&mut short)), baseline);
    assert_eq!(short.pointers(), &[PLAN_UNIT; 3]);
    assert_eq!(short.required(), 3);
    assert_eq!(short.outcome(), Outcome::Rows { len: 3 });
    short.fail("worker unwind");
    assert_eq!(short.used(), 3);
    assert_eq!(short.required(), 3);
    assert_eq!(
        short.outcome(),
        Outcome::Unavailable {
            reason: "worker unwind"
        }
    );
}

#[test]
fn gathered_reads_match_per_field_reads_rows_and_failure_precedence() {
    const UNIT: usize = PLAN_UNIT;
    const MEMBERS: usize = PLAN_MEMBERS;
    const UNIQUE: usize = PLAN_UNIQUE;
    const GROUP_NAME: usize = PLAN_GROUP_NAME;
    let (image, payloads) = plan_fixture();
    let unique = b"Synthetic Heights";
    let group_name = b"Synthetic Army Group Epsilon";
    let mut groups = vec![corders_group::Root::default(); 8];
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
    let run = |image: &MemoryImage, hole, records: bool| {
        let memory = Holed {
            image,
            hole,
            per_field: false,
            payloads: &payloads,
        };
        let mut groups = vec![corders_group::Root::default(); 8];
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
            &mut PlanOrder::default(),
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
    assert!(clean.names.ends_with(b"Synthetic Heights"));
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
                &mut PlanOrder::default(),
            );
            output.truncate(plan.outcome.len());
            links.truncate(plan.links);
            (plan.outcome, output, links)
        };
        assert_eq!(plans(&lists), plans(&own.5), "capacity {capacity}");
    }
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

#[test]
fn type_driven_name_records_match_per_field_reads_for_every_order_type() {
    use engine_native_memory::msvc_string::{CAPACITY, LENGTH};
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
        image.u64(BASE + registry::INVALID_REFERENCE_RVA.0, u64::MAX);
        image.bytes(UNIQUE, b"Synthetic Plan Alpha\0");
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
        let mut groups = vec![corders_group::Root::default(); 8];
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
            let text = b"Synthetic Plan Alpha";
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

#[test]
fn placed_rows_match_a_whole_row_sort_and_name_each_census_orders_row() {
    let mut state = 0x9e37_79b9_u64;
    for len in [0, 1, 2, 3, 17, 64, 257] {
        let groups: Vec<_> = (0..7)
            .map(|handle| corders_group::Root {
                handle,
                ..Default::default()
            })
            .collect();
        let visits: Vec<Visit> = (0..len)
            .flat_map(|index| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                [
                    Visit {
                        pointer: 0x1000 + index,
                        owner: (state % 7) as usize,
                        instance: (state >> 8) as u32 ^ index as u32,
                        ..Default::default()
                    },
                    Visit::default(),
                ]
            })
            .collect();
        let key = |visit: &Visit| {
            (u128::from(groups[visit.owner].handle) << 32) | u128::from(visit.instance)
        };
        let mut expected: Vec<_> = visits.iter().filter(|v| v.pointer != 0).map(key).collect();
        expected.sort_unstable();
        let mut order = PlanOrder::default();
        assert_eq!(
            order.place(&groups, census_orders(&visits, &[]), len),
            Ok(false)
        );
        let mut placed = vec![0; len];
        for (ordinal, visit) in visits.iter().filter(|v| v.pointer != 0).enumerate() {
            placed[order.rows()[ordinal] as usize] = key(visit);
        }
        assert_eq!(placed, expected);
    }
    let groups = [corders_group::Root::default()];
    let twice = [Visit {
        pointer: 1,
        instance: 9,
        ..Default::default()
    }; 2];
    assert_eq!(
        PlanOrder::default().place(&groups, census_orders(&twice, &[]), 2),
        Ok(true)
    );
}

#[test]
fn keyed_rows_take_a_matching_guess_and_search_otherwise() {
    let rows = [3_u128, 5, 8, 13];
    let calls = std::cell::Cell::new(0);
    let counted = |row: &u128| {
        calls.set(calls.get() + 1);
        *row
    };
    assert_eq!(keyed_row(&rows, Some(2), 8, counted), Ok(2));
    assert_eq!(calls.get(), 1, "a matching guess costs one key");
    for guess in [None, Some(0), Some(3), Some(4), Some(u32::MAX)] {
        for (index, &key) in rows.iter().enumerate() {
            assert_eq!(keyed_row(&rows, guess, key, |row| *row), Ok(index));
        }
        assert_eq!(
            keyed_row(&rows, guess, 6, |row| *row),
            rows.binary_search(&6)
        );
    }
}

#[test]
fn the_plan_walk_leaves_each_census_orders_row_only_when_complete() {
    let (image, _) = plan_fixture();
    let mut groups = vec![corders_group::Root::default(); 8];
    let mut visits = vec![Visit::default(); 16];
    let mut lists = SuccessorLists::default();
    let (mut census_rows, mut geometry) = (Vec::new(), Vec::new());
    let census = walk_listing_successors(
        &image,
        BASE,
        &mut census_rows,
        &mut geometry,
        &mut groups,
        &mut visits,
        &mut lists,
    );
    let roots = &groups[..census.root_outcome.len()];
    let (mut rows, mut assigned, mut links, mut names) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut order = PlanOrder::default();
    let walk = walk_plans_ordered(
        &image,
        BASE,
        roots,
        &visits,
        &lists,
        census.outcome,
        &mut rows,
        &mut assigned,
        &mut links,
        &mut names,
        &mut order,
    );
    assert_eq!(walk.outcome, Outcome::Rows { len: 3 });
    assert_eq!(order.rows().len(), 3);
    for (ordinal, (_, visit)) in census_orders(&visits, lists.census()).enumerate() {
        let row = &rows[order.rows()[ordinal] as usize];
        assert_eq!(
            (row.owner_group, row.instance),
            (roots[visit.owner].handle, visit.instance)
        );
    }
    assert!(rows[..3].is_sorted_by_key(|row| row.key()));
    let failed = walk_plans_ordered(
        &image,
        BASE,
        roots,
        &visits,
        &lists,
        Outcome::Unavailable { reason: "census" },
        &mut rows,
        &mut assigned,
        &mut links,
        &mut names,
        &mut order,
    );
    assert!(matches!(failed.outcome, Outcome::Unavailable { .. }));
    assert!(order.rows().is_empty());
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Touch {
    Hint(usize),
    Read(usize, usize),
}

struct Trace<'a> {
    image: &'a MemoryImage,
    log: std::cell::RefCell<Vec<Touch>>,
    fault: Option<usize>,
    spent: std::cell::Cell<bool>,
    serial: bool,
}

impl Memory for Trace<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if self.serial && address == ROOTS && output.len() > 8 {
            return false;
        }
        self.log
            .borrow_mut()
            .push(Touch::Read(address, output.len()));
        if let Some(fault) = self.fault
            && address <= fault
            && fault < address + output.len()
            && !self.spent.replace(true)
        {
            return false;
        }
        self.image.copy(address, output)
    }

    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        out: &mut [u8],
    ) -> usize {
        if self.serial && layout.fields() == CENSUS_FIELDS {
            return 0;
        }
        struct Scalar<'a, 'b>(&'a Trace<'b>);
        impl Memory for Scalar<'_, '_> {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                self.0.copy(at, out)
            }
        }
        Scalar(self).copy_records(sources, layout, out)
    }

    fn prefetch(&self, address: usize, _len: usize) {
        self.log.borrow_mut().push(Touch::Hint(address));
    }
}

fn traced_census(image: &MemoryImage, fault: Option<usize>) -> (Walk, Vec<Row>, Vec<Touch>) {
    traced_census_mode(image, fault, false)
}

fn traced_census_mode(
    image: &MemoryImage,
    fault: Option<usize>,
    serial: bool,
) -> (Walk, Vec<Row>, Vec<Touch>) {
    let memory = Trace {
        image,
        log: Default::default(),
        fault,
        serial,
        spent: Default::default(),
    };
    let (mut rows, mut geometry) = (Vec::new(), Vec::new());
    let walk = walk_listing_successors(
        &memory,
        BASE,
        &mut rows,
        &mut geometry,
        &mut vec![corders_group::Root::default(); 8],
        &mut vec![Visit::default(); 16],
        &mut SuccessorLists::default(),
    );
    rows.truncate(walk.outcome.len());
    (walk, rows, memory.log.into_inner())
}

#[test]
fn a_shared_one_shot_fault_is_met_at_the_censuss_own_first_touch() {
    let met_after_hint = |log: &[Touch], fault: usize, read: Touch, hint: usize| {
        let first = log
            .iter()
            .position(|touch| {
                matches!(*touch, Touch::Read(address, len)
                    if address <= fault && fault < address + len)
            })
            .expect("the fault is touched");
        assert_eq!(log[first], read);
        let hinted = log
            .iter()
            .position(|touch| *touch == Touch::Hint(hint))
            .expect("the shared header is hinted");
        assert!(hinted < first, "{hint:#x} hinted after the first touch");
    };
    let mut aliased = fixture();
    aliased.array(FIRST + PATH_DATA, FIRST + SUCCESSOR_DATA, 1);
    let (walk, rows, log) = traced_census(&aliased, Some(FIRST + SUCCESSOR_DATA));
    met_after_hint(
        &log,
        FIRST + SUCCESSOR_DATA,
        Touch::Read(FIRST + SUCCESSOR_DATA, 4),
        FIRST + SUCCESSOR_DATA,
    );
    assert_eq!(
        walk.outcome,
        Outcome::Unavailable {
            reason: "army order geometry unreadable"
        }
    );
    assert_eq!((walk.visited, walk.geometry_len, rows.len()), (0, 0, 0));

    let mut roster = fixture();
    roster.array(FIRST + SUCCESSOR_DATA, ROOTS + 8, 1);
    let (walk, rows, log) = traced_census(&roster, Some(ROOTS + 8));
    let (serial, serial_rows, _) = traced_census_mode(&roster, Some(ROOTS + 8), true);
    assert_eq!(
        serial.outcome,
        Outcome::Unavailable {
            reason: "army order pointer unreadable or null"
        }
    );
    assert_eq!((serial.visited, serial.geometry_len), (1, 5));
    assert_eq!(walk.outcome, Outcome::Rows { len: 3 });
    assert!(walk.visited >= serial.visited && rows.len() >= serial_rows.len());
    let first = log
        .iter()
        .find(|touch| {
            matches!(**touch, Touch::Read(at, len)
        if at <= ROOTS + 8 && ROOTS + 8 < at + len)
        })
        .unwrap();
    assert_eq!(*first, Touch::Read(ROOTS, 16));

    const NODES: usize = 0x50000;
    const EDGES: usize = 0x70000;
    let mut chain = fixture();
    chain.array(GROUP + corders_group::PRIMARY_ORDER_DATA, ROOTS, 1);
    chain.bytes(EDGES, &[0; 0x200]);
    chain.array(FIRST + SUCCESSOR_DATA, EDGES, 1);
    chain.u64(EDGES, NODES as u64);
    for index in 0..15 {
        let node = NODES + index * 0x1000;
        chain.order(node, index as u32 + 1, TYPE_LINE);
        if index < 14 {
            let edges = EDGES + (index + 1) * 0x10;
            chain.array(node + SUCCESSOR_DATA, edges, 1);
            chain.u64(edges, (node + 0x1000) as u64);
        }
    }
    let last = NODES + 14 * 0x1000;
    let (walk, rows, log) = traced_census(&chain, Some(last + SUCCESSOR_DATA));
    assert_eq!(walk.outcome, Outcome::Rows { len: 16 });
    assert_eq!((walk.visited, rows.len()), (16, 16));
    let covering = |touch: &Touch| {
        matches!(*touch, Touch::Read(address, len)
            if address <= last + SUCCESSOR_DATA && last + SUCCESSOR_DATA < address + len)
    };
    let first = log.iter().position(covering).unwrap();
    assert_ne!(log[first], Touch::Read(last + SUCCESSOR_DATA, 16));
    let hinted = log
        .iter()
        .position(|touch| *touch == Touch::Hint(last + SUCCESSOR_DATA))
        .expect("the 16th order's successor header is hinted");
    assert!(hinted < first);
}

#[test]
fn passes_meet_orders_in_census_order_whatever_the_table_layout() {
    let (mut image, _) = plan_fixture();
    image.u32(FIRST + PREDECESSOR_DATA + 12, 5);
    image.u32(SECOND + ASSIGNED_MEMBER_DATA + 12, 5);
    let mut inverted = 0;
    for capacity in [8, 16, 32, 64, 128, 256, 512, 1024] {
        let mut groups = vec![corders_group::Root::default(); 8];
        let mut visits = vec![Visit::default(); capacity];
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
        assert_eq!(census.outcome, Outcome::Rows { len: 3 });
        let ordered: Vec<usize> = census_orders(&visits, lists.census())
            .map(|(_, visit)| visit.pointer)
            .collect();
        assert_eq!(ordered, [FIRST, CHILD, SECOND], "capacity {capacity}");
        let table: Vec<usize> = census_orders(&visits, &[])
            .map(|(_, visit)| visit.pointer)
            .collect();
        let slot = |pointer| table.iter().position(|&at| at == pointer).unwrap();
        if slot(SECOND) < slot(FIRST) {
            inverted += 1;
        }
        let walk = walk_plans_ordered(
            &image,
            BASE,
            &groups[..1],
            &visits,
            &lists,
            census.outcome,
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut PlanOrder::default(),
        );
        assert_eq!(
            walk.outcome,
            Outcome::Unavailable {
                reason: "plan graph header invalid"
            },
            "capacity {capacity}"
        );
    }
    assert!(inverted != 0, "no table layout held SECOND before FIRST");
}

fn groups_fixture(groups: usize, orders: usize) -> (MemoryImage, Vec<Vec<usize>>) {
    let mut image = fixture();
    image.u64(BASE + registry::INVALID_REFERENCE_RVA.0, u64::MAX);
    image.u32(TABLE + registry::COUNT, groups as u32);
    let mut pointers = Vec::new();
    for group in 0..groups {
        let pointer = GROUP + group * 0x2000;
        let handle = ((group as u64 + 1) << 32) | corders_group::REFERENCE_TYPE as u64;
        let slot = BUCKETS + group * bucket::STRIDE;
        image.bytes(slot + bucket::DISTANCE, &[1]);
        image.u64(slot + bucket::TYPE, handle);
        image.u64(slot + bucket::OBJECT, pointer as u64);
        image.bytes(pointer, &[0; 0x240]);
        image.u64(pointer, (BASE + corders_group::VTABLE_RVA.0) as u64);
        image.u64(pointer + corders_group::HANDLE, handle);
        image.u32(pointer + corders_group::COUNTRY, group as u32 + 1);
        let roster = 0x90000 + group * 0x100;
        image.bytes(roster, &[0; 0x100]);
        image.array(
            pointer + corders_group::PRIMARY_ORDER_DATA,
            roster,
            orders as u32,
        );
        let mut owned = Vec::new();
        for index in 0..orders {
            let order = 0x100000 + group * 0x10000 + index * 0x1000;
            image.order(order, index as u32 + 1, TYPE_LINE);
            image.u64(order + OWNER, pointer as u64);
            image.u64(roster + index * 8, order as u64);
            if index == 0 {
                image.u32(order + TYPE, TYPE_MOVE as u32);
                for offset in NAME_STRINGS {
                    image.u64(order + offset + engine_native_memory::msvc_string::CAPACITY, 15);
                }
                let text = 0xc0000 + group * 0x100;
                image.bytes(text, b"Operation Bagration\0");
                let unique = order + NAME_STRINGS[4];
                image.u64(unique, text as u64);
                image.u64(unique + engine_native_memory::msvc_string::LENGTH, 19);
                image.u64(unique + engine_native_memory::msvc_string::CAPACITY, 31);
                let path = 0xb0000 + group * 0x100;
                image.bytes(path, &[0; 8]);
                image.u32(path, group as u32 + 10);
                image.u32(path + 4, group as u32 + 20);
                image.array(order + PATH_DATA, path, 2);
            }
            owned.push(order);
        }
        if orders >= 2 {
            let list = 0xa0000 + group * 0x100;
            image.bytes(list, &[0; 8]);
            image.u64(list, owned[1] as u64);
            image.array(owned[0] + SUCCESSOR_DATA, list, 1);
            let back = 0xa8000 + group * 0x100;
            image.bytes(back, &[0; 8]);
            image.u64(back, owned[0] as u64);
            image.array(owned[1] + PREDECESSOR_DATA, back, 1);
        }
        pointers.push(owned);
    }
    (image, pointers)
}

fn assign_members(image: &mut MemoryImage, order: usize, group: usize, index: usize, count: usize) {
    let members = 0xd0000 + (group * 16 + index) * 0x40;
    image.bytes(members, &[0; 0x40]);
    for member in 0..count {
        let unit = member_unit(group, index, member);
        image.bytes(unit, &[0; 0x100]);
        image.u64(unit, (BASE + crate::carmy::VTABLE_RVA.0) as u64);
        image.u64(
            unit + crate::carmy::SECONDARY_BASE,
            (BASE + crate::carmy::SECONDARY_VTABLE_RVA.0) as u64,
        );
        image.u64(
            unit + crate::cunit::HANDLE,
            (((group * 16 + index) as u64 + 1) << 32) | (0x33 + member as u64),
        );
        image.u64(
            members + member * 8,
            (unit + crate::cunit::SCHEDULED_MEMBER) as u64,
        );
    }
    image.array(order + ASSIGNED_MEMBER_DATA, members, count as u32);
}

fn member_unit(group: usize, index: usize, member: usize) -> usize {
    0xe0000 + (group * 16 + index) * 0x400 + member * 0x100
}

fn order_roots(image: &MemoryImage, capacity: usize) -> (Vec<corders_group::Root>, Outcome) {
    let mut groups = vec![corders_group::Root::default(); capacity];
    let outcome = corders_group::walk_roots(image, BASE, &mut groups);
    (groups, outcome)
}

fn group_splits(count: usize) -> Vec<Vec<std::ops::Range<usize>>> {
    (0_u32..1 << count.saturating_sub(1))
        .flat_map(|cuts| {
            let mut starts = vec![0];
            starts.extend((1..count).filter(|&at| cuts & (1 << (at - 1)) != 0));
            let ends = starts.iter().skip(1).copied().chain([count]);
            let split: Vec<_> = starts
                .iter()
                .copied()
                .zip(ends)
                .map(|(start, end)| start..end)
                .collect();
            let leading = std::iter::once(0..0).chain(split.clone()).collect();
            let trailing = split
                .iter()
                .cloned()
                .chain(std::iter::once(count..count))
                .collect();
            [split, leading, trailing]
        })
        .collect()
}

#[derive(Default)]
struct CensusStorage {
    rows: Vec<Row>,
    geometry: Vec<u32>,
    visits: Vec<Visit>,
    lists: SuccessorLists,
}

#[derive(Debug, PartialEq)]
struct Census {
    walk: Walk,
    rows: Vec<Row>,
    geometry: Vec<u32>,
    entered: Vec<(u32, usize, usize, u32)>,
    order: Vec<usize>,
}

impl Census {
    fn of(walk: Walk, storage: &CensusStorage) -> Self {
        let retained = match walk.outcome {
            Outcome::Rows { len } => len,
            _ => walk.visited,
        };
        let mut entered: Vec<_> = storage
            .visits
            .iter()
            .filter(|visit| visit.pointer != 0)
            .map(|visit| (visit.sequence, visit.pointer, visit.owner, visit.instance))
            .collect();
        entered.sort_unstable();
        Self {
            walk,
            rows: storage.rows[..retained].to_vec(),
            geometry: storage.geometry[..walk.geometry_len].to_vec(),
            entered,
            order: storage
                .lists
                .census()
                .iter()
                .map(|&slot| storage.visits[slot as usize].pointer)
                .collect(),
        }
    }
}

fn serial_census(
    image: &MemoryImage,
    groups: &[corders_group::Root],
    root_outcome: Outcome,
    storage: &mut CensusStorage,
) -> Census {
    let walk = walk_listing_successors_of(
        image,
        BASE,
        &mut storage.rows,
        &mut storage.geometry,
        groups,
        root_outcome,
        &mut storage.visits,
        &mut storage.lists,
    );
    Census::of(walk, storage)
}

fn part_censuses(
    image: &MemoryImage,
    groups: &[corders_group::Root],
    root_outcome: Outcome,
    ranges: &[std::ops::Range<usize>],
) -> Vec<(PartWalk, CensusStorage)> {
    ranges
        .iter()
        .map(|range| {
            let mut part = CensusStorage::default();
            let walk = walk_listing_successors_part(
                image,
                BASE,
                &mut part.rows,
                &mut part.geometry,
                groups,
                root_outcome,
                range.clone(),
                &mut part.visits,
                &mut part.lists,
            );
            (walk, part)
        })
        .collect()
}

fn stitch_part_censuses(
    parts: &[(PartWalk, CensusStorage)],
    storage: &mut CensusStorage,
) -> Census {
    let views: Vec<CensusPart<'_>> = parts
        .iter()
        .map(|(walk, part)| CensusPart {
            walk: *walk,
            rows: &part.rows,
            geometry: &part.geometry,
            visits: &part.visits,
        })
        .collect();
    let walk = stitch_census(
        &views,
        &mut storage.rows,
        &mut storage.geometry,
        &mut storage.visits,
        &mut storage.lists,
    );
    Census::of(walk, storage)
}

#[test]
fn stitched_census_parts_are_the_single_census_for_every_split() {
    let (_, pointers) = groups_fixture(4, 3);
    let unavailable = |reason| Outcome::Unavailable { reason };
    let mut cases: Vec<(&str, MemoryImage, usize, Outcome)> = Vec::new();
    cases.push((
        "clean",
        groups_fixture(4, 3).0,
        8,
        Outcome::Rows { len: 12 },
    ));
    cases.push((
        "more than a batch",
        groups_fixture(6, 3).0,
        8,
        Outcome::Rows { len: 18 },
    ));
    cases.push((
        "grown roots",
        groups_fixture(4, 3).0,
        3,
        Outcome::Rows { len: 12 },
    ));
    let (mut image, _) = groups_fixture(4, 3);
    image.u64(pointers[2][1], 0);
    cases.push((
        "vtable",
        image,
        8,
        unavailable("army order vtable mismatch"),
    ));
    let (mut image, _) = groups_fixture(4, 3);
    image.u64(pointers[0][2] + OWNER, 0);
    image.u64(pointers[3][0], 0);
    cases.push((
        "first of two",
        image,
        8,
        unavailable("army order owner backlink mismatch"),
    ));
    let (mut image, _) = groups_fixture(4, 3);
    image.u64(0x90000 + 3 * 0x100, pointers[1][2] as u64);
    image.u64(pointers[3][1], 0);
    cases.push((
        "shared first",
        image,
        8,
        unavailable("army order shared across owning groups"),
    ));
    let (mut image, _) = groups_fixture(4, 3);
    image.u64(pointers[3][0], 0);
    image.u64(0x90000 + 3 * 0x100 + 8, pointers[1][2] as u64);
    cases.push((
        "broken first",
        image,
        8,
        unavailable("army order vtable mismatch"),
    ));
    let (mut image, _) = groups_fixture(4, 3);
    image.u64(0x90000 + 2 * 0x100, pointers[1][1] as u64);
    cases.push((
        "shared successor",
        image,
        8,
        unavailable("army order shared across owning groups"),
    ));
    let (mut image, _) = groups_fixture(4, 3);
    image.u32(pointers[2][2] + ID, 1);
    cases.push((
        "duplicate key",
        image,
        8,
        unavailable("duplicate native army order reference"),
    ));
    let (mut image, _) = groups_fixture(4, 3);
    image.bytes(0xa4000, &[0; 8]);
    image.u64(0xa4000, pointers[1][0] as u64);
    image.array(pointers[1][1] + SUCCESSOR_DATA, 0xa4000, 1);
    cases.push(("cycle", image, 8, unavailable("army order successor cycle")));
    let (mut image, _) = groups_fixture(4, 3);
    image.bytes(0xa4100, &[0; 8]);
    image.u64(0xa4100, pointers[2][0] as u64);
    image.array(pointers[2][0] + VIRTUAL_SUCCESSOR_DATA, 0xa4100, 1);
    cases.push((
        "virtual cycle",
        image,
        8,
        unavailable("army order virtual successor cycle"),
    ));
    let (mut image, _) = groups_fixture(4, 3);
    image.u64(pointers[2][0] + PATH_DATA, 0xdead_0000);
    cases.push((
        "geometry",
        image,
        8,
        unavailable("army order geometry unreadable"),
    ));
    let (mut image, _) = groups_fixture(4, 3);
    image.u64(0xa0000 + 3 * 0x100, pointers[1][2] as u64);
    cases.push((
        "successor into an earlier group",
        image,
        8,
        unavailable("army order shared across owning groups"),
    ));
    let (mut image, _) = groups_fixture(4, 3);
    image.u32(pointers[1][2] + ID, 1);
    image.u64(pointers[3][1], 0);
    cases.push((
        "duplicate then broken",
        image,
        8,
        unavailable("army order vtable mismatch"),
    ));
    let (before, _) = groups_fixture(4, 3);
    let (before_groups, before_outcome) = order_roots(&before, 8);
    for (name, image, capacity, expected) in &cases {
        let (groups, root_outcome) = order_roots(image, *capacity);
        let mut storage = CensusStorage::default();
        serial_census(&before, &before_groups, before_outcome, &mut storage);
        let serial = serial_census(image, &groups, root_outcome, &mut storage);
        assert_eq!(serial.walk.outcome, *expected, "{name}");
        let splits = group_splits(root_outcome.len());
        assert_eq!(splits.len(), 3 << (root_outcome.len() - 1), "{name}");
        for ranges in splits {
            let mut storage = CensusStorage::default();
            let previous = part_censuses(
                &before,
                &before_groups,
                before_outcome,
                std::slice::from_ref(&(0..4)),
            );
            stitch_part_censuses(&previous, &mut storage);
            let parts = part_censuses(image, &groups, root_outcome, &ranges);
            let stitched = stitch_part_censuses(&parts, &mut storage);
            assert_eq!(stitched, serial, "{name} {ranges:?}");
        }
    }
    let state = |name| {
        let (_, image, capacity, _) = cases.iter().find(|(case, ..)| *case == name).unwrap();
        let (groups, root_outcome) = order_roots(image, *capacity);
        serial_census(image, &groups, root_outcome, &mut CensusStorage::default())
    };
    let vtable = state("vtable");
    assert_eq!(
        (
            vtable.walk.visited,
            vtable.rows.len(),
            vtable.geometry.len(),
            vtable.entered.len()
        ),
        (7, 7, 6, 7)
    );
    assert!(vtable.order.is_empty());
    let shared = state("shared first");
    assert_eq!((shared.walk.visited, shared.rows.len()), (9, 9));
}

#[derive(Default)]
struct PlanStorage {
    rows: Vec<PlanRow>,
    assigned: Vec<u64>,
    links: Vec<u32>,
    names: Vec<u8>,
    order: PlanOrder,
}

#[derive(Debug, PartialEq)]
struct Plans {
    outcome: Outcome,
    required_assigned: usize,
    rows: Vec<PlanRow>,
    assigned: Vec<u64>,
    links: Vec<u32>,
    names: Vec<u8>,
    placement: Vec<u32>,
    hosts: Vec<usize>,
    required_hosts: usize,
    hosts_outcome: Outcome,
}

impl Plans {
    fn of(walk: PlanWalk, storage: &PlanStorage, hosts: &AssignedSources) -> Self {
        Self {
            outcome: walk.outcome,
            required_assigned: walk.required_assigned,
            rows: storage.rows[..walk.outcome.len()].to_vec(),
            assigned: storage.assigned[..walk.assigned].to_vec(),
            links: storage.links[..walk.links].to_vec(),
            names: storage.names[..walk.names].to_vec(),
            placement: storage.order.rows().to_vec(),
            hosts: hosts.pointers().to_vec(),
            required_hosts: hosts.required(),
            hosts_outcome: hosts.outcome(),
        }
    }
}

#[allow(clippy::type_complexity)]
fn serial_and_stitched_plans(
    image: &MemoryImage,
) -> (Plans, Vec<(Vec<std::ops::Range<usize>>, Plans)>) {
    let (groups, root_outcome) = order_roots(image, 8);
    let count = root_outcome.len();
    let serial = {
        let mut census = CensusStorage::default();
        let walk = serial_census(image, &groups, root_outcome, &mut census).walk;
        let mut plans = PlanStorage::default();
        let mut hosts = AssignedSources::default();
        let walk = walk_plans_ordered_with_sources(
            image,
            BASE,
            &groups[..count],
            &census.visits,
            &census.lists,
            walk.outcome,
            &mut plans.rows,
            &mut plans.assigned,
            &mut plans.links,
            &mut plans.names,
            &mut plans.order,
            Some(&mut hosts),
        );
        Plans::of(walk, &plans, &hosts)
    };
    let mut splits = Vec::new();
    for ranges in group_splits(count) {
        let censuses = part_censuses(image, &groups, root_outcome, &ranges);
        let walks: Vec<(PlanWalk, PlanStorage, AssignedSources)> = censuses
            .iter()
            .map(|(census, part)| {
                let mut plans = PlanStorage::default();
                let mut hosts = AssignedSources::default();
                let walk = walk_plans_ordered_with_sources(
                    image,
                    BASE,
                    &groups[..count],
                    &part.visits,
                    &part.lists,
                    census.walk.outcome,
                    &mut plans.rows,
                    &mut plans.assigned,
                    &mut plans.links,
                    &mut plans.names,
                    &mut plans.order,
                    Some(&mut hosts),
                );
                (walk, plans, hosts)
            })
            .collect();
        let census = stitch_part_censuses(&censuses, &mut CensusStorage::default()).walk;
        let parts: Vec<PlanPart<'_>> = walks
            .iter()
            .zip(&censuses)
            .map(|((walk, plans, hosts), (census, part))| PlanPart {
                walk: *walk,
                rows: &plans.rows,
                assigned: &plans.assigned,
                links: &plans.links,
                names: &plans.names,
                order: &plans.order,
                hosts: Some(hosts),
                orders: &part.rows[..census.walk.visited.min(part.rows.len())],
            })
            .collect();
        let mut plans = PlanStorage::default();
        let mut hosts = AssignedSources::default();
        let walk = stitch_plans(
            &parts,
            census.outcome,
            &mut plans.rows,
            &mut plans.assigned,
            &mut plans.links,
            &mut plans.names,
            &mut plans.order,
            Some(&mut hosts),
        );
        splits.push((ranges, Plans::of(walk, &plans, &hosts)));
    }
    (serial, splits)
}

#[test]
fn stitched_plan_parts_are_the_single_plan_walk_for_every_split() {
    let with_members = || {
        let (mut image, pointers) = groups_fixture(6, 3);
        for (group, orders) in pointers.iter().enumerate() {
            assign_members(&mut image, orders[0], group, 0, 2);
            assign_members(&mut image, orders[2], group, 2, 3);
        }
        (image, pointers)
    };
    let (_, pointers) = with_members();
    let unavailable = |reason| Outcome::Unavailable { reason };
    let mut cases: Vec<(&str, MemoryImage, Outcome)> = Vec::new();
    cases.push(("clean", with_members().0, Outcome::Rows { len: 18 }));
    let (mut image, _) = with_members();
    image.u32(pointers[2][1] + PREDECESSOR_DATA + 12, 5);
    cases.push((
        "predecessors",
        image,
        unavailable("plan graph header invalid"),
    ));
    let (mut image, _) = with_members();
    image.u32(pointers[4][1] + PREDECESSOR_DATA + 12, 5);
    image.u32(pointers[1][2] + ASSIGNED_MEMBER_DATA + 12, 5);
    cases.push((
        "first of two",
        image,
        unavailable("plan assigned member header invalid"),
    ));
    let member = || {
        let (mut image, _) = with_members();
        image.u64(member_unit(3, 2, 1), 0);
        image
    };
    cases.push((
        "member",
        member(),
        unavailable("plan assigned division identity mismatch"),
    ));
    let (mut image, _) = with_members();
    image.u64(pointers[1][2], 0);
    cases.push((
        "census",
        image,
        unavailable("plans require a complete army order census"),
    ));
    let mut states = Vec::new();
    for (name, image, expected) in &cases {
        let (serial, splits) = serial_and_stitched_plans(image);
        assert_eq!(serial.outcome, *expected, "{name}");
        assert_eq!(splits.len(), 3 * 32, "{name}");
        for (ranges, stitched) in splits {
            assert_eq!(stitched, serial, "{name} {ranges:?}");
        }
        states.push(serial);
    }
    let clean = &states[0];
    assert_eq!(
        (
            clean.assigned.len(),
            clean.links.len(),
            clean.names.len(),
            clean.hosts.len()
        ),
        (30, 12, 6 * 19, 30)
    );
    let member = &states[3];
    assert_eq!(
        (
            member.assigned.len(),
            member.required_assigned,
            member.hosts.len(),
            member.required_hosts
        ),
        (18, 18, 18, 20)
    );
    assert_eq!(
        member.hosts_outcome,
        unavailable("plan assigned hosts require complete core plans")
    );
    assert_eq!(member.names.len(), 4 * 19);
    assert!(member.rows.is_empty() && member.placement.is_empty());
}

#[test]
fn a_one_shot_fault_two_parts_share_is_met_in_time_order_and_recorded_once() {
    struct OneShot<'a> {
        image: &'a MemoryImage,
        at: usize,
        spent: std::cell::Cell<bool>,
    }
    impl Memory for OneShot<'_> {
        fn copy(&self, address: usize, out: &mut [u8]) -> bool {
            if address <= self.at && self.at < address + out.len() && !self.spent.replace(true) {
                return false;
            }
            self.image.copy(address, out)
        }
    }
    let (image, _) = groups_fixture(4, 3);
    let (groups, root_outcome) = order_roots(&image, 8);
    let groups = &groups[..root_outcome.len()];
    let ranges = [0..2, 2..4];
    let censuses = part_censuses(&image, groups, root_outcome, &ranges);
    let census = stitch_part_censuses(&censuses, &mut CensusStorage::default()).walk;
    assert_eq!(census.outcome, Outcome::Rows { len: 12 });
    let at = BASE + registry::INVALID_REFERENCE_RVA.0;
    let reason = "plan field unreadable or address overflow";
    let serial = {
        let memory = OneShot {
            image: &image,
            at,
            spent: Default::default(),
        };
        let mut storage = CensusStorage::default();
        let walk = serial_census(&image, groups, root_outcome, &mut storage).walk;
        let mut plans = PlanStorage::default();
        let mut hosts = AssignedSources::default();
        let walk = walk_plans_ordered_with_sources(
            &memory,
            BASE,
            groups,
            &storage.visits,
            &storage.lists,
            walk.outcome,
            &mut plans.rows,
            &mut plans.assigned,
            &mut plans.links,
            &mut plans.names,
            &mut plans.order,
            Some(&mut hosts),
        );
        Plans::of(walk, &plans, &hosts)
    };
    assert_eq!(serial.outcome, Outcome::Unavailable { reason });
    for first in [0, 1] {
        let memory = OneShot {
            image: &image,
            at,
            spent: Default::default(),
        };
        let mut walks: [Option<(PlanWalk, PlanStorage, AssignedSources)>; 2] = [None, None];
        for index in [first, 1 - first] {
            let (part_walk, part) = &censuses[index];
            let mut plans = PlanStorage::default();
            let mut hosts = AssignedSources::default();
            let walk = walk_plans_ordered_with_sources(
                &memory,
                BASE,
                groups,
                &part.visits,
                &part.lists,
                part_walk.walk.outcome,
                &mut plans.rows,
                &mut plans.assigned,
                &mut plans.links,
                &mut plans.names,
                &mut plans.order,
                Some(&mut hosts),
            );
            walks[index] = Some((walk, plans, hosts));
        }
        let walks = walks.map(Option::unwrap);
        let failed: Vec<usize> = (0..2)
            .filter(|&index| matches!(walks[index].0.outcome, Outcome::Unavailable { .. }))
            .collect();
        assert_eq!(failed, [first], "one part records the fault, once");
        let parts: Vec<PlanPart<'_>> = walks
            .iter()
            .zip(&censuses)
            .map(|((walk, plans, hosts), (census, part))| PlanPart {
                walk: *walk,
                rows: &plans.rows,
                assigned: &plans.assigned,
                links: &plans.links,
                names: &plans.names,
                order: &plans.order,
                hosts: Some(hosts),
                orders: &part.rows[..census.walk.visited.min(part.rows.len())],
            })
            .collect();
        let mut plans = PlanStorage::default();
        let mut hosts = AssignedSources::default();
        let walk = stitch_plans(
            &parts,
            census.outcome,
            &mut plans.rows,
            &mut plans.assigned,
            &mut plans.links,
            &mut plans.names,
            &mut plans.order,
            Some(&mut hosts),
        );
        let stitched = Plans::of(walk, &plans, &hosts);
        assert_eq!(stitched.outcome, Outcome::Unavailable { reason }, "{first}");
        let links: usize = walks[..first].iter().map(|(walk, ..)| walk.links).sum();
        assert_eq!(stitched.links.len(), links, "{first}");
        if first == 0 {
            assert_eq!(stitched, serial);
        } else {
            assert_ne!(links, 0);
        }
    }
}

