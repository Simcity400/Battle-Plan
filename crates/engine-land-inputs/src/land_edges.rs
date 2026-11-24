use engine_deployment::manpower_value_entry;
use engine_native::{carmy, cland_combat as combat, cland_combatant as side, cnaval_combat, cunit};
use engine_native_land::unit_history;
use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_native_parts::{cland_border_war_combat, cland_border_war_combatant};
use engine_sites::Rva;

pub const START_COMBAT_RVA: Rva = Rva(0x00c0_b8d0);
pub const ARMY_COMBAT_ENDED_RVA: Rva = Rva(0x00ce_7870);

pub const OPENED: u64 = 10;
pub const EXITED: u64 = 11;
pub const OPENING_WORDS: usize = 6;
pub const MEMBER_WORDS: usize = 4;
pub const EXIT_WORDS: usize = 10;

pub const COMBAT_UNREADABLE: u8 = 1;
pub const DIVISION_UNREADABLE: u8 = 2;
pub const SIDE_UNKNOWN: u8 = 4;
pub const HOUR_UNREADABLE: u8 = 8;
pub const NO_SIDE: u8 = u8::MAX;

pub const fn optional(value: Option<i32>) -> u64 {
    match value {
        Some(value) => 1 << 32 | value as u32 as u64,
        None => 0,
    }
}

pub const fn unpack(word: u64) -> Option<i32> {
    if word >> 32 == 1 { Some(word as u32 as i32) } else { None }
}

fn field<T: engine_native_memory::memory::Raw>(memory: &impl Memory, pointer: usize, offset: usize) -> Option<T> {
    pointer.checked_add(offset).and_then(|at| read(memory, at))
}

fn is(memory: &impl Memory, base: usize, pointer: usize, vtable: Rva) -> bool {
    pointer != 0 && field::<u64>(memory, pointer, 0).is_some_and(|value| Some(value as usize) == engine_sites::at(base, vtable))
}

fn hour(memory: &impl Memory, base: usize) -> Option<i32> {
    crate::land_events::state_hour(memory, base).ok().map(|(_, hour)| hour)
}

#[derive(Clone, Copy)]
struct Sides {
    handle: u64,
    units: [Array; 2],
}

enum Read {
    NotLand,
    Unregistered,
    Unreadable(u64),
    Land(Sides),
}

fn sides(memory: &impl Memory, base: usize, pointer: usize) -> Read {
    if pointer == 0 {
        return Read::NotLand;
    }
    let Some(vtable) = field::<u64>(memory, pointer, 0) else {
        return Read::Unreadable(0);
    };
    let classes = [combat::VTABLE_RVA, cland_border_war_combat::VTABLE_RVA, cnaval_combat::VTABLE_RVA];
    let expected = match engine_sites::which(base, vtable as usize, &classes) {
        Some(combat::VTABLE_RVA) => side::VTABLE_RVA,
        Some(cland_border_war_combat::VTABLE_RVA) => cland_border_war_combatant::VTABLE_RVA,
        Some(_) => return Read::NotLand,
        None => return Read::Unreadable(0),
    };
    let Some(handle) = field::<u64>(memory, pointer, combat::HANDLE) else {
        return Read::Unreadable(0);
    };
    if handle == 0 {
        return Read::Unregistered;
    }
    let mut units = [Array { data: 0, count: 0 }; 2];
    for (n, offset) in [combat::ATTACKER, combat::DEFENDER].into_iter().enumerate() {
        let read = field::<u64>(memory, pointer, offset).map(|at| at as usize).filter(|&at| {
            is(memory, base, at, expected) && field::<u64>(memory, at, side::COMBAT) == Some(pointer as u64)
        });
        let array = read.and_then(|at| field::<[u8; 16]>(memory, at, side::UNITS)).and_then(|header| Array::from_header(header, 8));
        match array {
            Some(array) => units[n] = array,
            None => return Read::Unreadable(handle),
        }
    }
    Read::Land(Sides { handle, units })
}

fn manpower(memory: &impl Memory, army: usize) -> Option<i32> {
    let entries = Array::read(memory, army.checked_add(carmy::MANPOWER_DATA)?, manpower_value_entry::STRIDE)?;
    let mut total = 0_i64;
    for index in 0..entries.count {
        let entry = entries.element(index, manpower_value_entry::STRIDE)?;
        total += i64::from(field::<i32>(memory, entry, manpower_value_entry::COUNT)?);
    }
    i32::try_from(total).ok()
}

fn counter(memory: &impl Memory, army: usize) -> Option<i32> {
    unit_history::career_counter(memory, army).ok()
}

fn division(memory: &impl Memory, base: usize, army: usize) -> Option<u64> {
    is(memory, base, army, carmy::VTABLE_RVA).then(|| field::<u64>(memory, army, cunit::HANDLE)).flatten().filter(|&handle| handle != 0)
}

pub struct Opening {
    hour: Option<i32>,
    combat: u64,
    withheld: u8,
    units: [Array; 2],
    pub words: usize,
}

impl Opening {
    pub fn read(memory: &impl Memory, base: usize, pointer: usize) -> Option<Self> {
        let hour = hour(memory, base);
        let (combat, withheld, units) = match sides(memory, base, pointer) {
            Read::NotLand | Read::Unregistered => return None,
            Read::Unreadable(combat) => (combat, COMBAT_UNREADABLE, [Array { data: 0, count: 0 }; 2]),
            Read::Land(sides) => (sides.handle, 0, sides.units),
        };
        let members = units[0].count as usize + units[1].count as usize;
        Some(Self { hour, combat, withheld, units, words: OPENING_WORDS + MEMBER_WORDS * members })
    }

    pub fn copy(&self, memory: &impl Memory, base: usize, put: &mut dyn FnMut(u64)) {
        put(OPENED);
        put(self.combat);
        put(optional(self.hour));
        put(u64::from(self.withheld));
        put(self.units[0].count as u64 + self.units[1].count as u64);
        for (n, units) in self.units.iter().enumerate() {
            for ordinal in 0..units.count {
                let army = units.element(ordinal, 8).and_then(|at| field::<u64>(memory, at, 0)).map_or(0, |army| army as usize);
                let handle = division(memory, base, army);
                let withheld = if handle.is_some() { 0 } else { DIVISION_UNREADABLE };
                put(n as u64 | u64::from(withheld) << 8);
                put(handle.unwrap_or(0));
                put(optional(handle.and_then(|_| manpower(memory, army))));
                put(optional(handle.and_then(|_| counter(memory, army))));
            }
        }
    }
}

#[derive(Clone, Copy)]
pub struct Exit {
    hour: Option<i32>,
    combat: u64,
    withheld: u8,
    side: u8,
    division: u64,
    before: Option<i32>,
}

impl Exit {
    pub fn before(memory: &impl Memory, base: usize, army: usize, pointer: usize) -> Option<Self> {
        let hour = hour(memory, base);
        let (combat, mut withheld, units) = match sides(memory, base, pointer) {
            Read::NotLand | Read::Unregistered => return None,
            Read::Unreadable(combat) => (combat, COMBAT_UNREADABLE, None),
            Read::Land(sides) => (sides.handle, 0, Some(sides.units)),
        };
        let division = division(memory, base, army);
        if division.is_none() {
            withheld |= DIVISION_UNREADABLE;
        }
        let side = units.and_then(|units| {
            units.iter().position(|units| (0..units.count).any(|ordinal| {
                units.element(ordinal, 8).and_then(|at| field::<u64>(memory, at, 0)) == Some(army as u64)
            }))
        });
        if units.is_some() && side.is_none() {
            withheld |= SIDE_UNKNOWN;
        }
        Some(Self {
            hour,
            combat,
            withheld,
            side: side.map_or(NO_SIDE, |side| side as u8),
            division: division.unwrap_or(0),
            before: division.and_then(|_| counter(memory, army)),
        })
    }

    pub fn copy(&self, memory: &impl Memory, base: usize, army: usize, put: &mut dyn FnMut(u64)) {
        let read = self.division != 0 && division(memory, base, army) == Some(self.division);
        put(EXITED);
        put(self.combat);
        put(optional(self.hour));
        put(u64::from(self.withheld));
        put(u64::from(self.side));
        put(self.division);
        put(optional(self.before));
        put(optional(read.then(|| counter(memory, army)).flatten()));
        put(optional(read.then(|| manpower(memory, army)).flatten()));
    }
}

#[cfg(test)]
#[path = "land_edges_tests.rs"]
mod tests;
