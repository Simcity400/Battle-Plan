use super::*;

fn loc() -> Localisation {
    [
        ("PROVINCE_WEATHER_HEADER", "Weather:"),
        ("PROV_TOOLTIP_WEATHER_MODIFIER", "$FRAME$:$TEXT$"),
        ("PROV_TOOLTIP_TEMPERATURE_MODIFIER", "T$FRAME$:$TEXT$"),
        ("PROV_TOOLTIP_TEMPERATURE", "$WEATHER$($TEMP|0$°C)"),
        ("weather_none", "Clear"),
        ("weather_rain_light", "Rain"),
        ("weather_rain_heavy", "Storm"),
        ("weather_snow", "Snow"),
        ("weather_blizzard", "Blizzard"),
        ("weather_sandstorm", "Sandstorm"),
        ("weather_mud", "Mud"),
        ("cold", "Cold"),
        ("hot", "Hot"),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect()
}

fn ranges() -> [TemperatureRange; 4] {
    [
        TemperatureRange {
            lower: -900_000,
            upper: -100_000,
            modifier: Some("cold".into()),
        },
        TemperatureRange {
            lower: 200_000,
            upper: 300_000,
            modifier: Some("hot".into()),
        },
        TemperatureRange {
            lower: 300_000,
            upper: 400_000,
            modifier: None,
        },
        TemperatureRange {
            lower: 400_000,
            upper: 500_000,
            modifier: Some("hot".into()),
        },
    ]
}

#[test]
fn independent_flags_keep_native_order_and_raw_mud() {
    let province = WeatherProvinceRow {
        raw_mud: 2,
        display_mud: 0,
        ..Default::default()
    };
    let region = WeatherRegionRow {
        light_rain: 1,
        heavy_rain: 1,
        snow: 1,
        blizzard: 1,
        sandstorm: 1,
        ..Default::default()
    };
    assert_eq!(
        row(&loc(), &ranges(), &province, &region).unwrap(),
        "Weather: 1:Rain 2:Storm 3:Snow 4:Blizzard 5:Sandstorm 6:Mud (0°C)"
    );
    let custom_only = WeatherProvinceRow {
        raw_mud: 0,
        display_mud: 1,
        ..Default::default()
    };
    assert_eq!(
        row(&loc(), &ranges(), &custom_only, &Default::default()).unwrap(),
        "Weather: \u{11}YClear \u{11}!(0°C)"
    );
}

#[test]
fn temperature_preserves_negative_zero_and_half_open_raw_thresholds() {
    let render = |temperature| {
        row(
            &loc(),
            &ranges(),
            &WeatherProvinceRow {
                temperature,
                ..Default::default()
            },
            &Default::default(),
        )
        .unwrap()
    };
    assert!(render(-1).ends_with("(-0°C)"));
    assert!(render(199_999).contains("Clear"));
    assert_eq!(render(200_001), "Weather: T2:Hot (2°C)");
    assert_eq!(render(300_000), "Weather: T3: (3°C)");
    assert!(render(500_000).contains("Clear"));
}

#[test]
fn missing_localisation_key_keeps_native_literal_bytes() {
    let mut localisation = loc();
    localisation = localisation
        .iter()
        .filter(|(key, _)| *key != "weather_rain_heavy")
        .map(|(key, value)| (key.into(), value.into()))
        .collect();
    assert_eq!(
        row(
            &localisation,
            &ranges(),
            &Default::default(),
            &WeatherRegionRow {
                heavy_rain: 1,
                ..Default::default()
            }
        )
        .unwrap(),
        "Weather: 2:weather_rain_heavy (0°C)"
    );
}

#[test]
fn native_text_argument_is_a_copy_then_appended_even_when_content_uses_it() {
    let mut localisation: Localisation = loc()
        .iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect();
    localisation = localisation
        .iter()
        .filter(|(key, _)| *key != "PROV_TOOLTIP_TEMPERATURE")
        .map(|(key, value)| (key.into(), value.into()))
        .chain([(
            "PROV_TOOLTIP_TEMPERATURE".into(),
            "<$TEXT$>($TEMP|0$°C)".into(),
        )])
        .collect();
    assert_eq!(
        row(
            &localisation,
            &ranges(),
            &Default::default(),
            &WeatherRegionRow {
                heavy_rain: 1,
                ..Default::default()
            }
        )
        .unwrap(),
        "Weather: 2:Storm <2:Storm >(0°C)"
    );
}
