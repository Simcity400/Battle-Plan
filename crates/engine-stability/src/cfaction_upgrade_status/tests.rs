use super::*;
use engine_native_memory::{memory::SliceMemory, msvc_string};
use engine_native_parts::modifier_entry;
use std::cell::RefCell;

const FACTION: usize = 0x1000;
const DATA: usize = 0x3000;

fn put(bytes: &mut [u8], at: usize, value: impl AsRef<[u8]>) {
    let value = value.as_ref();
    bytes[at..at + value.len()].copy_from_slice(value);
}
fn array(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    put(bytes, at, (data as u64).to_le_bytes());
    put(bytes, at + 8, count.to_le_bytes());
    put(bytes, at + 12, count.to_le_bytes());
}
fn fixture(count: u32) -> Vec<u8> {
    let mut bytes = vec![0; DATA + count as usize * SLOT_STRIDE];
    let status = FACTION + STATUS;
    put(&mut bytes, status, (VTABLE_RVA.0 as u64).to_le_bytes());
    put(&mut bytes, status + OWNER, (FACTION as u64).to_le_bytes());
    array(&mut bytes, status + SLOTS, DATA, count);
    for ordinal in 0..count as usize {
        let slot = DATA + ordinal * SLOT_STRIDE;
        put(
            &mut bytes,
            slot + SLOT_COUNTRY,
            (ordinal as u32 + 1).to_le_bytes(),
        );
        let date = slot + SLOT_SPYMASTER_DATE;
        put(
            &mut bytes,
            date,
            (cgame_date::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            date + cgame_date::PERSISTENT_BASE,
            (cgame_date::PERSISTENT_VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            date + cgame_date::DATE_BASE,
            (-17_i32).to_le_bytes(),
        );
        for offset in [SLOT_SPYMASTER_MODIFIER, SLOT_MODIFIER] {
            let modifier = slot + offset;
            put(
                &mut bytes,
                modifier,
                (cmodifier::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                modifier + cmodifier::DISPLAY_NAME + msvc_string::CAPACITY,
                15_u64.to_le_bytes(),
            );
        }
    }
    bytes
}

struct Traced<'a> {
    bytes: &'a [u8],
    calls: RefCell<Vec<(usize, usize)>>,
}
impl Memory for Traced<'_> {
    fn copy(&self, at: usize, output: &mut [u8]) -> bool {
        self.calls.borrow_mut().push((at, output.len()));
        SliceMemory {
            base: 0,
            bytes: self.bytes,
        }
        .copy(at, output)
    }
}

#[test]
fn faction_upgrade_demand_reads_only_the_selected_slot_and_reuses_storage() {
    let mut bytes = fixture(3);
    let entries = bytes.len();
    bytes.resize(entries + modifier_entry::STRIDE * 2, 0);
    let selected = DATA + 2 * SLOT_STRIDE;
    let modifier = selected + SLOT_MODIFIER;
    array(&mut bytes, modifier + cmodifier::ACCUMULATOR, entries, 2);
    for (ordinal, raw) in [-91_i64, 0].into_iter().enumerate() {
        let entry = entries + ordinal * modifier_entry::STRIDE;
        put(
            &mut bytes,
            entry + modifier_entry::KEY,
            1739_u32.to_le_bytes(),
        );
        put(&mut bytes, entry + modifier_entry::VALUE, raw.to_le_bytes());
    }
    let literal = [b'x', 0, 0xff];
    put(&mut bytes, modifier + cmodifier::DISPLAY_NAME, literal);
    put(
        &mut bytes,
        modifier + cmodifier::DISPLAY_NAME + msvc_string::LENGTH,
        3_u64.to_le_bytes(),
    );
    let memory = Traced {
        bytes: &bytes,
        calls: RefCell::default(),
    };
    let status = authenticate(&memory, 0, FACTION).unwrap();
    let slots = status.slots(&memory).unwrap();
    let mut output = Slot::default();
    status.slot_into(&memory, slots, 2, &mut output).unwrap();
    assert_eq!(output.owner_country, 3);
    assert_eq!(output.advisor_character, None);
    assert_eq!(output.spymaster_change_hour, -17);
    assert_eq!(output.modifier.name, literal);
    assert_eq!(
        output.modifier.entries,
        [
            cmodifier::Entry {
                key: 1739,
                raw: -91
            },
            cmodifier::Entry { key: 1739, raw: 0 }
        ]
    );
    assert!(output.spymaster.name.is_empty());
    assert!(output.spymaster.entries.is_empty());
    assert!(
        !memory
            .calls
            .borrow()
            .iter()
            .any(|&(at, _)| (DATA..selected).contains(&at))
    );
    let storage = (
        output.modifier.name.as_ptr(),
        output.modifier.entries.as_ptr(),
    );
    status.slot_into(&memory, slots, 2, &mut output).unwrap();
    assert_eq!(
        storage,
        (
            output.modifier.name.as_ptr(),
            output.modifier.entries.as_ptr()
        )
    );
    memory.calls.borrow_mut().clear();
    assert!(status.slot_into(&memory, slots, 3, &mut output).is_err());
    assert!(memory.calls.borrow().is_empty());
}

#[test]
fn faction_upgrade_sharing_failure_does_not_suppress_minimum_upgrades_or_slots() {
    let bytes = fixture(1);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let status = authenticate(&memory, 0, FACTION).unwrap();
    assert!(
        status
            .research_into(&memory, &mut Default::default())
            .is_err()
    );
    assert!(status.doctrine_into(&memory, &mut Vec::new()).is_err());
    let mut upgrades = Vec::new();
    status.member_upgrades_into(&memory, &mut upgrades).unwrap();
    assert!(upgrades.is_empty());
    let mut slot = Slot::default();
    status
        .slot_into(&memory, status.slots(&memory).unwrap(), 0, &mut slot)
        .unwrap();
    assert_eq!(slot.owner_country, 1);
}

#[test]
fn faction_upgrade_status_and_each_embedded_date_and_modifier_require_identity() {
    for offset in [
        STATUS,
        STATUS + OWNER,
        DATA - FACTION + SLOT_SPYMASTER_DATE,
        DATA - FACTION + SLOT_SPYMASTER_DATE + cgame_date::PERSISTENT_BASE,
        DATA - FACTION + SLOT_SPYMASTER_MODIFIER,
        DATA - FACTION + SLOT_MODIFIER,
    ] {
        let mut bytes = fixture(1);
        put(&mut bytes, FACTION + offset, 123_u64.to_le_bytes());
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        if offset == STATUS || offset == STATUS + OWNER {
            assert!(authenticate(&memory, 0, FACTION).is_err());
        } else {
            let status = authenticate(&memory, 0, FACTION).unwrap();
            assert!(
                status
                    .slot_into(
                        &memory,
                        status.slots(&memory).unwrap(),
                        0,
                        &mut Slot::default()
                    )
                    .is_err()
            );
        }
    }
}
