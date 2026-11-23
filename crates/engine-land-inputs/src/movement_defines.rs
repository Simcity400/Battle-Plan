use engine_native_memory::{Memory, read};
use crate::carmy_attrition;
use engine_sites::Rva;

#[derive(Clone, Copy, Debug)]
pub struct Define {
    pub name: &'static str,
    pub rva: Rva,
    pub narrow: bool,
}

const fn wide(name: &'static str, rva: Rva) -> Define {
    Define {
        name,
        rva,
        narrow: false,
    }
}

pub const DEFINES: [Define; 11] = [
    wide("out_of_supply_speed", OUT_OF_SUPPLY_SPEED_RVA),
    wide("retreat_speed", RETREAT_SPEED_RVA),
    wide("withdrawal_speed", WITHDRAWAL_SPEED_RVA),
    wide("out_of_fuel_speed", OUT_OF_FUEL_SPEED_RVA),
    wide("fuel_capacity_scale", FUEL_CAPACITY_SCALE_RVA),
    Define {
        name: "supply_days",
        rva: carmy_attrition::MAX_OUT_OF_SUPPLY_DAYS_RVA,
        narrow: true,
    },
    Define {
        name: "supply_grace",
        rva: carmy_attrition::SUPPLY_GRACE_RVA,
        narrow: true,
    },
    wide("air_speed", AIR_SPEED_RVA),
    wide(
        "air_defense_denominator",
        engine_native_parts::csub_unit_definition::AIR_DEFENSE_DENOMINATOR_RVA,
    ),
    wide(
        "air_defense_reduction",
        engine_native_parts::csub_unit_definition::AIR_DEFENSE_REDUCTION_RVA,
    ),
    wide(
        "out_of_supply_attrition",
        carmy_attrition::OUT_OF_SUPPLY_ATTRITION_RVA,
    ),
];

pub const OUT_OF_SUPPLY_SPEED_RVA: Rva = Rva(0x033e_3990);
pub const RETREAT_SPEED_RVA: Rva = Rva(0x033e_3c20);
pub const WITHDRAWAL_SPEED_RVA: Rva = Rva(0x033e_3cb8);
pub const OUT_OF_FUEL_SPEED_RVA: Rva = Rva(0x033e_4690);
pub const FUEL_CAPACITY_SCALE_RVA: Rva = Rva(0x033e_3e60);
pub const AIR_SPEED_RVA: Rva = Rva(0x033e_1850);

pub fn capture(memory: &impl Memory, base: usize) -> [Result<i64, &'static str>; DEFINES.len()] {
    DEFINES.map(|define| {
        let at = engine_sites::at(base, define.rva).ok_or("movement define address overflow")?;
        if define.narrow {
            read::<i32>(memory, at).map(i64::from)
        } else {
            read::<i64>(memory, at)
        }
        .ok_or("movement define unreadable")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Source {
        refuse: usize,
        calls: RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for Source {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.calls.borrow_mut().push((address, output.len()));
            let Some(index) = DEFINES.iter().position(|define| define.rva.0 == address) else {
                return false;
            };
            if index == self.refuse {
                return false;
            }
            let value = -(index as i64) * 1_000 - 1;
            if DEFINES[index].narrow {
                output.copy_from_slice(&(value as i32).to_le_bytes());
            } else {
                output.copy_from_slice(&value.to_le_bytes());
            }
            true
        }
    }

    #[test]
    fn each_define_is_read_once_at_its_width_and_refused_alone() {
        for refuse in 0..=DEFINES.len() {
            let source = Source {
                refuse,
                calls: RefCell::new(Vec::new()),
            };
            let values = capture(&source, 0);
            let calls = source.calls.borrow();
            assert_eq!(calls.len(), DEFINES.len());
            for (index, (define, value)) in DEFINES.iter().zip(values).enumerate() {
                assert_eq!(
                    calls[index],
                    (define.rva.0, if define.narrow { 4 } else { 8 })
                );
                if index == refuse {
                    assert!(value.is_err());
                } else {
                    assert_eq!(value, Ok(-(index as i64) * 1_000 - 1));
                }
            }
        }
    }
}
