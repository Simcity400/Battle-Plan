use super::*;
use std::{cell::RefCell, collections::BTreeMap};

const GAME: usize = 0x1000;
const COUNTRY1: usize = 0x10000;
const COUNTRY2: usize = 0x14000;
const UNIT: usize = 0x20000;
const PROVINCE: usize = 0x30000;
const STATE: usize = 0x40000;
const BUCKET: usize = 0x50000;
const VALUES: usize = 0x60000;
const DEFINITIONS: usize = 0x70000;

#[derive(Default)]
struct Image {
    bytes: BTreeMap<usize, u8>,
    reads: RefCell<Vec<(usize, usize)>>,
    fault: Option<std::ops::Range<usize>>,
}
impl Memory for Image {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, destination.len()));
        let Some(end) = address.checked_add(destination.len()) else {
            return false;
        };
        if self
            .fault
            .as_ref()
            .is_some_and(|r| address < r.end && end > r.start)
        {
            return false;
        }
        for (index, byte) in destination.iter_mut().enumerate() {
            let Some(value) = self.bytes.get(&(address + index)) else {
                return false;
            };
            *byte = *value;
        }
        true
    }
}
impl Image {
    fn put(&mut self, address: usize, bytes: &[u8]) {
        for (index, &byte) in bytes.iter().enumerate() {
            self.bytes.insert(address + index, byte);
        }
    }
    fn u32(&mut self, at: usize, value: u32) {
        self.put(at, &value.to_le_bytes());
    }
    fn u64(&mut self, at: usize, value: u64) {
        self.put(at, &value.to_le_bytes());
    }
    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.u64(at, data as u64);
        self.u32(at + 8, count);
        self.u32(at + 12, count);
    }
    fn accumulator(&mut self, at: usize, data: usize, entries: &[(u32, i64)]) {
        self.u64(at, data as u64);
        self.u32(at + 12, entries.len() as u32);
        for (index, &(key, raw)) in entries.iter().enumerate() {
            self.u32(data + index * 16, key);
            self.u64(data + index * 16 + 8, raw as u64);
        }
    }
    fn definition(&mut self, key: u32, flags: u32, selector: u32) {
        self.u64(engine_native_parts::modifier_definition::TABLE_RVA.0, DEFINITIONS as u64);
        let row = DEFINITIONS + key as usize * engine_native_parts::modifier_definition::STRIDE;
        self.u32(row + engine_native_parts::modifier_definition::FLAGS, flags);
        self.u32(row + engine_native_parts::modifier_definition::FEATURE_SELECTOR, selector);
    }
    fn touched(&self, address: usize) -> bool {
        self.reads.borrow().iter().any(|&(at, _)| at == address)
    }
}

fn fixture() -> Image {
    let mut image = Image::default();
    image.array(GAME + cgame_state::COUNTRIES, 0x8000, 3);
    image.array(GAME + cgame_state::COUNTRY_LINKS, 0x9000, 3);
    for (slot, pointer) in [(1, COUNTRY1), (2, COUNTRY2)] {
        image.u64(0x8000 + slot * 8, pointer as u64);
        image.u32(0x9000 + slot * 4, slot as u32);
        image.u64(pointer, ccountry::VTABLE_RVA.0 as u64);
        image.u32(pointer + ccountry::ID, slot as u32);
        image.accumulator(pointer + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR, 0, &[]);
    }
    image.u32(UNIT + cunit::ROSTER_OWNER, 1);
    image.u32(UNIT + cunit::EXPEDITIONARY_OWNER, 2);
    image.u64(UNIT + cunit::LOCATION, PROVINCE as u64);
    image.u64(PROVINCE, cprovince::VTABLE_RVA.0 as u64);
    image.u64(PROVINCE + cprovince::STATE, STATE as u64);
    image.u64(STATE, crate::cstate::VTABLE_RVA.0 as u64);
    image.u32(STATE + crate::cstate::CONTROLLER, 2);
    image.put(STATE + crate::cstate::COUNTRY_MODIFIERS, &[0; 17]);
    image.u64(STATE + crate::cstate::COUNTRY_MODIFIERS, BUCKET as u64);
    image.put(STATE + crate::cstate::COUNTRY_MODIFIERS + 16, &[1]);
    image.put(BUCKET + crate::cstate::COUNTRY_MODIFIER_STRIDE + 4, &[0]);
    image.put(BUCKET + 4, &[0]);
    image.accumulator(
        STATE + crate::cstate::BASE_MODIFIERS,
        VALUES,
        &[
            (keys::DISABLE_STRATEGIC_REDEPLOYMENT, 0),
            (keys::DISABLE_STRATEGIC_REDEPLOYMENT_FOR_CONTROLLER, 17),
        ],
    );
    for key in [
        keys::DISABLE_STRATEGIC_REDEPLOYMENT,
        keys::DISABLE_STRATEGIC_REDEPLOYMENT_FOR_CONTROLLER,
    ] {
        image.definition(key, 0, 0);
    }
    image
}
fn read_action(
    image: &Image,
) -> Result<Option<StrategicRedeploymentInputs>, StrategicActionFailure> {
    match capture(image, 0, GAME, UNIT, 1, 0, 0, 1, None) {
        StrategicAction::Unreached => Ok(None),
        StrategicAction::Available(inputs) => Ok(Some(inputs)),
        StrategicAction::Withheld(reason) => Err(reason),
    }
}

#[test]
fn strategic_action_state_override_uses_roster_and_base_fallback() {
    let mut image = fixture();
    let base = read_action(&image).unwrap().unwrap();
    assert_eq!(
        base,
        StrategicRedeploymentInputs {
            country_modifier: 0,
            state_modifier: Some(0),
            friendly_state_modifier: Some(17),
        }
    );
    image.put(BUCKET + 4, &[1]);
    image.u32(BUCKET + 8, 1);
    image.accumulator(
        BUCKET + 0x20,
        VALUES + 0x100,
        &[(keys::DISABLE_STRATEGIC_REDEPLOYMENT, -9)],
    );
    image.fault =
        Some(STATE + crate::cstate::BASE_MODIFIERS..STATE + crate::cstate::BASE_MODIFIERS + 16);
    image.reads.borrow_mut().clear();
    let selected = read_action(&image).unwrap().unwrap();
    assert_eq!(selected.state_modifier, Some(-9));
    assert!(!image.touched(STATE + crate::cstate::CONTROLLER));
    image.u32(BUCKET + 8, 2);
    image.fault = None;
    assert_eq!(read_action(&image).unwrap(), Some(base));
}

#[test]
fn strategic_action_country_and_missing_state_skip_later_branches() {
    let mut image = fixture();
    image.accumulator(
        COUNTRY2 + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR,
        VALUES + 0x200,
        &[(keys::DISABLE_STRATEGIC_REDEPLOYMENT, i64::MIN)],
    );
    image.fault = Some(UNIT + cunit::LOCATION..UNIT + cunit::LOCATION + 8);
    assert_eq!(
        read_action(&image).unwrap(),
        Some(StrategicRedeploymentInputs {
            country_modifier: i64::MIN,
            ..Default::default()
        })
    );
    assert!(!image.touched(UNIT + cunit::LOCATION));
    image.accumulator(COUNTRY2 + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR, 0, &[]);
    image.fault = None;
    image.u64(PROVINCE + cprovince::STATE, 0);
    assert_eq!(
        read_action(&image).unwrap(),
        Some(StrategicRedeploymentInputs {
            country_modifier: 0,
            ..Default::default()
        })
    );
}

#[test]
fn strategic_action_friendly_gate_skips_controller_modifier_in_active_war() {
    let mut image = fixture();
    image.u32(STATE + crate::cstate::CONTROLLER, 1);
    const DIPLOMACY: usize = 0x90000;
    const RELATIONS: usize = 0x91000;
    const TUPLE: usize = 0x92000;
    const WAR: usize = 0x93000;
    image.u64(COUNTRY2 + ccountry::DIPLOMACY_STATUS, DIPLOMACY as u64);
    image.u64(DIPLOMACY, engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0 as u64);
    image.u64(
        DIPLOMACY + engine_native_parts::cdiplomacy_status_layout::COUNTRY,
        COUNTRY2 as u64,
    );
    image.array(
        DIPLOMACY + engine_native_parts::cdiplomacy_status_layout::RELATIONS,
        RELATIONS,
        3,
    );
    image.u64(RELATIONS + 8, TUPLE as u64);
    image.u64(TUPLE, crate::crelation::status::VTABLE_RVA.0 as u64);
    image.u64(
        TUPLE + crate::crelation::status::DIPLOMACY_STATUS,
        DIPLOMACY as u64,
    );
    image.u32(TUPLE + crate::crelation::status::TARGET_COUNTRY, 1);
    image.u64(TUPLE + crate::crelation::status::WAR_RELATION, WAR as u64);
    image.u64(WAR, crate::cwar_relation::VTABLE_RVA.0 as u64);
    image.u32(WAR + crate::crelation::TYPE_ID, crate::cwar_relation::TYPE);
    image.put(WAR + crate::crelation::RETIRED, &[0]);
    image.fault = Some(VALUES + 24..VALUES + 32);
    let inputs = read_action(&image).unwrap().unwrap();
    assert_eq!(inputs.friendly_state_modifier, None);
    assert!(!image.touched(VALUES + 24));
}

#[test]
fn strategic_action_unreached_branch_costs_no_reads() {
    let image = Image::default();
    for (count, target, retreat, strategic) in [
        (0, 0, 0, 1),
        (u32::MAX, 0, 0, 1),
        (1, 1, 0, 1),
        (1, 0, 1, 1),
        (1, 0, 0, 0),
    ] {
        assert_eq!(
            capture(
                &image, 0, GAME, UNIT, count, target, retreat, strategic, None
            ),
            StrategicAction::Unreached
        );
    }
    assert!(image.reads.borrow().is_empty());
}

#[test]
fn strategic_action_effective_getter_probes_without_capacity_or_suffix_reads() {
    const ACC: usize = 0x1000;
    let mut image = Image::default();
    image.accumulator(ACC, VALUES, &[(1, 3), (2, i64::MIN), (3, 4), (4, 5)]);
    image.definition(2, 0, 0);
    image.fault = Some(VALUES + 48..VALUES + 64);
    assert_eq!(
        engine_native_parts::modifier_accumulator::requested_effective_at(&image, 0, ACC, 2),
        Ok(i64::MIN)
    );
    let keys = image
        .reads
        .borrow()
        .iter()
        .filter_map(|&(at, len)| (len == 4 && (VALUES..VALUES + 64).contains(&at)).then_some(at))
        .collect::<Vec<_>>();
    assert_eq!(keys, [VALUES + 32, VALUES + 16, VALUES, VALUES + 16]);
    assert!(!image.touched(ACC + 8));
    assert!(!image.touched(VALUES + 8));
    assert!(!image.touched(VALUES + 40));
}

#[test]
fn strategic_action_effective_getter_feature_before_raw_and_native_clamp() {
    use engine_native_parts::modifier_definition as definitions;
    const ACC: usize = 0x1000;
    let mut image = Image::default();
    image.accumulator(ACC, VALUES, &[(2, i64::MAX)]);
    image.definition(2, definitions::CLAMP_MAXIMUM, 4);
    image.u64(definitions::ELIGIBILITY_CALLBACK_RVA.0, 0);
    image.fault = Some(VALUES + 8..VALUES + 16);
    assert_eq!(
        engine_native_parts::modifier_accumulator::requested_effective_at(&image, 0, ACC, 2),
        Ok(0)
    );
    assert!(!image.touched(VALUES + 8));
    assert!(!image.touched(DEFINITIONS + 2 * definitions::STRIDE + definitions::FLAGS));
    image.u64(
        definitions::ELIGIBILITY_CALLBACK_RVA.0,
        definitions::ELIGIBILITY_FLAGS_GETTER_RVA.0 as u64,
    );
    image.u32(definitions::TRADE_FEATURE_FLAGS_RVA.0, 4);
    image.fault = None;
    assert_eq!(
        engine_native_parts::modifier_accumulator::requested_effective_at(&image, 0, ACC, 2),
        Ok(100000)
    );
    image.u64(VALUES + 8, i64::MIN as u64);
    assert_eq!(
        engine_native_parts::modifier_accumulator::requested_effective_at(&image, 0, ACC, 2),
        Ok(0)
    );
    image.definition(2, 0, 4);
    assert_eq!(
        engine_native_parts::modifier_accumulator::requested_effective_at(&image, 0, ACC, 2),
        Ok(i64::MIN)
    );
    image.u64(definitions::ELIGIBILITY_CALLBACK_RVA.0, 77);
    assert_eq!(
        engine_native_parts::modifier_accumulator::requested_effective_at(&image, 0, ACC, 2),
        Err("modifier eligibility callback unsupported")
    );
}

#[test]
fn strategic_action_absent_empty_and_negative_count_are_distinct() {
    const ACC: usize = 0x1000;
    let mut image = Image::default();
    image.accumulator(ACC, VALUES, &[(3, 19)]);
    assert_eq!(
        engine_native_parts::modifier_accumulator::requested_effective_at(&image, 0, ACC, 2),
        Ok(0)
    );
    assert!(!image.touched(VALUES + 8));
    assert!(!image.touched(engine_native_parts::modifier_definition::TABLE_RVA.0));
    image.u32(ACC + 12, 0);
    assert_eq!(
        engine_native_parts::modifier_accumulator::requested_effective_at(&image, 0, ACC, 2),
        Ok(0)
    );
    image.u32(ACC + 12, u32::MAX);
    assert_eq!(
        engine_native_parts::modifier_accumulator::requested_effective_at(&image, 0, ACC, 2),
        Err("strategic action modifier count negative")
    );
}

#[test]
fn strategic_action_country_ownership_uses_signed_positive_expeditionary() {
    let mut image = fixture();
    image.accumulator(
        COUNTRY1 + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR,
        VALUES + 0x200,
        &[(keys::DISABLE_STRATEGIC_REDEPLOYMENT, 21)],
    );
    image.u32(UNIT + cunit::EXPEDITIONARY_OWNER, u32::MAX);
    assert_eq!(
        read_action(&image).unwrap(),
        Some(StrategicRedeploymentInputs {
            country_modifier: 21,
            ..Default::default()
        })
    );
    image.u32(UNIT + cunit::EXPEDITIONARY_OWNER, 0);
    assert_eq!(read_action(&image).unwrap().unwrap().country_modifier, 21);
}

#[test]
fn strategic_action_roster_alias_selects_canonical_state_override() {
    let mut image = fixture();
    image.array(GAME + cgame_state::COUNTRIES, 0x8000, 4);
    image.array(GAME + cgame_state::COUNTRY_LINKS, 0x9000, 4);
    image.u64(0x8000 + 3 * 8, COUNTRY1 as u64);
    image.u32(0x9000 + 3 * 4, 1);
    image.u32(UNIT + cunit::ROSTER_OWNER, 3);
    image.put(BUCKET + 4, &[1]);
    image.u32(BUCKET + 8, 1);
    image.accumulator(
        BUCKET + 0x20,
        VALUES + 0x100,
        &[(keys::DISABLE_STRATEGIC_REDEPLOYMENT, 31)],
    );
    assert_eq!(
        read_action(&image).unwrap().unwrap().state_modifier,
        Some(31)
    );
}

#[test]
fn strategic_action_friendly_state_fault_maps_to_its_own_failure() {
    let mut image = fixture();
    assert!(read_action(&image).unwrap().is_some());
    image.fault = Some(VALUES + 24..VALUES + 32);
    assert_eq!(
        read_action(&image),
        Err(StrategicActionFailure::FriendlyStateModifier)
    );
}
