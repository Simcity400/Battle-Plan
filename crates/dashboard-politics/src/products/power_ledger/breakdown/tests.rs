use super::super::{PowerSpendingReply, SpendCategory, SpendCoverage, SpendCurrency, SpendDay, SpendGain, SpendItem, SpendPayment, SpendReconciliation};
use super::at_hour;
use dashboard_base::products::breakdown::BreakdownState;

fn item(group: &str, category: &str, name: Option<&str>) -> SpendItem {
    SpendItem {
        id: group.into(),
        group_id: group.into(),
        cause: String::new(),
        category: category.into(),
        recurring: false,
        first_hour: 0,
        first_read: 0,
        name: name.map(Into::into),
        action: String::new(),
        icon: None,
        character: None,
        target_tag: None,
        target_name: None,
        unknown: None,
        tooltip_token: None,
    }
}

fn payment(item: u32, read: i32, known: Option<i64>) -> SpendPayment {
    SpendPayment { item, hour: Some(read), read, sequence: None, ordinal: None, known_raw: known.map(|value| value.to_string()), payments: 1, unknown: 0, uncovered: false }
}

fn reply() -> PowerSpendingReply {
    PowerSpendingReply {
        branch_id: "1".into(),
        tag: "GER".into(),
        currency: SpendCurrency::Political,
        from: 0,
        to: 100,
        decimals: 5,
        items: vec![item("war", "decisions", Some("War measures")), item("daily", "daily", None), item("eco", "decisions", Some("Economy")), item("odd", "notIdentified", None)],
        categories: vec![
            SpendCategory { id: "decisions".into(), name: "Decisions".into(), identified: true },
            SpendCategory { id: "daily".into(), name: "Daily costs".into(), identified: true },
            SpendCategory { id: "notIdentified".into(), name: "Not identified".into(), identified: false },
        ],
        payments: vec![payment(0, 5, Some(64_000_000)), payment(2, 6, Some(35_200_000)), payment(3, 7, Some(6_300_000)), payment(0, 40, Some(1))],
        days: vec![SpendDay { item: 1, day: Some(0), read: 10, days: 1, charge_raw: Some("207100000".into()), payments: 1, unknown: 0, uncovered: 0, realized_raw: None, credits: 0 }],
        gains: vec![
            SpendGain { hour: None, read: 9, gained_raw: Some("530600000".into()), capped_raw: Some("100000".into()), days: 1, unknown_days: 0, unmatched: 0 },
            SpendGain { hour: None, read: 30, gained_raw: Some("1".into()), capped_raw: Some("0".into()), days: 1, unknown_days: 0, unmatched: 0 },
        ],
        coverage: vec![
            SpendCoverage { first_hour: 0, last_hour: 1, covered: false, reason: Some("Spending was not recorded".into()) },
            SpendCoverage { first_hour: 2, last_hour: 100, covered: true, reason: None },
        ],
        gain_coverage: Vec::new(),
        balances: Vec::new(),
        reconciliation: SpendReconciliation::default(),
        icons: Vec::new(),
        idea_tooltips: None,
        warnings: Vec::new(),
    }
}

#[test]
fn the_breakdown_at_an_hour_counts_only_what_was_read_by_then() {
    let at = at_hour(&reply(), 20);
    let parts = &at.breakdown.parts;
    assert_eq!(parts.iter().map(|part| part.name.as_str()).collect::<Vec<_>>(), ["Daily costs", "Decisions", "Other spending"]);
    assert_eq!(at.breakdown.total.raw.as_deref(), Some("312600000"), "the payment read at hour 40 is not counted at hour 20");
    assert_eq!(parts[1].children.iter().map(|part| part.name.as_str()).collect::<Vec<_>>(), ["War measures", "Economy"]);
    assert_eq!(parts[0].ordinal, 2, "categories keep the order they were first recorded in");
    assert!(parts[2].other);
    assert_eq!(at.breakdown.total.state, BreakdownState::Partial, "two hours were not recorded");
    assert_eq!(at.breakdown.unrecorded_hours, 2);
    assert_eq!(at.through, 29, "the next gain is read at hour 30");
    assert_eq!(at.gained_raw.as_deref(), Some("530600000"));
    assert_eq!(at.lost_at_cap_raw.as_deref(), Some("100000"));
}

#[test]
fn a_payment_whose_cost_was_not_recorded_makes_the_total_a_lower_bound() {
    let mut held = reply();
    held.payments.push(payment(2, 8, None));
    let at = at_hour(&held, 20);
    assert_eq!(at.breakdown.total.state, BreakdownState::AtLeast);
    assert!(at.breakdown.parts.iter().all(|part| part.share.is_none()));
    let decisions = at.breakdown.parts.iter().find(|part| part.key == "decisions").expect("decisions");
    assert_eq!(decisions.state, BreakdownState::AtLeast);
    assert_eq!(decisions.reason.as_deref(), Some("The cost of 1 payment was not recorded."));
}
