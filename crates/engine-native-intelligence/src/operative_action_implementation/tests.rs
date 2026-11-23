use super::*;
use std::{cell::Cell, collections::BTreeMap};

struct Fixture {
    fields: BTreeMap<usize, Vec<u8>>,
    getter_reads: Cell<usize>,
}

impl engine_native_memory::Memory for Fixture {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if address == VTABLE_RVAS[0].0 + COUNTRY_GETTER_SLOT {
            self.getter_reads.set(self.getter_reads.get() + 1);
        }
        let Some(value) = self
            .fields
            .get(&address)
            .filter(|value| value.len() == destination.len())
        else {
            return false;
        };
        destination.copy_from_slice(value);
        true
    }
}

fn fixture() -> Fixture {
    Fixture {
        fields: BTreeMap::from([
            (
                0x100 + crate::operative_action::IMPLEMENTATION,
                0x200_u64.to_le_bytes().to_vec(),
            ),
            (
                0x300 + crate::operative_action::IMPLEMENTATION,
                0x400_u64.to_le_bytes().to_vec(),
            ),
            (0x200, (VTABLE_RVAS[0].0 as u64).to_le_bytes().to_vec()),
            (0x400, (VTABLE_RVAS[0].0 as u64).to_le_bytes().to_vec()),
            (0x200 + TARGET_COUNTRY, 7_u32.to_le_bytes().to_vec()),
            (0x400 + TARGET_COUNTRY, 8_u32.to_le_bytes().to_vec()),
            (
                VTABLE_RVAS[0].0 + COUNTRY_GETTER_SLOT,
                (COUNTRY_GETTER_RVA.0 as u64).to_le_bytes().to_vec(),
            ),
        ]),
        getter_reads: Cell::new(0),
    }
}

#[test]
fn country_getter_is_authenticated_once_per_vtable_without_reusing_mutable_targets() {
    let mut memory = fixture();
    let mut vtables = Vtables::new(0);
    assert_eq!(vtables.country(&memory, 0x100, 1), Ok(7));
    assert_eq!(vtables.country(&memory, 0x300, 1), Ok(8));
    memory
        .fields
        .insert(0x200 + TARGET_COUNTRY, 9_u32.to_le_bytes().to_vec());
    assert_eq!(vtables.country(&memory, 0x100, 1), Ok(9));
    assert_eq!(memory.getter_reads.get(), 1);
    memory.fields.insert(0x400, 0_u64.to_le_bytes().to_vec());
    assert_eq!(
        vtables.country(&memory, 0x300, 1),
        Err("unsupported operative mission implementation vtable")
    );
    assert_eq!(memory.getter_reads.get(), 1);
}

#[test]
fn failed_country_getter_authentication_is_not_retried_or_carried_to_another_walk() {
    let mut memory = fixture();
    memory.fields.insert(
        VTABLE_RVAS[0].0 + COUNTRY_GETTER_SLOT,
        0_u64.to_le_bytes().to_vec(),
    );
    let mut vtables = Vtables::new(0);
    assert_eq!(
        vtables.country(&memory, 0x100, 1),
        Err("unsupported operative mission country getter")
    );
    memory.fields.insert(
        VTABLE_RVAS[0].0 + COUNTRY_GETTER_SLOT,
        (COUNTRY_GETTER_RVA.0 as u64).to_le_bytes().to_vec(),
    );
    assert_eq!(
        vtables.country(&memory, 0x300, 1),
        Err("unsupported operative mission country getter")
    );
    assert_eq!(memory.getter_reads.get(), 1);
    assert_eq!(Vtables::new(0).country(&memory, 0x300, 1), Ok(8));
    assert_eq!(memory.getter_reads.get(), 2);
}

#[test]
fn targets_read_a_state_only_for_state_variants_and_authenticate_once() {
    let mut memory = fixture();
    memory
        .fields
        .insert(0x200 + TARGET_POINTER, 0x900_u64.to_le_bytes().to_vec());
    let mut vtables = Vtables::new(0);
    assert_eq!(
        vtables.targets(&memory, 0x100, 1),
        Ok(Targets {
            country: 7,
            state: Some(0x900)
        })
    );
    memory
        .fields
        .insert(0x200 + TARGET_POINTER, 0_u64.to_le_bytes().to_vec());
    assert_eq!(
        vtables.targets(&memory, 0x100, 1),
        Ok(Targets {
            country: 7,
            state: None
        })
    );
    assert_eq!(vtables.country(&memory, 0x100, 1), Ok(7));
    assert_eq!(memory.getter_reads.get(), 1);
    memory.fields.remove(&(0x200 + TARGET_POINTER));
    assert_eq!(
        vtables.targets(&memory, 0x100, 1),
        Err("operative mission target state unreadable")
    );
    memory
        .fields
        .insert(0x400, (VTABLE_RVAS[2].0 as u64).to_le_bytes().to_vec());
    memory.fields.insert(
        VTABLE_RVAS[2].0 + COUNTRY_GETTER_SLOT,
        (COUNTRY_GETTER_RVA.0 as u64).to_le_bytes().to_vec(),
    );
    assert_eq!(
        vtables.targets(
            &memory,
            0x300,
            crate::operative_action::COUNTER_INTELLIGENCE
        ),
        Ok(Targets {
            country: 8,
            state: None
        })
    );
}
