use super::*;
use replay::frame::{TABLE_COUNT, TableView};
pub(super) fn frame(hour: i32, count: u32) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour,
        tables: std::array::from_fn::<_, TABLE_COUNT, _>(|_| {
            TableView::Unavailable {
                reason: "synthetic".into(),
            }
            .into()
        }),
        metrics: Default::default(),
    };
    put(
        &mut frame,
        TableRows::Countries(
            (1..=count)
                .map(|id| CountryRow {
                    id,
                    canonical_slot: id,
                    tag: format!("C{id}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::CountryLinks(
            (1..=count)
                .map(|id| CountryLinkRow {
                    id,
                    canonical_slot: id,
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::PoliticsCaptureCoverage(
            (1..=count)
                .map(|id| PoliticsCaptureCoverageRow {
                    country: id,
                    instance: 1,
                    epoch: 1,
                    since_hour: hour,
                    through_hour: hour,
                    streams: PoliticsCaptureCoverageRow::ALL_STREAMS,
                })
                .collect(),
        ),
    );
    for rows in [
        TableRows::CountryPowerChanges(Vec::new()),
        TableRows::CountryPowerDaily(Vec::new()),
        TableRows::CountryPowerDailyCosts(Vec::new()),
        TableRows::GovernmentChanges(Vec::new()),
    ] {
        put(&mut frame, rows);
    }
    frame
}
pub(super) fn put(frame: &mut DecodedFrame, rows: TableRows) {
    let index = rows.table().index();
    frame.tables[index] = TableView::Complete(rows).into();
}
pub(super) fn event(
    sequence: u64,
    cause: PowerCause,
    operation: PowerOperation,
    amount: Option<i64>,
) -> CountryPowerChangeRow {
    CountryPowerChangeRow {
        country: 1,
        instance: 1,
        epoch: 1,
        hour: 24,
        sequence,
        cause,
        operation,
        requested_delta_raw: amount,
        balance_before_raw: 10,
        balance_after_raw: 0,
        ..Default::default()
    }
}
fn present_fixture(scan: &Scanned) -> PoliticsLedgerReply {
    present(scan, 1, "C1", None, &AtomicBool::new(false)).unwrap()
}

#[test]
fn published_keys_survive_failed_reads_and_count_repeated_complete_rows_once() {
    let mut first = frame(24, 1);
    put(
        &mut first,
        TableRows::CountryPowerChanges(vec![event(
            1,
            PowerCause::Decision,
            PowerOperation::Subtract,
            Some(-17),
        )]),
    );
    put(
        &mut first,
        TableRows::CountryPowerDailyCosts(vec![CountryPowerDailyCostRow {
            country: 1,
            instance: 1,
            epoch: 1,
            day_hour: 24,
            pp_sequence: 2,
            cost_raw: 7,
            ..Default::default()
        }]),
    );
    let mut scan = Scanned::default();
    observe_all(&first, &mut scan, None).unwrap();
    scan.prune_publications(&first);

    let mut failed = frame(25, 1);
    for table in [
        TableId::CountryPowerChanges,
        TableId::CountryPowerDailyCosts,
    ] {
        failed.tables[table.index()] = TableView::Unavailable {
            reason: "synthetic failed publication".into(),
        }
        .into();
    }
    observe_all(&failed, &mut scan, None).unwrap();
    scan.prune_publications(&failed);
    assert_eq!(scan.events.len(), 1);
    assert_eq!(scan.costs.len(), 1);

    let mut repeated = first.clone();
    repeated.hour = 26;
    observe_all(&repeated, &mut scan, None).unwrap();
    scan.prune_publications(&repeated);
    assert_eq!(scan.countries["C1"].mutations.len(), 1);
    assert_eq!(scan.countries["C1"].daily_costs.len(), 1);

    let empty = frame(27, 1);
    observe_all(&empty, &mut scan, None).unwrap();
    scan.prune_publications(&empty);
    assert!(scan.events.is_empty());
    assert!(scan.costs.is_empty());
}

#[test]
fn published_key_continuation_does_not_retain_completed_payments() {
    let mut scan = Scanned::default();
    for hour in 24..=24 * 400 {
        let mut current = frame(hour, 1);
        let mut payment = event(
            hour as u64,
            PowerCause::Decision,
            PowerOperation::Subtract,
            Some(-17),
        );
        payment.hour = hour;
        put(&mut current, TableRows::CountryPowerChanges(vec![payment]));
        observe_all(&current, &mut scan, None).unwrap();
        scan.prune_publications(&current);
        assert_eq!(scan.events.len(), 1);
        assert_eq!(scan.published_events[0].len(), 1);
    }
}

#[test]
fn unsupported_country_tags_are_named_counted_and_only_refuse_their_country() {
    let countries = |tags: Vec<Vec<u8>>| {
        TableRows::Countries(
            tags.into_iter()
                .enumerate()
                .map(|(index, tag)| CountryRow {
                    id: index as u32 + 1,
                    canonical_slot: index as u32 + 1,
                    tag,
                    ..Default::default()
                })
                .collect(),
        )
    };
    let mut valid = frame(24, 2);
    put(
        &mut valid,
        countries(vec![b"C1".to_vec(), "éA".as_bytes().to_vec()]),
    );
    let mut payment = event(
        1,
        PowerCause::NotIdentified,
        PowerOperation::Subtract,
        Some(-1),
    );
    payment.country = 2;
    put(
        &mut valid,
        TableRows::CountryPowerChanges(vec![payment.clone()]),
    );
    let mut scan = Scanned::default();
    observe_all(&valid, &mut scan, None).unwrap();
    assert_eq!(scan.name_at(2, 24).unwrap().tag, "éA");
    assert_eq!(
        present(&scan, 1, "éA", None, &AtomicBool::new(false))
            .unwrap()
            .spending
            .len(),
        1
    );

    let mut invalid = frame(24, 7);
    put(
        &mut invalid,
        countries(vec![
            b"C1".to_vec(),
            "éA".as_bytes().to_vec(),
            vec![b'G', 0xff, b'r'],
            Vec::new(),
            b"A\0B".to_vec(),
            vec![b'x'; 129],
            vec![b'y'; 1024 * 1024],
        ]),
    );
    put(&mut invalid, TableRows::CountryPowerChanges(vec![payment]));
    let mut scan = Scanned::default();
    observe_all(&invalid, &mut scan, None).unwrap();
    assert_eq!(scan.refused.len(), 3);
    let message = scan
        .refused
        .values()
        .map(|(reason, count)| format!("{reason} (count: {count})"))
        .collect::<Vec<_>>()
        .join("; ");
    for (id, bytes, reason) in [
        (3, "b\"G\\xffr\" (3 bytes)", "not UTF-8"),
        (4, "b\"\" (0 bytes)", "empty"),
        (5, "b\"A\\x00B\" (3 bytes)", "contains NUL"),
    ] {
        assert!(message.contains(&format!("country {id} canonical slot {id}: recorded tag")));
        assert!(message.contains(bytes));
        assert!(message.contains(reason));
    }
    assert!(message.len() < 2048);
    assert_eq!(scan.countries.len(), 4);
    scan.name_frame(&invalid, None, &BTreeSet::from([6, 7]), true);
    assert_eq!(scan.name_at(6, 24).unwrap().tag, "x".repeat(129));
    assert_eq!(scan.name_at(7, 24).unwrap().tag, "y".repeat(1024 * 1024));
    assert_eq!(
        present(&scan, 1, "éA", None, &AtomicBool::new(false))
            .unwrap()
            .spending
            .len(),
        1
    );
    assert!(
        present(&scan, 1, "C1", None, &AtomicBool::new(false))
            .unwrap()
            .warnings
            .is_empty()
    );
}

#[test]
fn every_captured_cause_has_an_explicit_category_and_operations_do_not_guess_from_balance() {
    let causes = [
        PowerCause::NotIdentified,
        PowerCause::DailyGain,
        PowerCause::Idea,
        PowerCause::Advisor,
        PowerCause::AdvisorRemoval,
        PowerCause::Decision,
        PowerCause::TargetedDecision,
        PowerCause::DecisionTimeout,
        PowerCause::Diplomacy,
        PowerCause::WarJustification,
        PowerCause::JoinWar,
        PowerCause::Focus,
        PowerCause::FocusRefund,
        PowerCause::ScriptEffect,
        PowerCause::ScaledScriptEffect,
        PowerCause::Autonomy,
        PowerCause::SpyMaster,
        PowerCause::ScientistRecruitment,
        PowerCause::ScientistFaction,
        PowerCause::ScorchedEarth,
        PowerCause::PrideOfFleet,
        PowerCause::Blockade,
        PowerCause::MioPolicy,
        PowerCause::Medal,
        PowerCause::Console,
        PowerCause::ResetSetup,
        PowerCause::AbilityActivation,
        PowerCause::AbilityUpkeep,
        PowerCause::Allocation,
        PowerCause::AllocationRefund,
        PowerCause::LeaderTrait,
        PowerCause::LeaderPromotion,
        PowerCause::LeaderAssignment,
        PowerCause::LeaderRecruitment,
        PowerCause::Raid,
        PowerCause::AirAction,
        PowerCause::PreferredTactic,
        PowerCause::OfficerUpgrade,
        PowerCause::CaptainUpgrade,
        PowerCause::TimedActivity,
        PowerCause::CapCorrection,
    ];
    let mut frame = frame(24, 1);
    put(
        &mut frame,
        TableRows::CountryPowerChanges(
            causes
                .iter()
                .enumerate()
                .map(|(index, cause)| {
                    event(index as u64, *cause, PowerOperation::Subtract, Some(-100))
                })
                .collect(),
        ),
    );
    let mut scan = Scanned::default();
    observe_all(&frame, &mut scan, None).unwrap();
    let reply = present_fixture(&scan);
    assert_eq!(reply.spending.len(), causes.len() - 1); // PP DailyGain is the net daily application.
    assert!(
        reply
            .spend_categories
            .iter()
            .any(|item| item.name == "Not identified")
    );
    let expected = [
        "notIdentified",
        "dailyDeficit",
        "ideas",
        "advisors",
        "advisors",
        "decisions",
        "decisions",
        "decisions",
        "diplomacy",
        "warJustification",
        "diplomacy",
        "focus",
        "focus",
        "scriptEffects",
        "scriptEffects",
        "autonomy",
        "spyMaster",
        "scientists",
        "scientists",
        "scorchedEarth",
        "prideOfFleet",
        "blockade",
        "mioPolicy",
        "medals",
        "console",
        "setup",
        "abilities",
        "upkeep",
        "allocation",
        "allocation",
        "traits",
        "promotions",
        "assignments",
        "recruitment",
        "raids",
        "airActions",
        "tactics",
        "officerUpgrades",
        "captainUpgrades",
        "timedActivities",
        "capCorrection",
    ];
    assert_eq!(causes.len(), expected.len());
    for (cause, expected) in causes.into_iter().zip(expected) {
        let (id, name) = category(cause);
        assert_eq!(id, expected, "{cause:?}");
        assert!(!name.is_empty());
    }
    assert!(
        reply
            .spend_categories
            .iter()
            .all(|category| category.identified == (category.id != "notIdentified"))
    );
    let operations = [
        PowerOperation::Add,
        PowerOperation::Subtract,
        PowerOperation::Set,
        PowerOperation::Reset,
        PowerOperation::AllocationClamp,
        PowerOperation::Clamp,
        PowerOperation::Grant,
        PowerOperation::Refund,
    ];
    let mut frame = frame.clone();
    put(
        &mut frame,
        TableRows::CountryPowerChanges(
            operations
                .iter()
                .enumerate()
                .map(|(index, operation)| {
                    event(
                        100 + index as u64,
                        PowerCause::Decision,
                        *operation,
                        Some(-500),
                    )
                })
                .chain([
                    event(200, PowerCause::Decision, PowerOperation::Subtract, None),
                    event(201, PowerCause::Decision, PowerOperation::Grant, Some(500)),
                    event(202, PowerCause::Decision, PowerOperation::Subtract, Some(0)),
                ])
                .chain(
                    [
                        (PowerOperation::Add, -700),
                        (PowerOperation::Subtract, -700),
                        (PowerOperation::Add, 700),
                        (PowerOperation::Subtract, 700),
                        (PowerOperation::Grant, -700),
                        (PowerOperation::Clamp, -700),
                        (PowerOperation::Refund, -700),
                    ]
                    .into_iter()
                    .enumerate()
                    .map(|(index, (operation, amount))| {
                        let mut row = event(
                            300 + index as u64,
                            PowerCause::DailyGain,
                            operation,
                            Some(amount),
                        );
                        row.currency = PowerCurrency::Command;
                        if index == 0 {
                            row.balance_before_raw = 0;
                            row.balance_after_raw = 0;
                        }
                        row
                    }),
                )
                .collect(),
        ),
    );
    observe_all(&frame, &mut scan, None).unwrap();
    let reply = present_fixture(&scan);
    let added: Vec<_> = reply
        .spending
        .iter()
        .filter(|row| {
            row.id.ends_with(":100") || row.id.ends_with(":101") || row.id.ends_with(":200")
        })
        .collect();
    assert_eq!(added.len(), 3);
    assert!(added.iter().any(|row| row.charge_raw.is_none()));
    assert!(
        added
            .iter()
            .any(|row| row.charge_raw.as_deref() == Some("500")
                && row.realized_delta_raw.as_deref() == Some("-10"))
    );
    assert!(!reply.spending.iter().any(|row| row.id.ends_with(":105")
        || row.id.ends_with(":201")
        || row.id.ends_with(":202")));
    let daily_command: Vec<_> = reply
        .spending
        .iter()
        .filter(|row| row.currency == "Command")
        .collect();
    assert_eq!(daily_command.len(), 2);
    assert!(
        daily_command
            .iter()
            .all(|row| row.category == "dailyDeficit" && row.charge_raw.as_deref() == Some("700"))
    );
    assert!(daily_command.iter().any(|row| row.sequence == "300"));
    assert!(daily_command.iter().any(|row| row.sequence == "301"));
    let swallowed = daily_command
        .iter()
        .find(|row| row.sequence == "300")
        .unwrap();
    assert_eq!(swallowed.charge_raw.as_deref(), Some("700"));
    assert_eq!(swallowed.realized_delta_raw.as_deref(), Some("0"));
    let item = reply
        .spend_items
        .iter()
        .find(|item| item.id == swallowed.item_id)
        .unwrap();
    assert_eq!(item.action, "Daily deficit");
    assert!(item.unknown.is_none());
}

#[test]
fn upkeep_is_one_daily_row_with_each_paid_read_retained_and_no_synthesized_days() {
    let mut first = frame(24, 1);
    let mut paid = event(
        1,
        PowerCause::AbilityUpkeep,
        PowerOperation::Subtract,
        Some(-300),
    );
    paid.currency = PowerCurrency::Command;
    put(
        &mut first,
        TableRows::CountryPowerChanges(vec![paid.clone()]),
    );
    let mut scan = Scanned::default();
    observe_all(&first, &mut scan, None).unwrap();
    let mut later = frame(25, 1);
    paid.hour = 25;
    paid.sequence = 2;
    put(
        &mut later,
        TableRows::CountryPowerChanges(vec![paid.clone()]),
    );
    put(
        &mut later,
        TableRows::PoliticsCaptureCoverage(vec![PoliticsCaptureCoverageRow {
            country: 1,
            instance: 1,
            epoch: 1,
            since_hour: 25,
            through_hour: 25,
            streams: PoliticsCaptureCoverageRow::PP,
        }]),
    );
    observe_all(&later, &mut scan, None).unwrap();
    let reply = present_fixture(&scan);
    assert_eq!(reply.spending.len(), 1);
    let spend = &reply.spending[0];
    assert_eq!(spend.charge_raw.as_deref(), Some("600"));
    assert_eq!(spend.hour, 24);
    assert_eq!(spend.read_hour, 24);
    assert_eq!(spend.changes.len(), 2);
    assert!(spend.changes[0].covered);
    assert!(!spend.changes[1].covered);
    assert_eq!(spend.changes[0].charge_raw.as_deref(), Some("300"));
    assert_eq!(spend.changes[1].read_hour, 25);
    let mut next = frame(72, 1);
    paid.hour = 72;
    paid.sequence = 3;
    put(&mut next, TableRows::CountryPowerChanges(vec![paid]));
    observe_all(&next, &mut scan, None).unwrap();
    let reply = present_fixture(&scan);
    assert_eq!(reply.spending.len(), 2);
    assert!(!reply.spending.iter().any(|row| row.hour == 48));
}

#[test]
fn component_spends_keep_their_read_and_name_count_negative_credits_without_daily_net() {
    let mut first = frame(24, 1);
    put(
        &mut first,
        TableRows::CountryPowerChanges(vec![
            event(1, PowerCause::DailyGain, PowerOperation::Add, Some(-20)),
            event(
                2,
                PowerCause::TimedActivity,
                PowerOperation::Subtract,
                Some(-3),
            ),
        ]),
    );
    put(
        &mut first,
        TableRows::CountryPowerDailyCosts(vec![
            CountryPowerDailyCostRow {
                country: 1,
                instance: 1,
                epoch: 1,
                day_hour: 24,
                pp_sequence: 1,
                cost_raw: 23,
                ..Default::default()
            },
            CountryPowerDailyCostRow {
                country: 1,
                instance: 1,
                epoch: 1,
                day_hour: 24,
                pp_sequence: 1,
                ordinal: 1,
                cost_raw: -4,
                ..Default::default()
            },
        ]),
    );
    put(
        &mut first,
        TableRows::CountryPowerDaily(vec![CountryPowerDailyRow {
            country: 1,
            instance: 1,
            epoch: 1,
            day_hour: 24,
            pp_sequence: Some(1),
            pp_gross_gain_raw: Some(3),
            pp_cost_raw: Some(19),
            pp_net_gain_raw: Some(-16),
            ..Default::default()
        }]),
    );
    let mut scan = Scanned::default();
    observe_all(&first, &mut scan, None).unwrap();
    let mut next = first.clone();
    next.hour = 25;
    put(
        &mut next,
        TableRows::CountryPowerDaily(vec![CountryPowerDailyRow {
            country: 1,
            instance: 1,
            epoch: 1,
            day_hour: 24,
            pp_sequence: Some(1),
            pp_gross_gain_raw: Some(3),
            pp_cost_raw: Some(19),
            pp_net_gain_raw: Some(-16),
            cp_gain_raw: Some(1),
            cp_realized_delta_raw: Some(1),
            cp_sequence: Some(3),
            ..Default::default()
        }]),
    );
    observe_all(&next, &mut scan, None).unwrap();
    let reply = present_fixture(&scan);
    assert_eq!(reply.spending.len(), 2);
    let daily = reply
        .spending
        .iter()
        .find(|row| row.id.starts_with("daily:"))
        .unwrap();
    assert_eq!(daily.charge_raw.as_deref(), Some("23"));
    assert_eq!(daily.read_hour, 24);
    assert!(daily.realized_delta_raw.is_none());
    assert_eq!(reply.daily[0].read_hour, 25);
    assert_eq!(reply.daily[0].pp_read_hour, Some(24));
    assert_eq!(reply.daily[0].cp_read_hour, Some(25));
    assert!(reply.warnings.iter().any(|reason| {
        reason.contains("Negative daily political power cost component")
            && reason.contains("count: 1")
    }));
    assert!(
        !reply
            .spending_coverage
            .iter()
            .find(|cover| cover.since_hour == Some(24))
            .unwrap()
            .political
    );
}

#[test]
fn missing_frames_and_tables_remain_explicit_gaps_and_late_event_names_are_not_backfilled() {
    let mut scan = Scanned::default();
    observe_all(&frame(24, 1), &mut scan, None).unwrap();
    let mut lost = frame(25, 1);
    lost.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "lost".into(),
    }
    .into();
    observe_all(&lost, &mut scan, None).unwrap();
    let mut resumed = frame(27, 1);
    let mut late = event(1, PowerCause::Decision, PowerOperation::Subtract, Some(-1));
    late.hour = 25;
    late.identity.target_country = Some(1);
    put(&mut resumed, TableRows::CountryPowerChanges(vec![late]));
    observe_all(&resumed, &mut scan, None).unwrap();
    let reply = present_fixture(&scan);
    assert!(
        reply
            .spending_coverage
            .iter()
            .any(|span| span.since_hour == Some(25)
                && span.through_hour == Some(26)
                && !span.political)
    );
    assert!(!reply.spending[0].covered);
    assert!(reply.spend_items[0].target_tag.is_none());
    assert!(reply.spending[0].country_name.is_none());
    assert_eq!(reply.spending[0].read_hour, 27);
}

#[test]
fn five_thousand_countries_share_indexed_observation_and_stable_histories_compress() {
    let mut frame = frame(24, 5000);
    put(
        &mut frame,
        TableRows::CountryPowerChanges(
            (1..=5000)
                .map(|id| CountryPowerChangeRow {
                    country: id,
                    instance: 1,
                    epoch: 1,
                    hour: 24,
                    sequence: u64::from(id),
                    cause: PowerCause::NotIdentified,
                    operation: PowerOperation::Subtract,
                    requested_delta_raw: Some(-1),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    let mut scan = Scanned::default();
    observe_all(&frame, &mut scan, None).unwrap();
    assert_eq!(scan.countries.len(), 5000);
    frame.hour = 25;
    put(
        &mut frame,
        TableRows::PoliticsCaptureCoverage(
            (1..=5000)
                .map(|id| PoliticsCaptureCoverageRow {
                    country: id,
                    instance: 1,
                    epoch: 1,
                    since_hour: 25,
                    through_hour: 25,
                    streams: PoliticsCaptureCoverageRow::ALL_STREAMS,
                })
                .collect(),
        ),
    );
    observe_all(&frame, &mut scan, None).unwrap();
    assert_eq!(scan.naming.len(), 5000);
    assert!(
        scan.countries
            .values()
            .all(|reply| reply.coverage.len() == 1)
    );
    let reply = present(&scan, 1, "C5000", None, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.spending.len(), 1);
    assert_eq!(reply.spending[0].country_tag, "C5000");
    assert!(json_len(&reply, MAX_REPLY_BYTES).is_some());
    assert_eq!(reply.spending_coverage[0].read_hour, 24);
    assert_eq!(reply.spending_coverage[0].through_hour, Some(25));
}

#[test]
fn response_budget_refuses_before_large_history_or_art_is_cloned() {
    let mut budget = ReplyBudget::new();
    let icon = "x".repeat(MAX_REPLY_BYTES);
    assert!(
        budget
            .add(&icon)
            .unwrap_err()
            .to_string()
            .contains("complete-response budget")
    );
    let mut scan = Scanned::default();
    scan.country("C1")
        .spending_coverage
        .push(LedgerSpendCoverage {
            read_hour: 0,
            since_hour: None,
            through_hour: None,
            political: false,
            command: false,
            reason: Some(icon),
        });
    assert!(
        present(&scan, 1, "C1", None, &AtomicBool::new(false))
            .unwrap_err()
            .to_string()
            .contains("complete-response budget")
    );
    let mut scan = Scanned::default();
    scan.warnings.insert(
        Some("C1".into()),
        BTreeMap::from([("x".repeat(MAX_REPLY_BYTES), 1)]),
    );
    assert!(
        present(&scan, 1, "C1", None, &AtomicBool::new(false))
            .unwrap_err()
            .to_string()
            .contains("complete-response budget")
    );
}

#[test]
fn absent_spend_tables_and_unmatched_daily_application_never_prove_zero() {
    for missing in [
        TableId::CountryPowerChanges,
        TableId::CountryPowerDaily,
        TableId::CountryPowerDailyCosts,
    ] {
        let mut frame = frame(24, 1);
        frame.tables[missing.index()] = TableView::Unavailable {
            reason: "lost".into(),
        }
        .into();
        let mut scan = Scanned::default();
        observe_all(&frame, &mut scan, None).unwrap();
        let reply = present_fixture(&scan);
        assert!(reply.spending.is_empty());
        assert!(!reply.spending_coverage[0].political);
        assert_eq!(
            reply.spending_coverage[0].command,
            missing != TableId::CountryPowerChanges
        );
    }
    let mut frame = frame(24, 1);
    put(
        &mut frame,
        TableRows::CountryPowerDailyCosts(vec![CountryPowerDailyCostRow {
            country: 1,
            instance: 1,
            epoch: 1,
            day_hour: 24,
            pp_sequence: 77,
            cost_raw: 5,
            ..Default::default()
        }]),
    );
    let mut scan = Scanned::default();
    observe_all(&frame, &mut scan, None).unwrap();
    let reply = present_fixture(&scan);
    assert!(reply.spending.is_empty());
    assert!(!reply.spending_coverage[0].political);
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.contains("matched PP application is unavailable"))
    );
}

#[test]
fn unpriced_adds_leave_currency_gaps_while_explicit_grants_refunds_and_sets_stay_excluded() {
    for currency in [PowerCurrency::Political, PowerCurrency::Command] {
        for operation in [
            PowerOperation::Add,
            PowerOperation::Grant,
            PowerOperation::Refund,
            PowerOperation::Set,
        ] {
            let mut frame = frame(24, 1);
            let mut row = event(1, PowerCause::Decision, operation, None);
            row.currency = currency;
            put(&mut frame, TableRows::CountryPowerChanges(vec![row]));
            let mut scan = Scanned::default();
            observe_all(&frame, &mut scan, None).unwrap();
            let reply = present_fixture(&scan);
            assert_eq!(reply.mutations.len(), 1);
            assert!(reply.spending.is_empty());
            let covered = &reply.spending_coverage[0];
            assert_eq!(
                covered.political,
                !(operation == PowerOperation::Add && currency == PowerCurrency::Political)
            );
            assert_eq!(
                covered.command,
                !(operation == PowerOperation::Add && currency == PowerCurrency::Command)
            );
            if operation == PowerOperation::Add {
                assert!(
                    covered
                        .reason
                        .as_deref()
                        .unwrap()
                        .contains("no recorded evaluated operand")
                );
            }
        }
    }
}

#[test]
fn unknown_targets_and_script_origins_keep_distinct_item_identities() {
    let mut frame = frame(24, 1);
    let mut first = event(1, PowerCause::Diplomacy, PowerOperation::Subtract, Some(-1));
    first.identity.target_country = Some(77);
    let mut second = first.clone();
    second.sequence = 2;
    second.identity.target_country = Some(78);
    let mut third = event(3, PowerCause::ScriptEffect, PowerOperation::Add, Some(-1));
    third.identity.origin_key = b"first_origin".to_vec();
    let mut fourth = third.clone();
    fourth.sequence = 4;
    fourth.identity.origin_key = b"second_origin".to_vec();
    put(
        &mut frame,
        TableRows::CountryPowerChanges(vec![first, second, third, fourth]),
    );
    let mut scan = Scanned::default();
    observe_all(&frame, &mut scan, None).unwrap();
    let reply = present_fixture(&scan);
    assert_eq!(reply.spend_items.len(), 4);
    assert_eq!(
        reply
            .spend_items
            .iter()
            .map(|item| &item.group_id)
            .collect::<BTreeSet<_>>()
            .len(),
        4
    );
    assert!(
        reply
            .spend_items
            .iter()
            .all(|item| item.target_name.is_none())
    );
    assert!(
        reply
            .spend_items
            .iter()
            .all(|item| item.action.contains("Not identified"))
    );
}

#[test]
fn daily_native_cost_requires_matching_decomposition_and_zero_is_proven() {
    for (cost, components, expected) in [
        (5, vec![], false),
        (5, vec![(1, 5)], true),
        (5, vec![(2, 5)], false),
        (0, vec![], true),
        (5, vec![(1, 4)], false),
    ] {
        let mut frame = frame(24, 1);
        put(
            &mut frame,
            TableRows::CountryPowerChanges(vec![event(
                1,
                PowerCause::DailyGain,
                PowerOperation::Add,
                Some(-cost),
            )]),
        );
        put(
            &mut frame,
            TableRows::CountryPowerDaily(vec![CountryPowerDailyRow {
                country: 1,
                instance: 1,
                epoch: 1,
                day_hour: 24,
                pp_sequence: Some(1),
                pp_gross_gain_raw: Some(0),
                pp_cost_raw: Some(cost),
                pp_net_gain_raw: Some(-cost),
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::CountryPowerDailyCosts(
                components
                    .into_iter()
                    .map(|(sequence, amount)| CountryPowerDailyCostRow {
                        country: 1,
                        instance: 1,
                        epoch: 1,
                        day_hour: 24,
                        pp_sequence: sequence,
                        cost_raw: amount,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        let mut scan = Scanned::default();
        observe_all(&frame, &mut scan, None).unwrap();
        let reply = present_fixture(&scan);
        assert_eq!(
            reply.spending_coverage[0].political, expected,
            "cost {cost}"
        );
        if cost == 5
            && reply
                .spending
                .first()
                .is_some_and(|spend| spend.charge_raw.as_deref() == Some("4"))
        {
            assert!(!reply.spending[0].covered);
        }
        if cost == 0 {
            assert!(reply.spending.is_empty());
        }
    }
}

#[test]
fn country_names_join_raw_alias_links_to_the_recorded_slot() {
    let mut frame = frame(24, 1);
    put(
        &mut frame,
        TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 1,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 77,
                canonical_slot: 1,
            },
        ]),
    );
    let mut scan = Scanned::default();
    scan.observe(&frame, None).unwrap();
    scan.name_frame(&frame, None, &BTreeSet::from([77]), true);
    assert_eq!(scan.name_at(77, 24).unwrap().tag, "C1");
    assert!(scan.name_at(77, 25).is_none());
}

#[test]
fn same_specific_item_groups_action_variants_without_losing_event_sentences() {
    let mut frame = frame(24, 1);
    let mut first = event(1, PowerCause::Decision, PowerOperation::Subtract, Some(-1));
    first.identity.item_key = b"same_decision".to_vec();
    let mut second = first.clone();
    second.sequence = 2;
    second.cause = PowerCause::TargetedDecision;
    let mut third = first.clone();
    third.sequence = 3;
    third.cause = PowerCause::DecisionTimeout;
    let mut fourth = first.clone();
    fourth.sequence = 4;
    fourth.identity.action = PowerAction::Remove;
    put(
        &mut frame,
        TableRows::CountryPowerChanges(vec![first, second, third, fourth]),
    );
    let mut scan = Scanned::default();
    observe_all(&frame, &mut scan, None).unwrap();
    let reply = present_fixture(&scan);
    assert_eq!(reply.spending.len(), 4);
    assert_eq!(
        reply
            .spend_items
            .iter()
            .map(|item| &item.group_id)
            .collect::<BTreeSet<_>>()
            .len(),
        1
    );
}

#[test]
fn bounded_observation_over_three_months_with_hourly_leader_changes() {
    let mut scan = Scanned::default();
    let mut current = frame(0, 1);
    let mut leaders: Vec<_> = (0..3000)
        .map(|id| UnitLeaderRow {
            id,
            reference_type: 1,
            ..Default::default()
        })
        .collect();
    for hour in 0..90 * 24 {
        current.hour = hour;
        for leader in &mut leaders {
            leader.skill_required_experience = hour;
        }
        put(&mut current, TableRows::UnitLeaders(leaders.clone()));
        put(
            &mut current,
            TableRows::PoliticsCaptureCoverage(vec![PoliticsCaptureCoverageRow {
                country: 1,
                instance: 1,
                epoch: 1,
                since_hour: hour,
                through_hour: hour,
                streams: PoliticsCaptureCoverageRow::ALL_STREAMS,
            }]),
        );
        let rows = if hour % 24 == 0 {
            let mut row = event(
                hour as u64,
                PowerCause::LeaderPromotion,
                PowerOperation::Subtract,
                Some(-1),
            );
            row.hour = hour;
            vec![row]
        } else {
            Vec::new()
        };
        put(&mut current, TableRows::CountryPowerChanges(rows));
        observe_all(&current, &mut scan, None).unwrap();
    }
    assert_eq!(scan.countries["C1"].mutations.len(), 90);
    assert!(scan.naming.len() <= 180);
    let first = scan.naming.values().next().unwrap();
    assert!(scan.naming.values().all(|name| Arc::ptr_eq(name, first)));
    assert_eq!(scan.countries["C1"].spending_coverage.len(), 1);
}

#[test]
fn reasons_are_counted_by_country_and_daily_item_identity_is_stable() {
    let mut scan = Scanned::default();
    for hour in [24, 48, 72] {
        let mut current = frame(hour, 2);
        put(
            &mut current,
            TableRows::CountryPowerDaily(vec![CountryPowerDailyRow {
                country: 1,
                instance: 1,
                epoch: 1,
                day_hour: hour,
                pp_sequence: Some(hour as u64),
                pp_cost_raw: Some(1),
                ..Default::default()
            }]),
        );
        put(
            &mut current,
            TableRows::CountryPowerDailyCosts(vec![CountryPowerDailyCostRow {
                country: 1,
                instance: 1,
                epoch: 1,
                day_hour: hour,
                pp_sequence: hour as u64,
                cost_raw: 1,
                cause: PowerCause::DailyGain,
                ..Default::default()
            }]),
        );
        observe_all(&current, &mut scan, None).unwrap();
    }
    let one = present_fixture(&scan);
    assert_eq!(one.spending.len(), 3);
    assert_eq!(one.spend_items.len(), 1);
    assert!(
        one.warnings
            .iter()
            .any(|reason| reason == "recorded content has not been resolved (count: 3)")
    );
    assert!(
        present(&scan, 1, "C2", None, &AtomicBool::new(false))
            .unwrap()
            .warnings
            .is_empty()
    );
    scan.note(
        Some("C1"),
        "Only this country lacks a native application".into(),
        90,
    )
    .unwrap();
    assert!(
        present_fixture(&scan)
            .warnings
            .iter()
            .any(|reason| reason.ends_with("(count: 90)"))
    );
    assert!(
        present(&scan, 1, "C2", None, &AtomicBool::new(false))
            .unwrap()
            .warnings
            .is_empty()
    );
}

#[test]
fn unmatched_daily_cost_invalidates_only_its_read_in_a_merged_interval() {
    let mut scan = Scanned::default();
    for hour in 24..=26 {
        observe_all(&frame(hour, 1), &mut scan, None).unwrap();
    }
    scan.countries
        .get_mut("C1")
        .unwrap()
        .daily_costs
        .push(DailyCost {
            row: CountryPowerDailyCostRow {
                country: 1,
                instance: 1,
                epoch: 1,
                day_hour: 25,
                pp_sequence: 100,
                cost_raw: 1,
                cause: PowerCause::DailyGain,
                ..Default::default()
            },
            read_hour: 25,
            covered: true,
            facts: EventFacts::default(),
        });
    let reply = present_fixture(&scan);
    assert_eq!(
        reply
            .spending_coverage
            .iter()
            .map(|row| (
                row.since_hour.unwrap(),
                row.through_hour.unwrap(),
                row.political,
                row.command
            ))
            .collect::<Vec<_>>(),
        vec![
            (24, 24, true, true),
            (25, 25, false, true),
            (26, 26, true, true)
        ]
    );
}

#[test]
fn complete_countries_absence_is_zero_and_missing_countries_remains_a_gap() {
    let mut scan = Scanned::default();
    observe_all(&frame(24, 0), &mut scan, None).unwrap();
    observe_all(&frame(25, 1), &mut scan, None).unwrap();
    let mut missing = frame(26, 0);
    missing.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "missing census".into(),
    }
    .into();
    observe_all(&missing, &mut scan, None).unwrap();
    observe_all(&frame(27, 0), &mut scan, None).unwrap();
    let reply = present_fixture(&scan);
    assert_eq!(
        reply
            .spending_coverage
            .iter()
            .map(|row| (
                row.since_hour.unwrap(),
                row.through_hour.unwrap(),
                row.political,
                row.command
            ))
            .collect::<Vec<_>>(),
        vec![
            (24, 24, true, true),
            (25, 25, true, true),
            (26, 26, false, false),
            (27, 27, true, true)
        ]
    );
}

#[test]
fn previous_read_is_retained_whole_only_at_a_dated_hour() {
    let mut scan = Scanned {
        dated: Some(BTreeSet::from([25])),
        ..Default::default()
    };
    observe_all(&frame(24, 1), &mut scan, None).unwrap();
    let previous = scan.previous.as_ref().unwrap();
    assert!(previous.complete::<tables::Countries>().is_ok());
    assert!(
        previous
            .complete::<tables::PoliticsCaptureCoverage>()
            .is_err()
    );
    observe_all(&frame(25, 1), &mut scan, None).unwrap();
    let previous = scan.previous.as_ref().unwrap();
    assert!(
        previous
            .complete::<tables::PoliticsCaptureCoverage>()
            .is_ok()
    );
}

#[test]
fn spending_view_omits_raw_tables_and_uses_the_shared_response_budget() {
    let mut scan = Scanned::default();
    let mut current = frame(24, 1);
    put(
        &mut current,
        TableRows::CountryPowerChanges(vec![event(
            1,
            PowerCause::Decision,
            PowerOperation::Subtract,
            Some(-1),
        )]),
    );
    observe_all(&current, &mut scan, None).unwrap();
    let reply = present_view(
        &scan,
        1,
        "C1",
        None,
        &AtomicBool::new(false),
        LedgerView::Spending,
    )
    .unwrap();
    let json = serde_json::to_value(&reply).unwrap();
    for field in [
        "mutations",
        "governments",
        "daily",
        "dailyCosts",
        "coverage",
    ] {
        assert!(json.get(field).is_none(), "{field}");
    }
    assert_eq!(json["spending"].as_array().unwrap().len(), 1);
    assert_eq!(
        MAX_REPLY_BYTES,
        dashboard_base::products::MAX_PRODUCT_REPLY_BYTES
    );
    let raw = present_fixture(&scan);
    assert_eq!(raw.mutations.len(), 1);
}

#[test]
fn balance_view_keeps_signed_income_applications_and_payments_without_raw_tables() {
    let mut current = frame(24, 1);
    let mut gain = event(1, PowerCause::DailyGain, PowerOperation::Add, Some(250002));
    gain.balance_after_raw = 250012;
    let mut payment = event(
        2,
        PowerCause::Decision,
        PowerOperation::Subtract,
        Some(-15000001),
    );
    payment.balance_after_raw = -9999990;
    let mut refund = event(
        3,
        PowerCause::FocusRefund,
        PowerOperation::Add,
        Some(700001),
    );
    refund.balance_after_raw = 700011;
    put(
        &mut current,
        TableRows::CountryPowerChanges(vec![gain, payment, refund]),
    );
    put(
        &mut current,
        TableRows::CountryPowerDaily(vec![CountryPowerDailyRow {
            country: 1,
            instance: 1,
            epoch: 1,
            day_hour: 24,
            pp_sequence: Some(1),
            pp_gross_gain_raw: Some(200001),
            pp_cost_raw: Some(-50001),
            pp_net_gain_raw: Some(250002),
            pp_realized_delta_raw: Some(250002),
            ..Default::default()
        }]),
    );
    let mut scan = Scanned::default();
    observe_all(&current, &mut scan, None).unwrap();
    let reply = present_view(
        &scan,
        1,
        "C1",
        None,
        &AtomicBool::new(false),
        LedgerView::Balance,
    )
    .unwrap();
    assert_eq!(reply.daily[0].pp_gross_raw.as_deref(), Some("200001"));
    assert_eq!(reply.daily[0].pp_cost_raw.as_deref(), Some("-50001"));
    assert_eq!(reply.daily[0].read_hour, 24);
    assert_eq!(reply.balance_changes.len(), 3);
    assert_eq!(reply.balance_changes[0].action, "Daily income applied");
    assert_eq!(
        reply.balance_changes[1].amount_raw.as_deref(),
        Some("-15000001")
    );
    assert_eq!(reply.balance_changes[1].realized_delta_raw, "-10000000");
    assert_eq!(reply.balance_changes[2].realized_delta_raw, "700001");
    assert_eq!(reply.spending.len(), 1);
    assert_eq!(reply.spending[0].charge_raw.as_deref(), Some("15000001"));
    assert_eq!(reply.spending[0].changes[0].hour, Some(24));
    let json = serde_json::to_value(&reply).unwrap();
    for field in ["mutations", "governments", "dailyCosts", "coverage"] {
        assert!(json.get(field).is_none(), "{field}");
    }
}

#[test]
fn balance_view_labels_signed_cp_daily_deficit_and_counts_an_item_refusal_once() {
    let mut current = frame(24, 1);
    let mut deficit = event(1, PowerCause::DailyGain, PowerOperation::Add, Some(-120001));
    deficit.currency = PowerCurrency::Command;
    let decision = event(2, PowerCause::Decision, PowerOperation::Subtract, Some(-1));
    put(
        &mut current,
        TableRows::CountryPowerChanges(vec![deficit, decision]),
    );
    let mut scan = Scanned::default();
    observe_all(&current, &mut scan, None).unwrap();
    let reply = present_view(
        &scan,
        1,
        "C1",
        None,
        &AtomicBool::new(false),
        LedgerView::Balance,
    )
    .unwrap();
    assert_eq!(reply.balance_changes[0].action, "Daily deficit");
    assert_eq!(
        reply.balance_changes[0].amount_raw.as_deref(),
        Some("-120001")
    );
    let reason = reply.balance_changes[1].reason.as_ref().unwrap();
    assert!(reply.warnings.contains(&format!("{reason} (count: 1)")));
    assert!(!reply.warnings.contains(&format!("{reason} (count: 2)")));
}

#[test]
fn previously_named_country_with_invalid_tag_has_its_own_refusal_and_unknown_coverage() {
    let mut scan = Scanned::default();
    observe_all(&frame(24, 2), &mut scan, None).unwrap();
    for hour in [25, 26] {
        let mut current = frame(hour, 2);
        put(
            &mut current,
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
        observe_all(&current, &mut scan, None).unwrap();
    }
    let reply = present_fixture(&scan);
    assert!(
        reply
            .warnings
            .iter()
            .any(|reason| reason.contains("not UTF-8") && reason.contains("count: 2"))
    );
    assert!(
        reply
            .spending_coverage
            .iter()
            .any(|row| row.since_hour == Some(25)
                && row.through_hour == Some(26)
                && !row.political
                && !row.command)
    );
    let other = present(&scan, 1, "C2", None, &AtomicBool::new(false)).unwrap();
    assert!(
        !other
            .warnings
            .iter()
            .any(|reason| reason.contains("not UTF-8"))
    );
}

#[test]
fn government_view_keeps_execution_causes_and_event_tokens_without_payment_payloads() {
    let mut current = frame(24, 1);
    put(
        &mut current,
        TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 91,
                literal: b"mod_old".to_vec(),
            },
            LexicalTokenRow {
                token: 92,
                literal: b"mod_new".to_vec(),
            },
        ]),
    );
    put(
        &mut current,
        TableRows::GovernmentChanges(vec![GovernmentChangeRow {
            country: 1,
            instance: 1,
            epoch: 1,
            sequence: 7,
            hour: 24,
            before_party: Some(41),
            after_party: Some(42),
            before_ideology: Some(91),
            after_ideology: Some(92),
            cause: GovernmentCause::CoupCompletion,
            ..Default::default()
        }]),
    );
    put(
        &mut current,
        TableRows::CountryPowerChanges(vec![event(
            1,
            PowerCause::Decision,
            PowerOperation::Subtract,
            Some(-1),
        )]),
    );
    let mut scan = Scanned::default();
    observe_all(&current, &mut scan, None).unwrap();
    let reply = present_view(
        &scan,
        1,
        "C1",
        None,
        &AtomicBool::new(false),
        LedgerView::Governments,
    )
    .unwrap();
    assert_eq!(reply.governments.len(), 1);
    assert_eq!(reply.governments[0].cause, "CoupCompletion");
    assert_eq!(
        reply.governments[0].before_token.as_deref(),
        Some("mod_old")
    );
    assert_eq!(reply.governments[0].after_token.as_deref(), Some("mod_new"));
    assert!(reply.spending.is_empty());
    assert!(reply.command_holds.is_empty());
    let json = serde_json::to_value(&reply).unwrap();
    for field in [
        "mutations",
        "daily",
        "dailyCosts",
        "coverage",
        "balanceChanges",
    ] {
        assert!(json.get(field).is_none(), "{field}");
    }
    let mut later = frame(26, 1);
    put(
        &mut later,
        TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 91,
            literal: b"later_identity".to_vec(),
        }]),
    );
    assert_eq!(
        government_token(&later, Some(&current), 24, Some(91)).as_deref(),
        Some("mod_old")
    );
    assert!(government_token(&later, None, 24, Some(91)).is_none());
}

#[test]
fn daily_currencies_keep_their_independent_supplying_reads_in_either_order() {
    for pp_first in [true, false] {
        let complete = CountryPowerDailyRow {
            country: 1,
            instance: 1,
            epoch: 1,
            day_hour: 24,
            pp_gross_gain_raw: Some(3),
            pp_cost_raw: Some(1),
            pp_net_gain_raw: Some(2),
            pp_realized_delta_raw: Some(2),
            pp_sequence: Some(1),
            cp_gain_raw: Some(5),
            cp_realized_delta_raw: Some(5),
            cp_sequence: Some(2),
        };
        let mut early = complete.clone();
        if pp_first {
            early.cp_gain_raw = None;
            early.cp_realized_delta_raw = None;
            early.cp_sequence = None;
        } else {
            early.pp_gross_gain_raw = None;
            early.pp_cost_raw = None;
            early.pp_net_gain_raw = None;
            early.pp_realized_delta_raw = None;
            early.pp_sequence = None;
        }
        let mut first = frame(24, 1);
        put(&mut first, TableRows::CountryPowerDaily(vec![early]));
        let mut scan = Scanned::default();
        observe_all(&first, &mut scan, None).unwrap();
        let early_reply = present_fixture(&scan);
        assert_eq!(early_reply.daily[0].pp_read_hour, pp_first.then_some(24));
        assert_eq!(early_reply.daily[0].cp_read_hour, (!pp_first).then_some(24));
        let mut next = first.clone();
        next.hour = 25;
        put(&mut next, TableRows::CountryPowerDaily(vec![complete]));
        observe_all(&next, &mut scan, None).unwrap();
        next.hour = 26;
        observe_all(&next, &mut scan, None).unwrap();
        let reply = present_fixture(&scan);
        assert_eq!(reply.daily.len(), 1);
        assert_eq!(reply.daily[0].read_hour, 25);
        assert_eq!(
            reply.daily[0].pp_read_hour,
            Some(if pp_first { 24 } else { 25 })
        );
        assert_eq!(
            reply.daily[0].cp_read_hour,
            Some(if pp_first { 25 } else { 24 })
        );
        assert_eq!(reply.daily[0].pp_gross_raw.as_deref(), Some("3"));
        assert_eq!(reply.daily[0].cp_income_raw.as_deref(), Some("5"));
    }
}

#[test]
fn balance_sentences_resolve_each_spending_item_once_before_upkeep_grouping() {
    let mut input = frame(24, 1);
    let mut upkeep = event(
        2,
        PowerCause::AbilityUpkeep,
        PowerOperation::Subtract,
        Some(-7),
    );
    upkeep.currency = PowerCurrency::Command;
    let mut next = upkeep.clone();
    next.sequence = 3;
    put(
        &mut input,
        TableRows::CountryPowerChanges(vec![
            event(1, PowerCause::Decision, PowerOperation::Subtract, Some(-17)),
            upkeep,
            next,
        ]),
    );
    let mut scan = Scanned::default();
    scan.observe(&input, None).unwrap();
    let before = RESOLVED.with(std::cell::Cell::get);
    let reply = present_view(
        &scan,
        1,
        "C1",
        None,
        &AtomicBool::new(false),
        LedgerView::Balance,
    )
    .unwrap();
    assert_eq!(RESOLVED.with(std::cell::Cell::get) - before, 3);
    assert_eq!(reply.balance_changes.len(), 3);
    for (stored, balance) in scan.countries["C1"]
        .mutations
        .iter()
        .zip(&reply.balance_changes)
    {
        let (item, _, _) = resolve_item(
            &scan,
            &stored.ledger(),
            &stored.facts,
            None,
            &mut BTreeMap::new(),
            mutation_id(&stored.ledger()),
            None,
        );
        assert_eq!(
            (&balance.action, &balance.reason),
            (&item.action, &item.unknown)
        );
    }
}
