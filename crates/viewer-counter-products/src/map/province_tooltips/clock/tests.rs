use super::*;
use replay_data::rows::DayNightClock;

fn row(id: u32, clock: DayNightClock) -> WeatherProvinceRow {
    WeatherProvinceRow {
        id,
        clock,
        ..Default::default()
    }
}

fn clock(local_hour: i32, night_factor: i64) -> Clock {
    Clock {
        local_hour,
        night_factor,
    }
}

#[test]
fn held_clocks_do_not_consult_the_hour_gates_or_content() {
    for (hour, night, frame) in [(0, 100_000, 2), (24, 49_999, 1), (7, 50_000, 2)] {
        for province in [
            row(5, DayNightClock::held(clock(hour, night))),
            row(5, DayNightClock::historical(clock(hour, night))),
            row(0, DayNightClock::held(clock(hour, night))),
        ] {
            let resolved = resolve(
                &province,
                |_| panic!("held clock used derivation"),
                || panic!("held clock consulted the gates"),
            )
            .unwrap();
            assert_eq!(
                resolved,
                LocalClock {
                    hour,
                    night,
                    icon_frame: frame
                }
            );
        }
    }
}

#[test]
fn updated_clocks_are_derived_for_the_row_without_consulting_the_gates() {
    let province = row(5, DayNightClock::UPDATED);
    let resolved = resolve(
        &province,
        |id| {
            assert_eq!(id, 5);
            Ok(clock(13, 0))
        },
        || panic!("an updated clock needs no gates"),
    )
    .unwrap();
    assert_eq!(
        resolved,
        LocalClock {
            hour: 13,
            night: 0,
            icon_frame: 1
        }
    );
    assert_eq!(
        resolve(&province, |_| Err("defines unread".into()), || Ok(None)).unwrap_err(),
        "defines unread"
    );
}

#[test]
fn old_recordings_derive_only_when_the_gates_prove_the_update() {
    let province = row(5, DayNightClock::NOT_RECORDED);
    assert_eq!(
        resolve(&province, |_| Ok(clock(1, 100_000)), || Ok(Some([0, 0]))),
        Ok(LocalClock {
            hour: 1,
            night: 100_000,
            icon_frame: 2
        })
    );
    for gates in [None, Some([1, 0]), Some([0, 1])] {
        let error = resolve(
            &province,
            |_| panic!("suspended manager used derivation"),
            || Ok(gates),
        )
        .unwrap_err();
        assert!(error.starts_with("Province local clock/night refused:"));
        assert!(error.contains("suspended") || error.contains("suspension flags unavailable"));
    }
    assert!(
        resolve(
            &province,
            |_| unreachable!(),
            || Err("simulation features absent".into())
        )
        .unwrap_err()
        .contains("simulation features absent")
    );
    assert!(
        resolve(
            &row(0, DayNightClock::NOT_RECORDED),
            |_| panic!("slot 0 used derivation"),
            || Ok(Some([0, 0]))
        )
        .unwrap_err()
        .contains("slot 0")
    );
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
        assert_eq!(LocalClock::from(clock(0, night)).icon_frame, frame);
    }
}

#[test]
fn content_defines_keep_native_conversions() {
    let raw = content_reader_defines::defines::Defines::new().unwrap();
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
    let loaded = replay_data::day_night::Defines::read(
        |group, name| raw.int32(group, name),
        |group, name| raw.fixed5(group, name),
    )
    .unwrap();
    assert_eq!(
        loaded,
        replay_data::day_night::Defines {
            gmt_offset: 8,
            south_pole_offset: 0,
            north_pole_offset: 100_000,
            day_night_feather: 20_000,
        }
    );
}
