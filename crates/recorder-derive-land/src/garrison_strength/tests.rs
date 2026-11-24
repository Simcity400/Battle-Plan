use super::*;
use recorder_derive_framework::{
    testing::deployed::{Edits, Run, world},
    framework::Definitions,
};
use replay_columnar::manifest::Status;
use replay_data::rows::Recorded;

const INPUTS: &[TableId] = &[TableId::Countries, TableId::CountryLinks, TableId::StateGarrisons, TableId::StateAssignedManpower];

fn writers() -> Vec<Box<dyn Writer>> {
    vec![Box::new(GarrisonStrength::default())]
}

fn definitions() -> Definitions {
    Definitions::read_none().with_define(MANPOWER_POWER, Ok(2)).with_define(EQUIPMENT_POWER, Ok(3))
}

fn run() -> Run {
    let mut run = Run::new(writers, INPUTS);
    run.define(definitions());
    run
}

fn operands(allocated: i64, required: i64, weight: i64) -> Recorded<GarrisonEquipmentInputs> {
    Recorded::Value(GarrisonEquipmentInputs {
        rows: vec![GarrisonEquipment { allocated_raw: allocated, required_lookup_raw: required, required_raw: required, weight_raw: weight, ..Default::default() }],
        failure: None,
    })
}

fn garrison(controller: u32, state: u32, occupied: u32, equipment: Recorded<GarrisonEquipmentInputs>) -> StateGarrisonRow {
    StateGarrisonRow { controller, state, occupied, required_manpower: 100, strength_ratio: 7, equipment, ..Default::default() }
}

fn assigned(controller: u32, state: u32, ordinal: u32, occupied: u32, amount: i32) -> StateAssignedManpowerRow {
    StateAssignedManpowerRow { controller, state, ordinal, occupied, source_country: controller, amount }
}

fn strengths(run: &Run) -> Vec<(u128, Recorded<i64>, u8)> {
    run.rows::<GarrisonStrengthRow>(TableId::GarrisonStrengths).into_iter().map(|row| (GarrisonStrengthRow::key_of(row.controller, row.state), row.strength, row.reason)).collect()
}

fn known(controller: u32, state: u32, strength: i64) -> (u128, Recorded<i64>, u8) {
    (GarrisonStrengthRow::key_of(controller, state), Recorded::Value(strength), GarrisonStrengthRow::KNOWN)
}

fn unknown(controller: u32, state: u32, reason: u8) -> (u128, Recorded<i64>, u8) {
    (GarrisonStrengthRow::key_of(controller, state), Recorded::NotRecorded, reason)
}

fn hour_zero() -> Edits {
    let mut edits = Edits::default();
    world(&mut edits)
        .put(TableId::StateGarrisons, garrison(10, 1, 20, operands(50_000, 100_000, 200_000)))
        .put(TableId::StateGarrisons, garrison(10, 2, 20, Recorded::NotRecorded))
        .put(TableId::StateGarrisons, garrison(10, 3, 99, operands(1, 1, 1)))
        .put(TableId::StateGarrisons, StateGarrisonRow { serialized_flag: 254, ..garrison(30, 4, 20, Recorded::NotRecorded) })
        .put(TableId::StateGarrisons, garrison(10, 5, 20, Recorded::NotRecorded))
        .put(TableId::StateGarrisons, garrison(10, 6, 20, Recorded::Value(GarrisonEquipmentInputs { rows: Vec::new(), failure: Some(b"pool moved".to_vec()) })))
        .put(TableId::StateAssignedManpower, assigned(10, 1, 0, 20, 60))
        .put(TableId::StateAssignedManpower, assigned(11, 1, 1, 20, 40))
        .put(TableId::StateAssignedManpower, assigned(10, 1, 2, 30, 500))
        .put(TableId::StateAssignedManpower, assigned(10, 2, 0, 20, 10))
        .put(TableId::StateAssignedManpower, assigned(10, 6, 0, 20, 10));
    edits
}

#[test]
fn garrison_strengths_follow_their_inputs_and_say_why_one_is_not_known() {
    let mut run = run();
    run.hour(&hour_zero());
    assert_eq!(run.status(TableId::GarrisonStrengths), Status::Complete, "{}", run.why(TableId::GarrisonStrengths));
    let first = vec![
        known(10, 1, 87_500),
        unknown(10, 2, GarrisonStrengthRow::OPERANDS),
        unknown(10, 3, GarrisonStrengthRow::UNRESOLVED),
        known(10, 5, 0),
        unknown(10, 6, GarrisonStrengthRow::FAILED),
        known(30, 4, 0),
    ];
    assert_eq!(strengths(&run), first, "state 1 manned 100 of 100 through both links; a row for another occupied country is not its garrison's");

    let mut edits = Edits::default();
    edits.put(TableId::StateAssignedManpower, assigned(11, 1, 1, 20, -10));
    run.hour(&edits);
    let mut second = first;
    second[0] = known(10, 1, 75_000);
    assert_eq!(strengths(&run), second, "half manned: the state's garrison written again, the others as they were");

    let mut edits = Edits::default();
    edits.status(TableId::StateAssignedManpower, Status::Truncated { missing: 1 });
    run.hour(&edits);
    let mut third = second.clone();
    for at in [0, 1, 3, 4] {
        third[at].1 = Recorded::NotRecorded;
        third[at].2 = GarrisonStrengthRow::ASSIGNED;
    }
    assert_eq!(strengths(&run), third, "assignments not read whole: no garrison's manpower is known, a failed requirement still is zero");

    let mut edits = Edits::default();
    edits.put(TableId::StateAssignedManpower, assigned(77, 1, 3, 20, 5));
    run.hour(&edits);
    let mut fourth = second.clone();
    fourth[0] = unknown(10, 1, GarrisonStrengthRow::ASSIGNED);
    assert_eq!(strengths(&run), fourth, "read whole again, with a row of state 1 no link names: its garrison's manpower is not known");

    let mut edits = Edits::default();
    edits
        .delete(TableId::StateAssignedManpower, (77u128 << 64) | (1 << 32) | 3)
        .delete(TableId::StateGarrisons, GarrisonStrengthRow::key_of(10, 2))
        .put(TableId::StateGarrisons, StateGarrisonRow { required_manpower: 50, ..garrison(10, 1, 20, operands(100_000, 100_000, 200_000)) });
    run.hour(&edits);
    assert_eq!(
        strengths(&run),
        vec![known(10, 1, 100_000), unknown(10, 3, GarrisonStrengthRow::UNRESOLVED), known(10, 5, 0), unknown(10, 6, GarrisonStrengthRow::FAILED), known(30, 4, 0)],
        "fully equipped and 50 of 50 manned; a garrison gone takes its row"
    );
}

#[test]
fn absent_strength_powers_withhold_the_table_by_name() {
    let mut run = Run::new(writers, INPUTS);
    run.hour(&hour_zero());
    let why = run.why(TableId::GarrisonStrengths);
    assert!(why.contains("define NResistance.GARRISON_STR_POW_MANPOWER is not held"), "{why}");
    assert!(strengths(&run).is_empty());
    run.define(definitions());
    run.hour(&Edits::default());
    assert_eq!(run.status(TableId::GarrisonStrengths), Status::Complete);
    assert_eq!(strengths(&run).first(), Some(&known(10, 1, 87_500)));
}

#[test]
fn a_resume_writes_what_a_run_that_never_stopped_writes() {
    let mut hours = vec![hour_zero()];
    let mut edits = Edits::default();
    edits.put(TableId::StateAssignedManpower, assigned(11, 1, 1, 20, -10));
    hours.push(edits);
    let mut edits = Edits::default();
    edits.put(TableId::CountryLinks, recorder_derive_framework::testing::deployed::link(99, 2));
    hours.push(edits);
    let mut edits = Edits::default();
    edits.delete(TableId::StateAssignedManpower, (10u128 << 64) | (1 << 32));
    hours.push(edits);
    let mut whole = run();
    let mut seen = Vec::new();
    for edits in &hours {
        whole.hour(edits);
        seen.push(strengths(&whole));
    }
    assert_eq!(seen[2][2], known(10, 3, 0), "a link naming the occupier resolves state 3, assigned nothing");
    for resume in 0..hours.len() - 1 {
        let mut resumed = run();
        for (at, edits) in hours.iter().enumerate() {
            resumed.hour(edits);
            if at == resume {
                resumed = resumed.resumed();
            }
            assert_eq!(strengths(&resumed), seen[at], "resumed after hour {resume}, at hour {at}");
        }
    }
}

#[test]
fn strength_uses_weighted_equipment_and_content_powers_with_each_fixed5_step() {
    let half = [GarrisonEquipment { allocated_raw: 50_000, required_lookup_raw: 100_000, weight_raw: 200_000, ..Default::default() }];
    assert_eq!(displayed(100, 100, &half, (2, 3)), 87_500);
    assert_eq!(displayed(100, 50, &half, (2, 3)), 75_000);
    assert_eq!(displayed(100, 100, &half, (1, 1)), 50_000);
    let full = [GarrisonEquipment { allocated_raw: 100_000, required_lookup_raw: 100_000, weight_raw: 200_000, ..Default::default() }];
    assert_eq!(displayed(100, 100, &full, (2, 3)), 100_000);
    assert_eq!(shortage_power(33_333, 3), 3_703);
    let weighted = [
        GarrisonEquipment { allocated_raw: 100_000, required_lookup_raw: 100_000, weight_raw: 400_000, ..Default::default() },
        GarrisonEquipment { allocated_raw: 0, required_lookup_raw: 100_000, weight_raw: 100_000, ..Default::default() },
    ];
    assert_eq!(displayed(100, 100, &weighted, (1, 1)), 80_000);
    assert_eq!(equipment_fraction(&[GarrisonEquipment { allocated_raw: i64::MAX, required_lookup_raw: i64::MAX, weight_raw: i64::MAX, ..Default::default() }]), 100_000);
    assert_eq!(displayed(100, 100, &[], (2, 3)), 100_000, "no operand rows: fully equipped");
}

#[test]
fn signed_corrections_and_nonpositive_requirements_keep_native_branches() {
    let one = |allocated: i64, required: i64| [GarrisonEquipment { allocated_raw: allocated, required_lookup_raw: required, weight_raw: 100_000, ..Default::default() }];
    assert_eq!(displayed(100, 100, &one(-1, 100_000), (1, 1)), 0);
    assert_eq!(displayed(100, 100, &one(0, 0), (1, 1)), 100_000);
    assert_eq!(displayed(100, 100, &one(50_000, -100_000), (1, 1)), 100_000);
    assert_eq!(displayed(100, -50, &one(100_000, 100_000), (1, 1)), -50_000);
    assert_eq!(displayed(100, 100, &one(150_000, 100_000), (1, 1)), 100_000);
}
