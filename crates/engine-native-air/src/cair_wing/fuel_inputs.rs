use super::{BASE_FUEL, MISSION, Row};
use engine_native_memory::{Memory, memory::Raw, read};
use crate::air_mission;
use engine_native_parts::creference_registry::ScopedResolver;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Inputs {
    pub base_fuel: Option<i64>,
    pub current_type: Option<i32>,
    pub effective_aircraft: Option<i32>,
    pub context_mode: Option<u8>,
    pub capacity_class: Option<i32>,
    pub override_resolved: Option<bool>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Gathered {
    pub base_fuel: i64,
    pub mission_vtable: u64,
    pub current_type: i32,
    pub effective_aircraft: i32,
    pub context_mode: u8,
    pub wing_backlink: u64,
    pub capacity_class: i32,
}

pub fn from_gathered(
    memory: &impl Memory,
    base: usize,
    wing: usize,
    row: &Row,
    gathered: &Gathered,
    references: &mut ScopedResolver,
) -> Inputs {
    let expected_vtable = engine_sites::at(base, air_mission::VTABLE_RVA).map(|at| at as u64);
    let mut out = Inputs {
        base_fuel: Some(gathered.base_fuel),
        ..Default::default()
    };
    if expected_vtable != Some(gathered.mission_vtable) {
        return out;
    }
    out.current_type = Some(gathered.current_type);
    match gathered.current_type {
        current if current != 0 => {
            out.effective_aircraft = Some(gathered.effective_aircraft);
            out.context_mode = Some(gathered.context_mode);
        }
        0 if row.mission_mask & air_mission::TRAINING_MASK != 0
            && row.eligibility_veto == 0
            && row.eligibility_counter <= row.eligibility_limit
            && row.planes > 0 =>
        {
            out.override_resolved = if row.base_override == 0 {
                Some(false)
            } else {
                references
                    .resolve(memory, row.base_override)
                    .ok()
                    .map(|value| value.is_some())
            };
            if out.override_resolved == Some(false) && gathered.wing_backlink == wing as u64 {
                out.capacity_class = Some(gathered.capacity_class);
                out.context_mode = Some(gathered.context_mode);
            }
        }
        _ => {}
    }
    out
}

fn field<T: Raw>(memory: &impl Memory, wing: usize, offset: usize) -> Option<T> {
    wing.checked_add(offset).and_then(|at| read(memory, at))
}

pub fn capture(
    memory: &impl Memory,
    base: usize,
    wing: usize,
    row: &Row,
    references: &mut ScopedResolver,
) -> Inputs {
    let expected_vtable = engine_sites::at(base, air_mission::VTABLE_RVA).map(|at| at as u64);
    let (base_fuel, vtable, current_type) = match (
        wing.checked_add(BASE_FUEL),
        wing.checked_add(MISSION),
        wing.checked_add(MISSION + air_mission::CURRENT_TYPE),
    ) {
        (Some(fuel_at), Some(vtable_at), Some(type_at)) => {
            let mut fuel_buf = [0_u8; 8];
            let mut vtable_buf = [0_u8; 8];
            let mut type_buf = [0_u8; 4];
            let mut destinations: [&mut [u8]; 3] = [&mut fuel_buf, &mut vtable_buf, &mut type_buf];
            if memory.copy_scattered([fuel_at, vtable_at, type_at], &mut destinations) == 3 {
                let vtable = u64::from_le_bytes(vtable_buf);
                (
                    Some(i64::from_le_bytes(fuel_buf)),
                    Some(vtable),
                    if expected_vtable == Some(vtable) {
                        Some(i32::from_le_bytes(type_buf))
                    } else {
                        None
                    },
                )
            } else {
                let base_fuel = field(memory, wing, BASE_FUEL);
                let vtable = field(memory, wing, MISSION);
                let current_type = if expected_vtable == vtable {
                    field(memory, wing, MISSION + air_mission::CURRENT_TYPE)
                } else {
                    None
                };
                (base_fuel, vtable, current_type)
            }
        }
        _ => {
            let base_fuel = field(memory, wing, BASE_FUEL);
            let vtable = field(memory, wing, MISSION);
            let current_type = if expected_vtable == vtable {
                field(memory, wing, MISSION + air_mission::CURRENT_TYPE)
            } else {
                None
            };
            (base_fuel, vtable, current_type)
        }
    };
    let mut out = Inputs {
        base_fuel,
        ..Default::default()
    };
    if expected_vtable != vtable {
        return out;
    }
    out.current_type = current_type;
    match out.current_type {
        Some(current) if current != 0 => {
            match (
                wing.checked_add(MISSION + air_mission::EFFECTIVE_AIRCRAFTS),
                wing.checked_add(MISSION + air_mission::CONTEXT_MODE),
            ) {
                (Some(eff_at), Some(mode_at)) => {
                    let mut eff_buf = [0_u8; 4];
                    let mut mode_buf = [0_u8; 1];
                    let mut destinations: [&mut [u8]; 2] = [&mut eff_buf, &mut mode_buf];
                    if memory.copy_scattered([eff_at, mode_at], &mut destinations) == 2 {
                        out.effective_aircraft = Some(i32::from_le_bytes(eff_buf));
                        out.context_mode = Some(mode_buf[0]);
                    } else {
                        out.effective_aircraft =
                            field(memory, wing, MISSION + air_mission::EFFECTIVE_AIRCRAFTS);
                        out.context_mode =
                            field(memory, wing, MISSION + air_mission::CONTEXT_MODE);
                    }
                }
                _ => {
                    out.effective_aircraft =
                        field(memory, wing, MISSION + air_mission::EFFECTIVE_AIRCRAFTS);
                    out.context_mode = field(memory, wing, MISSION + air_mission::CONTEXT_MODE);
                }
            }
        }
        Some(0)
            if row.mission_mask & air_mission::TRAINING_MASK != 0
                && row.eligibility_veto == 0
                && row.eligibility_counter <= row.eligibility_limit
                && row.planes > 0 =>
        {
            out.override_resolved = if row.base_override == 0 {
                Some(false)
            } else {
                references
                    .resolve(memory, row.base_override)
                    .ok()
                    .map(|value| value.is_some())
            };
            if out.override_resolved == Some(false) {
                match (
                    wing.checked_add(MISSION + air_mission::WING),
                    wing.checked_add(MISSION + air_mission::CAPACITY_CLASS),
                    wing.checked_add(MISSION + air_mission::CONTEXT_MODE),
                ) {
                    (Some(wing_at), Some(cap_at), Some(mode_at)) => {
                        let mut wing_buf = [0_u8; 8];
                        let mut cap_buf = [0_u8; 4];
                        let mut mode_buf = [0_u8; 1];
                        let mut destinations: [&mut [u8]; 3] =
                            [&mut wing_buf, &mut cap_buf, &mut mode_buf];
                        if memory.copy_scattered([wing_at, cap_at, mode_at], &mut destinations) == 3
                        {
                            if u64::from_le_bytes(wing_buf) == wing as u64 {
                                out.capacity_class = Some(i32::from_le_bytes(cap_buf));
                                out.context_mode = Some(mode_buf[0]);
                            }
                        } else if field::<u64>(memory, wing, MISSION + air_mission::WING)
                            == Some(wing as u64)
                        {
                            out.capacity_class =
                                field(memory, wing, MISSION + air_mission::CAPACITY_CLASS);
                            out.context_mode =
                                field(memory, wing, MISSION + air_mission::CONTEXT_MODE);
                        }
                    }
                    _ => {
                        if field::<u64>(memory, wing, MISSION + air_mission::WING)
                            == Some(wing as u64)
                        {
                            out.capacity_class =
                                field(memory, wing, MISSION + air_mission::CAPACITY_CLASS);
                            out.context_mode =
                                field(memory, wing, MISSION + air_mission::CONTEXT_MODE);
                        }
                    }
                }
            }
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;
    use std::cell::RefCell;

    const WING: usize = 32;
    struct Traced {
        bytes: Vec<u8>,
        reads: RefCell<Vec<(usize, usize)>>,
        denied: Option<usize>,
    }
    impl Memory for Traced {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, output.len()));
            if self.denied == Some(address) {
                return false;
            }
            SliceMemory {
                base: 0,
                bytes: &self.bytes,
            }
            .copy(address, output)
        }
    }
    fn memory(current: i32) -> Traced {
        let mut bytes = vec![0; WING + BASE_FUEL + 16];
        bytes[WING + MISSION..WING + MISSION + 8]
            .copy_from_slice(&(air_mission::VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[WING + BASE_FUEL..WING + BASE_FUEL + 8].copy_from_slice(&123_456_i64.to_le_bytes());
        bytes[WING + MISSION + air_mission::CURRENT_TYPE
            ..WING + MISSION + air_mission::CURRENT_TYPE + 4]
            .copy_from_slice(&current.to_le_bytes());
        Traced {
            bytes,
            reads: Default::default(),
            denied: None,
        }
    }
    #[test]
    fn idle_fuel_keeps_the_stat_without_reading_stale_execution_or_training_words() {
        let memory = memory(0);
        let result = capture(
            &memory,
            0,
            WING,
            &Row::default(),
            &mut ScopedResolver::new(0),
        );
        assert_eq!(result.base_fuel, Some(123_456));
        assert_eq!(result.current_type, Some(0));
        assert_eq!(result.effective_aircraft, None);
        assert_eq!(result.context_mode, None);
        assert_eq!(result.capacity_class, None);
        assert!(
            !memory
                .reads
                .borrow()
                .iter()
                .any(|&(at, _)| at == WING + MISSION + air_mission::EFFECTIVE_AIRCRAFTS)
        );
    }
    #[test]
    fn execution_failures_are_independent_and_training_uses_existing_eligibility() {
        let mut memory = memory(4);
        memory.denied = Some(WING + MISSION + air_mission::EFFECTIVE_AIRCRAFTS);
        let result = capture(
            &memory,
            0,
            WING,
            &Row::default(),
            &mut ScopedResolver::new(0),
        );
        assert_eq!(result.base_fuel, Some(123_456));
        assert_eq!(result.current_type, Some(4));
        assert_eq!(result.effective_aircraft, None);
        assert_eq!(result.context_mode, Some(0));
        assert_eq!(result.capacity_class, None);

        let mut memory = self::memory(0);
        memory.bytes[WING + MISSION + air_mission::WING..WING + MISSION + air_mission::WING + 8]
            .copy_from_slice(&(WING as u64).to_le_bytes());
        let row = Row {
            mission_mask: air_mission::TRAINING_MASK,
            planes: 1,
            ..Default::default()
        };
        let result = capture(&memory, 0, WING, &row, &mut ScopedResolver::new(0));
        assert_eq!(result.capacity_class, Some(0));
        assert_eq!(result.effective_aircraft, None);
        let denied = Row {
            eligibility_veto: 1,
            ..row
        };
        assert_eq!(
            capture(&memory, 0, WING, &denied, &mut ScopedResolver::new(0)).capacity_class,
            None
        );
    }

    #[test]
    fn from_gathered_matches_capture_across_idle_active_and_training_wings() {
        for (current, mask, planes, veto, backlink) in [
            (0, 0, 10, 0, WING as u64),
            (4, 1, 10, 0, WING as u64),
            (0, air_mission::TRAINING_MASK, 1, 0, WING as u64),
            (0, air_mission::TRAINING_MASK, 1, 0, 0),
            (0, air_mission::TRAINING_MASK, 1, 1, WING as u64),
        ] {
            let mut memory = self::memory(current);
            memory.bytes[WING + MISSION + air_mission::EFFECTIVE_AIRCRAFTS
                ..WING + MISSION + air_mission::EFFECTIVE_AIRCRAFTS + 4]
                .copy_from_slice(&42_i32.to_le_bytes());
            memory.bytes[WING + MISSION + air_mission::CONTEXT_MODE] = 3;
            memory.bytes[WING + MISSION + air_mission::WING
                ..WING + MISSION + air_mission::WING + 8]
                .copy_from_slice(&backlink.to_le_bytes());
            memory.bytes[WING + MISSION + air_mission::CAPACITY_CLASS
                ..WING + MISSION + air_mission::CAPACITY_CLASS + 4]
                .copy_from_slice(&7_i32.to_le_bytes());
            let row = Row {
                mission_mask: mask,
                planes,
                eligibility_veto: veto,
                ..Default::default()
            };
            let gathered = Gathered {
                base_fuel: 123_456,
                mission_vtable: air_mission::VTABLE_RVA.0 as u64,
                current_type: current,
                effective_aircraft: 42,
                context_mode: 3,
                wing_backlink: backlink,
                capacity_class: 7,
            };
            let expected = capture(&memory, 0, WING, &row, &mut ScopedResolver::new(0));
            let actual = from_gathered(&memory, 0, WING, &row, &gathered, &mut ScopedResolver::new(0));
            assert_eq!(actual, expected);
        }
    }
}
