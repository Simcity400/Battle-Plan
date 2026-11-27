use super::*;

fn groups() -> CountryGroupReply {
    serde_json::from_value(serde_json::json!({
        "branchId": "1",
        "humans": [],
        "factions": [{
            "id": "axis", "name": "Axis", "nameReason": null, "names": [], "recordedName": "axis", "firstHour": 100, "lastHour": 900,
            "members": [{ "tag": "GER", "firstHour": 100, "lastHour": 900 }, { "tag": "ITA", "firstHour": 300, "lastHour": 500 }],
            "leaders": [], "incomplete": []
        }],
        "wars": [{ "first": "ITA", "second": "ETH", "firstHour": 200, "lastHour": 399 }],
        "warStarts": [{ "hour": 200, "attacking": "ITA", "defending": "ETH" }, { "hour": 250, "attacking": "GER", "defending": "POL" }],
        "capitulations": [{ "hour": 400, "tag": "ETH" }, { "hour": 420, "tag": "POL" }],
        "identityChanges": [],
        "countryIdentities": [],
        "unobserved": { "humans": [], "factions": [], "wars": [], "unattributed": [] },
        "warnings": []
    }))
    .unwrap()
}

fn titles(out: &Markers) -> Vec<(i32, &str)> {
    out.shown.iter().map(|marker| (marker.hour, marker.title.as_str())).collect()
}

#[test]
fn a_war_with_an_unresolved_opponent_still_marks_the_known_countrys_history() {
    for (attacking, defending) in [(Some("AAA"), None), (None, Some("AAA"))] {
        let mut groups = groups();
        groups.war_starts = serde_json::from_value(serde_json::json!([{
            "hour": 275, "attacking": attacking, "defending": defending,
            "attackingMissing": attacking.is_none().then_some("Attacking country link not resolved"),
            "defendingMissing": defending.is_none().then_some("Defending country link not resolved")
        }])).unwrap();
        let holds = |tag: &str, _: i32| tag == "AAA";
        let mut before = Markers { hour: 274, shown: Vec::new(), next: None };
        war_markers(&mut before, &holds, &groups, &[]);
        assert!(before.shown.is_empty());
        assert_eq!(before.next, Some(275));
        let mut at = Markers { hour: 275, shown: Vec::new(), next: None };
        war_markers(&mut at, &holds, &groups, &[]);
        assert_eq!(at.shown.len(), 1);
        assert!(at.shown[0].title.contains("country link not resolved"));
        assert_eq!(at.shown[0].other, None, "an unresolved opponent is never a made-up country");
        let mut hidden = Markers { hour: 275, shown: Vec::new(), next: None };
        war_markers(&mut hidden, &holds, &groups, &[HourSpan { first_hour: 275, last_hour: 275 }]);
        assert!(hidden.shown.is_empty());
        let mut other = Markers { hour: 275, shown: Vec::new(), next: None };
        let notes = war_markers(&mut other, &|tag: &str, _: i32| tag == "BBB", &groups, &[]);
        assert!(other.shown.is_empty());
        assert!(notes.iter().any(|note| note.contains("any may be this country's, and none is marked")));
    }
}

#[test]
fn a_country_s_wars_and_factions_are_marked_up_to_the_hour_and_the_next_one_bounds_the_reply() {
    let groups = groups();
    let mut out = Markers { hour: 450, shown: Vec::new(), next: None };
    let ita = |tag: &str, _: i32| tag == "ITA";
    war_markers(&mut out, &ita, &groups, &[]);
    faction_markers(&mut out, &ita, &groups, (0, 1000), &[]);
    assert_eq!(titles(&out), [(200, "War with ETH began"), (400, "ETH capitulated"), (300, "Joined Axis")]);
    assert_eq!(out.next, Some(501), "leaving the faction is after the hour: it is not shown, it is when the reply is next asked");
    assert!(out.shown.iter().all(|marker| marker.hour <= 450));
    assert_eq!(out.shown[1].verb, Some(StabilityMarkerVerb::Capitulated));
    assert!(out.shown[1].alert);

    let mut founder = Markers { hour: 1000, shown: Vec::new(), next: None };
    let ger = |tag: &str, _: i32| tag == "GER";
    faction_markers(&mut founder, &ger, &groups, (0, 900), &[]);
    assert_eq!(titles(&founder), [(100, "Founded Axis")], "the sole founder founded it; a member to the branch's end never left");
    war_markers(&mut founder, &ger, &groups, &[]);
    assert_eq!(titles(&founder)[1..], [(250, "War with POL began")], "a country capitulating to someone else's war is not this country's enemy");
}

#[test]
fn an_event_is_matched_to_the_slot_at_its_own_hour_not_at_the_hour_asked() {
    let groups = groups();
    let until_260 = |tag: &str, hour: i32| tag == "GER" && hour < 260;
    let mut out = Markers { hour: 1000, shown: Vec::new(), next: None };
    war_markers(&mut out, &until_260, &groups, &[]);
    faction_markers(&mut out, &until_260, &groups, (0, 1000), &[]);
    assert_eq!(titles(&out), [(250, "War with POL began"), (100, "Founded Axis")], "a war begun under a tag the slot later lost is still its war");

    let from_350 = |tag: &str, hour: i32| tag == "ITA" && hour >= 350;
    let mut late = Markers { hour: 1000, shown: Vec::new(), next: None };
    war_markers(&mut late, &from_350, &groups, &[]);
    faction_markers(&mut late, &from_350, &groups, (0, 1000), &[]);
    assert_eq!(titles(&late), [(400, "ETH capitulated"), (501, "Left Axis")], "nothing before the slot held the tag is its event");
}

#[test]
fn a_slot_that_changes_tag_inside_its_faction_neither_leaves_nor_joins() {
    let mut groups = groups();
    groups.factions[0].members = serde_json::from_value(serde_json::json!([
        { "tag": "GER", "firstHour": 100, "lastHour": 599 }, { "tag": "DEU", "firstHour": 600, "lastHour": 800 }
    ]))
    .unwrap();
    let slot = |tag: &str, hour: i32| (tag == "GER" && hour < 600) || (tag == "DEU" && hour >= 600);
    let mut out = Markers { hour: 1000, shown: Vec::new(), next: None };
    faction_markers(&mut out, &slot, &groups, (0, 1000), &[]);
    assert_eq!(titles(&out), [(100, "Founded Axis"), (801, "Left Axis")], "the tag change at 600 is no event: the slot stayed a member");
}

#[test]
fn markers_a_holder_hides_say_when_and_why() {
    use dashboard_query::identity::HolderRun;
    let held = |first, last| HolderRun { first, last, holder: Holder::Found { slot: 7, id: 7 }, starts_segment: false };
    let holders = Holders::new(
        [("GER".to_owned(), vec![held(0, 199), HolderRun { first: 200, last: 260, holder: Holder::Unresolved, starts_segment: false }, held(261, 1000)])].into_iter().collect(),
        vec![(100, 120, "Countries table unread".to_owned()), (700, 720, "Countries table unread".to_owned())],
    );
    let mut out = Markers { hour: 230, shown: Vec::new(), next: None };
    let notes = holder_notes(&mut out, &holders, &Country::Tag("GER".into()), (0, 1000), "War and faction markers");
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(notes[0].ends_with("Who holds GER is not known: Countries table unread"), "{notes:?}");
    assert!(notes[0].contains(" to "), "a span over by the hour asked has its end: {notes:?}");
    assert!(notes[1].ends_with(&format!("Who holds GER is not known: {TAG_UNLINKED}")) && !notes[1].contains(" to "), "a span still open says only its start: {notes:?}");
    assert_eq!(out.next, Some(261), "the open span's end is the next change; a later span starts after it");
    let mut early = Markers { hour: 50, shown: Vec::new(), next: None };
    assert!(holder_notes(&mut early, &holders, &Country::Tag("GER".into()), (0, 1000), "War and faction markers").is_empty(), "nothing is said before it is known");
    assert_eq!(early.next, Some(100));
    let mut last = Markers { hour: 720, shown: Vec::new(), next: None };
    assert_eq!(holder_notes(&mut last, &holders, &Country::Tag("GER".into()), (0, 720), "War markers").len(), 3);
    assert_eq!(last.next, None, "a span open to the branch's end has no later change");

    let other = Holders::new(
        [("GER".to_owned(), vec![held(0, 1000)]), ("ITA".to_owned(), vec![HolderRun { first: 200, last: 260, holder: Holder::Unresolved, starts_segment: false }])].into_iter().collect(),
        Vec::new(),
    );
    let mut slot = Markers { hour: 230, shown: Vec::new(), next: None };
    assert!(holder_notes(&mut slot, &other, &Country::Slot(7), (0, 1000), "Faction markers").is_empty(), "another tag's unresolved holder says nothing of a slot that kept its own");

    let found = |slot: u32, first: i32, last: i32| HolderRun { first, last, holder: Holder::Found { slot, id: slot }, starts_segment: true };
    let unresolved = |first: i32, last: i32| HolderRun { first, last, holder: Holder::Unresolved, starts_segment: true };
    let nested = Holders::new(
        [("AAA".to_owned(), vec![found(5, 100, 119)]), ("BBB".to_owned(), vec![found(5, 105, 109)]), ("CCC".to_owned(), vec![unresolved(100, 119)])].into_iter().collect(),
        Vec::new(),
    );
    let mut overlap = Markers { hour: 115, shown: Vec::new(), next: None };
    assert!(holder_notes(&mut overlap, &nested, &Country::Slot(5), (100, 200), "War markers").is_empty(), "a slot held throughout, under a tag nested in another, has no unknown hour");
    let moved = Holders::new(
        [("GER".to_owned(), vec![found(7, 0, 99), found(8, 100, 300)]), ("CCC".to_owned(), vec![unresolved(100, 120)])].into_iter().collect(),
        Vec::new(),
    );
    let mut left = Markers { hour: 110, shown: Vec::new(), next: None };
    let notes = holder_notes(&mut left, &moved, &Country::Slot(7), (0, 300), "War markers");
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("The chosen country's tag is not known") && notes[0].ends_with("it was last recorded as GER"), "the slot's own tag is what is missing, named by its last record: {notes:?}");
}

#[test]
fn an_unattributed_war_start_is_said_once_with_its_span_and_a_late_join_says_it_is_late() {
    let mut out = Markers { hour: 300, shown: Vec::new(), next: None };
    let note = unattributed_note(&mut out, &[150, 210, 400]).unwrap();
    assert!(note.starts_with("2 war starts or capitulations from ") && note.contains(" to "), "{note}");
    assert_eq!(out.next, Some(400));
    assert!(unattributed_note(&mut Markers { hour: 100, shown: Vec::new(), next: None }, &[150]).is_none());

    let mut groups = groups();
    groups.unobserved.factions = serde_json::from_value(serde_json::json!([{ "firstHour": 299, "lastHour": 299 }])).unwrap();
    groups.factions[0].names = serde_json::from_value(serde_json::json!([
        { "firstHour": 100, "lastHour": 699, "name": "Axis", "reason": null, "token": null },
        { "firstHour": 700, "lastHour": 900, "name": "Pact", "reason": null, "token": null }
    ]))
    .unwrap();
    groups.factions[0].name = Some("Pact".to_owned());
    let mut late = Markers { hour: 450, shown: Vec::new(), next: None };
    let notes = faction_markers(&mut late, &|tag: &str, _| tag == "ITA", &groups, (0, 1000), &[]);
    assert!(titles(&late).is_empty(), "a join right after an unobserved hour is not dated");
    assert_eq!(notes.len(), 1, "but it is said: {notes:?}");
    assert!(notes[0].starts_with("In Axis by "), "named as it was then, not by its later name: {notes:?}");

    let mut joined = groups.clone();
    joined.factions[0].members = serde_json::from_value(serde_json::json!([
        { "tag": "AAA", "firstHour": 298, "lastHour": 300 }, { "tag": "BBB", "firstHour": 300, "lastHour": 500 }
    ]))
    .unwrap();
    let mut gap = Markers { hour: 450, shown: Vec::new(), next: None };
    let slot = |tag: &str, hour: i32| (tag == "AAA" && hour == 299) || (tag == "BBB" && hour >= 300);
    let notes = faction_markers(&mut gap, &slot, &joined, (0, 1000), &[]);
    assert!(notes.iter().any(|note| note.starts_with("In Axis by ")), "a roster unread the hour before says nothing of who was in it then: {notes:?}");
}

#[test]
fn an_hour_no_one_knows_who_held_the_country_is_no_departure_and_no_join() {
    use dashboard_query::identity::HolderRun;
    let mut groups = groups();
    groups.factions[0].members = serde_json::from_value(serde_json::json!([
        { "tag": "GER", "firstHour": 100, "lastHour": 298 }, { "tag": "GER", "firstHour": 300, "lastHour": 900 }
    ]))
    .unwrap();
    let held = |first, last| HolderRun { first, last, holder: Holder::Found { slot: 7, id: 7 }, starts_segment: false };
    let holders = Holders::new(
        [("GER".to_owned(), vec![held(0, 298), HolderRun { first: 299, last: 299, holder: Holder::Unresolved, starts_segment: false }, held(300, 1000)])].into_iter().collect(),
        Vec::new(),
    );
    let country = Country::Tag("GER".into());
    let hidden: Vec<HourSpan> = holder_gaps(&holders, &country, 0).into_iter().map(|(first_hour, last_hour, _)| HourSpan { first_hour, last_hour }).collect();
    let mut out = Markers { hour: 400, shown: Vec::new(), next: None };
    let notes = faction_markers(&mut out, &|tag: &str, hour| tag == "GER" && hour != 299, &groups, (0, 1000), &hidden);
    assert_eq!(titles(&out), [(100, "Founded Axis")], "no departure at 299 nor a join at 300 around an hour whose holder is unknown");
    assert!(notes.is_empty(), "and no late join either: the roster was read: {notes:?}");
    assert_eq!(holder_notes(&mut out, &holders, &country, (0, 1000), "Faction markers").len(), 1, "the holder warning says why");
}

#[test]
fn an_enemy_capitulating_while_no_one_knows_who_holds_the_country_is_not_marked() {
    let groups = groups();
    let hidden = [HourSpan { first_hour: 400, last_hour: 400 }];
    let mut out = Markers { hour: 450, shown: Vec::new(), next: None };
    war_markers(&mut out, &|tag: &str, _| tag == "ITA", &groups, &hidden);
    assert_eq!(titles(&out), [(200, "War with ETH began")], "the capitulation at 400 falls in the hour the holder warning covers");
}
