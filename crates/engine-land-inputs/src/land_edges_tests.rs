use super::*;
use engine_native::cgame_state;
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
    fn army(&mut self, at: usize, handle: u64, manpower: [u32; 2], casualties: u32) {
        self.qword(at, carmy::VTABLE_RVA.0 as u64);
        self.qword(at + cunit::HANDLE, handle);
        self.array(at + carmy::MANPOWER_DATA, at + 0x4000, 2);
        for (n, amount) in manpower.into_iter().enumerate() {
            self.dword(at + 0x4000 + n * manpower_value_entry::STRIDE, 7);
            self.dword(at + 0x4000 + n * manpower_value_entry::STRIDE + manpower_value_entry::COUNT, amount);
        }
        self.dword(at + unit_history::CAREER_COUNTER, casualties);
    }
}

const COMBAT: usize = 0x2000;
const BATTLE: u64 = 0x8000_0002_0000_003e;
const FIRST: u64 = 0x8000_0001_0000_1269;
const SECOND: u64 = 0x8000_0001_0000_126a;
const THIRD: u64 = 0x8000_0001_0000_126b;

fn fixture() -> Fixture {
    let mut memory = Fixture::default();
    memory.qword(cgame_state::SINGLETON_RVA.0, 0x1000);
    memory.dword(0x1000 + cgame_state::CURRENT_HOUR, 80);
    memory.qword(COMBAT, combat::VTABLE_RVA.0 as u64);
    memory.qword(COMBAT + combat::HANDLE, BATTLE);
    memory.qword(COMBAT + combat::ATTACKER, 0x3000);
    memory.qword(COMBAT + combat::DEFENDER, 0x3800);
    for (pointer, units, count) in [(0x3000, 0x5000, 1), (0x3800, 0x5800, 3)] {
        memory.qword(pointer, side::VTABLE_RVA.0 as u64);
        memory.qword(pointer + side::COMBAT, COMBAT as u64);
        memory.array(pointer + side::UNITS, units, count);
    }
    memory.qword(0x5000, 0x10000);
    memory.qword(0x5800, 0x20000);
    memory.qword(0x5808, 0x30000);
    memory.qword(0x5810, 0x40000);
    memory.army(0x10000, FIRST, [9000, 1000], 300);
    memory.army(0x20000, SECOND, [5000, 0], 0);
    memory.army(0x30000, THIRD, [1, 2], 40);
    memory.qword(0x40000, 0x1234);
    memory
}

fn copied(copy: impl FnOnce(&mut dyn FnMut(u64))) -> Vec<u64> {
    let mut words = Vec::new();
    copy(&mut |word| words.push(word));
    words
}

#[test]
fn an_opening_copies_each_side_member_with_its_manpower_and_casualties() {
    let memory = fixture();
    let opening = Opening::read(&memory, 0, COMBAT).expect("a land combat");
    let words = copied(|put| opening.copy(&memory, 0, put));
    assert_eq!(opening.words, words.len() + 1);
    let none = u64::from(DIVISION_UNREADABLE) << 8 | 1;
    assert_eq!(
        words,
        [
            OPENED, BATTLE, optional(Some(80)), 0, 4,
            0, FIRST, optional(Some(10_000)), optional(Some(300)),
            1, SECOND, optional(Some(5000)), optional(Some(0)),
            1, THIRD, optional(Some(3)), optional(Some(40)),
            none, 0, 0, 0,
        ]
    );
}

#[test]
fn an_opening_is_withheld_by_name_when_a_side_does_not_belong_to_the_combat() {
    let mut memory = fixture();
    memory.qword(0x3800 + side::COMBAT, 0x9999);
    let opening = Opening::read(&memory, 0, COMBAT).expect("a land combat");
    let words = copied(|put| opening.copy(&memory, 0, put));
    assert_eq!(words, [OPENED, BATTLE, optional(Some(80)), u64::from(COMBAT_UNREADABLE), 0]);
    assert_eq!(opening.words, words.len() + 1);
}

#[test]
fn naval_and_unregistered_combats_have_no_land_edge() {
    let mut memory = fixture();
    assert!(Opening::read(&memory, 0, 0).is_none());
    memory.qword(COMBAT + combat::HANDLE, 0);
    assert!(Opening::read(&memory, 0, COMBAT).is_none(), "a combat the game never registered is not a battle");
    assert!(Exit::before(&memory, 0, 0x10000, COMBAT).is_none());
    memory.qword(COMBAT, cnaval_combat::VTABLE_RVA.0 as u64);
    assert!(Opening::read(&memory, 0, COMBAT).is_none());
}

#[test]
fn an_exit_carries_the_casualties_counter_on_both_sides_of_the_flush() {
    let mut memory = fixture();
    memory.qword(0x20000 + carmy::PENDING_STRENGTH_DAMAGE, 2_500_000);
    let exit = Exit::before(&memory, 0, 0x20000, COMBAT).expect("a land combat");
    memory.dword(0x20000 + unit_history::CAREER_COUNTER, 812);
    memory.dword(0x24000 + manpower_value_entry::COUNT, 4188);
    memory.qword(0x20000 + carmy::PENDING_STRENGTH_DAMAGE, 0);
    let words = copied(|put| exit.copy(&memory, 0, 0x20000, put));
    assert_eq!(EXIT_WORDS, words.len() + 1);
    assert_eq!(
        words,
        [EXITED, BATTLE, optional(Some(80)), 0, 1, SECOND, optional(Some(0)), optional(Some(812)), optional(Some(4188)), 2_500_000],
        "the pending strength damage is the pool read before the flush clears it"
    );
}

#[test]
fn an_exit_names_what_it_could_not_read() {
    let mut memory = fixture();
    memory.army(0x50000, FIRST, [1, 1], 5);
    let stranger = Exit::before(&memory, 0, 0x50000, COMBAT).expect("a land combat");
    let words = copied(|put| stranger.copy(&memory, 0, 0x50000, put));
    assert_eq!(&words[3..6], [u64::from(SIDE_UNKNOWN | POOL_UNREADABLE), u64::from(NO_SIDE), FIRST]);
    let unknown = Exit::before(&memory, 0, 0x40000, COMBAT).expect("a land combat");
    let words = copied(|put| unknown.copy(&memory, 0, 0x40000, put));
    assert_eq!(&words[3..], [u64::from(DIVISION_UNREADABLE | POOL_UNREADABLE), 1, 0, 0, 0, 0, 0]);
    memory.0.remove(&(0x1000 + cgame_state::CURRENT_HOUR));
    let hourless = Exit::before(&memory, 0, 0x10000, COMBAT).expect("a land combat");
    let words = copied(|put| hourless.copy(&memory, 0, 0x10000, put));
    assert_eq!(words[2], optional(None));
    let freed = Exit::before(&memory, 0, 0x10000, COMBAT).expect("a land combat");
    memory.qword(0x10000, 0);
    let words = copied(|put| freed.copy(&memory, 0, 0x10000, put));
    assert_eq!(&words[6..], [optional(Some(300)), optional(None), optional(None), 0], "a division gone after the flush is not read");
    assert_eq!(unpack(optional(Some(-3))), Some(-3));
    assert_eq!(unpack(optional(None)), None);
}

const GROUP: u64 = 0x8000_0004_0000_0010;

fn hit_fixture() -> Fixture {
    let mut memory = fixture();
    memory.qword(0x10000 + carmy::ORDERS_GROUP, 0x60000);
    memory.qword(0x60000 + engine_native::corders_group::HANDLE, GROUP);
    memory.qword(0x20000 + carmy::ORDERS_GROUP, 0);
    memory
}

#[test]
fn a_hit_names_its_combat_attacker_target_and_the_attackers_army() {
    let memory = hit_fixture();
    let observer = 0x3000 + side::STATS_OBSERVER;
    let hit = Hit::read(&memory, 0, observer, 0x10000, 0x20000, 412_345);
    assert_eq!(hit, Hit { combat: BATTLE, attacker: FIRST, target: SECOND, army: GROUP, strength: 412_345, withheld: 0 });
    let words = copied(|put| hit.copy(put));
    assert_eq!(HIT_WORDS, words.len() + 1);
    assert_eq!(Hit::decode(&words), Some(hit));
    let outside = Hit::read(&memory, 0, 0x3800 + side::STATS_OBSERVER, 0x20000, 0x10000, 7);
    assert_eq!((outside.army, outside.withheld), (0, 0), "a division in no army is no army, not unreadable");
}

#[test]
fn a_hit_names_each_part_it_could_not_read() {
    let mut memory = hit_fixture();
    let observer = 0x3000 + side::STATS_OBSERVER;
    let stranger = Hit::read(&memory, 0, 0x9000, 0x40000, 0x50000, 1);
    assert_eq!(stranger.withheld, COMBAT_UNREADABLE | DIVISION_UNREADABLE | TARGET_UNREADABLE | ARMY_UNREADABLE);
    assert_eq!((stranger.combat, stranger.attacker, stranger.target, stranger.army), (0, 0, 0, 0));
    memory.qword(0x60000 + engine_native::corders_group::HANDLE, 0);
    assert_eq!(Hit::read(&memory, 0, observer, 0x10000, 0x20000, 1).withheld, ARMY_UNREADABLE);
    memory.qword(COMBAT, cnaval_combat::VTABLE_RVA.0 as u64);
    assert_eq!(Hit::read(&memory, 0, observer, 0x10000, 0x20000, 1).withheld & COMBAT_UNREADABLE, COMBAT_UNREADABLE);
    assert_eq!(Hit::decode(&[EXITED, 0, 0, 0, 0, 0, 0]), None);
    assert_eq!(Hit::decode(&[HIT, 0, 0]), None);
}
