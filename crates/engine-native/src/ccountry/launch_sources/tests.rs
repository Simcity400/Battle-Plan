use super::*;
use std::cell::{Cell, RefCell};

const ROOT: usize = 0x2000;
const STATE: usize = 0x1000;
const NAVY: usize = 0x5000;

struct Image {
    bytes: Vec<u8>,
    fault_once: Option<usize>,
    failed: Cell<bool>,
    calls: RefCell<Vec<(usize, usize)>>,
}

impl Memory for Image {
    fn authenticated_reserved_country(
        &self,
        _: usize,
        _: usize,
    ) -> Option<Result<crate::ccountry::Country, &'static str>> {
        Some(Ok(crate::ccountry::Country {
            pointer: ROOT,
            canonical_slot: 0,
            id: 0,
        }))
    }

    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.calls.borrow_mut().push((address, destination.len()));
        if self.fault_once.is_some_and(|fault| {
            address <= fault && fault < address.saturating_add(destination.len())
        }) && !self.failed.replace(true)
        {
            return false;
        }
        let Some(source) = address
            .checked_add(destination.len())
            .and_then(|end| self.bytes.get(address..end))
        else {
            return false;
        };
        destination.copy_from_slice(source);
        true
    }
}

fn word(image: &mut Image, address: usize, value: u32) {
    image.bytes[address..address + 4].copy_from_slice(&value.to_le_bytes());
}
fn pointer(image: &mut Image, address: usize, value: usize) {
    image.bytes[address..address + 8].copy_from_slice(&(value as u64).to_le_bytes());
}
fn header(image: &mut Image, address: usize, data: usize, count: u32) {
    pointer(image, address, data);
    word(image, address + 8, count);
    word(image, address + 12, count);
}

fn image() -> Image {
    let mut image = Image {
        bytes: vec![0; 0xa000],
        fault_once: None,
        failed: Cell::new(false),
        calls: RefCell::new(Vec::new()),
    };
    header(&mut image, ROOT + crate::ccountry::OWNED_STATES, 0x3000, 3);
    header(&mut image, STATE + cgame_state::STATES, 0x3100, 10);
    for (object, id) in [(0x4000, 9), (0x4200, 3)] {
        pointer(&mut image, object, cstate::VTABLE_RVA.0);
        word(&mut image, object + cstate::ID, id);
        pointer(&mut image, 0x3100 + id as usize * 8, object);
    }
    for (ordinal, object) in [0x4000, 0x4200, 0x4000].into_iter().enumerate() {
        pointer(&mut image, 0x3000 + ordinal * 8, object);
    }
    header(&mut image, NAVY + 0x18, 0x6000, 3);
    word(&mut image, NAVY + naval_base_layout::HOLDER_COUNTRY, 999);
    for (object, province) in [(0x7000, (-3_i32) as u32), (0x7100, 0)] {
        pointer(&mut image, object, naval_base_layout::VTABLE_RVA.0);
        pointer(&mut image, object + naval_base_layout::HOLDER, NAVY);
        word(&mut image, object + naval_base_layout::PROVINCE_ID, province);
    }
    for (ordinal, object) in [0x7000, 0x7100, 0x7000].into_iter().enumerate() {
        pointer(&mut image, 0x6000 + ordinal * 8, object);
    }
    image
}

#[test]
fn naming_failure_preserves_one_count_and_ordered_repeated_state_bodies() {
    let image = image();
    let mut walk = crate::ccountry::read_reserved_naming_observed_inner(&image, 0, STATE, crate::walk::Buffer::Count, None);
    assert!(matches!(
        walk.outcome,
        crate::walk::Outcome::Unavailable { .. }
    ));
    let mut scratch = vec![Occurrence::default(); 3];
    let mut output = vec![0; 3];
    let result =
        walk.launch_states
            .take()
            .unwrap()
            .capture(&image, 0, STATE, &mut scratch, &mut output);
    assert_eq!(result.len, Ok(3));
    assert_eq!(output, [9, 3, 9]);
    let calls = image.calls.borrow();
    assert_eq!(
        calls
            .iter()
            .filter(|&&(at, _)| at == ROOT + crate::ccountry::OWNED_STATE_COUNT)
            .count(),
        1
    );
    assert!(
        calls
            .iter()
            .any(|&(at, len)| at == ROOT + crate::ccountry::OWNED_STATES && len == 12)
    );
    assert!(
        !calls
            .iter()
            .any(|&(at, len)| at == ROOT + crate::ccountry::OWNED_STATES && len == 16)
    );
    assert_eq!(
        calls
            .iter()
            .filter(|&&(at, _)| at == 0x4000 + cstate::ID)
            .count(),
        1
    );
    assert!(scratch.iter().all(|item| item.pointer == 0));
}

#[test]
fn naval_order_repeats_and_signed_words_survive_without_country_resolution() {
    let image = image();
    let mut scratch = vec![Occurrence::default(); 3];
    let mut output = vec![LaunchNavalBase::default(); 3];
    assert_eq!(
        capture_naval(&image, 0, NAVY, &mut scratch, &mut output).provider,
        Ok(NavalProvider::Bases(3))
    );
    assert_eq!(
        output
            .iter()
            .map(|value| (value.base_province as i32, value.country))
            .collect::<Vec<_>>(),
        [(-3, 999), (0, 999), (-3, 999)]
    );
    let calls = image.calls.borrow();
    assert_eq!(
        calls
            .iter()
            .filter(|&&(at, _)| at == NAVY + naval_base_layout::HOLDER_COUNTRY)
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|&&(at, _)| at == 0x7000 + naval_base_layout::PROVINCE_ID)
            .count(),
        1
    );
    assert!(scratch.iter().all(|item| item.pointer == 0));
}

#[test]
fn zero_count_skips_descriptor_and_selecting_state_inputs() {
    let mut image = image();
    word(&mut image, ROOT + crate::ccountry::OWNED_STATE_COUNT, 0);
    image.fault_once = Some(ROOT + crate::ccountry::OWNED_STATES);
    let control = ReservedOwnedStates::new(&image, 0, STATE);
    let result = control.capture(&image, 0, STATE, &mut Vec::new(), &mut Vec::new());
    assert_eq!((result.len, result.required), (Ok(0), 0));
    assert!(!image.failed.get());
    assert_eq!(
        &*image.calls.borrow(),
        &[(ROOT + crate::ccountry::OWNED_STATE_COUNT, 4)]
    );
}

#[test]
fn failed_count_is_sticky_and_negative_count_remains_exact() {
    let mut image = image();
    image.fault_once = Some(ROOT + crate::ccountry::OWNED_STATE_COUNT);
    let control = ReservedOwnedStates::new(&image, 0, STATE);
    assert_eq!(
        control.count(),
        Err("reserved owned state count unreadable")
    );
    let result = control.capture(
        &image,
        0,
        STATE,
        &mut vec![Occurrence::default(); 3],
        &mut vec![0; 3],
    );
    assert_eq!(
        (result.len, result.required),
        (Err("reserved owned state count unreadable"), 0)
    );
    assert_eq!(
        image.calls.borrow().len(),
        1,
        "a second observation would recover and hide this failure"
    );
    for count in [-1_i32, i32::MIN] {
        image.calls.borrow_mut().clear();
        word(
            &mut image,
            ROOT + crate::ccountry::OWNED_STATE_COUNT,
            count as u32,
        );
        let control = ReservedOwnedStates::new(&image, 0, STATE);
        assert_eq!(control.count(), Ok(count));
        let result = control.capture(&image, 0, STATE, &mut Vec::new(), &mut Vec::new());
        assert_eq!(
            (result.len, result.required),
            (Err("reserved launch state count is negative"), 0)
        );
        assert_eq!(image.calls.borrow().len(), 1);
    }
}

#[test]
fn state_body_and_selector_failures_are_atomic_for_repeated_objects() {
    for fault in [0x4000 + cstate::ID, 0x3100 + 9 * 8] {
        let mut image = image();
        image.fault_once = Some(fault);
        let mut scratch = vec![Occurrence::default(); 3];
        let mut output = vec![77; 3];
        let result = ReservedOwnedStates::new(&image, 0, STATE).capture(
            &image,
            0,
            STATE,
            &mut scratch,
            &mut output,
        );
        assert!(result.len.is_err());
        assert_eq!(result.required, 3);
        assert_eq!(output, [0; 3]);
        assert_eq!(
            image
                .calls
                .borrow()
                .iter()
                .filter(|&&(at, _)| at == fault)
                .count(),
            1
        );
        assert!(scratch.iter().all(|item| item.pointer == 0));
    }
}

#[test]
fn empty_navy_and_native_null_are_distinct_without_owner_reads() {
    let mut image = image();
    header(&mut image, NAVY + NAVAL_BASES, 0, 0);
    image.fault_once = Some(NAVY + naval_base_layout::HOLDER_COUNTRY);
    assert_eq!(
        capture_naval(&image, 0, NAVY, &mut Vec::new(), &mut Vec::new()).provider,
        Ok(NavalProvider::Bases(0))
    );
    assert!(!image.failed.get());
    assert_eq!(&*image.calls.borrow(), &[(NAVY + NAVAL_BASES, 16)]);
    image.calls.borrow_mut().clear();
    assert_eq!(
        capture_naval(&image, 0, 0, &mut Vec::new(), &mut Vec::new()).provider,
        Ok(NavalProvider::NativeNull)
    );
    assert!(image.calls.borrow().is_empty());
}

#[test]
fn naval_repeated_body_failure_keeps_demand_and_clears_scratch() {
    let mut image = image();
    let mut output = vec![LaunchNavalBase::default(); 3];
    image.fault_once = Some(0x7000 + naval_base_layout::PROVINCE_ID);
    let mut scratch = vec![Occurrence::default(); 3];
    output.fill(LaunchNavalBase {
        base_province: 77,
        country: 77,
    });
    let result = capture_naval(&image, 0, NAVY, &mut scratch, &mut output);
    assert_eq!(
        (result.provider, result.required),
        (Err("reserved launch naval base body unreadable"), 3)
    );
    assert_eq!(output, [LaunchNavalBase::default(); 3]);
    assert_eq!(
        image
            .calls
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == 0x7000 + naval_base_layout::PROVINCE_ID)
            .count(),
        1
    );
    assert!(scratch.iter().all(|item| item.pointer == 0));
}

#[test]
fn null_list_objects_and_foreign_holders_never_publish_an_empty_or_prefix() {
    for null_object in [true, false] {
        let mut image = image();
        if null_object {
            pointer(&mut image, 0x6008, 0);
        } else {
            pointer(&mut image, 0x7000 + naval_base_layout::HOLDER, NAVY + 0x200);
        }
        let mut scratch = vec![Occurrence::default(); 3];
        let mut output = vec![LaunchNavalBase::default(); 3];
        let result = capture_naval(&image, 0, NAVY, &mut scratch, &mut output);
        assert!(result.provider.is_err());
        assert_eq!(result.required, 3);
        assert_eq!(output, [LaunchNavalBase::default(); 3]);
        assert!(scratch.iter().all(|item| item.pointer == 0));
        let states = ReservedOwnedStates::new(&image, 0, STATE).capture(
            &image,
            0,
            STATE,
            &mut scratch,
            &mut vec![0; 3],
        );
        assert_eq!(
            states.len,
            Ok(3),
            "a naval fault is not a state-list prerequisite"
        );
    }
}

#[test]
fn malformed_descriptor_and_null_state_keep_full_count_without_fake_state_zero() {
    for null_state in [true, false] {
        let mut image = image();
        if null_state {
            pointer(&mut image, 0x3008, 0);
        } else {
            word(&mut image, ROOT + crate::ccountry::OWNED_STATE_CAPACITY, 2);
        }
        let control = ReservedOwnedStates::new(&image, 0, STATE);
        assert_eq!(control.count(), Ok(3));
        let mut scratch = vec![Occurrence::default(); 3];
        let mut output = vec![77; 3];
        let result = control.capture(&image, 0, STATE, &mut scratch, &mut output);
        assert_eq!(result.required, 3);
        assert!(result.len.is_err());
        assert_eq!(output, [0; 3]);
        assert!(scratch.iter().all(|item| item.pointer == 0));
        assert!(
            !image
                .calls
                .borrow()
                .iter()
                .any(|&(at, _)| at == 0x4000 || at == 0x4200)
        );
    }
}

#[test]
fn launch_naval_scratch_and_output_grow_for_mod_sized_original_rosters() {
    let mut scratch = Vec::new();
    let mut output = Vec::new();
    for count in [50_usize, 120] {
        let mut image = image();
        let list = 0xa000;
        image.bytes.resize(list + count * 8, 0);
        header(&mut image, NAVY + NAVAL_BASES, list, count as u32);
        for index in 0..count {
            pointer(
                &mut image,
                list + index * 8,
                if index % 2 == 0 { 0x7000 } else { 0x7100 },
            );
        }
        image.calls.borrow_mut().clear();
        let captured = capture_naval_buffer(
            &image,
            0,
            NAVY,
            crate::walk::Buffer::Growing(&mut scratch),
            crate::walk::Buffer::Growing(&mut output),
        );
        assert_eq!(
            captured,
            NavalCapture {
                required: count,
                provider: Ok(NavalProvider::Bases(count))
            }
        );
        assert!(
            output[..count]
                .iter()
                .enumerate()
                .all(|(i, value)| value.country == 999
                    && value.base_province == if i % 2 == 0 { (-3_i32) as u32 } else { 0 })
        );
        assert!(scratch.iter().all(|row| row.pointer == 0));
        assert_eq!(
            image
                .calls
                .borrow()
                .iter()
                .filter(|&&(at, _)| at == NAVY + NAVAL_BASES)
                .count(),
            1
        );
    }
}

#[test]
fn launch_state_scratch_and_output_grow_from_the_original_owned_count() {
    let mut scratch = Vec::new();
    let mut output = Vec::new();
    for count in [50_usize, 120] {
        let mut image = image();
        let list = 0xa000;
        image.bytes.resize(list + count * 8, 0);
        header(
            &mut image,
            ROOT + crate::ccountry::OWNED_STATES,
            list,
            count as u32,
        );
        for index in 0..count {
            pointer(
                &mut image,
                list + index * 8,
                if index % 2 == 0 { 0x4000 } else { 0x4200 },
            );
        }
        let mut naming = crate::ccountry::read_reserved_naming_observed_inner(&image, 0, STATE, crate::walk::Buffer::Count, None);
        let original = naming.launch_states.take().unwrap();
        let captured = original.capture(&image, 0, STATE, &mut scratch, &mut output);
        assert_eq!(captured.len, Ok(count));
        assert_eq!(captured.required, count);
        assert!(
            output[..count]
                .iter()
                .enumerate()
                .all(|(i, value)| *value == if i % 2 == 0 { 9 } else { 3 })
        );
        assert!(scratch.iter().all(|row| row.pointer == 0));
        assert_eq!(
            image
                .calls
                .borrow()
                .iter()
                .filter(|&&(at, _)| at == ROOT + crate::ccountry::OWNED_STATE_COUNT)
                .count(),
            1
        );
    }
}
