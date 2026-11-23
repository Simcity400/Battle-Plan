use super::*;

fn parsed(input: &[u8]) -> io::Result<Pack> {
    parse(&content_reader_parser::parser::parse(input)?.items)
}

#[test]
fn owned_goal_references_preserve_order_empty_and_missing_payloads() {
    let pack = parsed(
        br"countries={AAA={diplomacy={
            wargoals={
            declared_mod_goal={id={id=7 type=4713} wargoaldata_actor=AAA
                wargoaldata_recipient=BBB type=mod_goal states={4 2 4}}
            mod_empty_goal={id={id=8 type=4713} wargoaldata_actor=AAA
                wargoaldata_recipient=CCC type=mod_empty_goal}
            }
            available_wargoals={wargoal={id=8 type=4713} wargoal={id=7 type=4713}
                wargoal={id=9 type=4713} wargoal={id=0 type=0}}
        }} BBB={} CCC={diplomacy={}}}",
    )
    .unwrap();
    let handle = |id: u64| (id << 32) | 4713;
    assert_eq!(
        pack.owned_goals[b"AAA".as_slice()],
        [handle(8), handle(7), handle(9), 0]
    );
    assert!(pack.owned_goals[b"BBB".as_slice()].is_empty());
    assert!(pack.owned_goals[b"CCC".as_slice()].is_empty());
    assert_eq!(pack.goals[&handle(7)].states, [4, 2, 4]);
    assert_eq!(pack.goals[&handle(7)].goal_type.as_slice(), b"mod_goal");
    assert!(pack.goals[&handle(8)].states.is_empty());
    assert!(!pack.goals.contains_key(&handle(9)));
}

#[test]
fn owned_goal_rosters_refuse_malformed_and_duplicate_payloads() {
    for input in [
        "countries={AAA={diplomacy={available_wargoals={unknown={id=7 type=4713}}}}}",
        "countries={AAA={diplomacy={available_wargoals={wargoal={id=7}}}}}",
        "countries={AAA={diplomacy={wargoals={g={id={id=7 type=4713} wargoaldata_actor=AAA type=g}}}}}",
        "countries={AAA={diplomacy={wargoals={g={id={id=7 type=4713} wargoaldata_actor=AAA wargoaldata_recipient=BBB type=g states={x}}}}}}",
        "countries={AAA={diplomacy={wargoals={g={id={id=7 type=4713} wargoaldata_actor=AAA wargoaldata_recipient=BBB type=g} g={id={id=7 type=4713} wargoaldata_actor=AAA wargoaldata_recipient=BBB type=g}}}}}",
        "countries={AAA={diplomacy={wargoals={{id={id=7 type=4713} wargoaldata_actor=AAA wargoaldata_recipient=BBB type=g}}}}}",
        "countries={AAA={diplomacy={wargoals={\"\"={id={id=7 type=4713} wargoaldata_actor=AAA wargoaldata_recipient=BBB type=g}}}}}",
    ] {
        assert!(parsed(input.as_bytes()).is_err(), "{input}");
    }
}

#[test]
fn missing_country_roster_save_coverage_is_not_known_empty() {
    assert!(parsed(b"date=1936.1.1.1").is_err());
    assert_eq!(parsed(b"countries={}").unwrap(), Pack::default());
}
