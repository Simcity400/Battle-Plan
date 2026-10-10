use super::*;

#[test]
fn cache_and_supply_terms_keep_native_truncation_and_branch_reach() {
    let defines = Defines {
        base: 3,
        radar_bonus: 50000,
        air_superiority_bonus: 25000,
    };
    assert_eq!(
        detection_multiplier([-3, 7], defines, false, Err("unreached")),
        Ok(3000)
    );
    assert_eq!(
        detection_multiplier([-3, 7], defines, true, Ok(Some(50000))),
        Ok(1500)
    );
    assert_eq!(
        detection_multiplier([-3, 7], defines, true, Ok(Some(200000))),
        Ok(3000)
    );
    assert_eq!(
        detection_multiplier([-3, 7], defines, true, Ok(None)),
        Ok(0)
    );
    assert_eq!(
        detection_multiplier([-3, 7], defines, true, Err("original supply refusal")),
        Err("original supply refusal")
    );
}

#[test]
fn negative_supply_is_not_clamped_and_products_wrap_before_division() {
    let defines = Defines {
        base: 100,
        radar_bonus: 0,
        air_superiority_bonus: 0,
    };
    assert_eq!(
        detection_multiplier([0, 0], defines, true, Ok(Some(-100000))),
        Ok(-100000)
    );
    let defines = Defines {
        base: 0,
        radar_bonus: 2,
        air_superiority_bonus: 0,
    };
    assert_eq!(
        detection_multiplier([i64::MAX, 0], defines, false, Err("unreached")),
        Ok(0)
    );
}

#[test]
fn defined_names_come_from_the_recorded_stack_in_native_field_order() {
    let mut calls = Vec::new();
    let found = Defines::read(|domain, key| {
        calls.push((domain, key));
        Ok::<_, &'static str>(calls.len() as i64)
    })
    .unwrap();
    assert_eq!(
        found,
        Defines {
            base: 1,
            radar_bonus: 2,
            air_superiority_bonus: 3
        }
    );
    assert_eq!(
        calls,
        [
            ("NNavy", "DETECTION_CHANCE_MULT_BASE"),
            ("NNavy", "DETECTION_CHANCE_MULT_RADAR_BONUS"),
            ("NNavy", "DETECTION_CHANCE_MULT_AIR_SUPERIORITY_BONUS"),
        ]
    );
}
