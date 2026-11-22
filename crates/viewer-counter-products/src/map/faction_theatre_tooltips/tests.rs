use super::*;
use replay_data::rows::{CountryLinkRow, TableRows, UnitLeaderRow};
use viewer_foundation::test_support::{test_frame, test_put as put};

fn entries() -> Vec<(&'static str, &'static str)> {
    vec![
        ("FACTION_THEATER_TOOLTIP", "$THEATER$|$OWNER_TAG$ $OWNER_NAME$|$COMMANDER$|$NUM_COUNTRIES$:$COUNTRIES$|$NUM_REGIONS$:$REGIONS$"),
        ("FACTION_THEATER_TOOLTIP_COMMANDER", "$TITLE$ $NAME$"),
        ("FACTION_THEATER_TOOLTIP_NO_COMMANDER", "no commander"),
        ("FACTION_THEATER_TOOLTIP_COUNTRY", "[$TAG$] $NAME$"),
        ("FACTION_THEATER_TOOLTIP_NO_COUNTRIES", "no countries"),
        ("FACTION_THEATER_TOOLTIP_REGION", "$NAME$"),
        ("FACTION_THEATER_TOOLTIP_NO_REGIONS", "no regions"),
        ("fm_leader", "Field Marshal"),
        ("cc_leader", "General"),
        ("navy_leader", "Admiral"),
        ("operative_skill_2", "Agent"),
        ("OPERATIVE", "Operative"),
        ("STRATEGICREGION_1", "Rhine"),
        ("STRATEGICREGION_2", "Alps"),
    ]
}

fn localisation(entries: Vec<(&str, &str)>) -> Localisation {
    let mut loc: Localisation = entries.into_iter().map(|(key, value)| (key.to_owned(), value.to_owned())).collect();
    loc.language = "english".into();
    loc
}

fn loc() -> Localisation {
    localisation(entries())
}

fn country(tag: &str, name: &str) -> Country {
    Country { tag: tag.into(), name: name.into() }
}

fn sample() -> Facts {
    Facts {
        name: Ok("Western Front".into()),
        owner: Ok(country("GER", "Germany")),
        commander: Ok(Some(Leader { name: "Rommel".into(), category: 0, skill: Some(4) })),
        countries: Ok(vec![country("GER", "Germany"), country("ITA", "Italy")]),
        regions: Ok(vec!["Rhine".into(), "Alps".into()]),
    }
}

fn refusals(tip: &Tooltip) -> Vec<&str> {
    tip.refused.keys().map(String::as_str).collect()
}

#[test]
fn a_theatre_lists_its_owner_commander_countries_and_regions_in_stored_order() {
    let tip = lines(&loc(), &sample());
    assert_eq!(tip.title, "Western Front|GER Germany|Field Marshal Rommel|2:[GER] Germany\n[ITA] Italy|2:Rhine\nAlps");
    assert_eq!(tip.body, "", "the delayed click line is left out");
    assert!(tip.refused.is_empty(), "{:?}", tip.refused);
}

#[test]
fn an_item_ending_in_a_newline_gets_no_second_one() {
    let mut entries = entries();
    entries.retain(|(key, _)| *key != "FACTION_THEATER_TOOLTIP_REGION");
    entries.push(("FACTION_THEATER_TOOLTIP_REGION", "$NAME$\n"));
    let loc = localisation(entries);
    let tip = lines(&loc, &sample());
    assert!(tip.title.ends_with("|2:Rhine\nAlps\n"), "{}", tip.title);
}

#[test]
fn empty_lists_and_no_commander_use_their_own_keys() {
    let tip = lines(
        &loc(),
        &Facts { commander: Ok(None), countries: Ok(Vec::new()), regions: Ok(Vec::new()), ..sample() },
    );
    assert_eq!(tip.title, "Western Front|GER Germany|no commander|0:no countries|0:no regions");
    assert!(tip.refused.is_empty(), "{:?}", tip.refused);
}

#[test]
fn the_role_title_follows_the_leader_category() {
    let title = |category, skill| {
        let leader = Leader { name: "X".into(), category, skill };
        let tip = lines(&loc(), &Facts { commander: Ok(Some(leader)), ..sample() });
        tip.title.split('|').nth(2).map(str::to_owned).unwrap_or_default()
    };
    assert_eq!(title(0, None), "Field Marshal X");
    assert_eq!(title(1, Some(3)), "General X");
    assert_eq!(title(2, Some(3)), "Admiral X");
    assert_eq!(title(3, Some(2)), "Agent X");
    assert_eq!(title(3, Some(5)), "Operative X", "a skill without its own key falls back");
    assert_eq!(title(9, Some(1)), " X", "an unknown category has an empty title");
    let tip = lines(&loc(), &Facts { commander: Ok(Some(Leader { name: "X".into(), category: 3, skill: None })), ..sample() });
    assert_eq!(tip.title, "");
    assert_eq!(refusals(&tip).len(), 1);
    assert!(refusals(&tip)[0].starts_with("faction theatre commander: the operative has no skill definition"));
}

#[test]
fn a_missing_operand_withholds_the_whole_text_naming_each() {
    let tip = lines(
        &loc(),
        &Facts {
            name: Err("FactionTheatres.name not recorded".into()),
            countries: Err("FactionTheatres.countries not recorded".into()),
            ..sample()
        },
    );
    assert_eq!(tip.title, "");
    assert_eq!(
        refusals(&tip),
        [
            "faction theatre countries: FactionTheatres.countries not recorded",
            "faction theatre name: FactionTheatres.name not recorded",
        ]
    );
    let mut entries = entries();
    entries.retain(|(key, _)| *key != "FACTION_THEATER_TOOLTIP_COUNTRY");
    let missing = localisation(entries);
    let tip = lines(&missing, &sample());
    assert_eq!(
        tip.title,
        "Western Front|GER Germany|Field Marshal Rommel|2:FACTION_THEATER_TOOLTIP_COUNTRY\nFACTION_THEATER_TOOLTIP_COUNTRY|2:Rhine\nAlps",
        "a missing localisation key is shown as the key, as the game shows it"
    );
}

fn regions() -> BTreeMap<u32, StrategicRegion> {
    [1, 2].into_iter().map(|id| (id, StrategicRegion { id, name_key: format!("STRATEGICREGION_{id}"), ..Default::default() })).collect()
}

#[test]
fn a_row_of_an_earlier_format_withholds_the_facts_it_lacks() {
    let row = FactionTheatreRow {
        faction: 7,
        ordinal: 0,
        regions: vec![1, 2],
        country_count: 2,
        name: Recorded::NotRecorded,
        owner: Recorded::NotRecorded,
        commander: Recorded::NotRecorded,
        countries: Recorded::NotRecorded,
        ..Default::default()
    };
    let theatre = Theatre::of(&row);
    let facts = facts_of(&theatre, test_frame(0));
    assert_eq!(facts.regions, Ok(vec!["Rhine".to_owned(), "Alps".to_owned()]));
    let tip = lines(&loc(), &facts);
    assert_eq!(tip.title, "");
    assert_eq!(
        refusals(&tip),
        [
            "faction theatre commander: FactionTheatres.commander not recorded",
            "faction theatre countries: FactionTheatres.countries not recorded",
            "faction theatre name: FactionTheatres.name not recorded",
            "faction theatre owner: FactionTheatres.owner not recorded",
        ]
    );
}

fn facts_of(theatre: &Theatre<'_>, frame: DecodedFrame) -> Facts {
    facts(&loc(), "english", &regions(), &frame, theatre)
}

#[test]
fn the_commander_resolves_through_unit_leaders_and_unresolved_links_are_named() {
    let mut frame = test_frame(0);
    put(
        &mut frame,
        TableRows::UnitLeaders(vec![UnitLeaderRow {
            reference_type: 9,
            id: 5,
            category: 3,
            skill_status: 2,
            skill_level: 2,
            inherited_text_b: b"Hans".to_vec(),
            ..Default::default()
        }]),
    );
    put(&mut frame, TableRows::CountryLinks(vec![CountryLinkRow { id: 1, canonical_slot: 1 }]));
    let theatre = Theatre {
        name: Recorded::Value(b"Front"),
        owner: Recorded::Value(3),
        commander: Recorded::Value(Some(5 << 32 | 9)),
        countries: Recorded::Value(&[4]),
        regions: &[1, 8],
    };
    let facts = facts_of(&theatre, frame);
    assert_eq!(facts.name, Ok("Front".to_owned()));
    assert_eq!(facts.commander, Ok(Some(Leader { name: "Hans".into(), category: 3, skill: Some(2) })));
    assert_eq!(facts.owner, Err("country link 3 has no CountryLinks row".to_owned()));
    assert_eq!(facts.countries, Err("country link 4 has no CountryLinks row".to_owned()));
    assert_eq!(facts.regions, Err("strategic region 8 is absent from the content".to_owned()));
    let absent = Theatre { commander: Recorded::Value(Some(6 << 32 | 9)), ..theatre.clone() };
    let mut frame = test_frame(0);
    put(&mut frame, TableRows::UnitLeaders(Vec::new()));
    assert_eq!(facts_of(&absent, frame).commander, Err(format!("UnitLeaders leader {} is absent", 6u64 << 32 | 9)));
    let none = Theatre { commander: Recorded::Value(None), ..theatre };
    assert_eq!(facts_of(&none, test_frame(0)).commander, Ok(None), "no commander reads no leaders");
}

#[test]
fn a_recorded_row_gives_its_name_owner_commander_and_countries() {
    let row = FactionTheatreRow {
        faction: 7,
        ordinal: 0,
        regions: vec![1],
        name: Recorded::Value(b"Front".to_vec()),
        owner: Recorded::Value(3),
        commander: Recorded::Value(Some(5 << 32 | 9)),
        countries: Recorded::Value(vec![4, 3]),
        ..Default::default()
    };
    let theatre = Theatre::of(&row);
    assert_eq!(theatre.name, Recorded::Value(&b"Front"[..]));
    assert_eq!(theatre.owner, Recorded::Value(3));
    assert_eq!(theatre.commander, Recorded::Value(Some(5 << 32 | 9)));
    assert_eq!(theatre.countries, Recorded::Value(&[4, 3][..]));
    assert_eq!(theatre.regions, &[1]);
}
