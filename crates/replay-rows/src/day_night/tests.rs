use super::*;

fn defines() -> Defines {
    Defines {
        gmt_offset: 8,
        south_pole_offset: 0,
        north_pole_offset: 100_000,
        day_night_feather: 20_000,
    }
}

#[test]
fn whole_day_forecast_matches_each_native_slot_with_a_held_season() {
    let defines = defines();
    let clock = ProvinceClock::new([31, 17], [80, 40], &defines);
    let day = EPOCH + 172 * 24;
    let factors = clock.forecast(
        seasonal_direction(day),
        forecast_feather(defines.day_night_feather),
    );
    for (slot, expected) in factors.into_iter().enumerate() {
        assert_eq!(
            clock
                .update(&HourOperands::new(day + slot as i32, &defines).unwrap())
                .night_factor,
            expected
        );
    }
    assert_ne!(
        factors,
        clock.forecast(
            seasonal_direction(EPOCH),
            forecast_feather(defines.day_night_feather)
        )
    );
    let zeros = clock.forecast([0; 3], forecast_feather(defines.day_night_feather));
    assert!(zeros.iter().all(|&value| value == 50_000));
}

fn clock(hour: i32, centroid: [i32; 2]) -> Clock {
    let defines = defines();
    ProvinceClock::new(centroid, [80, 40], &defines)
        .update(&HourOperands::new(hour, &defines).unwrap())
}

#[test]
fn native_fixed_hour_quantization_and_night_frames_are_preserved() {
    let start = EPOCH + 172 * 24;
    for (utc, local_hour, night_factor, frame) in [
        (0, 1, 100_000, 2),
        (6, 7, 50_000, 2),
        (12, 13, 0, 1),
        (18, 19, 50_000, 2),
        (23, 23, 100_000, 2),
    ] {
        let clock = clock(start + utc, [8, 20]);
        assert_eq!(
            clock,
            Clock {
                local_hour,
                night_factor
            }
        );
        assert_eq!(clock.icon_frame(), frame);
    }
}

#[test]
fn longitude_is_native_centroid_fixed_arithmetic() {
    let start = EPOCH + 172 * 24;
    assert_eq!(clock(start, [7, 20]).local_hour, 24);
    assert_eq!(clock(start, [9, 20]).local_hour, 1);
    assert_eq!(normalize([3, 4, 0]), [60_000, 80_000, 0]);
}

#[test]
fn every_record_uses_the_hour_index_and_the_dates_season() {
    let defines = defines();
    let record = ProvinceClock::new([30, 5], [80, 40], &defines);
    let summer = HourOperands::new(EPOCH + 172 * 24 + 3, &defines).unwrap();
    let winter = HourOperands::new(EPOCH + 355 * 24 + 3, &defines).unwrap();
    assert_eq!(summer.index, winter.index);
    assert_ne!(summer.direction, winter.direction);
    assert_eq!(
        record.update(&summer).local_hour,
        record.update(&winter).local_hour
    );
    assert_ne!(
        record.update(&summer).night_factor,
        record.update(&winter).night_factor
    );
}

#[test]
fn dates_before_the_epoch_are_refused() {
    assert!(HourOperands::new(EPOCH - 1, &defines()).is_err());
    assert!(HourOperands::new(EPOCH, &defines()).is_ok());
}

#[test]
fn zero_map_dimensions_store_the_native_refusal() {
    let record = ProvinceClock::new([8, 20], [0, 0], &defines());
    assert_eq!(record.phase, REFUSED + SCALE);
}

#[test]
fn a_zero_feather_stores_the_refused_scale() {
    let defines = Defines {
        day_night_feather: 0,
        ..defines()
    };
    assert_eq!(HourOperands::new(EPOCH, &defines).unwrap().scale, REFUSED);
}

#[test]
fn seasonal_turns_use_zero_based_dates_and_truncated_fixed_point() {
    assert_eq!(seasonal_rotation(172 * 24), -500_000);
    assert_eq!(seasonal_rotation(355 * 24), 500_000);
    assert_eq!(seasonal_rotation(173 * 24), -494_540);
    assert_eq!(seasonal_rotation(356 * 24), 494_510);
    assert_eq!(seasonal_rotation(0), 445_060);
    for day in 0..365 {
        let hour = day * 24;
        assert_eq!(seasonal_direction(hour), seasonal_direction(hour + 23));
        assert_eq!(
            seasonal_direction(hour),
            seasonal_direction(hour + 365 * 24)
        );
        assert_eq!(
            seasonal_direction(hour),
            seasonal_direction(hour - 365 * 24)
        );
        assert!(seasonal_rotation(hour).abs() <= 500_000);
        assert_eq!(seasonal_direction(hour)[0], 0);
    }
}

#[test]
fn trigonometry_keeps_native_quadrants_and_polynomial_error() {
    assert_eq!(sine(0), 0);
    assert_eq!(sine(157_080), 100_453);
    assert_eq!(sine(314_159), 1);
    assert_eq!(sine(628_319), 0);
    assert_eq!(sine(-157_080), -100_453);
    assert_eq!(cosine(0), 100_453);
    assert_eq!(seasonal_direction(264 * 24)[2], SCALE);
}
