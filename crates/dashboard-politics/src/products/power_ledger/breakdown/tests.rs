use super::super::{PowerSpendingReply, SpendCategory, SpendCoverage, SpendCurrency, SpendDay, SpendGain, SpendItem, SpendKind, SpendPayment, SpendReconciliation};
use super::{holes, index, unrecorded};
use dashboard_base::products::breakdown::{BreakdownAmount, BreakdownState};

fn exact(numerator: &str, denominator: &str) -> Option<BreakdownAmount> {
    Some(BreakdownAmount { numerator: numerator.into(), denominator: denominator.into() })
}

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
        kind: None,
        ledger: (0, 0, 0),
    }
}

fn payment(item: u32, read: i32, known: Option<i64>) -> SpendPayment {
    SpendPayment { item, hour: Some(read), read, sequence: None, ordinal: None, known_raw: known.map(|value| value.to_string()), payments: 1, unknown: 0, uncovered: false }
}

fn covered(first_hour: i32, last_hour: i32) -> SpendCoverage {
    SpendCoverage { first_hour, last_hour, covered: true, reason: None }
}

fn reply() -> PowerSpendingReply {
    PowerSpendingReply {
        branch_id: "1".into(),
        tag: "AAA".into(),
        currency: SpendCurrency::Political,
        from: 0,
        to: 100,
        decimals: 5,
        items: vec![item("plan-a", "decisions", Some("Plan A")), item("daily", "daily", None), item("plan-b", "decisions", Some("Plan B")), item("odd", "notIdentified", None)],
        categories: vec![
            SpendCategory { id: "decisions".into(), name: "Decisions".into(), identified: true },
            SpendCategory { id: "daily".into(), name: "Daily costs".into(), identified: true },
            SpendCategory { id: "notIdentified".into(), name: "Not identified".into(), identified: false },
        ],
        payments: vec![payment(0, 5, Some(64_000_000)), payment(2, 6, Some(35_200_000)), payment(3, 7, Some(6_300_000)), payment(0, 40, Some(1))],
        days: vec![SpendDay { item: 1, day: Some(0), read: 10, days: 1, charge_raw: Some("207100000".into()), payments: 1, unknown: 0, uncovered: 0, realized_raw: None, credits: 0, unknown_credits: 0 }],
        gains: vec![
            SpendGain { hour: None, read: 9, gained_raw: Some("530600000".into()), capped_raw: Some("100000".into()), days: 1, unknown_days: 0, unmatched: 0 },
            SpendGain { hour: None, read: 30, gained_raw: Some("1".into()), capped_raw: Some("0".into()), days: 1, unknown_days: 0, unmatched: 0 },
        ],
        coverage: vec![SpendCoverage { first_hour: 0, last_hour: 1, covered: false, reason: Some("Spending was not recorded".into()) }, covered(2, 100)],
        gain_coverage: vec![covered(0, 100)],
        balances: Vec::new(),
        reconciliation: SpendReconciliation::default(),
        icons: Vec::new(),
        idea_tooltips: None,
        warnings: vec!["A warning about the whole branch".into()],
    }
}

fn unread() -> Vec<(i32, i32, String)> {
    vec![(0, 1, "Spending was not recorded".into())]
}

#[test]
fn the_breakdown_at_an_hour_counts_only_what_was_read_by_then() {
    let at = index(&reply(), &unread(), &[]).at(20);
    let parts = &at.breakdown.parts;
    assert_eq!(parts.iter().map(|part| part.name.as_str()).collect::<Vec<_>>(), ["Daily costs", "Decisions", "Other spending"]);
    assert_eq!(at.breakdown.total.amount, exact("3126", "1"), "the payment read at hour 40 is not counted at hour 20");
    assert_eq!(parts[1].children.iter().map(|part| part.name.as_str()).collect::<Vec<_>>(), ["Plan A", "Plan B"]);
    assert_eq!(parts[0].ordinal, 2, "categories keep the order they were first recorded in");
    assert!(parts[2].other);
    assert_eq!(parts[1].art.as_ref().map(|art| (art.key.as_str(), art.text)), Some(("decision_icon_small", Some(true))), "a known category carries its text icon");
    assert_eq!((parts[0].art.as_ref(), parts[2].art.as_ref()), (None, None), "a category the mapping does not know, and other spending, carry none");
    assert!(parts[1].children.iter().all(|part| part.art.is_none()), "items carry no category picture");
    assert_eq!(at.breakdown.total.state, BreakdownState::Partial, "two hours were not recorded");
    assert_eq!(at.breakdown.unrecorded_hours, 2);
    assert_eq!(at.through, 29, "the next gain is read at hour 30");
    assert_eq!(at.gained, exact("5306", "1"));
    assert_eq!(at.lost_at_cap, exact("1", "1"));
    assert!(at.warnings.is_empty(), "a warning about the whole branch is not a fact of this hour");
    let held = index(&reply(), &unread(), &[]);
    for hour in [3, 8, 20, 35, 60] {
        assert_eq!(held.at(hour), index(&reply(), &unread(), &[]).at(hour), "the answer at {hour} is a lookup a fresh build repeats");
    }
}

#[test]
fn a_payment_whose_cost_was_not_recorded_makes_the_total_a_lower_bound() {
    let mut held = reply();
    held.payments.push(payment(2, 8, None));
    let at = index(&held, &unread(), &[]).at(20);
    assert_eq!(at.breakdown.total.state, BreakdownState::AtLeast);
    assert!(at.breakdown.parts.iter().all(|part| part.share.is_none()));
    let decisions = at.breakdown.parts.iter().find(|part| part.key == "decisions").expect("decisions");
    assert_eq!(decisions.state, BreakdownState::AtLeast);
    assert_eq!(decisions.reason.as_deref(), Some("Some payment costs were not recorded."));
}

#[test]
fn hours_between_coverage_runs_are_not_claimed_and_gains_need_their_own_coverage() {
    assert_eq!(holes(&[covered(0, 9), covered(20, 30)], 0, 40), [(10, 19), (31, 40)], "hours in no run are found, between runs and after the last");
    let gap = vec![(10, 19, "The recording has a gap".to_owned())];
    let at = index(&reply(), &gap, &[]).at(25);
    assert_eq!((at.breakdown.total.state, at.breakdown.unrecorded_hours), (BreakdownState::Partial, 10));
    let unread_gains = index(&reply(), &[], &[(0, 100, "Power changes were not read".to_owned())]).at(20);
    assert_eq!(unread_gains.gained, exact("5306", "1"), "what was read is the figure");
    assert!(unread_gains.gained_reason.is_some(), "hours whose gains were not read make it partial, never exact");
}

#[test]
fn hours_when_no_holder_is_known_are_unrecorded_never_a_zero() {
    use dashboard_query::{identity::{Holder, HolderRun, Holders}, timeline::{Join, Stretch, Timeline}};
    let run = |first: i32, last: i32| HolderRun { first, last, holder: Holder::Found { slot: 1, id: 10 }, starts_segment: true };
    let holders = Holders::new([("AAA".to_owned(), vec![run(0, 39), run(60, 100)])].into(), vec![(40, 59, "No holder is known at these hours".to_owned())]);
    let timeline = Timeline { id: 1, first: 0, last: 100, stretches: vec![Stretch { run: 0, first: 0, last: 100, join: Join::Start }], unheld: Vec::new() };
    let gaps = unrecorded(&timeline, &holders, &"AAA".into(), &[covered(0, 39), covered(60, 100)], 0, 100);
    assert_eq!(gaps, [(40, 59, "No holder is known at these hours".to_owned())], "a hole with no run for the tag while no holder is known is not a recorded zero");
    let unlinked = Holders::new(
        [
            ("AAA".to_owned(), vec![run(0, 39), HolderRun { first: 40, last: 59, holder: Holder::Unresolved, starts_segment: false }]),
            ("BBB".to_owned(), vec![run(60, 100)]),
        ]
        .into(),
        Vec::new(),
    );
    let slot_gaps = unrecorded(&timeline, &unlinked, &dashboard_base::products::Country::Slot(1), &[covered(0, 39), covered(60, 100)], 0, 100);
    assert_eq!(slot_gaps, [(40, 59, "The country's holder was not recorded".to_owned())], "a slot query keeps unresolved tag runs as unrecorded holes");
}

#[test]
fn an_unmatched_daily_cost_qualifies_each_measure_and_keeps_what_was_witnessed() {
    let mut unmatched = reply();
    unmatched.gains[0].unmatched = 1;
    unmatched.gains[0].capped_raw = None;
    let index = index(&unmatched, &[], &[]);
    let at = index.at(20);
    assert_eq!(at.gained, exact("5306", "1"), "the witnessed gain stays the figure");
    assert_eq!(at.gained_reason.as_deref(), Some("Some gains were not recorded."));
    assert_eq!((at.lost_at_cap.clone(), at.lost_at_cap_reason.as_deref()), (exact("0", "1"), Some("Some losses at the cap were not recorded.")), "the loss is qualified by its own reason");
    assert_eq!(index.at(35).gained, exact("530600001", "100000"), "a later witnessed gain still adds to the running sum");
}

#[test]
fn no_gain_is_claimed_before_the_first_hour_power_changes_were_read() {
    let mut held = reply();
    held.gains.retain(|gain| gain.read >= 30);
    let index = index(&held, &[], &[(0, 19, "Power changes were not read".to_owned())]);
    let early = index.at(10);
    assert_eq!(early.gained, None, "an entirely unread stretch is not a gain of zero");
    assert!(early.gained_reason.as_deref().is_some_and(|reason| reason.contains("Power changes were not read")));
    assert_eq!(index.at(20).gained, exact("0", "1"), "from the first read hour the witnessed sum is the figure, partial");
    assert!(index.at(20).gained_reason.is_some());
}

#[test]
fn spending_whose_item_was_not_recorded_names_the_reason_from_its_read() {
    let mut held = reply();
    held.items[3].unknown = Some("Power items were not read at this hour".into());
    let index = index(&held, &unread(), &[]);
    assert!(index.at(6).warnings.is_empty(), "the payment is read at 7");
    assert_eq!(index.at(7).warnings, ["The item of some spending was not recorded: Power items were not read at this hour"]);
}

#[test]
fn decisions_group_by_their_category_with_each_decision_beneath() {
    let mut held = reply();
    let kind = |id: Option<&str>, name: Option<&str>| Some(SpendKind { id: id.map(Into::into), name: name.map(Into::into), unknown: id.is_none().then(|| "Decision x is not defined in the installed content".into()) });
    held.items[0].kind = kind(Some("decision:group_a"), Some("Group A"));
    held.items[2].kind = kind(None, None);
    let at = index(&held, &unread(), &[]).at(20);
    let decisions = at.breakdown.parts.iter().find(|part| part.key == "decisions").expect("decisions");
    assert_eq!(decisions.children.iter().map(|part| part.name.as_str()).collect::<Vec<_>>(), ["Group A", "Category not recorded"]);
    assert_eq!(decisions.children[0].children.iter().map(|part| part.name.as_str()).collect::<Vec<_>>(), ["Plan A"], "each decision is the third level");
    assert!(decisions.children[1].other, "an unknown category is the reserved grey, never a guess");
}

#[test]
fn a_credit_is_named_from_the_day_it_was_read() {
    let mut held = reply();
    held.items[1].action = "Daily cost".into();
    held.days.push(SpendDay { item: 1, day: Some(48), read: 50, days: 1, charge_raw: Some("0".into()), payments: 1, unknown: 0, uncovered: 0, realized_raw: None, credits: 1, unknown_credits: 0 });
    held.days.push(SpendDay { item: 1, day: None, read: 60, days: 0, charge_raw: Some("0".into()), payments: 0, unknown: 0, uncovered: 0, realized_raw: None, credits: 0, unknown_credits: 1 });
    let index = index(&held, &unread(), &[]);
    assert!(index.at(49).warnings.is_empty());
    assert_eq!(index.at(49).through, 49);
    assert_eq!(index.at(50).warnings.len(), 1);
    assert_eq!(index.at(59).through, 59, "a credit whose amount was not recorded is a dated change of its own");
    assert_eq!(index.at(60).breakdown, index.at(59).breakdown, "a credit is not spending: it adds no amount and moves no date");
    assert_eq!(
        index.at(60).warnings[1],
        "Negative daily cost component Daily cost: its credit's amount was not recorded and is not counted as spending",
        "a credit whose amount was not recorded is named, never dropped"
    );
}

#[test]
fn a_category_the_localisation_does_not_name_keeps_its_key_with_the_reason() {
    let mut held = reply();
    held.items[0].kind = Some(SpendKind { id: Some("decision:group_a_cat".into()), name: Some("group_a_cat".into()), unknown: Some("The recorded localisation holds no name for group_a_cat".into()) });
    let at = index(&held, &unread(), &[]).at(20);
    let decisions = at.breakdown.parts.iter().find(|part| part.key == "decisions").expect("decisions");
    let group = decisions.children.iter().find(|part| part.key == "decision:group_a_cat").expect("the category");
    assert_eq!(group.name, "group_a_cat", "the key is shown, never a guessed name");
    assert_eq!(group.name_missing.as_deref(), Some("The recorded localisation holds no name for group_a_cat"), "and it is marked as missing with the reason");
}
