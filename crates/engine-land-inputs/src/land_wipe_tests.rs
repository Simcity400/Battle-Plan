use super::*;
use std::collections::BTreeMap;

#[derive(Default)]
struct Fixture(BTreeMap<usize, u8>);
impl Memory for Fixture {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        for (offset, target) in output.iter_mut().enumerate() {
            let Some(value) = self.0.get(&(address + offset)) else {
                return false;
            };
            *target = *value;
        }
        true
    }
}
impl Fixture {
    fn put(&mut self, at: usize, bytes: &[u8]) {
        for (offset, value) in bytes.iter().enumerate() {
            self.0.insert(at + offset, *value);
        }
    }
    fn qword(&mut self, at: usize, value: u64) {
        self.put(at, &value.to_le_bytes());
    }
    fn dword(&mut self, at: usize, value: u32) {
        self.put(at, &value.to_le_bytes());
    }
    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.qword(at, data as u64);
        self.dword(at + 8, count);
        self.dword(at + 12, count);
    }
}

const DIVISION: u64 = 0x8000_0001_0000_1269;
const TRIGGER: u64 = 0x8000_0002_0000_003e;
const OTHER_COMBAT: u64 = 0x8000_0003_0000_003e;

fn fixture() -> Fixture {
    let mut memory = Fixture::default();
    memory.qword(cgame_state::SINGLETON_RVA.0, 0x1000);
    memory.dword(0x1000 + cgame_state::CURRENT_HOUR, 80);
    memory.qword(0x2000, combat::VTABLE_RVA.0 as u64);
    memory.qword(0x2000 + combat::HANDLE, TRIGGER);
    memory.qword(0x2000 + combat::ATTACKER, 0x3000);
    memory.qword(0x2000 + combat::DEFENDER, 0x4000);
    memory.qword(0x2000 + combat::PROVINCE, 0x7000);
    for pointer in [0x3000, 0x4000] {
        memory.qword(pointer, side::VTABLE_RVA.0 as u64);
        memory.qword(pointer + side::COMBAT, 0x2000);
    }
    memory.put(0x3000 + side::IS_ATTACKER, &[1]);
    memory.put(0x4000 + side::IS_ATTACKER, &[0]);
    memory.array(0x4000 + COUNTRY_ARRAY, 0x9000, 2);
    memory.dword(0x9000, 0x8000_0007);
    // A later unreadable country must not hide the native first-country read.
    memory.qword(0x5000, cprovince::VTABLE_RVA.0 as u64);
    memory.dword(0x5000 + cprovince::ID, 42);
    memory.qword(0x7000, cprovince::VTABLE_RVA.0 as u64);
    memory.dword(0x7000 + cprovince::ID, 55);
    memory.qword(0x8000, carmy::VTABLE_RVA.0 as u64);
    memory.dword(0x8000 + cunit::CLASS, 0);
    memory.qword(0x8000 + cunit::HANDLE, DIVISION);
    memory.dword(0x8000 + cunit::LOGICAL_OWNER, 0x8000_0009);
    memory.qword(0x8000 + cunit::LOCATION, 0x5000);
    memory.array(0x8000 + cunit::COMBAT_REFERENCE_DATA, 0xb000, 1);
    memory.qword(0xb000, 0x6000);
    memory.qword(0x6000, combat::VTABLE_RVA.0 as u64);
    memory.qword(0x6000 + combat::HANDLE, OTHER_COMBAT);
    memory
}

fn words(memory: &Fixture, combatant: usize) -> Result<Vec<u64>, &'static str> {
    let plan = WipePlan::read(memory, 0, combatant, 0x8000)?.ok_or("not a land wipe")?;
    let mut result = Vec::new();
    plan.copy(memory, 0, |word| result.push(word))?;
    assert_eq!(plan.words, result.len() + 1);
    Ok(result)
}

#[test]
fn combat_wipe_keeps_native_first_country_and_separate_trigger() {
    assert_eq!(
        words(&fixture(), 0x3000).unwrap(),
        [
            7,
            80,
            DIVISION,
            0x8000_0009,
            0x8000_0007,
            42,
            55,
            TRIGGER,
            1,
            OTHER_COMBAT,
        ]
    );
}

#[test]
fn combat_wipe_default_country_follows_slot_zero_link_translation() {
    let mut memory = fixture();
    memory.array(0x4000 + COUNTRY_ARRAY, 0, 0);
    memory.array(0x1000 + cgame_state::COUNTRY_LINKS, 0xa000, 4);
    memory.dword(0xa000, 5);
    memory.dword(0xa004, 6);
    memory.dword(0xa008, 0);
    // Reserved-country link search stops before an unreadable fourth lane.
    assert_eq!(words(&memory, 0x3000).unwrap()[4], 2);
    memory.array(0x1000 + cgame_state::COUNTRY_LINKS, 0, 0);
    assert_eq!(words(&memory, 0x3000).unwrap()[4], 0);
}

#[test]
fn combat_wipe_empty_remaining_list_does_not_add_trigger_to_it() {
    let mut memory = fixture();
    memory.array(0x8000 + cunit::COMBAT_REFERENCE_DATA, 0, 0);
    let result = words(&memory, 0x3000).unwrap();
    assert_eq!(result.len(), 9);
    assert_eq!(result[7..], [TRIGGER, 0]);
}

#[test]
fn combat_wipe_rejects_unremoved_trigger_and_first_country_short_read() {
    let mut memory = fixture();
    memory.qword(0xb000, 0x2000);
    assert_eq!(
        words(&memory, 0x3000),
        Err("combat wipe triggering reference was not removed")
    );
    memory.qword(0xb000, 0x6000);
    memory.0.remove(&0x9002);
    assert_eq!(words(&memory, 0x3000), Err("combat wipe field unreadable"));
}

#[test]
fn combat_wipe_rejects_side_and_remaining_identity_failures() {
    let mut memory = fixture();
    memory.put(0x3000 + side::IS_ATTACKER, &[0]);
    assert_eq!(
        words(&memory, 0x3000),
        Err("combat wipe side discriminator differs")
    );
    memory.put(0x3000 + side::IS_ATTACKER, &[1]);
    memory.qword(0x4000 + side::COMBAT, 0x6000);
    assert_eq!(
        words(&memory, 0x3000),
        Err("combat wipe opposite backlink differs")
    );
    memory.qword(0x4000 + side::COMBAT, 0x2000);
    memory.qword(0x6000, carmy::VTABLE_RVA.0 as u64);
    assert_eq!(
        words(&memory, 0x3000),
        Err("combat wipe remaining class unsupported")
    );
}

#[test]
fn combat_wipe_defender_and_border_side_follow_native_opposite_selection() {
    let mut memory = fixture();
    memory.array(0x3000 + COUNTRY_ARRAY, 0x9000, 1);
    assert_eq!(words(&memory, 0x4000).unwrap()[4], 0x8000_0007);
    memory.qword(0x2000, cland_border_war_combat::VTABLE_RVA.0 as u64);
    for pointer in [0x3000, 0x4000] {
        memory.qword(pointer, cland_border_war_combatant::VTABLE_RVA.0 as u64);
    }
    assert_eq!(words(&memory, 0x3000).unwrap()[7], TRIGGER);
}

#[test]
fn combat_wipe_ignores_authenticated_naval_side_and_refuses_unknown_side() {
    let mut memory = fixture();
    memory.qword(0x3000, cnaval_combatant::VTABLE_RVA.0 as u64);
    // No unit read is attempted for the known unrelated native domain.
    assert!(WipePlan::read(&memory, 0, 0x3000, 0).unwrap().is_none());
    memory.qword(0x3000, carmy::VTABLE_RVA.0 as u64);
    assert_eq!(words(&memory, 0x3000), Err("combat wipe class unsupported"));
}

#[test]
fn combat_wipe_stale_identities_are_not_unsupported_classes() {
    for pointer in [0x8000, 0x2000, 0x4000, 0x5000, 0x7000] {
        let mut memory = fixture();
        memory.qword(pointer, 0);
        assert_eq!(
            words(&memory, 0x3000),
            Err("combat wipe pointer identity differs")
        );
    }
}

#[test]
fn casualty_country_does_not_read_the_later_leader_on_action_country_selection() {
    let mut memory = fixture();
    memory.put(0x8000 + 0x630, &[1]);
    memory.qword(0x4000 + 0x88, 0xdead_beef);
    memory.dword(0x4000 + 0x90, 1);
    assert_eq!(words(&memory, 0x3000).unwrap()[4], 0x8000_0007);
}
