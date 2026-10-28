//! The loaded defines the movement speed's supply, fuel, retreat and air terms and the
//! attrition hover's supply line read, each read once per accepted hourly pass from its
//! live global, so a define reload or a mod's value is the recorded one. Each leaf is
//! attempted once and refused alone.
use crate::{Memory, carmy_attrition, read};

/// One loaded define: its recorded name, global and width.
#[derive(Clone, Copy, Debug)]
pub struct Define {
    pub name: &'static str,
    pub rva: usize,
    /// A signed 32-bit global, sign-extended; otherwise a signed fixed5 word.
    pub narrow: bool,
}

const fn wide(name: &'static str, rva: usize) -> Define {
    Define {
        name,
        rva,
        narrow: false,
    }
}

/// In the order of the recorded `MovementDefines` fields.
pub const DEFINES: [Define; 11] = [
    // The out-of-supply speed penalty at full severity.
    wide("out_of_supply_speed", OUT_OF_SUPPLY_SPEED_RVA),
    // The retreat speed bonus and the withdrawal speed term.
    wide("retreat_speed", RETREAT_SPEED_RVA),
    wide("withdrawal_speed", WITHDRAWAL_SPEED_RVA),
    // The fuel ratio's floor and the capacity scale of the stat block's fuel word.
    wide("out_of_fuel_speed", OUT_OF_FUEL_SPEED_RVA),
    wide("fuel_capacity_scale", FUEL_CAPACITY_SCALE_RVA),
    // Days out of supply over this give the full penalty; hours of supply grace.
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
    // The enemy air superiority speed scale and the anti-air reduction's operands.
    wide("air_speed", AIR_SPEED_RVA),
    wide(
        "air_defense_denominator",
        crate::csub_unit_definition::AIR_DEFENSE_DENOMINATOR_RVA,
    ),
    wide(
        "air_defense_reduction",
        crate::csub_unit_definition::AIR_DEFENSE_REDUCTION_RVA,
    ),
    // The attrition hover's out-of-supply multiplier.
    wide(
        "out_of_supply_attrition",
        carmy_attrition::OUT_OF_SUPPLY_ATTRITION_RVA,
    ),
];

/// The out-of-supply speed penalty at full severity.
pub const OUT_OF_SUPPLY_SPEED_RVA: usize = 0x033e_3990;
/// The retreating speed term.
pub const RETREAT_SPEED_RVA: usize = 0x033e_3c20;
/// The term a withdrawing (non-retreating) division adds.
pub const WITHDRAWAL_SPEED_RVA: usize = 0x033e_3cb8;
/// The fuel ratio's floor.
pub const OUT_OF_FUEL_SPEED_RVA: usize = 0x033e_4690;
/// The scale of the stat block's fuel word into the fuel ratio's capacity.
pub const FUEL_CAPACITY_SCALE_RVA: usize = 0x033e_3e60;
/// The enemy air superiority speed scale.
pub const AIR_SPEED_RVA: usize = 0x033e_1850;

/// Each define's value, refused alone.
pub fn capture(memory: &impl Memory, base: usize) -> [Result<i64, &'static str>; DEFINES.len()] {
    DEFINES.map(|define| {
        let at = base
            .checked_add(define.rva)
            .ok_or("movement define address overflow")?;
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
            let Some(index) = DEFINES.iter().position(|define| define.rva == address) else {
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
                    (define.rva, if define.narrow { 4 } else { 8 })
                );
                if index == refuse {
                    assert!(value.is_err());
                } else {
                    // A narrow global is sign-extended.
                    assert_eq!(value, Ok(-(index as i64) * 1_000 - 1));
                }
            }
        }
    }
}
