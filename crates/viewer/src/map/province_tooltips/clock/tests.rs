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
