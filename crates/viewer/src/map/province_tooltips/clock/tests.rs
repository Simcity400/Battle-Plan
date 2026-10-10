use super::*;

fn defines() -> Defines {
    Defines {
        gmt_offset: 8,
        poles: [0, 100_000],
        feather: 20_000,
    }
}

#[test]
fn native_fixed_hour_quantization_and_night_frames_are_preserved() {
    let start = 43_800_000 + 172 * 24;
    let cases = [
        (0, 1, 100_000, 2),
        (6, 7, 50_000, 2),
        (12, 13, 0, 1),
        (18, 19, 50_000, 2),
        (23, 23, 100_000, 2),
    ];
    for (utc, local, night, frame) in cases {
        let clock = derive(start + utc, [8, 20], [80, 40], &defines(), Some([0, 0])).unwrap();
        assert_eq!(
            clock,
            LocalClock {
                hour: local,
                night,
                icon_frame: frame
            }
        );
    }
}

#[test]
fn longitude_is_native_centroid_fixed_arithmetic() {
    let start = 43_800_000 + 172 * 24;
    let west = derive(start, [7, 20], [80, 40], &defines(), Some([0, 0])).unwrap();
    let east = derive(start, [9, 20], [80, 40], &defines(), Some([0, 0])).unwrap();
    assert_eq!(west.hour, 24);
    assert_eq!(east.hour, 1);
    assert_eq!(normalize([3, 4, 0]), [60_000, 80_000, 0]);
}

#[test]
fn suspension_or_missing_flags_withhold_both_outputs_by_name() {
    for flags in [None, Some([1, 0]), Some([0, 1])] {
        let error = derive(43_800_000, [8, 20], [80, 40], &defines(), flags).unwrap_err();
        assert!(error.starts_with("Province local clock/night refused:"));
        assert!(error.contains("suspend") || error.contains("suspension flags unavailable"));
    }
}

#[test]
fn stored_pair_does_not_consult_current_hour_flags_or_content() {
    // Newly allocated native rows start with hour zero and night at one.
    for (hour, night, frame) in [(0, 100_000, 2), (24, 49_999, 1), (7, 50_000, 2)] {
        let province = WeatherProvinceRow {
            local_hour: hour.into(),
            night_factor: night.into(),
            ..Default::default()
        };
        // The historical calculation is deliberately unavailable: a stored
        // pair must not consult simulation flags, current hour or content.
        let clock = resolve(&province, || panic!("stored clock used derivation")).unwrap();
        assert_eq!(
            clock,
            LocalClock {
                hour,
                night,
                icon_frame: frame
            }
        );
    }
}

#[test]
fn old_recordings_use_derivation_and_keep_suspension_refusals() {
    let province = WeatherProvinceRow {
        local_hour: Recorded::NotRecorded,
        night_factor: Recorded::NotRecorded,
        ..Default::default()
    };
    let historical = || {
        derive(
            43_800_000 + 172 * 24,
            [8, 20],
            [80, 40],
            &defines(),
            Some([0, 0]),
        )
    };
    assert_eq!(resolve(&province, historical), historical());
    assert!(
        resolve(&province, || derive(
            43_800_000,
            [8, 20],
            [80, 40],
            &defines(),
            Some([1, 0])
        ))
        .unwrap_err()
        .contains("suspended")
    );
    assert!(
        resolve(&province, || Err("simulation features absent".into()))
            .unwrap_err()
            .contains("simulation features absent")
    );
}

#[test]
fn partial_recorded_pair_is_not_replaced_by_current_hour() {
    for (local_hour, night_factor) in [
        (Recorded::Value(0), Recorded::NotRecorded),
        (Recorded::NotRecorded, Recorded::Value(0)),
    ] {
        let province = WeatherProvinceRow {
            local_hour,
            night_factor,
            ..Default::default()
        };
        assert!(
            resolve(&province, || panic!("partial pair used derivation"))
                .unwrap_err()
                .contains("incomplete recorded pair")
        );
    }
}

#[test]
fn held_night_uses_native_signed_rounding_without_clamping() {
    for (night, frame) in [
        (-50_000, 0),
        (-49_999, 1),
        (49_999, 1),
        (50_000, 2),
        (150_000, 3),
    ] {
        assert_eq!(stored(0, night).icon_frame, frame);
    }
}

#[test]
fn content_defines_keep_native_conversions() {
    let raw = installed_content::defines::Defines::new().unwrap();
    raw.load(
        "clock.lua",
        br#"
        NDefines = { NGraphics = {
            GMT_OFFSET = 8.9, SOUTH_POLE_OFFSET = 0,
            NORTH_POLE_OFFSET = 1, DAY_NIGHT_FEATHER = 0.2
        } }
    "#,
    )
    .unwrap();
    let loaded = Defines::load(&raw).unwrap();
    assert_eq!(loaded.gmt_offset, 8);
    assert_eq!(loaded.poles, [0, 100_000]);
    assert_eq!(loaded.feather, 20_000);
}
