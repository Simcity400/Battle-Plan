use super::*;

fn node(key: &str, ordinal: u32) -> Node {
    Node::new(key, key, ordinal)
}

fn leaf(path: &[Node], amount: Amount) -> Leaf {
    Leaf { path: path.to_vec(), amount, count: None, last: None }
}

fn figure(whole: &str) -> Option<BreakdownAmount> {
    Some(BreakdownAmount { numerator: whole.into(), denominator: "1".into() })
}

fn keys(parts: &[BreakdownPart]) -> Vec<&str> {
    parts.iter().map(|part| part.key.as_str()).collect()
}

#[test]
fn the_whole_and_its_order_are_decided_here() {
    let decisions = node("decisions", 0);
    let leaves = [
        leaf(&[decisions.clone(), node("war", 0)], Amount::Known(Exact::whole(640))),
        leaf(&[node("daily", 1)], Amount::Known(Exact::whole(2071))),
        leaf(&[decisions, node("economy", 1)], Amount::Known(Exact::whole(352))),
        leaf(&[node("rest", 3).other()], Amount::Known(Exact::whole(5000))),
        leaf(&[node("laws", 2)], Amount::Known(Exact::whole(410))),
    ];
    let reply = breakdown(&leaves, &Unrecorded::default());
    assert_eq!(reply.total, BreakdownTotal { amount: figure("8473"), state: BreakdownState::Known, reason: None });
    assert_eq!(keys(&reply.parts), ["daily", "decisions", "laws", "rest"], "known parts by size, other after them");
    let decisions = &reply.parts[1];
    assert_eq!(decisions.amount, figure("992"));
    assert_eq!(keys(&decisions.children), ["war", "economy"]);
    assert_eq!(decisions.children[0].share, Some(640.0 / 8473.0), "a sub-part's share is of the whole");
    assert_eq!(decisions.children[0].parent_share, Some(640.0 / 992.0));
    assert_eq!(decisions.ordinal, 0, "colours follow the recording's order, not the drawn one");
}

#[test]
fn a_part_not_recorded_makes_the_whole_a_lower_bound_without_shares() {
    let leaves = [
        leaf(&[node("new ships", 0)], Amount::NotRecorded("New ships were not recorded".into())),
        leaf(&[node("repairs", 2)], Amount::Known(Exact::whole(211))),
        leaf(&[node("refits", 1)], Amount::NotRecorded("Refits were not recorded".into())),
    ];
    let reply = breakdown(&leaves, &Unrecorded::default());
    assert_eq!(reply.total.state, BreakdownState::AtLeast);
    assert_eq!(reply.total.amount, figure("211"));
    assert_eq!(keys(&reply.parts), ["repairs", "new ships", "refits"], "not recorded last, in recorded order");
    assert!(reply.parts.iter().all(|part| part.share.is_none()), "a lower bound has no shares");
    assert_eq!(reply.parts[1].state, BreakdownState::NotRecorded);
    assert_eq!(reply.parts[1].amount, None, "not recorded is never zero");
}

#[test]
fn nothing_recorded_is_not_a_zero() {
    let reply = breakdown(&[leaf(&[node("a", 0)], Amount::NotRecorded("Not read".into()))], &Unrecorded::default());
    assert_eq!(reply.total, BreakdownTotal { amount: None, state: BreakdownState::NotRecorded, reason: Some("Not read".into()) });
}

#[test]
fn nothing_spent_is_a_known_zero_and_unrecorded_hours_keep_it_partial() {
    let reply = breakdown(&[], &Unrecorded::default());
    assert_eq!(reply.total, BreakdownTotal { amount: figure("0"), state: BreakdownState::Known, reason: None });
    let gap = breakdown(&[], &Unrecorded { hours: 3, reason: Some("Spending was not recorded".into()) });
    assert_eq!(gap.total, BreakdownTotal { amount: figure("0"), state: BreakdownState::Partial, reason: Some("Spending was not recorded".into()) });
}

#[test]
fn unrecorded_hours_mark_the_whole_partial_and_keep_its_shares() {
    let leaves = [leaf(&[node("a", 0)], Amount::Known(Exact::whole(30))), leaf(&[node("b", 1)], Amount::Partial(Exact::whole(10), "Some costs were not recorded".into()))];
    let reply = breakdown(&leaves, &Unrecorded { hours: 2, reason: Some("Spending was not recorded for 2 hours".into()) });
    assert_eq!(reply.total.state, BreakdownState::Partial);
    assert_eq!(reply.total.reason.as_deref(), Some("Spending was not recorded for 2 hours; Some costs were not recorded"));
    assert_eq!(reply.unrecorded_hours, 2);
    assert_eq!(reply.parts[0].share, Some(0.75));
    assert_eq!(reply.parts[1].state, BreakdownState::Partial);
}

#[test]
fn corrections_below_zero_follow_the_positive_parts_without_a_share() {
    let leaves = [
        leaf(&[node("returned", 0)], Amount::Known(Exact::whole(-13))),
        leaf(&[node("lost", 1)], Amount::Known(Exact::whole(100))),
        leaf(&[node("level", 2)], Amount::Known(Exact::whole(0))),
    ];
    let reply = breakdown(&leaves, &Unrecorded::default());
    assert_eq!(keys(&reply.parts), ["lost", "level", "returned"]);
    assert_eq!(reply.total.amount, figure("87"));
    assert_eq!(reply.parts[2].share, None);
}

#[test]
fn counts_add_up_the_levels() {
    let a = node("a", 0);
    let leaves = [
        Leaf { path: vec![a.clone(), node("x", 0)], amount: Amount::Known(Exact::whole(1)), count: Some(2), last: None },
        Leaf { path: vec![a, node("y", 1)], amount: Amount::Known(Exact::whole(1)), count: Some(3), last: None },
    ];
    let reply = breakdown(&leaves, &Unrecorded::default());
    assert_eq!(reply.parts[0].count, Some(5));
}

#[test]
fn known_children_add_up_to_their_part_and_carry_its_share() {
    let alpha = node("alpha", 0);
    let leaves = [
        leaf(&[alpha.clone(), node("one", 0)], Amount::Known(Exact::whole(640))),
        leaf(&[alpha.clone(), node("two", 1)], Amount::Known(Exact::whole(352))),
        leaf(&[alpha], Amount::Known(Exact::whole(423))),
        leaf(&[node("beta", 1)], Amount::Known(Exact::whole(1415))),
    ];
    let reply = breakdown(&leaves, &Unrecorded::default());
    let alpha = reply.parts.iter().find(|part| part.key == "alpha").expect("alpha");
    assert_eq!(keys(&alpha.children), ["one", "two", REST_KEY], "an amount given to the part itself is its last known child");
    assert!(alpha.children[2].other);
    assert_eq!(alpha.children[2].amount, figure("423"));
    assert_eq!(alpha.children[0].parent_share, Some(640.0 / 1415.0), "a sub-part's share of its part uses the part as denominator");
    assert_eq!(alpha.children[0].share, Some(640.0 / 2830.0));
    assert_eq!(alpha.parent_share, alpha.share, "a first-level part's parent is the whole");
}

fn history() -> History {
    let mut built = HistoryBuilder::new(0, 100);
    let a = built.leaf(vec![node("a", 0)]);
    let b = built.leaf(vec![node("b", 1)]);
    built.add(10, a, Amount::Known(Exact::whole(5)), Some(1));
    built.add(20, b, Amount::Known(Exact::whole(7)), Some(1));
    built.add(30, a, Amount::Partial(Exact::whole(1), "inexact".into()), Some(1));
    built.add(40, b, Amount::NotRecorded("lost".into()), None);
    built.add(45, b, Amount::Known(Exact::whole(3)), None);
    built.unrecorded(50, 54, "gap");
    built.qualify(60, "unattributed");
    built.change(70);
    built.build()
}

#[test]
fn a_history_answers_each_hour_by_lookup_with_the_hour_of_the_next_change() {
    let held = history();
    let (before, through) = held.at(5);
    assert!(before.parts.is_empty(), "nothing read by hour 5");
    assert_eq!((before.total.amount.clone(), before.total.state, through), (figure("0"), BreakdownState::Known, 9));
    let (at15, through) = held.at(15);
    assert_eq!((keys(&at15.parts), at15.total.amount.clone(), through), (vec!["a"], figure("5"), 19), "b appears at 20, so 15 stands through 19");
    let (at35, _) = held.at(35);
    assert_eq!(at35.parts.iter().map(|part| (part.key.as_str(), part.state)).collect::<Vec<_>>(), [("b", BreakdownState::Known), ("a", BreakdownState::Partial)]);
    let (at47, through) = held.at(47);
    let b = at47.parts.iter().find(|part| part.key == "b").expect("b");
    assert_eq!((b.state, b.amount.clone(), through), (BreakdownState::AtLeast, figure("10"), 49), "an amount not recorded stays with the sum: what follows adds to a lower bound");
    let (gap, through) = held.at(52);
    assert_eq!((gap.unrecorded_hours, through), (3, 52), "inside an unrecorded stretch the count of hours changes every hour");
    let (after, through) = held.at(65);
    assert_eq!(after.unrecorded_hours, 5);
    assert!(after.total.reason.as_deref().is_some_and(|reason| reason.contains("unattributed") && reason.contains("gap")));
    assert_eq!(through, 69, "another shown value changes at 70");
    assert_eq!(held.at(80).1, 100, "the last answer stands to the history's end");
    for hour in [5, 15, 35, 47, 52, 65, 80] {
        assert_eq!(held.at(hour), history().at(hour), "a seek and a fresh build answer alike at {hour}");
    }
}

#[test]
fn a_dated_value_is_the_last_one_read_by_the_hour() {
    let dated = Dated::new(vec![(20, "b"), (10, "a"), (20, "c")]);
    assert_eq!((dated.at(5), dated.at(10), dated.at(25)), (None, Some(&"a"), Some(&"c")));
    assert_eq!(dated.hours().collect::<Vec<_>>(), [10, 20]);
    assert_eq!((counted(&[3, 7, 7, 9], 7), counted(&[3, 7, 7, 9], 2)), (3, 0));
}


#[test]
fn a_part_is_dated_by_the_latest_read_among_its_amounts() {
    let decisions = node("decisions", 0);
    let dated = |path: &[Node], amount: Amount, last: i32| Leaf { path: path.to_vec(), amount, count: None, last: Some(last) };
    let leaves = [
        dated(&[decisions.clone(), node("war", 0)], Amount::Known(Exact::whole(640)), 30),
        dated(&[decisions.clone(), node("economy", 1)], Amount::NotRecorded("The cost of 1 payment was not recorded.".into()), 90),
        dated(&[decisions, node("war", 0)], Amount::Known(Exact::whole(5)), 12),
        leaf(&[node("laws", 1)], Amount::Known(Exact::whole(410))),
    ];
    let reply = breakdown(&leaves, &Unrecorded::default());
    let decisions = reply.parts.iter().find(|part| part.key == "decisions").expect("decisions");
    assert_eq!(decisions.last, Some(90), "a group is dated by its latest child, a payment of unknown cost included");
    assert_eq!(decisions.children.iter().find(|part| part.key == "war").and_then(|part| part.last), Some(30));
    assert_eq!(reply.parts.iter().find(|part| part.key == "laws").and_then(|part| part.last), None, "an undated amount gives no date");
    let mut built = HistoryBuilder::new(0, 100).dated();
    let war = built.leaf(vec![node("decisions", 0), node("war", 0)]);
    built.add(12, war, Amount::Known(Exact::whole(5)), None);
    built.add(30, war, Amount::Known(Exact::whole(640)), None);
    let held = built.build();
    assert_eq!((held.at(20).0.parts[0].last, held.at(40).0.parts[0].last), (Some(12), Some(30)), "a dated history dates each part by its reads up to the hour");
}

#[test]
fn a_share_of_a_parent_is_given_only_when_both_amounts_are_known() {
    let decisions = node("decisions", 0);
    let partly = [
        leaf(&[decisions.clone(), node("war", 0)], Amount::Known(Exact::whole(640))),
        leaf(&[decisions, node("economy", 1)], Amount::Partial(Exact::whole(352), "some output was not recorded".into())),
        leaf(&[node("daily", 1)], Amount::Known(Exact::whole(2071))),
    ];
    let reply = breakdown(&partly, &Unrecorded::default());
    let decisions = reply.parts.iter().find(|part| part.key == "decisions").expect("decisions");
    assert_eq!(decisions.state, BreakdownState::Partial);
    assert!(decisions.children.iter().all(|child| child.parent_share.is_none()), "a part whose output is partly missing is no denominator: never 100% of it");
    assert_eq!(decisions.parent_share, None, "nor is a partial part a share of the whole");
    let daily = reply.parts.iter().find(|part| part.key == "daily").expect("daily");
    assert!(daily.parent_share.is_none(), "the whole is partial, so no part is an exact share of it");
    let known = breakdown(&partly[..1], &Unrecorded::default());
    assert_eq!(known.parts[0].children[0].parent_share, Some(1.0), "a known child of a known part keeps its share");
}

#[test]
fn fractions_add_up_exactly_and_a_sum_too_fine_to_add_is_not_recorded() {
    let third = Exact::new(2, 6).expect("a positive denominator");
    assert_eq!((third.numerator(), third.denominator()), (1, 3), "reduced");
    let leaves = [leaf(&[node("a", 0)], Amount::Known(third)), leaf(&[node("a", 0)], Amount::Known(third)), leaf(&[node("b", 1)], Amount::Known(third))];
    let reply = breakdown(&leaves, &Unrecorded::default());
    assert_eq!(reply.total.amount, figure("1"), "three thirds are one, never 0.99");
    assert_eq!(reply.parts[0].amount, Some(BreakdownAmount { numerator: "2".into(), denominator: "3".into() }));
    assert_eq!(Exact::new(1, 0), None);
    let fine = |denominator: i128| leaf(&[node("c", 2)], Amount::Known(Exact::new(1, denominator).expect("positive")));
    let reply = breakdown(&[fine(i128::MAX), fine(i128::MAX - 1)], &Unrecorded::default());
    assert_eq!((reply.total.amount, reply.total.state), (None, BreakdownState::NotRecorded), "an overflowed sum is never a figure");
    assert_eq!(reply.total.reason.as_deref(), Some(INEXACT));
}
