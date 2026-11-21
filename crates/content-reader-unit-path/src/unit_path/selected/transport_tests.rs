use super::*;

#[test]
fn extreme_cached_weight_preserves_i64_rounding_wrap_before_narrowing() {
    assert_eq!(unit_convoys(i64::MAX), 702_313_053);
    assert_eq!(unit_convoys(i64::MIN), 702_313_053);
    assert_eq!(unit_convoys(i64::MAX - 1), 702_313_053);
}

struct Inputs {
    target: bool,
    land: bool,
    source_base: bool,
    special: bool,
    transfer: Fact<Option<Transfer>>,
    weight: Fact<i64>,
    available: Fact<i32>,
    access: Fact<bool>,
    route: Fact<bool>,
}

impl Default for Inputs {
    fn default() -> Self {
        Self {
            target: true,
            land: true,
            source_base: true,
            special: false,
            transfer: Ok(None),
            weight: Ok(ONE),
            available: Ok(1),
            access: Ok(true),
            route: Ok(true),
        }
    }
}
impl AdmissionInputs for Inputs {
    fn target_base(&mut self) -> Fact<bool> {
        Ok(self.target)
    }
    fn source_land(&mut self) -> Fact<bool> {
        Ok(self.land)
    }
    fn source_base(&mut self) -> Fact<bool> {
        Ok(self.source_base)
    }
    fn coastal_special_template(&mut self) -> Fact<bool> {
        Ok(self.special)
    }
    fn transfer(&mut self) -> Fact<Option<Transfer>> {
        self.transfer
    }
    fn weight(&mut self) -> Fact<i64> {
        self.weight
    }
    fn available_convoys(&mut self) -> Fact<i32> {
        self.available
    }
    fn military_access(&mut self) -> Fact<bool> {
        self.access
    }
    fn sea_route(&mut self) -> Fact<bool> {
        self.route
    }
}

#[test]
fn missing_target_base_precedes_all_unreadable_inputs() {
    let mut input = Inputs {
        target: false,
        transfer: Err("transfer unreadable"),
        ..Inputs::default()
    };
    assert_eq!(
        admit(Unit::Army(1), 8, &mut input),
        Ok(rejected(Reason::NoTargetBase))
    );
}

#[test]
fn coastal_special_army_returns_before_transfer_convoy_and_route_but_gun_does_not() {
    let mut input = Inputs {
        source_base: false,
        special: true,
        transfer: Err("transfer unreadable"),
        weight: Err("weight unreadable"),
        route: Err("route unavailable"),
        ..Inputs::default()
    };
    assert_eq!(
        admit(Unit::Army(1), 8, &mut input),
        Ok(Admission {
            allowed: true,
            reason: None
        })
    );
    assert_eq!(
        admit(Unit::RailwayGun(1), 8, &mut input),
        Ok(rejected(Reason::NoSourceBase))
    );
}

#[test]
fn sea_source_needs_no_base_and_transfer_nonzero_status_precedes_source_equality() {
    let mut input = Inputs {
        land: false,
        source_base: false,
        transfer: Ok(Some(Transfer {
            status: Ok(-3),
            source: Err("unreached source"),
        })),
        ..Inputs::default()
    };
    assert_eq!(
        admit(Unit::Army(1), 8, &mut input),
        Ok(rejected(Reason::DuringCombat))
    );
    input.transfer = Ok(Some(Transfer {
        status: Ok(0),
        source: Ok(8),
    }));
    assert_eq!(
        admit(Unit::Army(1), 8, &mut input),
        Ok(rejected(Reason::AlreadyActive))
    );
    input.transfer = Ok(Some(Transfer {
        status: Ok(0),
        source: Ok(7),
    }));
    assert_eq!(
        admit(Unit::Army(1), 8, &mut input),
        Ok(Admission {
            allowed: true,
            reason: None
        })
    );
}

#[test]
fn exact_signed_availability_precedes_access_and_route_and_unknown_is_not_denial() {
    let mut input = Inputs {
        available: Ok(-1),
        access: Err("access unreadable"),
        ..Inputs::default()
    };
    assert_eq!(
        admit(Unit::Army(1), 8, &mut input),
        Ok(rejected(Reason::NotEnough {
            available: -1,
            required: 1
        }))
    );
    input.available = Ok(1);
    assert_eq!(
        admit(Unit::Army(1), 8, &mut input),
        Err("access unreadable")
    );
    input.access = Ok(false);
    assert_eq!(
        admit(Unit::Army(1), 8, &mut input),
        Ok(rejected(Reason::NoAccess))
    );
    input.access = Ok(true);
    input.route = Ok(false);
    assert_eq!(
        admit(Unit::Army(1), 8, &mut input),
        Ok(rejected(Reason::NoSeaRoute))
    );
}

#[test]
fn secondary_only_first_transport_ensures_lf_without_copying_secondary_bytes() {
    let outputs = Outputs {
        secondary: b"convoy description".to_vec(),
        ..Outputs::default()
    };
    let mut body = b"selection".to_vec();
    append_first(&mut body, &outputs);
    assert_eq!(body, b"selection\n");
    append_first(&mut body, &outputs);
    assert_eq!(body, b"selection\n");
    let mut empty = Vec::new();
    append_first(&mut empty, &outputs);
    assert!(empty.is_empty());
}

#[test]
fn first_transport_primary_multiline_preserves_empty_and_terminal_lines() {
    let mut body = b"selection".to_vec();
    append_first(
        &mut body,
        &Outputs {
            primary: b"a\n\nb\n".to_vec(),
            secondary: Vec::new(),
        },
    );
    assert_eq!(body, b"selection\n    a\n\n    b\n");
}

#[test]
fn transfer_status_and_source_refusals_are_reached_independently() {
    let mut input = Inputs {
        transfer: Ok(Some(Transfer {
            status: Err("status unreadable"),
            source: Ok(7),
        })),
        ..Inputs::default()
    };
    assert_eq!(
        admit(Unit::Army(1), 8, &mut input),
        Err("status unreadable")
    );
    input.transfer = Ok(Some(Transfer {
        status: Ok(0),
        source: Err("source unreadable"),
    }));
    assert_eq!(
        admit(Unit::Army(1), 8, &mut input),
        Err("source unreadable")
    );
}

#[derive(Default)]
struct Message {
    candidates: Vec<Unit>,
    weights: Vec<Unit>,
    risk_arguments: Vec<(Unit, i32)>,
    risk: i64,
    admit: Option<Unit>,
    skip: Option<Unit>,
}
impl MessageInputs for Message {
    fn target_base(&mut self, _: u32) -> Result<bool, String> {
        Ok(true)
    }
    fn source(&mut self, unit: Unit) -> Result<u32, String> {
        Ok(if self.skip == Some(unit) { 9 } else { 3 })
    }
    fn same_area(&mut self, source: u32, _: u32) -> Result<bool, String> {
        Ok(source == 9)
    }
    fn source_base(&mut self, _: u32) -> Result<bool, String> {
        Ok(false)
    }
    fn admission(&mut self, unit: Unit, _: u32, _: u32) -> Result<Admission, String> {
        self.candidates.push(unit);
        Ok(if self.admit == Some(unit) {
            Admission {
                allowed: true,
                reason: None,
            }
        } else {
            rejected(Reason::NotEnough {
                available: -7,
                required: 8,
            })
        })
    }
    fn weight(&mut self, unit: Unit) -> Result<i64, String> {
        self.weights.push(unit);
        Ok(if matches!(unit, Unit::RailwayGun(_)) {
            -100_001
        } else {
            1
        })
    }
    fn risk(&mut self, unit: Unit, _: u32, _: u32, convoys: i32) -> Result<i64, String> {
        self.risk_arguments.push((unit, convoys));
        Ok(self.risk)
    }
}
struct Text(bool);
impl super::super::CommonMoveText for Text {
    fn plain(&self, key: &str) -> Result<String, String> {
        Ok(key.into())
    }
    fn state(&self, _: &str, _: u32) -> Result<String, String> {
        panic!("unreached")
    }
    fn owner(&self, _: &str, _: u32) -> Result<String, String> {
        panic!("unreached")
    }
    fn rule(&self, _: &str, _: &crate::adjacency_rules::AdjacencyRule) -> Result<String, String> {
        panic!("unreached")
    }
    fn warning(&self, _: &str, _: i64) -> Result<String, String> {
        panic!("unreached")
    }
    fn days(&self, _: &str, _: i32) -> Result<String, String> {
        panic!("unreached")
    }
    fn transport_stock(&self, available: i32, required: i32) -> Result<String, String> {
        Ok(format!("FREE={available},NEED={required}"))
    }
    fn transport_risk(&self, risk: i64) -> Result<String, String> {
        Ok(if self.0 {
            String::new()
        } else {
            format!("ATTR={risk}")
        })
    }
    fn transport_description(&self, convoys: i32, icon: bool) -> Result<String, String> {
        Ok(if self.0 {
            String::new()
        } else {
            format!("CONVOYS={convoys},ICON={icon}")
        })
    }
}

#[test]
fn first_admitted_candidate_stops_but_all_selected_weights_and_repeats_contribute() {
    let selected = [
        Unit::Army(1),
        Unit::Other,
        Unit::Army(1),
        Unit::RailwayGun(2),
        Unit::Army(3),
    ];
    let mut inputs = Message {
        admit: Some(Unit::RailwayGun(2)),
        risk: 123,
        ..Default::default()
    };
    let output = message(&selected, 9, true, &mut inputs, &Text(false)).unwrap();
    assert_eq!(
        inputs.candidates,
        [Unit::Army(1), Unit::Army(1), Unit::RailwayGun(2)]
    );
    assert_eq!(
        inputs.weights,
        [
            Unit::Army(1),
            Unit::Army(1),
            Unit::RailwayGun(2),
            Unit::Army(3)
        ]
    );
    assert_eq!(inputs.risk_arguments, [(Unit::RailwayGun(2), 2)]);
    assert_eq!(output.primary, b"\nATTR=123");
    assert_eq!(output.secondary, b"\nCONVOYS=2,ICON=true");
}

#[test]
fn later_gate_skip_clears_the_prior_rejection_without_demanding_weight_or_risk() {
    let mut inputs = Message {
        skip: Some(Unit::Army(2)),
        ..Default::default()
    };
    let output = message(
        &[Unit::Army(1), Unit::Army(2)],
        9,
        false,
        &mut inputs,
        &Text(false),
    )
    .unwrap();
    assert_eq!(inputs.candidates, [Unit::Army(1)]);
    assert!(inputs.weights.is_empty() && inputs.risk_arguments.is_empty());
    assert_eq!(output, Outputs::default());
    let output = message(&[Unit::Army(1)], 9, false, &mut inputs, &Text(false)).unwrap();
    assert_eq!(output.secondary, b"FREE=-7,NEED=8");
}

#[test]
fn known_empty_localisations_keep_lf_and_zero_risk_has_no_primary() {
    let mut inputs = Message {
        admit: Some(Unit::Army(1)),
        risk: 1,
        ..Default::default()
    };
    let output = message(&[Unit::Army(1)], 9, false, &mut inputs, &Text(true)).unwrap();
    assert_eq!(output.primary, b"\n");
    assert_eq!(output.secondary, b"\n");
    let mut body = b"notice".to_vec();
    append_railway_second(&mut body, &output);
    assert_eq!(body, b"notice \n");
    inputs.risk = 0;
    let output = message(&[Unit::Army(1)], 9, false, &mut inputs, &Text(false)).unwrap();
    assert!(output.primary.is_empty());
    assert_eq!(output.secondary, b"\nCONVOYS=1,ICON=false");
    let prior = body.clone();
    append_railway_second(&mut body, &output);
    assert_eq!(body, prior);
}

#[test]
fn railway_second_appends_raw_primary_without_first_call_indentation() {
    let output = Outputs {
        primary: b"\nwarn\nsecond\n".to_vec(),
        secondary: b"ignored".to_vec(),
    };
    let mut body = b"notice".to_vec();
    append_railway_second(&mut body, &output);
    assert_eq!(body, b"notice \nwarn\nsecond\n");
}
