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
        "unobserved": { "humans": [], "factions": [], "wars": [] },
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
