use super::*;

fn run(first: i32, last: i32, holder: Holder) -> HolderRun {
    HolderRun { first, last, holder, starts_segment: true }
}

const GER: Holder = Holder::Found { slot: 5, id: 5 };
const GER_AFTER: Holder = Holder::Found { slot: 5, id: 377 };

#[test]
fn a_tag_that_changes_holder_starts_a_new_run_and_segment() {
    let runs = merged(vec![run(100, 109, GER), run(110, 119, GER_AFTER)]);
    assert_eq!(runs, vec![run(100, 109, GER), run(110, 119, GER_AFTER)]);
    let holders = Holders::new([("GER".to_owned(), runs)].into(), Vec::new());
    assert_eq!(holders.at("GER", 115).map(|run| run.holder), Some(GER_AFTER));
    assert_eq!(holders.segment_first("GER", 115), Some(110));
    assert_eq!(holders.segment_first("GER", 105), Some(100));
}

#[test]
fn the_same_holder_across_a_load_into_a_recorded_hour_is_one_run() {
    let runs = merged(vec![run(200, 299, GER), run(100, 199, GER)]);
    assert_eq!(runs, vec![run(100, 299, GER)]);
}

#[test]
fn hours_without_a_holder_break_the_run_and_unresolved_is_never_a_holder() {
    let runs = merged(vec![run(100, 109, GER), run(110, 114, Holder::Unresolved), run(115, 119, GER), run(130, 139, GER)]);
    assert_eq!(runs.len(), 4, "unresolved hours and a gap each break the holder's run");
    let holders = Holders::new([("GER".to_owned(), runs)].into(), Vec::new());
    assert_eq!(holders.at("GER", 112).map(|run| run.holder), Some(Holder::Unresolved));
    assert!(holders.at("GER", 125).is_none(), "an hour no run holds is absent, not a holder");
    assert_eq!(holders.segment_first("GER", 117), Some(115));
    let within = holders.within("GER", 105, 132);
    assert_eq!((within.first().map(|run| run.first), within.last().map(|run| run.last)), (Some(105), Some(132)));
}

#[test]
fn a_slot_follows_its_country_through_a_tag_change_while_the_tag_goes_to_another() {
    const OTHER: Holder = Holder::Found { slot: 9, id: 9 };
    let holders = Holders::new(
        [("GER".to_owned(), vec![run(100, 109, GER), run(110, 119, OTHER)]), ("DEU".to_owned(), vec![run(110, 119, GER)])].into(),
        Vec::new(),
    );
    let country = Country::Slot(5);
    assert_eq!(holders.at(&country, 115).map(|run| run.holder), Some(GER));
    assert_eq!(holders.at("GER", 115).map(|run| run.holder), Some(OTHER));
    assert_eq!(holders.at(&Country::from("GER"), 115).map(|run| run.holder), Some(OTHER));
    assert_eq!((holders.tag_at(&country, 105), holders.tag_at(&country, 115)), (Some("GER"), Some("DEU")));
    assert_eq!(holders.within(&country, 100, 119), vec![run(100, 119, GER)], "one holder through the switch is one run");
    let overlapping = Holders::new(
        [("AAA".to_owned(), vec![run(100, 119, GER_AFTER)]), ("BBB".to_owned(), vec![run(105, 109, GER)])].into(),
        Vec::new(),
    );
    assert_eq!(
        overlapping.within(&country, 100, 119),
        vec![run(100, 104, GER_AFTER), run(105, 109, GER), run(110, 119, GER_AFTER)],
    );
    assert!(holders.at(&country, 125).is_none(), "an hour the slot holds no tag is absent, never another country");
    assert_eq!(holders.tag_at(&country, 125), None);
    assert_eq!(holders.name(&country, 125), country.key(), "a fact at an hour the slot holds no tag is the slot, never a neighbour's name");
    assert_eq!(holders.name(&country, 95), country.key(), "nor a later holder's");
    assert_eq!(holders.label(&country, 125), None);
    assert_eq!(holders.say(&country, 125, |name| format!("{name} has no row")), NOT_HELD_AT_HOUR, "the reason is that the slot holds no tag then");
    assert_eq!((holders.nearest_name(&country, 125), holders.nearest_name(&country, 95)), ("DEU".to_owned(), "GER".to_owned()), "a label that is no fact at an hour asks for the nearest name by name");
    assert_eq!(holders.name(&Country::Slot(77), 105), Country::Slot(77).key(), "a slot never named is named by its key, never a bare number");
    assert_eq!(holders.label(&Country::Slot(77), 105), None, "it has no name to show");
    assert_eq!(holders.say(&Country::Slot(77), 105, |name| format!("{name} has no row")), NOT_IN_HISTORY);
    assert_eq!(holders.say(&country, 115, |name| format!("{name} has no row")), "DEU has no row");
    assert!(!holders.named(&Country::Slot(77)));
    assert!(Country::Slot(5).key().starts_with(SLOT_MARK) && Country::Slot(5).key() != "5", "a slot key is no tag a mod can define");
    assert_eq!(Country::from("5").key(), "5");
    assert_eq!(serde_json::from_str::<Country>("5").unwrap(), country);
    assert_eq!(serde_json::from_str::<Country>(r#""GER""#).unwrap(), Country::from("GER"));
}

#[test]
fn a_slot_names_the_tag_it_holds_at_each_hour_and_none_between() {
    let holders = Holders::new(
        [("SPR".to_owned(), vec![run(100, 149, GER)]), ("SPA".to_owned(), vec![run(160, 199, GER_AFTER)])].into(),
        Vec::new(),
    );
    assert_eq!(holders.slot_tag(5, 120), Some("SPR"));
    assert_eq!(holders.slot_tag(5, 155), None, "no tag between the two runs");
    assert_eq!(holders.slot_tag(5, 199), Some("SPA"));
    assert_eq!(holders.slot_tag(6, 120), None);
    let overlapping = Holders::new(
        [("AAA".to_owned(), vec![run(100, 119, GER)]), ("BBB".to_owned(), vec![run(105, 109, GER_AFTER)])].into(),
        Vec::new(),
    );
    assert_eq!(overlapping.slot_tag(5, 107), overlapping.tag_at(&Country::Slot(5), 107), "a later, higher holder id names the slot nowhere");
    assert_eq!(overlapping.slot_tag(5, 107), Some("AAA"));
}

#[test]
fn the_next_holder_change_is_the_first_run_edge_after_the_hour() {
    let holders = Holders::new(
        [("SPR".to_owned(), vec![run(100, 149, GER)]), ("SPA".to_owned(), vec![run(160, 199, GER_AFTER)])].into(),
        vec![(150, 159, "not read".to_owned())],
    );
    assert_eq!(holders.next_change(120), Some(150), "the stretch with no holder starts at 150");
    assert_eq!(holders.next_change(150), Some(160));
    assert_eq!(holders.next_change(199), Some(200), "the last run ends");
    assert_eq!(holders.next_change(200), None);
}

#[test]
fn a_reason_for_no_name_uses_only_what_was_known_at_its_hour() {
    let holders = Holders::new([("GER".to_owned(), vec![run(120, 139, GER)])].into(), vec![(150, 159, "Countries were not read".to_owned())]);
    let country = Country::Slot(5);
    let said = |hour: i32| holders.say(&country, hour, |name| format!("{name} has no row"));
    assert_eq!(said(110), NOT_IN_HISTORY, "before its first appearance nothing yet names the slot");
    assert_eq!(said(130), "GER has no row");
    assert_eq!(said(145), NOT_HELD_AT_HOUR);
    assert_eq!(said(155), format!("Who holds {} is not known: Countries were not read", country.key()), "an hour whose holders were not read says so");
    let unlinked = Holders::new([("GER".to_owned(), vec![run(120, 139, GER)]), ("ITA".to_owned(), vec![run(140, 149, Holder::Unresolved)])].into(), Vec::new());
    let said = |hour: i32| unlinked.say(&country, hour, |name| format!("{name} has no row"));
    assert_eq!(said(145), format!("Who holds {} is not known: a recorded tag's country was not resolved at this hour", country.key()), "R5-S8: a tag whose country is unresolved could be this slot's");
    assert_eq!(said(150), NOT_HELD_AT_HOUR, "once every recorded tag is resolved, the slot holds none");
}
