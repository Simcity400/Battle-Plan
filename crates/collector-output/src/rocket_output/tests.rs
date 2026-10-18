use super::*;
use replay_data::rows::RowCodec;

const DESIGN: u64 = 0x99_0000_0046;
const OTHER: u64 = 0x100_0000_0046;

fn spending(country: u32, variant: u64, spent: i64, instant: i64) -> Entry {
    Entry::Spending(RocketTick {
        country,
        variant,
        spent_raw: spent,
        instant_raw: instant,
        failure: RocketFailure::None,
    })
}

fn scheduled() -> TableInput {
    TableInput::Sampled(TableView::Unavailable {
        reason: "equipment census unavailable".into(),
    })
}

fn skipped() -> TableInput {
    TableInput::Sampled(TableView::Unavailable {
        reason: replay_data::cadence::SKIPPED_REASON.into(),
    })
}

fn drain(epoch: u64, first: u64, entries: &[Entry]) -> Drained {
    Drained {
        epoch,
        first,
        cursor: first + entries.len() as u64,
        copied: entries.len(),
        pending: entries.len(),
    }
}

fn sample(totals: &mut Totals, hour: i32, first: u64, entries: &[Entry]) {
    totals
        .observe(
            7,
            hour,
            Some(Ok(drain(1, first, entries))),
            entries,
            |entry| *entry,
        )
        .unwrap();
}

fn rows(totals: &Totals) -> Vec<RocketProductionOutputRow> {
    totals.publication(&scheduled()).unwrap().unwrap()
}

fn find(totals: &Totals, country: u32, variant: u64) -> RocketProductionOutputRow {
    rows(totals)
        .into_iter()
        .find(|row| (row.country, row.variant) == (country, variant))
        .unwrap()
}

#[test]
fn first_frame_discards_old_events_and_certifies_complete_empty_without_census() {
    let mut totals = Totals::default();
    sample(&mut totals, 100, 10, &[spending(5001, DESIGN, 7, 0)]);
    assert_eq!(rows(&totals), [row(0, 0, 100)]);
    assert_eq!(
        totals.publication(&skipped()).unwrap(),
        Err(replay_data::cadence::SKIPPED_REASON)
    );
    sample(&mut totals, 124, 11, &[spending(5001, DESIGN, -175_003, 0)]);
    assert_eq!(find(&totals, 5001, DESIGN).spent_raw, -175_003);
    assert_eq!(find(&totals, 5001, 0).spent_raw, 0);
    assert_eq!(find(&totals, 0, 0).ticks, 1);
}

#[test]
fn signed_spending_instant_and_delivery_sum_independently() {
    let mut totals = Totals::default();
    sample(&mut totals, 100, 0, &[]);
    let scope = RocketTick {
        country: 5001,
        variant: DESIGN,
        failure: RocketFailure::Amount,
        ..Default::default()
    };
    let entries = [
        spending(5001, DESIGN, -175_003, 0),
        spending(5001, DESIGN, 0, -950_009),
        Entry::delivery(Some(scope), DESIGN, -2 * UNIT_SCALE),
    ];
    sample(&mut totals, 124, 0, &entries);
    let design = find(&totals, 5001, DESIGN);
    assert_eq!(
        (design.spent_raw, design.instant_raw, design.units),
        (-175_003, -950_009, -2)
    );
    assert_eq!((design.ticks, design.deliveries), (2, 1));
    assert!(design.exact && design.units_exact);
    for row in rows(&totals) {
        row.check_key_fields().unwrap();
    }
}

#[test]
fn known_country_failure_is_local_and_unknown_owner_is_world_wide() {
    let mut totals = Totals::default();
    sample(&mut totals, 100, 0, &[]);
    let local = Entry::Spending(RocketTick {
        country: 5001,
        variant: 0,
        failure: RocketFailure::Design,
        ..Default::default()
    });
    sample(&mut totals, 124, 0, &[local, spending(5002, OTHER, 99, 0)]);
    assert!(find(&totals, 0, 0).exact);
    assert!(!find(&totals, 5001, 0).exact);
    assert!(find(&totals, 5002, 0).exact);
    assert_eq!(find(&totals, 0, 0).unknown_design_ticks, 1);
    let unknown = Entry::Spending(RocketTick {
        country: 0,
        variant: DESIGN,
        failure: RocketFailure::Country,
        ..Default::default()
    });
    sample(&mut totals, 148, 2, &[unknown]);
    assert!(!find(&totals, 0, 0).exact);
    assert!(!find(&totals, 0, DESIGN).exact);
    assert_eq!(find(&totals, 0, 0).unknown_country_ticks, 1);
    assert!(find(&totals, 5002, 0).exact);
    for row in rows(&totals) {
        row.check_key_fields().unwrap();
    }
}

#[test]
fn failed_spending_never_invalidates_known_delivery_and_fractional_delivery_is_unknown() {
    let mut totals = Totals::default();
    sample(&mut totals, 100, 0, &[]);
    let failed = RocketTick {
        country: 5001,
        variant: DESIGN,
        failure: RocketFailure::Instant,
        ..Default::default()
    };
    sample(
        &mut totals,
        124,
        0,
        &[
            Entry::Spending(failed),
            Entry::delivery(Some(failed), DESIGN, 3 * UNIT_SCALE),
        ],
    );
    let design = find(&totals, 5001, DESIGN);
    assert!(!design.exact && design.units_exact);
    assert_eq!((design.units, design.inexact_ticks), (3, 1));
    sample(
        &mut totals,
        148,
        2,
        &[Entry::delivery(Some(failed), DESIGN, 1)],
    );
    let design = find(&totals, 5001, DESIGN);
    assert!(!design.units_exact);
    assert_eq!(
        (design.units, design.deliveries, design.inexact_deliveries),
        (3, 2, 1)
    );
    assert!(find(&totals, 0, 0).units_exact);
    // A fresh DELIVERY design can succeed when the earlier spending read could
    // not authenticate it; only a proved conflicting entry CID is a mismatch.
    let no_design = RocketTick {
        country: 5002,
        failure: RocketFailure::Design,
        ..Default::default()
    };
    sample(
        &mut totals,
        172,
        3,
        &[
            Entry::Spending(no_design),
            Entry::delivery(Some(no_design), OTHER, UNIT_SCALE),
        ],
    );
    assert!(!find(&totals, 5002, 0).exact);
    assert!(find(&totals, 5002, 0).units_exact);
    assert!(find(&totals, 5002, OTHER).units_exact);
    assert_eq!(find(&totals, 5002, OTHER).units, 1);
}

#[test]
fn signed_native_i32_delivery_bounds_and_scope_mismatch_are_not_ready_rockets() {
    let scope = RocketTick {
        country: 5001,
        variant: DESIGN,
        ..Default::default()
    };
    for units in [i32::MIN, -2, 0, 2, i32::MAX] {
        assert!(
            matches!(Entry::delivery(Some(scope), DESIGN, i64::from(units) * UNIT_SCALE), Entry::Delivery { units: Some(value), coherent: true, .. } if value == units)
        );
    }
    for amount in [
        1,
        i64::from(i32::MAX) * UNIT_SCALE + UNIT_SCALE,
        i64::from(i32::MIN) * UNIT_SCALE - UNIT_SCALE,
    ] {
        assert!(matches!(
            Entry::delivery(Some(scope), DESIGN, amount),
            Entry::Delivery { units: None, .. }
        ));
    }
    assert!(matches!(
        Entry::delivery(Some(scope), OTHER, UNIT_SCALE),
        Entry::Delivery {
            coherent: false,
            ..
        }
    ));
    let mut totals = Totals::default();
    sample(&mut totals, 100, 0, &[]);
    sample(
        &mut totals,
        124,
        0,
        &[Entry::delivery(None, DESIGN, UNIT_SCALE)],
    );
    assert!(!find(&totals, 0, 0).units_exact);
    assert!(find(&totals, 0, 0).exact);
    assert_eq!(find(&totals, 0, DESIGN).units, 1);
}

#[test]
fn totals_persist_after_removal_and_keep_full_design_generation_and_country() {
    let mut totals = Totals::default();
    sample(&mut totals, 100, 0, &[]);
    sample(
        &mut totals,
        124,
        0,
        &[
            spending(5001, DESIGN, 10, 0),
            spending(5001, OTHER, 20, 0),
            spending(5002, DESIGN, 30, 0),
        ],
    );
    let before = rows(&totals);
    for hour in [148, 172, 196] {
        sample(&mut totals, hour, 3, &[]);
    }
    assert_eq!(rows(&totals), before);
    assert_eq!(before.len(), 6);
    assert_eq!(find(&totals, 5001, DESIGN).spent_raw, 10);
    assert_eq!(find(&totals, 5001, OTHER).spent_raw, 20);
    assert_eq!(find(&totals, 5002, DESIGN).spent_raw, 30);
}

#[test]
fn duplicate_drains_are_not_double_counted_and_partial_backlogs_do_not_publish() {
    let mut totals = Totals::default();
    sample(&mut totals, 100, 0, &[]);
    let entries = [spending(5001, DESIGN, 10, 0), spending(5001, DESIGN, 20, 0)];
    let mut first = drain(1, 0, &entries[..1]);
    first.cursor = 2;
    first.pending = 2;
    totals
        .observe(7, 124, Some(Ok(first)), &entries[..1], |entry| *entry)
        .unwrap();
    assert_eq!(
        totals.publication(&scheduled()).unwrap(),
        Err("rocket events before this sample are still in the log")
    );
    totals
        .observe(7, 125, Some(Ok(drain(1, 0, &entries))), &entries, |entry| {
            *entry
        })
        .unwrap();
    totals
        .observe(7, 126, Some(Ok(drain(1, 0, &entries))), &entries, |entry| {
            *entry
        })
        .unwrap();
    assert_eq!(find(&totals, 5001, DESIGN).spent_raw, 30);
    assert_eq!(find(&totals, 0, 0).ticks, 2);
    sample(
        &mut totals,
        148,
        2,
        &[
            Entry::Equipment,
            spending(5001, DESIGN, 1, 0),
            Entry::Equipment,
        ],
    );
    assert_eq!(find(&totals, 5001, DESIGN).spent_raw, 31);
    assert_eq!(find(&totals, 0, 0).ticks, 3);
    sample(&mut totals, 172, 5, &[]);
    assert_eq!(find(&totals, 0, 0).ticks, 3);
}

#[test]
fn lost_or_unobserved_interval_withholds_until_a_new_branch() {
    for lost in [
        Some(Err("writer escaped")),
        None,
        Some(Ok(Drained {
            epoch: 2,
            ..Default::default()
        })),
        Some(Ok(Drained {
            epoch: 1,
            first: 1,
            cursor: 1,
            ..Default::default()
        })),
    ] {
        let mut totals = Totals::default();
        sample(&mut totals, 100, 0, &[]);
        totals
            .observe(7, 124, lost, &[] as &[Entry], |entry| *entry)
            .unwrap();
        assert!(totals.publication(&scheduled()).unwrap().is_err());
        sample(&mut totals, 148, 0, &[]);
        assert!(totals.publication(&scheduled()).unwrap().is_err());
        totals
            .observe(
                8,
                148,
                Some(Ok(drain(1, 0, &[]))),
                &[] as &[Entry],
                |entry| *entry,
            )
            .unwrap();
        assert_eq!(rows(&totals), [row(0, 0, 148)]);
    }
}

#[test]
fn checked_amount_overflow_stays_unknown_and_does_not_poison_another_country() {
    let mut totals = Totals::default();
    sample(&mut totals, 100, 0, &[]);
    sample(&mut totals, 124, 0, &[spending(5001, DESIGN, i64::MAX, 0)]);
    sample(
        &mut totals,
        148,
        1,
        &[spending(5001, DESIGN, 1, -3), spending(5002, DESIGN, 20, 0)],
    );
    assert!(!find(&totals, 5001, DESIGN).exact);
    assert!(!find(&totals, 5001, 0).exact);
    assert!(find(&totals, 0, 0).exact);
    assert!(find(&totals, 5002, DESIGN).exact);
    assert_eq!(find(&totals, 5001, DESIGN).instant_raw, -3);
    totals.rows.get_mut(&(5002, DESIGN)).unwrap().units = i64::MAX;
    let scope = RocketTick {
        country: 5002,
        variant: DESIGN,
        ..Default::default()
    };
    sample(
        &mut totals,
        149,
        3,
        &[Entry::delivery(Some(scope), DESIGN, UNIT_SCALE)],
    );
    assert!(!find(&totals, 5002, DESIGN).units_exact);
    assert!(!find(&totals, 5002, 0).units_exact);
    assert!(find(&totals, 5002, DESIGN).exact);
    assert!(find(&totals, 0, 0).units_exact);
    sample(&mut totals, 172, 4, &[spending(5001, DESIGN, -1, 0)]);
    assert!(!find(&totals, 5001, DESIGN).exact);
    // Counter overflow cannot masquerade as a complete prefix.
    totals.rows.get_mut(&(0, 0)).unwrap().ticks = u64::MAX;
    sample(&mut totals, 196, 5, &[spending(5002, DESIGN, 1, 0)]);
    assert_eq!(
        totals.publication(&scheduled()).unwrap(),
        Err("rocket spending counter overflow")
    );
}

#[test]
fn malformed_owned_drains_refuse_and_live_sized_country_rows_have_no_population_cap() {
    let mut totals = Totals::default();
    sample(&mut totals, 100, 0, &[]);
    totals
        .observe(
            7,
            124,
            Some(Ok(Drained {
                epoch: 1,
                copied: 1,
                cursor: 1,
                ..Default::default()
            })),
            &[] as &[Entry],
            |entry| *entry,
        )
        .unwrap();
    assert_eq!(
        totals.publication(&scheduled()).unwrap(),
        Err("rocket drain entry count exceeds owned entries")
    );
    let mut totals = Totals::default();
    sample(&mut totals, 100, 0, &[]);
    let entries: Vec<_> = (1..=5000)
        .map(|country| spending(country, DESIGN, if country % 2 == 0 { -1 } else { 1 }, 0))
        .collect();
    sample(&mut totals, 124, 0, &entries);
    assert_eq!(rows(&totals).len(), 10_001);
    assert_eq!(find(&totals, 0, 0).ticks, 5000);
    assert!(find(&totals, 5000, DESIGN).exact);
    assert_eq!(find(&totals, 5000, DESIGN).spent_raw, -1);
}
