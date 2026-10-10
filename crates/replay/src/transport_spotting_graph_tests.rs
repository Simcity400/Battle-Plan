use super::*;

fn defines() -> Defines {
    Defines {
        base: 20_000,
        visibility_scale: P,
        navy_scale: P,
        navy_base: 70_000,
        radar_base: 30_000,
        decryption_base: 40_000,
        intel_minimum: P,
        intel_maximum: 5 * P,
    }
}

struct Source {
    first: Fact<Option<i64>>,
    detection: Fact<Vec<i64>>,
    weather: Fact<Option<i64>>,
    calls: Vec<&'static str>,
}

impl Inputs for Source {
    fn first_convoy_visibility(&mut self) -> Fact<Option<i64>> {
        self.calls.push("variant");
        self.first
    }
    fn defines(&mut self) -> Fact<Defines> {
        self.calls.push("defines");
        Ok(defines())
    }
    fn navy_detection(&mut self) -> Fact<Vec<i64>> {
        self.calls.push("navy");
        self.detection.clone()
    }
    fn air_contribution(&mut self, visibility: i64) -> Fact<i64> {
        assert_eq!(visibility, self.first.unwrap().unwrap());
        self.calls.push("air");
        Ok(0)
    }
    fn radar_counts(&mut self) -> Fact<(i32, i32)> {
        self.calls.push("radar");
        Ok((0, 10))
    }
    fn navy_intel_difference(&mut self) -> Fact<i64> {
        self.calls.push("intel");
        Ok(0)
    }
    fn weather_detection(&mut self) -> Fact<Option<i64>> {
        self.calls.push("weather");
        self.weather
    }
}

fn source(first: Option<i64>, detection: Vec<i64>) -> Source {
    Source {
        first: Ok(first),
        detection: Ok(detection),
        weather: Ok(None),
        calls: Vec::new(),
    }
}

#[test]
fn spotting_no_first_variant_is_zero_before_independent_failures() {
    let mut source = source(None, vec![]);
    source.detection = Err("native null spotting mission reached");
    source.weather = Err("original weather capture refused");
    assert_eq!(query(&mut source), Ok(0));
    assert_eq!(source.calls, ["variant"]);
}

#[test]
fn spotting_zero_visibility_still_reaches_all_native_inputs() {
    let mut source = source(Some(0), vec![1_000_000]);
    assert_eq!(query(&mut source), Ok(20_000));
    assert_eq!(
        source.calls,
        [
            "variant", "defines", "navy", "air", "radar", "intel", "weather"
        ]
    );
    source.calls.clear();
    source.detection = Err("native null spotting mission reached");
    assert_eq!(
        query(&mut source),
        Err("native null spotting mission reached")
    );
    assert_eq!(source.calls, ["variant", "defines", "navy"]);
}

#[test]
fn spotting_keeps_repeated_ledger_occurrences_and_weather_clamp() {
    let mut source = source(Some(P), vec![500_000, 500_000]);
    assert_eq!(query(&mut source), Ok(90_000));
    source.weather = Ok(Some(-P - 1));
    assert_eq!(query(&mut source), Ok(0));
    source.weather = Ok(Some(P));
    assert_eq!(query(&mut source), Ok(180_000));
}

#[test]
fn carrier_detection_keeps_repeated_fill_ratios_raw_modifier_and_failures() {
    assert_eq!(
        task_force_detection(10_000, 20_000, [Ok(P), Ok(P)], 0),
        Ok(50_000)
    );
    assert_eq!(task_force_detection(10_000, 20_000, [Ok(P)], -P - 1), Ok(0));
    assert_eq!(
        task_force_detection(10_000, 0, [Err("original carrier source refused")], 0),
        Err("original carrier source refused")
    );
    assert_eq!(task_force_detection(-1, 20_000, [], 0), Ok(0));
}

#[test]
fn radar_uses_signed_counts_wrapping_product_and_zero_denominator_sentinel() {
    assert_eq!(radar(3, 12), Ok(25_000));
    assert_eq!(radar(-3, 12), Ok(-25_000));
    assert_eq!(radar(0, 0), Ok(REFUSED));
    assert_eq!(
        radar(i32::MAX, 1),
        Ok(i64::from(i32::MAX).wrapping_mul(P).wrapping_mul(P) / P)
    );
}

#[test]
fn decryption_keeps_difference_sign_threshold_order_and_wrapping_negate() {
    let mut values = defines();
    assert_eq!(decryption(P - 1, values), Ok(0));
    assert_eq!(decryption(3 * P, values), Ok(20_000));
    assert_eq!(decryption(-3 * P, values), Ok(-20_000));
    assert_eq!(decryption(8 * P, values), Ok(40_000));
    assert_eq!(decryption(i64::MIN, values), Ok(-40_000));
    values.intel_maximum = values.intel_minimum;
    assert_eq!(decryption(3 * P, values), Ok(0));
    values.intel_maximum = 0;
    assert_eq!(decryption(3 * P, values), Ok(40_000));
}

fn air_defines() -> AirDefines {
    AirDefines {
        normalized_size: 100,
        detection_scale: P,
        base: 80_000,
    }
}

fn air(kind: u32, stats: [i64; 2]) -> AirMission {
    AirMission {
        current_type: Ok(kind),
        aircraft: Ok(100),
        disruption_factor: Err("normalized air-combat source refused"),
        detection: Ok(stats),
    }
}

#[test]
fn air_empty_and_zero_mission_factor_skip_unreached_operand_failures() {
    assert_eq!(
        air_contribution(
            &[],
            Err("unreached sampled province"),
            Err("original air operand unavailable"),
            Err("original air operand unavailable"),
            Err("original air operand unavailable"),
            [P, 0]
        ),
        Ok([0, 0])
    );
    assert_eq!(
        air_contribution(
            &[air(0, [P, 0])],
            Ok(5),
            Ok(air_defines()),
            Err("original air operand unavailable"),
            Err("original air operand unavailable"),
            [P, 0]
        ),
        Ok([0, 0])
    );
    assert_eq!(
        air_contribution(
            &[air(2, [P, 0])],
            Ok(5),
            Ok(air_defines()),
            Ok(&[P, 0]),
            Err("original air operand unavailable"),
            [P, 0]
        ),
        Ok([0, 0])
    );
    assert_eq!(
        air_contribution(
            &[air(0, [P, 0])],
            Err("original sampled province refused"),
            Ok(air_defines()),
            Err("unreached mission factors"),
            Err("unreached regional factor"),
            [P, 0]
        ),
        Err("original sampled province refused")
    );
}

#[test]
fn air_keeps_repeated_missions_and_reads_both_stats_with_zero_second_visibility() {
    assert_eq!(
        air_contribution(
            &[air(1, [P, 0]); 10],
            Ok(5),
            Ok(air_defines()),
            Ok(&[P]),
            Ok(P),
            [P, 0]
        ),
        Ok([1_000_000, 80_000])
    );
    let mut mission = air(1, [P, 0]);
    mission.detection = Err("original second mission stat refused");
    assert_eq!(
        air_contribution(
            &[mission],
            Ok(5),
            Ok(air_defines()),
            Ok(&[P]),
            Ok(P),
            [P, 0]
        ),
        Err("original second mission stat refused")
    );
    assert_eq!(
        air_contribution(
            &[air(1, [P, -P])],
            Ok(5),
            Ok(air_defines()),
            Ok(&[P]),
            Err("unreached regional factor"),
            [P, 0]
        ),
        Ok([0, 0])
    );
}

#[test]
fn air_refuses_reached_disruption_sources_missing_factors_and_assertion_forms() {
    assert_eq!(
        air_contribution(
            &[air(2, [P, 0])],
            Ok(5),
            Ok(air_defines()),
            Ok(&[P, P]),
            Ok(P),
            [P, 0]
        ),
        Err("normalized air-combat source refused")
    );
    assert_eq!(
        air_contribution(
            &[air(2, [P, 0])],
            Ok(5),
            Ok(air_defines()),
            Ok(&[P]),
            Ok(P),
            [P, 0]
        ),
        Err("selected transport air mission missing from AIR_MISSION_SPOTTING_FACTORS")
    );
    for kind in [3, 1 << 31] {
        assert_eq!(
            air_contribution(
                &[air(kind, [P, 0])],
                Ok(5),
                Ok(air_defines()),
                Ok(&[P]),
                Ok(P),
                [P, 0]
            ),
            Err("selected transport air spotting mission is not one positive bit")
        );
    }
}

#[test]
fn spotting_define_loads_apply_native_clamps_and_preserve_unbounded_names() {
    let mut names = Vec::new();
    let values = Defines::read::<()>(|section, name| {
        names.push((section, name));
        Ok(match name {
            "BASE_SPOTTING" => -1,
            "NAVY_SPOTTER_DETECTION_FACTOR" => -7,
            "VISIBILITY_MULTIPLIER_FOR_SPOTTING" => -13,
            _ => 20_000_000,
        })
    })
    .unwrap();
    assert_eq!(
        (
            values.base,
            values.visibility_scale,
            values.navy_scale,
            values.navy_base,
            values.radar_base,
            values.decryption_base,
            values.intel_minimum,
            values.intel_maximum
        ),
        (
            0, -13, 0, 10_000_000, 10_000_000, 10_000_000, 20_000_000, 20_000_000
        )
    );
    assert_eq!(names.len(), 8);
    assert_eq!(names[0], ("NNavy", "BASE_SPOTTING"));
    assert_eq!(
        names[7],
        (
            "NIntel",
            "NAVY_INTEL_BASE_SPOTTING_BONUS_MAX_INTEL_FOR_BONUS"
        )
    );
    let values = AirDefines::read::<()>(
        |section, name| {
            assert_eq!(section, "NNavy");
            Ok(if name == "AIR_SPOTTER_DETECTION_FACTOR" {
                -3
            } else {
                20_000_000
            })
        },
        |section, name| {
            assert_eq!(
                (section, name),
                ("NNavy", "AIR_SPOTTER_NORMALIZED_AIRWING_SIZE")
            );
            Ok(-1)
        },
    )
    .unwrap();
    assert_eq!(
        values,
        AirDefines {
            normalized_size: 1,
            detection_scale: 0,
            base: 10_000_000
        }
    );
}

#[test]
fn air_disruption_native_early_unity_does_not_demand_normalization() {
    let missing = Err("original air normalization refused");
    for (raw, count, kind) in [(0, 100, 2), (P, 0, 2), (P, -1, 2), (P, 100, 0x400)] {
        assert_eq!(
            disruption_factor(
                Ok(raw),
                count,
                kind,
                missing,
                Err("unreached normalized speed"),
                Err("unreached disruption defines")
            ),
            Ok(P)
        );
    }
    assert_eq!(
        disruption_factor(
            Ok(P),
            100,
            2,
            Ok([7, 0]),
            Err("unreached normalized speed"),
            Err("unreached disruption defines")
        ),
        Ok(P)
    );
    assert_eq!(
        disruption_factor_leaves(
            Ok(P),
            100,
            2,
            Err("unreached normalized attack"),
            Ok(0),
            Err("unreached normalized speed"),
            Err("unreached disruption defines")
        ),
        Ok(P)
    );
    assert_eq!(
        disruption_factor(
            Ok(P),
            100,
            2,
            missing,
            Ok(0),
            Ok(DisruptionDefines {
                defence: 0,
                attack: 0,
                speed: 0
            })
        ),
        missing.map(|_| P)
    );
}

#[test]
fn air_original_domain_distinguishes_empty_null_failed_and_stale_sources() {
    use crate::rows::{TransportAirCountry, TransportAirInputs, TransportAirRegion};
    let mut packet = TransportAirInputs {
        sample_hour: 7,
        countries: Some(vec![
            TransportAirCountry {
                slot: 0,
                ..Default::default()
            },
            TransportAirCountry {
                slot: 1,
                present: true,
                country: Some(77),
                regions: Some(vec![TransportAirRegion {
                    region: 0,
                    count: Some(0),
                    missions: None,
                }]),
                factors: None,
            },
        ]),
    };
    assert_eq!(air_region(&packet, 7, 1, 0).unwrap().1.count, Some(0));
    assert!(air_region(&packet, 7, 0, 0).unwrap_err().contains("null"));
    assert!(air_region(&packet, 8, 1, 0).unwrap_err().contains("hour"));
    assert!(
        air_region(&packet, 7, 1, 1)
            .unwrap_err()
            .contains("outside")
    );
    let region = &mut packet.countries.as_mut().unwrap()[1]
        .regions
        .as_mut()
        .unwrap()[0];
    region.count = Some(1);
    region.missions = Some(vec![None]);
    assert_eq!(
        air_region(&packet, 7, 1, 0)
            .unwrap()
            .1
            .missions
            .as_ref()
            .unwrap(),
        &[None]
    );
    packet.countries.as_mut().unwrap()[1]
        .regions
        .as_mut()
        .unwrap()[0]
        .missions = Some(vec![]);
    assert!(
        air_region(&packet, 7, 1, 0)
            .unwrap_err()
            .contains("occurrence")
    );
    packet.countries = None;
    assert!(air_region(&packet, 7, 1, 0).unwrap_err().contains("domain"));
}

#[test]
fn air_disruption_uses_native_newton_sqrt_magic_division_and_sentinel_clamp() {
    let defines = DisruptionDefines {
        defence: 0,
        attack: 0,
        speed: 0,
    };
    assert_eq!(sqrt(0), Ok(0));
    assert_eq!(sqrt(-7), Ok(0));
    // 1424DAE00 stops on the previous error's 1% tolerance after applying
    // its Newton step; 1424D8800 does not round this result to unity.
    assert_eq!(sqrt(P), Ok(100_001));
    assert_eq!(
        disruption_factor(Ok(P), 1, 2, Ok([0, 1]), Ok(0), Ok(defines)),
        Ok(68_375)
    );
    assert_eq!(
        disruption_factor(Ok(-P), 1, 2, Ok([0, 1]), Ok(0), Ok(defines)),
        Ok(P)
    );
    assert_eq!(
        disruption_factor(Ok(i64::MIN), 1, 2, Ok([0, 1]), Ok(0), Ok(defines)),
        Ok(100)
    );
    assert_eq!(
        disruption_factor(
            Ok(P),
            1,
            2,
            Ok([0, 1]),
            Err("original normalized speed refused"),
            Ok(defines)
        ),
        Err("original normalized speed refused")
    );
}
