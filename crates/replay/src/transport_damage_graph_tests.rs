use super::*;

struct Fixture {
    seen: Vec<&'static str>,
    fault: Option<&'static str>,
    variant: Option<Variant>,
    actual_speed: i64,
    targeted: i64,
    night: i64,
    night_modifier: i64,
}

impl Fixture {
    fn new() -> Self {
        Self {
            seen: Vec::new(),
            fault: None,
            variant: Some(Variant {
                visibility: Ok(ONE),
                surface_detection: Ok(0),
                submarine_detection: Ok(0),
                speed: Ok(ONE),
            }),
            actual_speed: ONE,
            targeted: 0,
            night: 0,
            night_modifier: 0,
        }
    }
    fn leaf<T>(&mut self, name: &'static str, value: T) -> Fact<T> {
        self.seen.push(name);
        if self.fault == Some(name) {
            Err(name)
        } else {
            Ok(value)
        }
    }
}

impl Inputs for Fixture {
    fn detection_multiplier(&mut self) -> Fact<i64> {
        self.leaf("mission", ONE)
    }
    fn combined_detection(&mut self) -> Fact<i64> {
        self.leaf("combined", 0)
    }
    fn detection_stats(&mut self) -> Fact<[i64; 2]> {
        self.leaf("stats", [ONE, 0])
    }
    fn carrier_fill_sum(&mut self) -> Fact<i64> {
        self.leaf("carriers", 0)
    }
    fn first_variant(&mut self) -> Fact<Option<Variant>> {
        self.leaf("variant", self.variant)
    }
    fn visibility_stats(&mut self) -> Fact<[i64; 2]> {
        self.leaf("own visibility", [0, 0])
    }
    fn detached_activity(&mut self) -> Fact<i32> {
        self.leaf("activity", 0)
    }
    fn visibility_modifier(&mut self) -> Fact<i64> {
        self.leaf("visibility modifier", 0)
    }
    fn actual_speed(&mut self) -> Fact<i64> {
        self.leaf("speed", self.actual_speed)
    }
    fn coordination(&mut self) -> Fact<i64> {
        self.leaf("coordination", 0)
    }
    fn spotting_modifier(&mut self) -> Fact<i64> {
        self.leaf("spotting", 0)
    }
    fn targeted_spotting_modifier(&mut self) -> Fact<i64> {
        self.leaf("targeted", self.targeted)
    }
    fn simulation_night_factor(&mut self) -> Fact<i64> {
        self.leaf("night", self.night)
    }
    fn night_spotting_modifier(&mut self) -> Fact<i64> {
        self.leaf("night modifier", self.night_modifier)
    }
}

fn defines() -> Defines {
    Defines {
        base: ONE,
        transfer_multiplier: ONE,
        surface_multiplier: ONE,
        submarine_multiplier: ONE,
        convoy_count_exponent: 0,
        visibility_multiplier: ONE,
        detection_base: 0,
        enemy_detection_multiplier: ONE,
        transport_speed_scale: ONE,
        catching_up_multiplier: 20_000,
        running_away_multiplier: 30_000,
        coordination_factor: ONE,
        repair_visibility_multiplier: 25_000,
    }
}

#[test]
fn transfer_empty_variant_retains_earlier_carrier_demand_and_skips_later_leaves() {
    let mut input = Fixture::new();
    input.variant = None;
    input.fault = Some("speed");
    assert_eq!(query(&mut input, defines(), i32::MIN), Ok(0));
    assert_eq!(
        input.seen,
        ["mission", "combined", "stats", "carriers", "variant"]
    );
    input.seen.clear();
    input.fault = Some("carriers");
    assert_eq!(query(&mut input, defines(), 1), Err("carriers"));
    assert!(!input.seen.contains(&"variant"));
}

#[test]
fn transfer_preserves_signed_speed_delta_and_native_night_threshold() {
    let mut input = Fixture::new();
    input.night = 80_000;
    input.fault = Some("night modifier");
    assert_eq!(query(&mut input, defines(), 1), Ok(100_100));
    assert!(!input.seen.contains(&"night modifier"));
    input.actual_speed = 150_000;
    assert_eq!(query(&mut input, defines(), 1), Ok(110_100));
    input.actual_speed = 50_000;
    assert_eq!(query(&mut input, defines(), 1), Ok(85_100));
}

#[test]
fn transfer_positive_entry_demands_night_after_targeted_term_turns_negative() {
    let mut input = Fixture::new();
    input.targeted = -200_000;
    input.night = 80_001;
    input.night_modifier = -50_000;
    assert_eq!(query(&mut input, defines(), 1), Ok(-50_050));
    assert_eq!(
        &input.seen[input.seen.len() - 2..],
        ["night", "night modifier"]
    );
}

#[test]
fn transfer_nonpositive_entry_skips_all_later_spotting_leaves() {
    let mut input = Fixture::new();
    input.actual_speed = -500_000;
    input.fault = Some("spotting");
    assert_eq!(query(&mut input, defines(), 1), Ok(-79_900));
    assert!(!input.seen.contains(&"spotting"));
}

#[test]
fn visibility_and_carrier_ratios_keep_native_signed_and_repair_laws() {
    assert_eq!(carrier_fill(100, 30), 30_000);
    assert_eq!(carrier_fill(-100, -30), 30_000);
    assert_eq!(carrier_fill(0, 99), 0);
    assert_eq!(carrier_fill(100, -30), 0);
    assert_eq!(carrier_fill(100, 150), ONE);
    assert_eq!(carrier_fill(i32::MAX, i32::MAX), 14_100); // native signed64 product wraps
    assert_eq!(visibility(100_003, 0, 1, defines()), 25_000);
    assert_eq!(visibility(100_003, -200_000, 0, defines()), 0);
    assert_eq!(detection([20_000, 40_000], 150_000, 25_000), 100_000);
}

#[test]
fn transfer_variant_stat_failures_remain_independent_and_late() {
    let mut input = Fixture::new();
    input.variant.as_mut().unwrap().speed = Err("convoy speed");
    assert_eq!(query(&mut input, defines(), 1), Err("convoy speed"));
    assert!(input.seen.contains(&"speed"));
    assert!(!input.seen.contains(&"coordination"));
}
