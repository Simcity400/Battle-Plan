use super::super::tests::{frame, put};
use super::*;

fn held_frame(hour: i32, total: i64, rows: Vec<CountryPowerChangeRow>) -> DecodedFrame {
    let mut value = frame(hour, 1);
    put(
        &mut value,
        TableRows::CountryCommandPower(vec![CountryCommandPowerRow {
            canonical_slot: 1,
            allocated_command_power_raw: total,
            ..Default::default()
        }]),
    );
    put(&mut value, TableRows::CountryPowerChanges(rows));
    value
}

fn event(
    hour: i32,
    sequence: u64,
    site: PowerSite,
    cause: PowerCause,
    amount: i64,
) -> CountryPowerChangeRow {
    CountryPowerChangeRow {
        country: 1,
        instance: 1,
        epoch: 1,
        hour,
        sequence,
        currency: PowerCurrency::Command,
        operation: if amount < 0 {
            PowerOperation::Subtract
        } else {
            PowerOperation::Refund
        },
        requested_delta_raw: Some(amount),
        cause,
        site,
        balance_before_raw: 1_000_000,
        balance_after_raw: 1_000_000 + amount,
        identity: PowerIdentity {
            object_kind: PowerObjectKind::Leader,
            object_reference: Some((7_u64 << 32) | 2),
            ..Default::default()
        },
    }
}

fn observe(scan: &mut Scanned, frame: &DecodedFrame) {
    observe_all(frame, scan, None).unwrap();
    observe_holds(frame, scan, None);
}

fn latest(scan: &Scanned) -> LedgerCommandHold {
    present_holds(scan, "C1").pop().unwrap()
}

#[test]
fn empty_snapshots_extend_only_equal_totals_and_refusals_across_gaps_and_resume() {
    let mut fast = CountryHolds::default();
    let mut dense = CountryHolds::default();
    for (hour, total, missing, reused) in [
        (24, Some(0), None, false),
        (25, Some(0), None, true),
        (26, Some(5), None, false),
        (27, Some(5), Some("ignored with a total"), true),
        (28, None, Some("first refusal"), false),
        (29, None, Some("second refusal"), false),
        (30, None, Some("second refusal"), true),
        (32, None, Some("second refusal"), false),
        (33, Some(-2), None, false),
        (34, Some(-2), None, true),
        (35, Some(5), None, false),
        (36, Some(5), None, false),
        (37, Some(5), None, true),
        (38, Some(5), None, false),
    ] {
        if hour == 35 {
            for state in [&mut fast, &mut dense] {
                state.active.insert(
                    "known".into(),
                    Holder {
                        kind: "Leader".into(),
                        item: (&PowerIdentity::default()).into(),
                        amount: 3,
                        cause: PowerCause::LeaderAssignment,
                    },
                );
            }
        }
        if hour == 36 {
            fast.active.clear();
            dense.active.clear();
        }
        if hour == 38 {
            fast = serde_json::from_slice(&serde_json::to_vec(&fast).unwrap()).unwrap();
        }
        let frame = frame(hour, 1);
        let before = snapshot_count();
        let changed = fast.observe_snapshot(&frame, total, None, missing);
        assert_eq!(
            snapshot_count() - before,
            usize::from(!reused),
            "hour {hour}"
        );
        let snapshot = dense.snapshot(&frame, total, None, missing);
        assert_eq!(changed, dense.append(snapshot), "hour {hour}");
        assert_eq!(
            serde_json::to_vec(&fast).unwrap(),
            serde_json::to_vec(&dense).unwrap(),
            "hour {hour}"
        );
    }
}

#[test]
fn allocate_release_reallocate_are_zero_spending_and_held_is_exact_each_hour() {
    let mut scan = Scanned::default();
    observe(&mut scan, &held_frame(24, 0, Vec::new()));
    assert_eq!(latest(&scan).total_raw.as_deref(), Some("0"));
    assert!(latest(&scan).holders.is_empty());
    observe(
        &mut scan,
        &held_frame(
            25,
            300_000,
            vec![event(
                25,
                1,
                PowerSite::AllocateCommandPower,
                PowerCause::AbilityActivation,
                -300_000,
            )],
        ),
    );
    let held = latest(&scan);
    assert_eq!(held.total_raw.as_deref(), Some("300000"));
    assert_eq!(held.holders.len(), 1);
    assert_eq!(held.holders[0].kind, "Leader");
    assert_eq!(held.holders[0].amount_raw.as_deref(), Some("300000"));
    observe(
        &mut scan,
        &held_frame(
            26,
            0,
            vec![event(
                26,
                2,
                PowerSite::AddCommandPower,
                PowerCause::AllocationRefund,
                300_000,
            )],
        ),
    );
    assert_eq!(latest(&scan).total_raw.as_deref(), Some("0"));
    assert!(latest(&scan).holders.is_empty());
    observe(
        &mut scan,
        &held_frame(
            27,
            500_000,
            vec![event(
                27,
                3,
                PowerSite::AllocateCommandPower,
                PowerCause::LeaderAssignment,
                -500_000,
            )],
        ),
    );
    assert_eq!(latest(&scan).total_raw.as_deref(), Some("500000"));
    assert_eq!(
        latest(&scan).holders[0].amount_raw.as_deref(),
        Some("500000")
    );
    let reply = present(&scan, 1, "C1", None, &AtomicBool::new(false)).unwrap();
    assert!(reply.spending.is_empty());
    assert_eq!(present_holds(&scan, "C1").len(), 4);
}

#[test]
fn rename_keeps_native_allocators_and_reused_tag_does_not_inherit_them() {
    let mut scan = Scanned::default();
    observe(
        &mut scan,
        &held_frame(
            24,
            300_000,
            vec![event(
                24,
                1,
                PowerSite::AllocateCommandPower,
                PowerCause::LeaderAssignment,
                -300_000,
            )],
        ),
    );
    let mut renamed = held_frame(
        25,
        400_000,
        vec![event(
            24,
            2,
            PowerSite::AllocateCommandPower,
            PowerCause::LeaderAssignment,
            -100_000,
        )],
    );
    put(
        &mut renamed,
        TableRows::Countries(vec![CountryRow {
            id: 1,
            canonical_slot: 1,
            tag: b"C2".to_vec(),
            ..Default::default()
        }]),
    );
    put(
        &mut renamed,
        TableRows::PoliticsCaptureCoverage(vec![PoliticsCaptureCoverageRow {
            country: 1,
            instance: 1,
            epoch: 1,
            since_hour: 24,
            through_hour: 25,
            streams: PoliticsCaptureCoverageRow::ALL_STREAMS,
        }]),
    );
    observe(&mut scan, &renamed);
    let held = present_holds(&scan, "C2").pop().unwrap();
    assert_eq!(held.holders.len(), 1);
    assert_eq!(held.holders[0].kind, "Leader");
    assert_eq!(held.holders[0].amount_raw.as_deref(), Some("400000"));
    assert_eq!(latest(&scan).total_raw.as_deref(), Some("0"));

    let mut reused = held_frame(26, 300_000, Vec::new());
    put(
        &mut reused,
        TableRows::PoliticsCaptureCoverage(vec![PoliticsCaptureCoverageRow {
            country: 1,
            instance: 2,
            epoch: 1,
            since_hour: 26,
            through_hour: 26,
            streams: PoliticsCaptureCoverageRow::ALL_STREAMS,
        }]),
    );
    observe(&mut scan, &reused);
    assert_eq!(latest(&scan).holders.len(), 1);
    assert_eq!(latest(&scan).holders[0].kind, "Not identified");
}

#[test]
fn native_sites_separate_holds_from_upfront_upkeep_and_raid_terminal_payments() {
    let mut scan = Scanned::default();
    let rows = vec![
        event(
            24,
            1,
            PowerSite::AllocateCommandPower,
            PowerCause::Raid,
            -500_000,
        ),
        event(24, 2, PowerSite::GroundCrew, PowerCause::AirAction, -40_000),
        event(
            24,
            3,
            PowerSite::ActivateAbility,
            PowerCause::AbilityActivation,
            -30_000,
        ),
        event(
            24,
            4,
            PowerSite::AbilityUpkeep,
            PowerCause::AbilityUpkeep,
            -20_000,
        ),
        event(
            24,
            5,
            PowerSite::SpendCommandPower,
            PowerCause::Raid,
            -10_000,
        ),
        event(
            24,
            6,
            PowerSite::SpendCommandPower,
            PowerCause::AirAction,
            -5_000,
        ),
    ];
    observe(&mut scan, &held_frame(24, 540_000, rows));
    let reply = present(&scan, 1, "C1", None, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.spending.len(), 4);
    assert_eq!(
        reply
            .spending
            .iter()
            .map(|row| row.charge_raw.as_deref().unwrap().parse::<i128>().unwrap())
            .sum::<i128>(),
        65_000
    );
    assert_eq!(latest(&scan).total_raw.as_deref(), Some("540000"));
    assert!(
        reply
            .mutations
            .iter()
            .filter(|row| is_hold(row))
            .all(|row| charge(row).is_none())
    );
}

#[test]
fn nonzero_start_and_anonymous_release_keep_holder_remainder_unidentified() {
    let mut scan = Scanned::default();
    observe(&mut scan, &held_frame(24, 400_000, Vec::new()));
    assert_eq!(latest(&scan).holders[0].kind, "Not identified");
    observe(
        &mut scan,
        &held_frame(
            25,
            700_000,
            vec![event(
                25,
                1,
                PowerSite::AllocateCommandPower,
                PowerCause::LeaderAssignment,
                -300_000,
            )],
        ),
    );
    assert_eq!(latest(&scan).holders.len(), 2);
    let mut release = event(
        26,
        2,
        PowerSite::AddCommandPower,
        PowerCause::AllocationRefund,
        100_000,
    );
    release.identity = PowerIdentity::default();
    observe(&mut scan, &held_frame(26, 600_000, vec![release]));
    let held = latest(&scan);
    assert_eq!(held.holders.len(), 1);
    assert_eq!(held.holders[0].kind, "Not identified");
    assert_eq!(held.holders[0].amount_raw.as_deref(), Some("600000"));
    assert!(held.reason.is_some());
}

#[test]
fn gaps_and_missing_reads_end_attribution_and_absence_proves_zero() {
    let mut scan = Scanned::default();
    observe(
        &mut scan,
        &held_frame(
            24,
            300_000,
            vec![event(
                24,
                1,
                PowerSite::AllocateCommandPower,
                PowerCause::LeaderAssignment,
                -300_000,
            )],
        ),
    );
    observe(&mut scan, &held_frame(26, 300_000, Vec::new()));
    let runs = present_holds(&scan, "C1");
    assert_eq!(runs[0].through_hour, 24);
    assert_eq!(runs[1].read_hour, 26);
    assert_eq!(runs[1].holders[0].kind, "Not identified");
    let mut absent = held_frame(27, 0, Vec::new());
    put(&mut absent, TableRows::Countries(Vec::new()));
    observe(&mut scan, &absent);
    assert_eq!(latest(&scan).total_raw.as_deref(), Some("0"));
    let mut missing = held_frame(28, 0, Vec::new());
    missing.tables[TableId::CountryCommandPower.index()] =
        replay_frame::frame::TableView::Unavailable {
            reason: "synthetic gap".into(),
        }
        .into();
    observe(&mut scan, &missing);
    assert!(latest(&scan).total_raw.is_none());
    assert!(latest(&scan).holders.is_empty());
}

#[test]
fn five_thousand_countries_merge_unchanged_held_snapshots_without_player_gates() {
    let mut scan = Scanned::default();
    for hour in 24..=25 {
        let mut value = frame(hour, 5_000);
        put(
            &mut value,
            TableRows::CountryCommandPower(
                (1..=5_000)
                    .map(|canonical_slot| CountryCommandPowerRow {
                        canonical_slot,
                        allocated_command_power_raw: i64::from(canonical_slot),
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        observe(&mut scan, &value);
    }
    assert_eq!(scan.holds.countries.len(), 5_000);
    for tag in ["C1", "C1500", "C5000"] {
        let held = present_holds(&scan, tag);
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].read_hour, 24);
        assert_eq!(held[0].through_hour, 25);
        assert!(held[0].total_raw.is_some());
    }
}

#[test]
fn signed_native_held_total_is_preserved_without_clamping_or_guessing() {
    let mut scan = Scanned::default();
    observe(
        &mut scan,
        &held_frame(
            24,
            300_000,
            vec![event(
                24,
                1,
                PowerSite::AllocateCommandPower,
                PowerCause::LeaderAssignment,
                -300_000,
            )],
        ),
    );
    observe(&mut scan, &held_frame(25, -123_456, Vec::new()));
    let held = latest(&scan);
    assert_eq!(held.total_raw.as_deref(), Some("-123456"));
    assert_eq!(held.holders.len(), 1);
    assert_eq!(held.holders[0].kind, "Not identified");
    assert_eq!(held.holders[0].amount_raw.as_deref(), Some("-123456"));
}

#[test]
fn raw_country_alias_coverage_joins_the_same_canonical_held_total() {
    let mut scan = Scanned::default();
    let mut allocation = event(
        24,
        1,
        PowerSite::AllocateCommandPower,
        PowerCause::LeaderAssignment,
        -300_000,
    );
    allocation.country = 100;
    let mut value = held_frame(24, 300_000, vec![allocation]);
    put(
        &mut value,
        TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 1,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 100,
                canonical_slot: 1,
            },
        ]),
    );
    put(
        &mut value,
        TableRows::PoliticsCaptureCoverage(vec![PoliticsCaptureCoverageRow {
            country: 100,
            instance: 1,
            epoch: 1,
            since_hour: 24,
            through_hour: 24,
            streams: PoliticsCaptureCoverageRow::ALL_STREAMS,
        }]),
    );
    observe(&mut scan, &value);
    let held = latest(&scan);
    assert_eq!(held.total_raw.as_deref(), Some("300000"));
    assert_eq!(held.holders.len(), 1);
    assert_eq!(held.holders[0].kind, "Leader");
}

#[test]
fn uncovered_allocation_does_not_attribute_a_recorded_total_to_its_holder() {
    let mut scan = Scanned::default();
    let mut value = held_frame(
        24,
        300_000,
        vec![event(
            24,
            1,
            PowerSite::AllocateCommandPower,
            PowerCause::LeaderAssignment,
            -300_000,
        )],
    );
    put(
        &mut value,
        TableRows::PoliticsCaptureCoverage(vec![PoliticsCaptureCoverageRow {
            country: 1,
            instance: 1,
            epoch: 1,
            since_hour: 24,
            through_hour: 24,
            streams: PoliticsCaptureCoverageRow::PP,
        }]),
    );
    observe(&mut scan, &value);
    let held = latest(&scan);
    assert_eq!(held.total_raw.as_deref(), Some("300000"));
    assert_eq!(held.holders.len(), 1);
    assert_eq!(held.holders[0].kind, "Not identified");
}

#[test]
fn different_recorded_abilities_on_one_holder_keep_distinct_identities() {
    let mut scan = Scanned::default();
    let mut a = event(
        24,
        1,
        PowerSite::AllocateCommandPower,
        PowerCause::AbilityActivation,
        -100_000,
    );
    a.identity.item_key = b"synthetic_a".to_vec();
    let mut b = event(
        24,
        2,
        PowerSite::AllocateCommandPower,
        PowerCause::AbilityActivation,
        -200_000,
    );
    b.identity.item_key = b"synthetic_b".to_vec();
    observe(&mut scan, &held_frame(24, 300_000, vec![a, b]));
    let held = latest(&scan);
    assert_eq!(held.holders.len(), 2);
    assert_ne!(held.holders[0].id, held.holders[1].id);
    assert_eq!(
        held.holders
            .iter()
            .map(|holder| holder
                .amount_raw
                .as_deref()
                .unwrap()
                .parse::<i128>()
                .unwrap())
            .sum::<i128>(),
        300_000
    );
}

#[test]
fn later_invalid_tag_refuses_only_its_known_country_instead_of_proving_zero_held() {
    let mut scan = Scanned::default();
    let mut value = frame(24, 2);
    put(
        &mut value,
        TableRows::CountryCommandPower(vec![
            CountryCommandPowerRow {
                canonical_slot: 1,
                allocated_command_power_raw: 300_000,
                ..Default::default()
            },
            CountryCommandPowerRow {
                canonical_slot: 2,
                allocated_command_power_raw: 50_000,
                ..Default::default()
            },
        ]),
    );
    observe(&mut scan, &value);
    assert_eq!(latest(&scan).total_raw.as_deref(), Some("300000"));
    let mut value = frame(25, 2);
    put(
        &mut value,
        TableRows::Countries(vec![
            CountryRow {
                id: 1,
                canonical_slot: 1,
                tag: vec![b'C', 0xff],
                ..Default::default()
            },
            CountryRow {
                id: 2,
                canonical_slot: 2,
                tag: b"C2".to_vec(),
                ..Default::default()
            },
        ]),
    );
    put(
        &mut value,
        TableRows::CountryCommandPower(vec![
            CountryCommandPowerRow {
                canonical_slot: 1,
                allocated_command_power_raw: 300_000,
                ..Default::default()
            },
            CountryCommandPowerRow {
                canonical_slot: 2,
                allocated_command_power_raw: 50_000,
                ..Default::default()
            },
        ]),
    );
    observe(&mut scan, &value);
    let refused = latest(&scan);
    assert!(refused.total_raw.is_none());
    let reason = refused.reason.as_deref().unwrap();
    assert!(reason.contains("country 1 canonical slot 1"));
    assert!(reason.contains("not UTF-8"));
    assert!(reason.contains("count: 1"));
    let other = present_holds(&scan, "C2");
    assert_eq!(other.last().unwrap().total_raw.as_deref(), Some("50000"));
    assert!(
        !other
            .last()
            .unwrap()
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("not UTF-8")
    );
}
