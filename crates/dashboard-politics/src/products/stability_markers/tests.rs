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
fn a_country_s_wars_and_factions_are_marked_up_to_the_hour_and_the_next_one_bounds_the_reply() {
    let groups = groups();
    let mut out = Markers { hour: 450, shown: Vec::new(), next: None };
    let ita = |tag: &str, _: i32| tag == "ITA";
    war_markers(&mut out, &ita, &groups);
    faction_markers(&mut out, &ita, &groups, (0, 1000));
    assert_eq!(titles(&out), [(200, "War with ETH began"), (400, "ETH capitulated"), (300, "Joined Axis")]);
    assert_eq!(out.next, Some(501), "leaving the faction is after the hour: it is not shown, it is when the reply is next asked");
    assert!(out.shown.iter().all(|marker| marker.hour <= 450));
    assert_eq!(out.shown[1].verb, Some(StabilityMarkerVerb::Capitulated));
    assert!(out.shown[1].alert);

    let mut founder = Markers { hour: 1000, shown: Vec::new(), next: None };
    let ger = |tag: &str, _: i32| tag == "GER";
    faction_markers(&mut founder, &ger, &groups, (0, 900));
    assert_eq!(titles(&founder), [(100, "Founded Axis")], "the sole founder founded it; a member to the branch's end never left");
    war_markers(&mut founder, &ger, &groups);
    assert_eq!(titles(&founder)[1..], [(250, "War with POL began")], "a country capitulating to someone else's war is not this country's enemy");
}

#[test]
fn an_event_is_matched_to_the_slot_at_its_own_hour_not_at_the_hour_asked() {
    let groups = groups();
    let until_260 = |tag: &str, hour: i32| tag == "GER" && hour < 260;
    let mut out = Markers { hour: 1000, shown: Vec::new(), next: None };
    war_markers(&mut out, &until_260, &groups);
    faction_markers(&mut out, &until_260, &groups, (0, 1000));
    assert_eq!(titles(&out), [(250, "War with POL began"), (100, "Founded Axis")], "a war begun under a tag the slot later lost is still its war");

    let from_350 = |tag: &str, hour: i32| tag == "ITA" && hour >= 350;
    let mut late = Markers { hour: 1000, shown: Vec::new(), next: None };
    war_markers(&mut late, &from_350, &groups);
    faction_markers(&mut late, &from_350, &groups, (0, 1000));
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
    faction_markers(&mut out, &slot, &groups, (0, 1000));
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
    let notes = holder_notes(&mut out, &holders, &Country::Tag("GER".into()), 0);
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(notes[0].ends_with("Who holds GER is not known: Countries table unread"), "{notes:?}");
    assert!(notes[0].contains(" to "), "a span over by the hour asked has its end: {notes:?}");
    assert!(notes[1].ends_with(&format!("Who holds GER is not known: {TAG_UNLINKED}")) && !notes[1].contains(" to "), "a span still open says only its start: {notes:?}");
    assert_eq!(out.next, Some(261), "the open span's end is the next change; a later span starts after it");
    let mut early = Markers { hour: 50, shown: Vec::new(), next: None };
    assert!(holder_notes(&mut early, &holders, &Country::Tag("GER".into()), 0).is_empty(), "nothing is said before it is known");
    assert_eq!(early.next, Some(100));
}
