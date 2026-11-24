use super::*;
use std::{cell::RefCell, collections::BTreeMap};

const BASE: usize = 0x1000_0000;
const STATE: usize = 0x10000;
const PROVINCES: usize = 0x20000;
const PROVINCE: usize = 0x30000;
const AREA: usize = 0x40000;
const ORDER: usize = 0x50000;
const GROUP: usize = 0x60000;
const ENTRIES: usize = 0x70000;
const HANDLE: u64 = (7 << 32) | corders_group::REFERENCE_TYPE as u64;

#[derive(Default)]
struct Image {
    bytes: BTreeMap<usize, u8>,
    reads: RefCell<Vec<(usize, usize)>>,
}
impl Memory for Image {
    fn copy(&self, at: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((at, output.len()));
        for (offset, byte) in output.iter_mut().enumerate() {
            let Some(value) = self.bytes.get(&(at + offset)) else {
                return false;
            };
            *byte = *value;
        }
        true
    }
}
impl Image {
    fn put(&mut self, at: usize, bytes: &[u8]) {
        self.bytes
            .extend(bytes.iter().enumerate().map(|(i, &b)| (at + i, b)));
    }
    fn u64(&mut self, at: usize, value: u64) {
        self.put(at, &value.to_le_bytes());
    }
    fn u32(&mut self, at: usize, value: u32) {
        self.put(at, &value.to_le_bytes());
    }
    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.u64(at, data as u64);
        self.u32(at + 8, count);
        self.u32(at + 12, count);
    }
    fn entry(&mut self, index: usize, start: i32, end: i32) {
        let at = ENTRIES + index * STRIDE;
        self.u64(at, (BASE + ENTRY_VTABLE_RVA.0) as u64);
        self.u64(at + 8, GROUP as u64);
        self.u32(at + 0x20, start as u32);
        self.u32(at + 0x24, end as u32);
    }
}
fn fixture(area: i32, count: u32) -> Image {
    let mut image = Image::default();
    image.array(STATE + crate::cgame_state::PROVINCES, PROVINCES, 2);
    image.u64(PROVINCES + 8, PROVINCE as u64);
    image.u64(PROVINCE, (BASE + cprovince::VTABLE_RVA.0) as u64);
    image.u32(PROVINCE + cprovince::ID, 1);
    image.u64(PROVINCE + cprovince::CONTROLLER_AREA, AREA as u64);
    image.u64(AREA, (BASE + ccontroller_area::VTABLE_RVA.0) as u64);
    image.u32(AREA + ccontroller_area::PROVINCE_COUNT, area as u32);
    image.array(ORDER + DATA, ENTRIES, count);
    image.put(GROUP + corders_group::IS_ARMY_GROUP, &[1]);
    image.array(GROUP + carmy_group::MEMBER_DATA, ENTRIES, count);
    image
}
fn capture(image: &Image, path_len: usize) -> (Walk, PlanRow, Vec<Section>) {
    capture_from(image, path_len, Some(image))
}
fn capture_from(
    image: &Image,
    path_len: usize,
    group_source: Option<&Image>,
) -> (Walk, PlanRow, Vec<Section>) {
    let roots = [corders_group::Root {
        pointer: GROUP,
        handle: HANDLE,
        country: 1,
    }];
    let visits = [Visit {
        pointer: ORDER,
        owner: 0,
        instance: 4,
        ..Default::default()
    }];
    let orders = [Row {
        owner_group: HANDLE,
        instance: 4,
        order_type: TYPE_LINE,
        path_len,
        ..Default::default()
    }];
    let mut plans = [PlanRow {
        owner_group: HANDLE,
        instance: 4,
        front_valid: 1,
        ..Default::default()
    }];
    let mut sections = Vec::new();
    let result = walk(
        image,
        BASE,
        STATE,
        &roots,
        &visits,
        &orders,
        &[1],
        Outcome::Rows { len: 1 },
        &mut plans,
        &mut sections,
        &mut Scratch::default(),
        group_source,
    );
    (result, plans[0], sections)
}

#[test]
fn small_area_and_empty_path_do_not_reach_child_storage() {
    let image = fixture(2, i32::MAX as u32);
    let (walk, plan, _) = capture(&image, 1);
    assert_eq!(walk.header_failures + walk.section_failures, 0);
    assert_eq!(
        plan.child_fronts,
        Some(Inputs {
            area_province_count: 2,
            ..Default::default()
        })
    );
    assert!(
        !image
            .reads
            .borrow()
            .iter()
            .any(|&(at, _)| at == ORDER + DATA)
    );
    image.reads.borrow_mut().clear();
    let (_, plan, _) = capture(&image, 0);
    assert_eq!(plan.child_fronts, None);
    assert!(image.reads.borrow().is_empty());
}

#[test]
fn count_survives_unreadable_huge_payload_without_arena_growth() {
    let image = fixture(3, i32::MAX as u32);
    let (walk, plan, sections) = capture(&image, 1);
    assert_eq!(walk.section_failures, 1);
    assert_eq!(plan.child_fronts.unwrap().count, Some(i32::MAX as u32));
    assert_eq!(plan.child_fronts.unwrap().sections, None);
    assert_eq!(sections.capacity(), 0);
}

#[test]
fn payload_failure_rolls_back_completed_batches_preserving_count() {
    let mut image = fixture(3, 17);
    for index in 0..16 {
        image.entry(index, index as i32, index as i32 + 1);
    }
    let (walk, plan, sections) = capture(&image, 1);
    assert_eq!(walk.sections, 0);
    assert_eq!(walk.section_failures, 1);
    assert_eq!(plan.child_fronts.unwrap().count, Some(17));
    assert_eq!(plan.child_fronts.unwrap().sections, None);
    assert_eq!(sections.len(), 16);
}

#[test]
fn demanded_prefix_never_reads_unreachable_tail_but_keeps_prefix_faults() {
    for group_count in [1, 2] {
        let mut image = fixture(3, 17);
        image.u32(GROUP + carmy_group::MEMBER_COUNT, group_count);
        image.entry(0, 0, 0);
        let (walk, plan, sections) = capture(&image, 1);
        let inputs = plan.child_fronts.unwrap();
        assert_eq!(inputs.count, Some(17));
        assert_eq!(inputs.group_child_count, Some(group_count as i32));
        assert_eq!(walk.section_failures, usize::from(group_count == 2));
        assert_eq!(walk.sections, usize::from(group_count == 1));
        assert_eq!(inputs.sections, (group_count == 1).then_some((0, 1)));
        if group_count == 1 {
            assert_eq!(sections[0].group, HANDLE);
        }
        assert!(!image.reads.borrow().iter().any(|&(at, len)| {
            let tail = ENTRIES + group_count as usize * STRIDE;
            at < ENTRIES + 17 * STRIDE && at + len > tail
        }));
    }
}

#[test]
fn slots_past_stored_count_do_not_extend_capture_or_bypass_null_authentication() {
    for null in [false, true] {
        let mut image = fixture(3, 2);
        image.u32(GROUP + carmy_group::MEMBER_COUNT, i32::MAX as u32);
        image.entry(0, 0, 0);
        image.entry(1, 1, 1);
        if null {
            image.u64(ENTRIES + STRIDE + 8, 0);
        }
        let (walk, plan, _) = capture(&image, 1);
        assert_eq!(walk.section_failures, usize::from(null));
        assert_eq!(plan.child_fronts.unwrap().count, Some(2));
        assert_eq!(plan.child_fronts.unwrap().group_child_count, Some(i32::MAX));
        assert_eq!(
            plan.child_fronts.unwrap().sections,
            (!null).then_some((0, 2))
        );
        assert!(
            !image
                .reads
                .borrow()
                .iter()
                .any(|&(at, len)| { at < ENTRIES + 3 * STRIDE && at + len > ENTRIES + 2 * STRIDE })
        );
    }
}

#[test]
fn duplicate_groups_signed_spans_and_order_remain_exact() {
    let mut image = fixture(3, 2);
    image.entry(0, -1, -1);
    image.entry(1, 3, 5);
    let (walk, plan, sections) = capture(&image, 1);
    assert_eq!(walk.sections, 2);
    assert_eq!(plan.child_fronts.unwrap().sections, Some((0, 2)));
    assert_eq!(
        sections,
        vec![
            Section {
                group: HANDLE,
                segment_start: -1,
                segment_end: -1
            },
            Section {
                group: HANDLE,
                segment_start: 3,
                segment_end: 5
            }
        ]
    );
    assert!(
        !image
            .reads
            .borrow()
            .iter()
            .any(|&(at, _)| at == GROUP || at == ORDER + super::super::PATH_DATA)
    );
}

#[test]
fn zero_and_refused_current_group_count_preserve_root_scalar_without_reading_spans() {
    let mut image = fixture(3, 2);
    image.u32(GROUP + carmy_group::MEMBER_COUNT, 0);
    let (walk, plan, sections) = capture(&image, 1);
    let inputs = plan.child_fronts.unwrap();
    assert_eq!(
        (inputs.count, inputs.group_child_count, inputs.sections),
        (Some(2), Some(0), None)
    );
    assert_eq!(walk.section_failures, 0);
    assert!(sections.is_empty());
    assert!(!image.reads.borrow().iter().any(|&(at, _)| at == ENTRIES));
    for offset in 0..4 {
        image
            .bytes
            .remove(&(GROUP + carmy_group::MEMBER_COUNT + offset));
    }
    image.reads.borrow_mut().clear();
    let (walk, plan, sections) = capture(&image, 1);
    let inputs = plan.child_fronts.unwrap();
    assert_eq!(
        (inputs.count, inputs.group_child_count, inputs.sections),
        (Some(2), None, None)
    );
    assert_eq!(walk.section_failures, 1);
    assert_eq!(sections.capacity(), 0);
    assert!(!image.reads.borrow().iter().any(|&(at, _)| at == ENTRIES));
}

#[test]
fn empty_stored_array_never_reaches_owner_kind_or_current_group_count() {
    let image = fixture(3, 0);
    let (walk, plan, sections) = capture(&image, 1);
    let inputs = plan.child_fronts.unwrap();
    assert_eq!(
        (inputs.count, inputs.group_child_count, inputs.sections),
        (Some(0), None, Some((0, 0)))
    );
    assert_eq!(walk.section_failures, 0);
    assert!(sections.is_empty());
    assert!(
        !image
            .reads
            .borrow()
            .iter()
            .any(|&(at, _)| at == GROUP + carmy_group::MEMBER_DATA
                || at == GROUP + corders_group::IS_ARMY_GROUP)
    );
}

#[test]
fn negative_group_count_keeps_its_signed_bits_without_header_sized_growth() {
    let mut image = fixture(3, 2);
    image.u32(GROUP + carmy_group::MEMBER_COUNT, i32::MIN as u32);
    let (walk, plan, sections) = capture(&image, 1);
    let inputs = plan.child_fronts.unwrap();
    assert_eq!(
        (inputs.count, inputs.group_child_count, inputs.sections),
        (Some(2), Some(i32::MIN), None)
    );
    assert_eq!(walk.section_failures, 1);
    assert_eq!(sections.capacity(), 0);
}

#[test]
fn owned_group_source_faults_never_retry_healthy_live_operands() {
    for fault in 0..4 {
        let mut live = fixture(3, 1);
        live.entry(0, 0, 0);
        live.u32(GROUP + carmy_group::MEMBER_COUNT, 99);
        let mut owned = fixture(3, 1);
        owned.u32(GROUP + carmy_group::MEMBER_COUNT, 7);
        match fault {
            1 => {
                owned.bytes.remove(&(GROUP + corders_group::IS_ARMY_GROUP));
            }
            2 => {
                owned.bytes.remove(&(GROUP + carmy_group::MEMBER_DATA));
            }
            _ => {}
        }
        let source = (fault != 3).then_some(&owned);
        let (walk, plan, sections) = capture_from(&live, 1, source);
        let inputs = plan.child_fronts.unwrap();
        assert_eq!(inputs.count, Some(1));
        assert_eq!(inputs.group_child_count, (fault == 0).then_some(7));
        assert_eq!(walk.section_failures, usize::from(fault != 0));
        assert_eq!(walk.sections, usize::from(fault == 0));
        if fault != 0 {
            assert_eq!(sections.capacity(), 0);
        }
        assert!(!live.reads.borrow().iter().any(|&(at, _)| {
            at == GROUP + corders_group::IS_ARMY_GROUP
                || at == GROUP + carmy_group::MEMBER_DATA
                || at == GROUP + carmy_group::MEMBER_COUNT
        }));
        assert!(
            !owned
                .reads
                .borrow()
                .iter()
                .any(|&(at, _)| { at == GROUP + carmy_group::MEMBER_COUNT })
        );
    }
}

fn three_orders() -> (Image, [Visit; 3], [Row; 3], [PlanRow; 3]) {
    let mut image = fixture(3, 2);
    image.entry(0, 0, 1);
    image.entry(1, 2, 3);
    image.array(ORDER + 0x1000 + DATA, 0, 0);
    image.array(ORDER + 0x2000 + DATA, ENTRIES + 2 * STRIDE, 1);
    image.entry(2, 7, 9);
    let visits = std::array::from_fn(|index| Visit {
        pointer: ORDER + index * 0x1000,
        owner: 0,
        instance: 4 + index as u32,
        ..Default::default()
    });
    let orders = std::array::from_fn(|index| Row {
        owner_group: HANDLE,
        instance: 4 + index as u32,
        order_type: TYPE_LINE,
        path_len: 1,
        ..Default::default()
    });
    let plans = std::array::from_fn(|index| PlanRow {
        owner_group: HANDLE,
        instance: 4 + index as u32,
        front_valid: 1,
        ..Default::default()
    });
    (image, visits, orders, plans)
}

#[test]
fn any_placement_gives_the_searched_walk() {
    let roots = [corders_group::Root {
        pointer: GROUP,
        handle: HANDLE,
        country: 1,
    }];
    let (image, mut visits, orders, _) = three_orders();
    visits.reverse();
    let run = |placement: &[u32]| {
        let (_, _, _, mut plans) = three_orders();
        let mut scratch = Scratch::default();
        let areas = walk_areas(
            &image,
            BASE,
            STATE,
            &roots,
            &visits,
            &orders,
            &[1],
            Outcome::Rows { len: 3 },
            &mut plans,
            placement,
            &[],
            &mut scratch,
        );
        let mut sections = Vec::new();
        let report = walk_groups(
            &image,
            BASE,
            &roots,
            &mut plans,
            &mut sections,
            &mut scratch,
            Some(&image),
            areas,
        );
        (areas, report, plans, sections)
    };
    let searched = run(&[]);
    assert_eq!(searched.0.area_failures, 0);
    assert!(searched.2.iter().all(|plan| plan.child_fronts.is_some()));
    for placement in [&[2, 1, 0][..], &[0, 1, 2], &[1], &[9, 9, 9]] {
        assert_eq!(run(placement), searched, "{placement:?}");
    }
}

#[test]
fn a_shared_one_shot_area_fault_is_met_by_the_first_order_in_census_order() {
    struct OneShot<'a> {
        image: &'a Image,
        spent: std::cell::Cell<bool>,
    }
    impl Memory for OneShot<'_> {
        fn copy(&self, at: usize, output: &mut [u8]) -> bool {
            if at <= AREA && AREA < at + output.len() && !self.spent.replace(true) {
                return false;
            }
            self.image.copy(at, output)
        }
    }
    let roots = [corders_group::Root {
        pointer: GROUP,
        handle: HANDLE,
        country: 1,
    }];
    let (image, visits, orders, plans) = three_orders();
    let memory = OneShot {
        image: &image,
        spent: Default::default(),
    };
    let table = [visits[1], visits[0]];
    let mut plans = [plans[0], plans[1]];
    let areas = walk_areas(
        &memory,
        BASE,
        STATE,
        &roots,
        &table,
        &orders[..2],
        &[1],
        Outcome::Rows { len: 2 },
        &mut plans,
        &[],
        &[1, 0],
        &mut Scratch::default(),
    );
    assert_eq!(areas.area_failures, 1);
    assert_eq!(plans[0].child_fronts, None);
    assert_eq!(
        plans[1]
            .child_fronts
            .map(|inputs| inputs.area_province_count),
        Some(3)
    );
}

#[test]
fn a_refused_area_phase_leaves_no_earlier_pass_for_the_group_phase() {
    let roots = [corders_group::Root {
        pointer: GROUP,
        handle: HANDLE,
        country: 1,
    }];
    let (image, visits, orders, mut plans) = three_orders();
    let mut scratch = Scratch::default();
    walk_areas(
        &image,
        BASE,
        STATE,
        &roots,
        &visits,
        &orders,
        &[1],
        Outcome::Rows { len: 3 },
        &mut plans,
        &[],
        &[],
        &mut scratch,
    );
    assert!(scratch.needs_group_source());
    let refused = walk_areas(
        &image,
        BASE,
        STATE,
        &roots,
        &visits,
        &orders,
        &[1],
        Outcome::Unavailable { reason: "census" },
        &mut plans,
        &[],
        &[],
        &mut scratch,
    );
    assert_eq!(refused.area_failures, 3);
    assert!(!scratch.needs_group_source());
    let mut sections = Vec::new();
    let report = walk_groups(
        &image,
        BASE,
        &roots,
        &mut plans,
        &mut sections,
        &mut scratch,
        Some(&image),
        refused,
    );
    assert_eq!(report, refused);
    assert!(sections.is_empty());
    assert!(plans.iter().all(|plan| plan.child_fronts.is_none()));
}

