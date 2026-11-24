use super::*;
use engine_native_parts::{
    carmy_group,
    creference_registry as registry,
    reference_bucket as bucket,
};
use crate::corders_group::Root;
use engine_native_memory::walk::Outcome;
use std::collections::BTreeMap;

const BASE: usize = 0x140000000;
const STATE: usize = 0x1000;
const GROUP: usize = 0x2000;
const LEADER_POINTER: usize = 0x3000;
const FORMATION: usize = 0x5000;
const TEMPLATE: usize = 0x6000;
const GROUP_HANDLE: u64 = (5 << 32) | 53;
const FORMATION_HANDLE: u64 = (7 << 32) | 53;
const LEADER_HANDLE: u64 = (3 << 32) | 4713;
const TEMPLATE_HANDLE: u64 = (4 << 32) | 52;

#[derive(Default)]
struct Image {
    spans: BTreeMap<usize, Vec<u8>>,
    hole: Option<(usize, usize)>,
    reads: std::cell::RefCell<Vec<(usize, usize)>>,
}
impl Memory for Image {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, output.len()));
        if self
            .hole
            .is_some_and(|(start, len)| address < start + len && start < address + output.len())
        {
            return false;
        }
        let Some((&start, bytes)) = self.spans.range(..=address).next_back() else {
            return false;
        };
        let Some(bytes) = bytes.get(address - start..address - start + output.len()) else {
            return false;
        };
        output.copy_from_slice(bytes);
        true
    }
}
impl Image {
    fn bytes(&mut self, address: usize, value: &[u8]) {
        if let Some((&start, bytes)) = self.spans.range_mut(..=address).next_back()
            && let Some(output) = bytes.get_mut(address - start..address - start + value.len())
        {
            output.copy_from_slice(value);
            return;
        }
        self.spans.insert(address, value.to_vec());
    }
    fn u64(&mut self, address: usize, value: u64) {
        self.bytes(address, &value.to_le_bytes());
    }
    fn i32(&mut self, address: usize, value: i32) {
        self.bytes(address, &value.to_le_bytes());
    }
    fn registered(&mut self, root: usize, table: usize, entries: &[(u64, usize)]) {
        self.u64(BASE + root, table as u64);
        let cells = table + 0x100;
        self.bytes(table, &[0; 32]);
        self.u64(table + registry::ENTRIES, cells as u64);
        self.i32(table + registry::COUNT, entries.len() as i32);
        self.i32(table + registry::MASK, 7);
        self.bytes(table + registry::OVERFLOW_DISTANCE, &[5]);
        self.bytes(cells, &[0; 14 * bucket::STRIDE]);
        self.bytes(cells + 13 * bucket::STRIDE + bucket::DISTANCE, &[255]);
        for &(handle, pointer) in entries {
            let hash = handle
                .rotate_left(32)
                .wrapping_mul(registry::HASH_MULTIPLIER);
            let mut index = ((hash >> 32) as u32 ^ hash as u32) & 7;
            let mut distance = 1;
            while self.spans[&cells][index as usize * bucket::STRIDE + bucket::DISTANCE] != 0 {
                index += 1;
                distance += 1;
            }
            let cell = cells + index as usize * bucket::STRIDE;
            self.bytes(cell + bucket::DISTANCE, &[distance]);
            self.u64(cell + bucket::TYPE, handle);
            self.u64(cell + bucket::OBJECT, pointer as u64);
        }
    }
    fn date(&mut self, address: usize, hour: i32) {
        self.bytes(address, &[0; 24]);
        self.u64(address, (BASE + cgame_date::VTABLE_RVA.0) as u64);
        self.i32(address + cgame_date::DATE_BASE, hour);
    }
}

fn fixture() -> Image {
    let mut image = Image::default();
    for (pointer, handle, vtable) in [
        (GROUP, GROUP_HANDLE, corders_group::VTABLE_RVA),
        (FORMATION, FORMATION_HANDLE, carmy_group::VTABLE_RVA),
    ] {
        image.bytes(pointer, &[0; 0x240]);
        image.u64(pointer, (BASE + vtable.0) as u64);
        image.u64(pointer + corders_group::HANDLE, handle);
        image.i32(pointer + corders_group::COUNTRY, 1);
    }
    image.u64(GROUP + LEADER, LEADER_POINTER as u64);
    image.bytes(LEADER_POINTER, &[0; carmy_leader::SIZE]);
    image.u64(LEADER_POINTER, (BASE + carmy_leader::VTABLE_RVA.0) as u64);
    image.u64(LEADER_POINTER + cunit_leader::TYPE, LEADER_HANDLE);
    image.i32(LEADER_POINTER + COOLDOWN_TYPE, 4);
    image.date(LEADER_POINTER + COOLDOWN_END, 1073);
    image.date(STATE + cgame_state::CURRENT_DATE, 1000);
    image.u64(
        LEADER_POINTER + carmy_leader::FORMATION_REFERENCE,
        FORMATION_HANDLE,
    );
    image.bytes(FORMATION + DEPLOYMENT_PENDING, &[1]);
    image.u64(FORMATION + HQ_TEMPLATE, TEMPLATE_HANDLE);
    image.bytes(TEMPLATE, &[0; 0x240]);
    image.u64(
        TEMPLATE,
        (BASE + creferenced_division_template::VTABLE_RVA.0) as u64,
    );
    image.u64(
        TEMPLATE + creferenced_division_template::TYPE,
        TEMPLATE_HANDLE,
    );
    image.i32(TEMPLATE + TEMPLATE_MANPOWER, 1000);
    image.i32(BASE + DEPLOY_DAYS_RVA.0, 10);
    image.i32(BASE + DEPLOY_MIN_RVA.0, 3);
    image.u64(BASE + REFERENCE_MANPOWER_RVA.0, 100_000_000);
    image.u64(BASE + MANPOWER_EXPONENT_RVA.0, 100_000);
    image.registered(
        registry::HIGH_ROOT_RVA.0,
        0x10000,
        &[(LEADER_HANDLE, LEADER_POINTER)],
    );
    image.registered(
        registry::LOW_ROOTS_RVA.0 + 53 * 8,
        0x11000,
        &[(GROUP_HANDLE, GROUP), (FORMATION_HANDLE, FORMATION)],
    );
    image.registered(
        registry::LOW_ROOTS_RVA.0 + 52 * 8,
        0x12000,
        &[(TEMPLATE_HANDLE, TEMPLATE)],
    );
    image
}

#[test]
fn cooldown_follows_leaders_formation_template_and_current_date() {
    let mut image = fixture();
    assert_eq!(leader_enroute_days(&image, BASE, STATE, GROUP), Ok(14));
    image.u64(GROUP + HQ_TEMPLATE, 0);
    assert_eq!(leader_enroute_days(&image, BASE, STATE, GROUP), Ok(14));
    image.i32(TEMPLATE + TEMPLATE_MANPOWER, 100);
    assert_eq!(leader_enroute_days(&image, BASE, STATE, GROUP), Ok(7));
    image.bytes(FORMATION + DEPLOYMENT_PENDING, &[0]);
    assert_eq!(leader_enroute_days(&image, BASE, STATE, GROUP), Ok(4));
    image.date(LEADER_POINTER + COOLDOWN_END, 953);
    assert_eq!(leader_enroute_days(&image, BASE, STATE, GROUP), Ok(-1));
    image.i32(LEADER_POINTER + COOLDOWN_TYPE, 0);
    image.hole = Some((STATE + cgame_state::CURRENT_DATE, 24));
    assert_eq!(leader_enroute_days(&image, BASE, STATE, GROUP), Ok(0));
    image.u64(GROUP + LEADER, 0);
    image.hole = Some((LEADER_POINTER, carmy_leader::SIZE));
    assert_eq!(leader_enroute_days(&image, BASE, STATE, GROUP), Ok(0));
}

#[test]
fn suffix_faults_preserve_every_group_prefix_and_independent_fields() {
    let mut image = fixture();
    image.bytes(GROUP + corders_group::WITHDRAWING, &[255]);
    image.i32(GROUP + corders_group::HQ_DISTANCE, -7);
    image.u64(GROUP + corders_group::BORDER_WAR, u64::MAX);
    let roots = [
        Root {
            pointer: GROUP,
            handle: GROUP_HANDLE,
            country: 1,
        },
        Root {
            pointer: FORMATION,
            handle: FORMATION_HANDLE,
            country: 1,
        },
    ];
    let mut output = [corders_group::CounterInputsRow::default(); 2];
    for (hole, cooldown, border, distance) in [
        (None, Some(14), Some(1), Some(-7)),
        (
            Some((LEADER_POINTER + COOLDOWN_END, 24)),
            None,
            Some(1),
            Some(-7),
        ),
        (
            Some((GROUP + corders_group::BORDER_WAR, 8)),
            Some(14),
            None,
            Some(-7),
        ),
        (
            Some((GROUP + corders_group::HQ_DISTANCE, 4)),
            Some(14),
            Some(1),
            None,
        ),
        (
            Some((TEMPLATE + TEMPLATE_MANPOWER, 4)),
            None,
            Some(1),
            Some(-7),
        ),
    ] {
        image.hole = hole;
        image.reads.borrow_mut().clear();
        assert_eq!(
            corders_group::walk_counter_inputs_in_state(
                &image,
                BASE,
                STATE,
                &roots,
                Outcome::Rows { len: 2 },
                &mut output
            ),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(
            (output[0].group, output[0].withdrawing),
            (GROUP_HANDLE, 255)
        );
        assert_eq!(
            (
                output[0].leader_enroute_days,
                output[0].border_war_presence,
                output[0].hq_distance
            ),
            (cooldown, border, distance)
        );
        assert_eq!(output[1].group, FORMATION_HANDLE);
        assert_eq!(output[1].leader_enroute_days, Some(0));
        let reads = image.reads.borrow();
        let prefix_end = reads
            .iter()
            .position(|row| *row == (FORMATION + corders_group::WITHDRAWING, 1))
            .unwrap();
        let suffix_start = reads
            .iter()
            .position(|&(at, len)| {
                at <= GROUP + corders_group::HQ_DISTANCE
                    && GROUP + corders_group::HQ_DISTANCE < at + len
            })
            .unwrap();
        assert!(prefix_end < suffix_start);
        if hole.is_none() {
            for field in [
                (GROUP + corders_group::COUNTRY, 4),
                (GROUP + corders_group::WITHDRAWING, 1),
                (GROUP + LEADER, 8),
                (GROUP + corders_group::HQ_DISTANCE, 4),
                (GROUP + corders_group::BORDER_WAR, 8),
                (GROUP + corders_group::INVASION_SOURCE_DISTANCE, 4),
            ] {
                let record_field =
                    field.0 == GROUP + corders_group::WITHDRAWING || field.0 == GROUP + LEADER;
                assert_eq!(
                    reads.iter().filter(|&&read| read == field).count(),
                    usize::from(record_field),
                    "{field:x?}"
                );
            }
        }
    }
}

#[test]
fn pending_deployment_uses_native_zero_and_minimum_branches() {
    let mut image = fixture();
    assert_eq!(deployment_days(&image, BASE, -1), Ok(10));
    image.i32(BASE + DEPLOY_DAYS_RVA.0, 5);
    image.i32(BASE + DEPLOY_MIN_RVA.0, -1);
    image.u64(BASE + MANPOWER_EXPONENT_RVA.0, 0);
    assert_eq!(deployment_days(&image, BASE, 1), Ok(5));
    image.i32(BASE + DEPLOY_MIN_RVA.0, 7);
    assert_eq!(deployment_days(&image, BASE, 1000), Ok(7));
    image.i32(BASE + DEPLOY_DAYS_RVA.0, 0);
    image.hole = Some((BASE + REFERENCE_MANPOWER_RVA.0, 8));
    assert_eq!(deployment_days(&image, BASE, 500), Ok(0));
}

