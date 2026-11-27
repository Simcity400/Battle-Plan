use super::*;
use crate::products::wars::{WarContributionGroup, measures::permyriad};

fn at(runs: &[(i32, String)], hour: i32) -> Option<&str> {
    runs.partition_point(|(from, _)| *from <= hour).checked_sub(1).map(|i| runs[i].1.as_str())
}

fn missing(spans: &[MissingSpan], hour: i32) -> bool {
    spans.iter().any(|span| span.first_hour <= hour && hour <= span.last_hour)
}

#[test]
fn repeated_native_roster_entries_receive_the_same_weight_as_the_snapshot() {
    let from_rows = |rows: Vec<(u128, &[i64])>| History {
        keys: rows.into_iter().map(|(key, values)| (key, vec![lanes::Step {
            hour: 100, cells: Some(values.iter().copied().map(Cell::Number).collect()),
        }])).collect(),
        statuses: vec![lanes::StatusRun { first: 100, last: 100, status: Status::Complete }],
        ..Default::default()
    };
    let [first, second] = ledger_keys(1, 2);
    let ledger = from_rows(vec![(first, &[13; 10]), (second, &[17; 10])]);
    let delivery = from_rows(vec![(3 << 32 | 2, &[19; 4])]);
    let links = RunLinks { slots: (1..=3).map(|slot| (slot, vec![(100, 100, Some(i64::from(slot)))])).collect() };
    let roster = [vec![("ONE", (1, 1))], vec![("TWO", (2, 2)), ("TWO", (2, 2)), ("THREE", (3, 3))]];
    let (values, unread) = score_inputs(&ledger, &delivery, &BTreeSet::from([(1, 2)]), &links, &roster, 100);
    assert_eq!(unread, [None, None]);
    assert_eq!(values[&1].ledgers.len(), 2);
    assert_eq!(values[&2].ledgers.len(), 1);
    assert_eq!(values[&3].deliveries.len(), 2);
    let mut member = Member::default();
    for hour in 100..=101 {
        for _ in 0..2 {
            member.admit(hour, hour, 1, "TWO");
        }
    }
    assert_eq!(member.reply.membership, [(100, 101, 1, "TWO".into())]);
}

pub(in crate::products::wars) fn assert_snapshot(reply: &WarContributionHistoryReply, hour: i32, snapshot: &WarContributionGroup) {
    assert!(!missing(&reply.gaps, hour));
    for (i, measure) in snapshot.measures.iter().enumerate() {
        assert_eq!(reply.measure_keys[i], measure.key);
        let sides = [&measure.viewpoint, &measure.opposing];
        let totals = std::array::from_fn::<_, 2, _>(|side| (!missing(&reply.sides[side].missing, hour))
            .then(|| at(&reply.sides[side].totals[i], hour).unwrap().parse::<i128>().unwrap()));
        let war_total = totals[0].zip(totals[1]).filter(|_| i != 0).map(|(a, b)| a + b);
        for (side, result) in sides.into_iter().enumerate() {
            assert_eq!(totals[side].is_some(), result.total.is_some());
            for expected in &result.members {
                let member = reply.contributors.iter().find(|member| member.membership.iter().any(|(first, last, position, tag)|
                    *first <= hour && hour <= *last && usize::from(*position) == side && tag == &expected.tag)).unwrap();
                let amount = at(&member.amounts[i], hour).unwrap();
                let value = amount.parse::<i128>().unwrap();
                assert_eq!(amount, expected.value);
                assert_eq!(totals[side].and_then(|whole| permyriad(value, whole)), expected.share_of_side);
                assert_eq!(war_total.and_then(|whole| permyriad(value, whole)), expected.share_of_war);
                assert_eq!(i == 5 && member.partial_casualties.iter().any(|span| span.first_hour <= hour && hour <= span.last_hour), expected.partial);
            }
        }
    }
}
