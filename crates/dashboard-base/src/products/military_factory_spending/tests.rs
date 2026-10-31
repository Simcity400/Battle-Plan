use super::*;
use crate::products::equipment::production::{Named, ProductionSpendingUnknown, Purpose};
use crate::products::production_spending::{
    prepared::Answered,
    prepared::tests::{indexed, prepared, with},
    tests::{applied, day, hour, line, units},
};
use crate::products::purpose_split::{PurposeMonth, month_of, months_of};
use replay_data::rows::{OutputPurpose, ProductionLineOutputRow, TableId, TableRows};
use replay_frame::frame::{DecodedFrame, TableView};
use std::sync::atomic::AtomicBool;

// The synthetic content's designs: 1 a rifle (land), 2 a plane (air), 3 a
// ship (naval); 5 has no recorded definition, so its type cannot resolve.
const RIFLE: u64 = 1;
const PLANE: u64 = 2;
const SHIP: u64 = 3;
const UNTYPED: u64 = 5;

/// Each day from day 0 to `last`, with the outputs `outputs` gives that day.
fn days(
    last: i32,
    outputs: impl Fn(i32) -> Option<Vec<ProductionLineOutputRow>>,
) -> Vec<DecodedFrame> {
    let lines = vec![
        line(1, PLANE, 100_000),
        line(2, RIFLE, 100_000),
        line(3, SHIP, 100_000),
    ];
    (0..=last)
        .flat_map(|index| day(index, lines.clone(), outputs(index)))
        .collect()
}

/// `domain`'s months from the first hour through day `last`'s end.
fn months(frames: &[DecodedFrame], domain: Domain, last: i32) -> Vec<PurposeMonth> {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let flag = AtomicBool::new(false);
    let units = units();
    let named = Named {
        units: Some(&units),
        texts: None,
    };
    let (_, fixture) = prepared(frames, &flag);
    let (index, classes) = indexed(&fixture, named);
    let answered = Answered {
        branch: 1,
        bounds: (hour(0), hour(last) + 23),
        from: None,
        hour: hour(last) + 23,
        end: hour(last) + 23,
    };
    months_of(
        &index,
        &classes,
        &fixture.store,
        &fixture.key,
        "opaque|tag",
        (hour(0), hour(last) + 23),
        &answered,
        domain,
        &flag,
    )
    .unwrap()
}

fn amount(month: &PurposeMonth, purpose: Purpose) -> Result<i128, ProductionSpendingUnknown> {
    month.amounts.map(|amounts| amounts[purpose.index()])
}

/// The first day whose read falls in the month after day 0's.
fn next_month() -> i32 {
    (1..)
        .find(|&index| month_of(hour(index)).0 != month_of(hour(0)).0)
        .unwrap()
}

#[test]
fn a_design_built_new_then_converted_splits_exactly_by_month_and_domain() {
    let next = next_month();
    // Line 1 builds planes until the day before the month ends, then
    // converts into the same design: a second bucket. Land and naval lines
    // and a Rocket site of the plane design spend too.
    let switch = next - 1;
    let frames = {
        let mut frames = days(next + 1, |index| {
            let mut rows = vec![
                applied(1, PLANE, 0, 1_000 * i64::from(index.min(switch))),
                applied(2, RIFLE, 0, 500 * i64::from(index)),
                with(applied(3, SHIP, 0, 300 * i64::from(index)), OutputPurpose::Refit),
            ];
            if index >= switch {
                let mut converted = with(
                    applied(1, PLANE, 0, 400 * i64::from(index - switch + 1)),
                    OutputPurpose::Converted,
                );
                converted.ordinal = 1;
                // Rows ascend by line and ordinal: line 1's second row.
                rows.insert(1, converted);
            }
            Some(rows)
        });
        for frame in &mut frames {
            if let TableView::Complete(TableRows::RocketProductionOutput(rows)) =
                &mut *frame.tables[TableId::RocketProductionOutput.index()]
            {
                rows.push(replay_data::rows::RocketProductionOutputRow {
                    country: 1,
                    variant: PLANE,
                    since_hour: hour(0),
                    ticks: 1,
                    spent_raw: 9_999,
                    exact: true,
                    units_exact: true,
                    ..Default::default()
                });
            }
        }
        frames
    };
    let air = months(&frames, Domain::Air, next + 1);
    assert_eq!(air.len(), 2);
    // The first month: new planes through the switch, then one day converted.
    assert_eq!(
        amount(&air[0], Purpose::New),
        Ok(1_000 * i128::from(switch))
    );
    assert_eq!(amount(&air[0], Purpose::Converted), Ok(400));
    // The next month converts only, and its new input is exactly zero.
    assert_eq!(amount(&air[1], Purpose::New), Ok(0));
    assert_eq!(amount(&air[1], Purpose::Converted), Ok(800));
    // Land, naval and Rocket input never reaches the air months.
    assert!(air.iter().all(|month| amount(month, Purpose::Refit) == Ok(0)));
    assert!(air.iter().all(|month| !month.unresolved && !month.purpose_unproven));
    let land = months(&frames, Domain::Land, next + 1);
    assert_eq!(
        amount(&land[0], Purpose::New),
        Ok(500 * i128::from(next - 1))
    );
    assert_eq!(amount(&land[1], Purpose::Converted), Ok(0));
}

#[test]
fn unrecorded_purpose_marks_its_month_however_its_input_nets_and_never_counts_as_proved() {
    // Lines 4 and 5 build planes in ticks whose purpose was not recorded: +1
    // and -1 IC on day 1, which nets to nothing.
    let frames = days(2, |index| {
        let unrecorded =
            |line, spent| with(applied(line, PLANE, 0, spent), OutputPurpose::Unknown);
        let mut rows = vec![applied(1, PLANE, 0, 1_000)];
        if index >= 1 {
            rows.extend([unrecorded(4, 100_000), unrecorded(5, -100_000)]);
        }
        Some(rows)
    });
    let air = months(&frames, Domain::Air, 2);
    assert_eq!(air.len(), 1);
    assert_eq!(amount(&air[0], Purpose::New), Ok(1_000));
    assert_eq!(amount(&air[0], Purpose::Unknown), Ok(0));
    assert!(air[0].purpose_unproven, "cancelling input stays uncertain");
    // Unrecorded input that does not net away is its own amount, never new.
    let frames = days(2, |index| {
        let mut rows = vec![applied(1, PLANE, 0, 1_000)];
        if index >= 1 {
            rows.push(with(applied(4, PLANE, 0, 60_000), OutputPurpose::Unknown));
        }
        Some(rows)
    });
    let air = months(&frames, Domain::Air, 2);
    assert_eq!(amount(&air[0], Purpose::New), Ok(1_000));
    assert_eq!(amount(&air[0], Purpose::Unknown), Ok(60_000));
    assert!(air[0].purpose_unproven);
}

#[test]
fn an_untyped_design_marks_its_month_and_missing_output_is_unknown_not_zero() {
    let frames = days(2, |index| {
        let rows = vec![
            applied(1, PLANE, 0, 1_000 * i64::from(index)),
            applied(6, UNTYPED, 0, 70 * i64::from(index)),
        ];
        // Day 2's publication is missed.
        (index < 2).then_some(rows)
    });
    let air = months(&frames, Domain::Air, 2);
    assert_eq!(air.len(), 1);
    // The untyped design could be air: the month says so.
    assert!(air[0].unresolved);
    // A missed publication leaves the month unknown, never zero.
    assert_eq!(
        amount(&air[0], Purpose::New),
        Err(ProductionSpendingUnknown::MissedPublication)
    );
}

/// 1 IC of new planes beside 2 IC of an untyped design's refit input: refits
/// are not air's, but the untyped design could still be an air design whose
/// input stays in the whole, so the month's total is not an exact 1 IC.
#[test]
fn untyped_input_of_a_purpose_the_domain_never_shows_still_marks_its_month() {
    let frames = days(1, |index| {
        Some(vec![
            applied(1, PLANE, 0, 100_000 * i64::from(index)),
            with(applied(6, UNTYPED, 0, 200_000 * i64::from(index)), OutputPurpose::Refit),
        ])
    });
    let air = months(&frames, Domain::Air, 1);
    assert_eq!(amount(&air[0], Purpose::New), Ok(100_000));
    assert!(air[0].unresolved, "the month's whole may lack the untyped input");
}

