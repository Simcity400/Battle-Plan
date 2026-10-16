use replay::rows::{GarrisonEquipment, Recorded, StateGarrisonRow};

const ONE: i64 = 100_000;

/// Displayed garrison strength, separate from the saved strength cache.
pub(super) fn displayed(
    row: &StateGarrisonRow,
    assigned: Option<i32>,
    powers: &Result<(i32, i32), String>,
    mut validate: impl FnMut(&GarrisonEquipment) -> Result<(), String>,
) -> Result<i64, String> {
    if row.serialized_flag != 0 {
        return Ok(0);
    }
    let assigned = assigned.ok_or_else(|| {
        "StateAssignedManpower is incomplete or unresolved at this read".to_owned()
    })?;
    if assigned == 0 {
        return Ok(0);
    }
    let inputs = match &row.equipment {
        Recorded::NotRecorded => {
            return Err("This recording lacks the garrison equipment strength operands".into());
        }
        Recorded::Value(inputs) => inputs,
    };
    if let Some(reason) = &inputs.failure {
        return Err(String::from_utf8_lossy(reason).into_owned());
    }
    let &(manpower_power, equipment_power) = powers.as_ref().map_err(Clone::clone)?;
    for entry in &inputs.rows {
        validate(entry)?;
    }
    let equipment = equipment_fraction(&inputs.rows);
    let manpower = if row.required_manpower > 0 {
        let denominator = i64::from(row.required_manpower).wrapping_mul(ONE);
        let numerator = i64::from(assigned).wrapping_mul(ONE);
        let fraction = if denominator == 0 || numerator <= -0x7fff_ffff_ffff_ffff {
            i64::from(u32::MAX)
        } else {
            numerator.wrapping_mul(ONE) / denominator
        };
        fraction.min(ONE)
    } else {
        ONE
    };
    Ok(ONE
        .wrapping_sub(shortage_power(ONE.wrapping_sub(equipment), equipment_power))
        .min(ONE.wrapping_sub(shortage_power(ONE.wrapping_sub(manpower), manpower_power))))
}

/// Low-64-bit products and sums, signed division toward zero,
/// with the allocation branch and unsigned divide sentinel guard.
fn equipment_fraction(entries: &[GarrisonEquipment]) -> i64 {
    let (mut allocated, mut required) = (0_i64, 0_i64);
    for entry in entries {
        let effective = if entry.allocated_raw < 0 {
            0
        } else {
            entry.allocated_raw.min(entry.required_lookup_raw)
        };
        allocated = allocated.wrapping_add(entry.weight_raw.wrapping_mul(effective) / ONE);
        required =
            required.wrapping_add(entry.weight_raw.wrapping_mul(entry.required_lookup_raw) / ONE);
    }
    if required <= 0 {
        ONE
    } else if (allocated as u64).wrapping_add(0x7fff_ffff_ffff_fffe) > 0xffff_ffff_ffff_fffc {
        i64::from(u32::MAX)
    } else {
        allocated.wrapping_mul(ONE) / required
    }
}

/// NResistance equipment and manpower powers are signed i32 values.
/// Every step rounds separately; exponentiation by squaring would
/// change the answer.
fn shortage_power(shortage: i64, power: i32) -> i64 {
    let mut result = shortage;
    for _ in 1..power {
        let next = shortage.wrapping_mul(result) / ONE;
        if next == result {
            break;
        }
        result = next;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::GarrisonEquipmentInputs;

    fn row(allocated: i64, required: i64, weight: i64) -> StateGarrisonRow {
        StateGarrisonRow {
            required_manpower: 100,
            strength_ratio: 7,
            equipment: Recorded::Value(GarrisonEquipmentInputs {
                rows: vec![GarrisonEquipment {
                    allocated_raw: allocated,
                    required_lookup_raw: required,
                    required_raw: required,
                    weight_raw: weight,
                    ..Default::default()
                }],
                failure: None,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn strength_uses_weighted_equipment_and_content_powers_with_each_fixed5_step() {
        let half = row(50_000, 100_000, 200_000);
        assert_eq!(
            displayed(&half, Some(100), &Ok((2, 3)), |_| Ok(())),
            Ok(87_500)
        );
        assert_eq!(
            displayed(&half, Some(50), &Ok((2, 3)), |_| Ok(())),
            Ok(75_000)
        );
        assert_eq!(
            displayed(&half, Some(100), &Ok((1, 1)), |_| Ok(())),
            Ok(50_000)
        );
        assert_eq!(
            displayed(
                &row(100_000, 100_000, 200_000),
                Some(100),
                &Ok((2, 3)),
                |_| Ok(())
            ),
            Ok(100_000)
        );
        assert_eq!(shortage_power(33_333, 3), 3_703);
        let mut weighted = row(100_000, 100_000, 400_000);
        let Recorded::Value(inputs) = &mut weighted.equipment else {
            unreachable!()
        };
        inputs.rows.push(GarrisonEquipment {
            allocated_raw: 0,
            required_lookup_raw: 100_000,
            weight_raw: 100_000,
            ..Default::default()
        });
        assert_eq!(
            displayed(&weighted, Some(100), &Ok((1, 1)), |_| Ok(())),
            Ok(80_000)
        );
        let Recorded::Value(inputs) = row(i64::MAX, i64::MAX, i64::MAX).equipment else {
            unreachable!()
        };
        assert_eq!(equipment_fraction(&inputs.rows), 100_000);
    }

    #[test]
    fn signed_corrections_and_nonpositive_requirements_keep_native_branches() {
        assert_eq!(
            displayed(&row(-1, 100_000, 100_000), Some(100), &Ok((1, 1)), |_| Ok(
                ()
            )),
            Ok(0)
        );
        assert_eq!(
            displayed(&row(0, 0, 100_000), Some(100), &Ok((1, 1)), |_| Ok(())),
            Ok(100_000)
        );
        assert_eq!(
            displayed(
                &row(50_000, -100_000, 100_000),
                Some(100),
                &Ok((1, 1)),
                |_| Ok(())
            ),
            Ok(100_000)
        );
        assert_eq!(
            displayed(
                &row(100_000, 100_000, 100_000),
                Some(-50),
                &Ok((1, 1)),
                |_| Ok(())
            ),
            Ok(-50_000)
        );
        assert_eq!(
            displayed(
                &row(150_000, 100_000, 100_000),
                Some(100),
                &Ok((1, 1)),
                |_| Ok(())
            ),
            Ok(100_000)
        );
    }

    #[test]
    fn early_zero_bypasses_equipment_content_and_unknowns_remain_named() {
        let mut missing = StateGarrisonRow {
            equipment: Recorded::NotRecorded,
            ..Default::default()
        };
        let unknown = Err("GARRISON_STR_POW_EQUIPMENT missing".into());
        assert_eq!(
            displayed(&missing, Some(0), &unknown, |_| unreachable!()),
            Ok(0)
        );
        missing.serialized_flag = 254;
        assert_eq!(
            displayed(&missing, None, &unknown, |_| unreachable!()),
            Ok(0)
        );
        missing.serialized_flag = 0;
        assert!(
            displayed(&missing, Some(1), &Ok((2, 3)), |_| Ok(()))
                .unwrap_err()
                .contains("lacks")
        );
        let mut complete = row(1, 1, 1);
        complete.equipment = Recorded::Value(GarrisonEquipmentInputs {
            rows: vec![],
            failure: None,
        });
        assert_eq!(
            displayed(&complete, Some(100), &Ok((2, 3)), |_| unreachable!()),
            Ok(100_000)
        );
        assert!(
            displayed(&row(1, 1, 1), Some(100), &Ok((2, 3)), |_| Err(
                "unknown mod equipment".into()
            ))
            .unwrap_err()
            .contains("unknown mod equipment")
        );
    }
}
