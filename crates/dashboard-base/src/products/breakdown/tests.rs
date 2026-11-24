use super::*;

fn node(key: &str, ordinal: u32) -> Node {
    Node::new(key, key, ordinal)
}

fn leaf(path: &[Node], amount: Amount) -> Leaf {
    Leaf { path: path.to_vec(), amount, count: None }
}

fn keys(parts: &[BreakdownPart]) -> Vec<&str> {
    parts.iter().map(|part| part.key.as_str()).collect()
}

#[test]
fn the_whole_and_its_order_are_decided_here() {
    let decisions = node("decisions", 0);
    let leaves = [
        leaf(&[decisions.clone(), node("war", 0)], Amount::Known(640)),
        leaf(&[node("daily", 1)], Amount::Known(2071)),
        leaf(&[decisions, node("economy", 1)], Amount::Known(352)),
        leaf(&[node("rest", 3).other()], Amount::Known(5000)),
        leaf(&[node("laws", 2)], Amount::Known(410)),
    ];
    let reply = breakdown(0, &leaves, &Unrecorded::default());
    assert_eq!(reply.total, BreakdownTotal { raw: Some("8473".into()), state: BreakdownState::Known, reason: None });
    assert_eq!(keys(&reply.parts), ["daily", "decisions", "laws", "rest"], "known parts by size, other after them");
    let decisions = &reply.parts[1];
    assert_eq!(decisions.raw.as_deref(), Some("992"));
    assert_eq!(keys(&decisions.children), ["war", "economy"]);
    assert_eq!(decisions.children[0].share, Some(640.0 / 8473.0), "a sub-part's share is of the whole");
    assert_eq!(decisions.ordinal, 0, "colours follow the recording's order, not the drawn one");
}

#[test]
fn a_part_not_recorded_makes_the_whole_a_lower_bound_without_shares() {
    let leaves = [
        leaf(&[node("new ships", 0)], Amount::NotRecorded("New ships were not recorded".into())),
        leaf(&[node("repairs", 2)], Amount::Known(211)),
        leaf(&[node("refits", 1)], Amount::NotRecorded("Refits were not recorded".into())),
    ];
    let reply = breakdown(0, &leaves, &Unrecorded::default());
    assert_eq!(reply.total.state, BreakdownState::AtLeast);
    assert_eq!(reply.total.raw.as_deref(), Some("211"));
    assert_eq!(keys(&reply.parts), ["repairs", "new ships", "refits"], "not recorded last, in recorded order");
    assert!(reply.parts.iter().all(|part| part.share.is_none()), "a lower bound has no shares");
    assert_eq!(reply.parts[1].state, BreakdownState::NotRecorded);
    assert_eq!(reply.parts[1].raw, None, "not recorded is never zero");
}

#[test]
fn nothing_recorded_is_not_a_zero() {
    let reply = breakdown(5, &[leaf(&[node("a", 0)], Amount::NotRecorded("Not read".into()))], &Unrecorded::default());
    assert_eq!(reply.total, BreakdownTotal { raw: None, state: BreakdownState::NotRecorded, reason: Some("Not read".into()) });
}

#[test]
fn unrecorded_hours_mark_the_whole_partial_and_keep_its_shares() {
    let leaves = [leaf(&[node("a", 0)], Amount::Known(30)), leaf(&[node("b", 1)], Amount::Partial(10, "Some costs were not recorded".into()))];
    let reply = breakdown(0, &leaves, &Unrecorded { hours: 2, reason: Some("Spending was not recorded for 2 hours".into()) });
    assert_eq!(reply.total.state, BreakdownState::Partial);
    assert_eq!(reply.total.reason.as_deref(), Some("Spending was not recorded for 2 hours; Some costs were not recorded"));
    assert_eq!(reply.unrecorded_hours, 2);
    assert_eq!(reply.parts[0].share, Some(0.75));
    assert_eq!(reply.parts[1].state, BreakdownState::Partial);
}

#[test]
fn corrections_below_zero_follow_the_positive_parts_without_a_share() {
    let leaves = [
        leaf(&[node("returned", 0)], Amount::Known(-13)),
        leaf(&[node("lost", 1)], Amount::Known(100)),
        leaf(&[node("level", 2)], Amount::Known(0)),
    ];
    let reply = breakdown(0, &leaves, &Unrecorded::default());
    assert_eq!(keys(&reply.parts), ["lost", "level", "returned"]);
    assert_eq!(reply.total.raw.as_deref(), Some("87"));
    assert_eq!(reply.parts[2].share, None);
}

#[test]
fn counts_add_up_the_levels() {
    let a = node("a", 0);
    let leaves = [
        Leaf { path: vec![a.clone(), node("x", 0)], amount: Amount::Known(1), count: Some(2) },
        Leaf { path: vec![a, node("y", 1)], amount: Amount::Known(1), count: Some(3) },
    ];
    let reply = breakdown(0, &leaves, &Unrecorded::default());
    assert_eq!(reply.parts[0].count, Some(5));
}
