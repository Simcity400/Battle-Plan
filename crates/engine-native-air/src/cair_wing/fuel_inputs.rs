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
    let mut out = Inputs {
        base_fuel: field(memory, wing, BASE_FUEL),
        ..Default::default()
    };
    if engine_sites::at(base, air_mission::VTABLE_RVA).map(|at| at as u64)
        != field(memory, wing, MISSION)
    {
        return out;
    }
    out.current_type = field(memory, wing, MISSION + air_mission::CURRENT_TYPE);
    match out.current_type {
        Some(current) if current != 0 => {
            out.effective_aircraft =
                field(memory, wing, MISSION + air_mission::EFFECTIVE_AIRCRAFTS);
            out.context_mode = field(memory, wing, MISSION + air_mission::CONTEXT_MODE);
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
            if out.override_resolved == Some(false)
                && field::<u64>(memory, wing, MISSION + air_mission::WING) == Some(wing as u64)
            {
                out.capacity_class = field(memory, wing, MISSION + air_mission::CAPACITY_CLASS);
                out.context_mode = field(memory, wing, MISSION + air_mission::CONTEXT_MODE);
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
}
