use super::{CountryLook, IdeaCountry, IdeaName, PoliticsContent};
use std::sync::Arc;

#[test]
fn idea_case_aliases_use_authored_name_description_and_picture_keys() {
    let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/mod.lua",
            "NDefines = { NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = { \"mod_seat\" } } }",
        ),
        (
            "common/ideas/mod.txt",
            "ideas = { country = { mod_Spirit = { } } }",
        ),
        (
            "localisation/test_l_english.yml",
            "\u{feff}l_english:\n mod_Spirit:0 \"Authored name\"\n mod_Spirit_desc:0 \"Authored description\"\n",
        ),
    ]);
    let content = PoliticsContent::load(Arc::new(catalogue)).unwrap();
    let country = IdeaCountry {
        tag: "MOD",
        slot: 0,
        look: Some(CountryLook {
            name_tag: "",
            original_or_own_tag: Ok("MOD"),
            cosmetic_tag: "",
            civil_war: Ok(false),
        }),
    };
    let (artwork, _) = content.idea_artwork("MOD_SPIRIT", None, country);
    assert_eq!(artwork.name.as_deref(), Some("Authored name"));
    assert_eq!(artwork.description.as_deref(), Some("Authored description"));
    assert_eq!(
        content.idea_name_offline("MOD_SPIRIT").unwrap(),
        IdeaName::Key("mod_Spirit".into())
    );
    assert_eq!(
        content.idea("MOD_SPIRIT").unwrap().unwrap().sprite(),
        "GFX_idea_mod_Spirit"
    );
    assert!(!content.defines("MOD_SPIRIT"));
}

#[test]
fn unknown_gui_decode_reaches_idea_and_portrait_problems() {
    let (mut catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/mod.lua",
            "NDefines = { NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = { \"mod_seat\" } } }",
        ),
        (
            "common/ideas/mod.txt",
            "ideas = { country = { mod_spirit = { picture = mod_picture } } }",
        ),
        (
            "interface/mod.gfx",
            "spriteTypes = { spriteType = { name = GFX_idea_mod_picture textureFile = gfx/mod/icon.png } }",
        ),
    ]);
    let reason = "settings.txt is unavailable: test read failure";
    catalogue.gui_decode = Err(reason.into());
    let content = PoliticsContent::load(Arc::new(catalogue)).unwrap();
    let country = IdeaCountry {
        tag: "MOD tag",
        slot: 0,
        look: None,
    };
    let (artwork, problem) = content.idea_artwork("mod_spirit", None, country);
    assert!(artwork.icon.is_none());
    assert_eq!(
        problem.as_deref(),
        Some(format!("mod_spirit: icon unavailable: {reason}").as_str())
    );
    for (kind, name) in [(0, "GFX_idea_mod_picture"), (1, "gfx/mod/icon.png")] {
        let why = content
            .recorded_portrait(kind, name.as_bytes())
            .unwrap_err();
        assert!(why.contains(reason), "{why}");
    }
    let why = content
        .sprite_png("gfx/mod/direct.png")
        .unwrap_err()
        .to_string();
    assert!(why.contains(reason), "{why}");
}

#[test]
fn an_idea_is_named_for_its_country() {
    let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n",
                " MOD_spirit:0 \"Token Name\"\n",
                " GER_MOD_spirit:0 \"German Name\"\n",
                " POLITICS_ADVISOR_NAME_PATTERN:0 \"$NAME$ :: $SURNAME$\"\n",
            ),
        ),
        (
            "common/defines/00_defines.lua",
            "NDefines = { NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = { \"theorist\" } } }",
        ),
        (
            "common/idea_tags/00.txt",
            "idea_categories = { country = { type = national_spirit } government = { slot = political_advisor } }",
        ),
        (
            "common/ideas/mod.txt",
            "ideas = { country = { MOD_spirit = { } } political_advisor = { MOD_nameless = { level = 2 } \
             MOD_empty = { name = \"\" } MOD_cut = { name = \"\u{0}rest\" } } }",
        ),
        (
            "common/names/00_names.txt",
            "default = { male = { names = { John } } } \
             GER = { male = { names = { Karl } surnames = { Weber } } } \
             GER_red = { male = { names = { Rosa } surnames = { Rot } } } \
             ITA = { male = { names = { A B C D E F G H } surnames = { a b c d e f g } } }",
        ),
    ]);
    let content = PoliticsContent::load(Arc::new(catalogue)).unwrap();
    let fixed = CountryLook {
        name_tag: "",
        original_or_own_tag: Ok("GER"),
        cosmetic_tag: "",
        civil_war: Ok(false),
    };
    let sov = CountryLook {
        original_or_own_tag: Ok("SOV"),
        ..fixed
    };
    let italian = CountryLook {
        original_or_own_tag: Ok("ITA"),
        ..fixed
    };
    let country = |tag, look| IdeaCountry { tag, slot: 3, look };
    let key = |key: &str| Ok(IdeaName::Key(key.to_owned()));
    assert_eq!(
        content.idea_name("MOD_spirit", country("GER", Some(fixed))),
        key("GER_MOD_spirit")
    );
    assert_eq!(
        content.idea_name("MOD_spirit", country("SOV", Some(sov))),
        key("MOD_spirit")
    );
    let dynamic = CountryLook {
        name_tag: "GER",
        original_or_own_tag: Ok("GER"),
        cosmetic_tag: "",
        civil_war: Err("hour 1 country civil wars is unavailable"),
    };
    let refused = content
        .idea_name("MOD_spirit", country("D01", Some(dynamic)))
        .unwrap_err();
    assert!(
        refused.contains("whether it is in civil war")
            && refused.ends_with("country civil wars is unavailable"),
        "{refused}"
    );
    assert!(
        content
            .idea_name("MOD_spirit", country("GER", None))
            .unwrap_err()
            .contains("name tag of country appearance")
    );
    assert_eq!(
        content.idea_name("MOD_nameless", country("GER", Some(fixed))),
        Ok(IdeaName::Generated("Karl :: Weber".to_owned()))
    );
    let ita = content.names.for_country("", "ITA");
    let (first, surname) = content.names.draw(ita, 7).unwrap();
    assert_eq!(
        content.idea_name("MOD_nameless", country("ITA", Some(italian))),
        Ok(IdeaName::Generated(format!(
            "{} :: {}",
            String::from_utf8_lossy(first),
            String::from_utf8_lossy(surname)
        )))
    );
    for token in ["MOD_empty", "MOD_cut"] {
        assert!(
            matches!(
                content.idea_name(token, country("GER", Some(fixed))),
                Ok(IdeaName::Generated(_))
            ),
            "{token}"
        );
    }
    let cosmetic = CountryLook {
        name_tag: "",
        original_or_own_tag: Ok("GER"),
        cosmetic_tag: "GER_red",
        civil_war: Ok(false),
    };
    assert_eq!(
        content.idea_name("MOD_nameless", country("GER", Some(cosmetic))),
        Ok(IdeaName::Generated("Rosa :: Rot".to_owned()))
    );
    assert_eq!(
        content.idea_name("MOD_nameless", country("SOV", Some(sov))),
        Ok(IdeaName::Generated(String::new()))
    );
}

#[test]
fn generated_advisor_names_read_the_ungated_original_only_after_cosmetic_miss() {
    use crate::identity::NamingCountry;
    use replay_data::{rows::Recorded, rows::TableId, rows::TableRows, rows::tables};
    use replay_frame::frame::TableView;
    let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n",
                " GER_MOD_keyed:0 \"Own idea key\"\n",
                " POLITICS_ADVISOR_NAME_PATTERN:0 \"$NAME$ :: $SURNAME$\"\n",
            ),
        ),
        (
            "common/defines/mod.lua",
            "NDefines = { NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = { } } }",
        ),
        (
            "common/idea_tags/mod.txt",
            "idea_categories = { government = { slot = political_advisor } }",
        ),
        (
            "common/ideas/mod.txt",
            "ideas = { political_advisor = { MOD_generated = { } MOD_keyed = { } } }",
        ),
        (
            "common/names/mod.txt",
            "GER = { male = { names = { Own } surnames = { Name } } } \
             MOD_original = { male = { names = { Original } surnames = { Name } } } \
             MOD_cosmetic = { male = { names = { Cosmetic } surnames = { Name } } }",
        ),
    ]);
    let content = PoliticsContent::load(Arc::new(catalogue)).unwrap();
    let mut frame = crate::scope::tests::positive_frame();
    let name = |frame: &replay_frame::frame::DecodedFrame, token| {
        let row = &frame.complete::<tables::Countries>().unwrap()[0];
        let naming = NamingCountry::read(frame, row);
        content.idea_name(token, naming.for_tag("GER"))
    };
    assert_eq!(
        name(&frame, "MOD_generated"),
        Ok(IdeaName::Generated("Own :: Name".into()))
    );
    for nonpositive in [0, -1] {
        if let TableView::Complete(TableRows::CountryAppearance(rows)) =
            &mut *frame.tables[TableId::CountryAppearance.index()]
        {
            rows[0].original_tag_reference = Recorded::Value(Some(nonpositive));
            rows[0].original_tag = Recorded::NotRecorded;
        }
        assert_eq!(
            name(&frame, "MOD_generated"),
            Ok(IdeaName::Generated("Own :: Name".into()))
        );
    }
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *frame.tables[TableId::CountryAppearance.index()]
    {
        rows[0].original_tag_reference = Recorded::Value(Some(300));
        rows[0].original_tag = Recorded::Value(Some(b"MOD_original".to_vec()));
    }
    assert_eq!(
        name(&frame, "MOD_generated"),
        Ok(IdeaName::Generated("Original :: Name".into()))
    );
    for (reference, unread, reason) in [
        (
            Recorded::NotRecorded,
            Recorded::Value(Some(b"MOD_original".to_vec())),
            "original tag reference of country appearance not recorded",
        ),
        (
            Recorded::Value(None),
            Recorded::Value(Some(b"MOD_original".to_vec())),
            "original tag reference of country appearance capture refused",
        ),
        (
            Recorded::Value(Some(300)),
            Recorded::NotRecorded,
            "original tag of country appearance not recorded",
        ),
        (
            Recorded::Value(Some(300)),
            Recorded::Value(None),
            "original tag of country appearance capture refused",
        ),
        (
            Recorded::Value(Some(300)),
            Recorded::Value(Some(vec![0xFF])),
            "country original-or-own tag is not UTF-8",
        ),
    ] {
        if let TableView::Complete(TableRows::CountryAppearance(rows)) =
            &mut *frame.tables[TableId::CountryAppearance.index()]
        {
            rows[0].original_tag_reference = reference;
            rows[0].original_tag = unread;
            rows[0].cosmetic_tag.clear();
        }
        assert!(name(&frame, "MOD_generated").unwrap_err().contains(reason));
        assert_eq!(
            name(&frame, "MOD_keyed"),
            Ok(IdeaName::Key("GER_MOD_keyed".into()))
        );
        if let TableView::Complete(TableRows::CountryAppearance(rows)) =
            &mut *frame.tables[TableId::CountryAppearance.index()]
        {
            rows[0].cosmetic_tag = b"MOD_cosmetic".to_vec();
        }
        assert_eq!(
            name(&frame, "MOD_generated"),
            Ok(IdeaName::Generated("Cosmetic :: Name".into()))
        );
        if let TableView::Complete(TableRows::CountryAppearance(rows)) =
            &mut *frame.tables[TableId::CountryAppearance.index()]
        {
            rows[0].cosmetic_tag = b"MOD_missing".to_vec();
        }
        assert!(name(&frame, "MOD_generated").unwrap_err().contains(reason));
    }
}

#[test]
fn a_dynamic_country_in_civil_war_skips_its_keys() {
    use crate::identity::NamingCountry;
    use replay_data::{
        rows::CountryCivilWarRow, rows::CountryCounterInputRow, rows::CountryWarRow, rows::TableId,
        rows::TableRows, rows::WarRelationRow, rows::tables,
    };
    use replay_frame::{frame::DecodedFrame, frame::TableView};
    let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n",
                " MOD_spirit:0 \"Token Name\"\n",
                " SOV_MOD_spirit:0 \"Soviet Name\"\n",
                " POLITICS_ADVISOR_NAME_PATTERN:0 \"$NAME$ :: $SURNAME$\"\n",
            ),
        ),
        (
            "common/defines/00_defines.lua",
            "NDefines = { NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = { \"theorist\" } } }",
        ),
        (
            "common/idea_tags/00.txt",
            "idea_categories = { country = { type = national_spirit } government = { slot = political_advisor } }",
        ),
        (
            "common/ideas/mod.txt",
            "ideas = { country = { MOD_spirit = { } } political_advisor = { MOD_nameless = { } } }",
        ),
        (
            "common/names/00_names.txt",
            "GER = { male = { names = { Karl } surnames = { Weber } } }",
        ),
    ]);
    let content = PoliticsContent::load(Arc::new(catalogue)).unwrap();
    let frame = |civil: Vec<CountryCivilWarRow>,
                 wars: Vec<CountryWarRow>,
                 relations: Vec<WarRelationRow>| {
        let mut frame = crate::scope::tests::frame();
        if let TableView::Complete(TableRows::CountryAppearance(rows)) =
            &mut *frame.tables[TableId::CountryAppearance.index()]
        {
            rows[1].name_tag = b"GER".to_vec();
            rows[1].original_tag_reference = replay_data::rows::Recorded::Value(Some(100));
            rows[1].original_tag = replay_data::rows::Recorded::Value(Some(b"GER".to_vec()));
        }
        *frame.tables[TableId::CountryCivilWars.index()] =
            TableView::Complete(TableRows::CountryCivilWars(civil));
        *frame.tables[TableId::CountryWars.index()] =
            TableView::Complete(TableRows::CountryWars(wars));
        *frame.tables[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(relations));
        frame
    };
    let name = |frame: &DecodedFrame, token: &str| {
        let countries = frame.complete::<tables::Countries>().unwrap();
        let naming = NamingCountry::read(frame, &countries[1]);
        content.idea_name(token, naming.for_tag("SOV"))
    };
    let key = |key: &str| Ok(IdeaName::Key(key.to_owned()));
    let list = |country, target| CountryCivilWarRow {
        country,
        ordinal: 0,
        target,
    };
    let war = |end_hour| WarRelationRow {
        first_country: 100,
        second_country: 200,
        start_hour: 0,
        end_hour,
        ..Default::default()
    };
    let calm = frame(vec![], vec![], vec![]);
    assert_eq!(name(&calm, "MOD_spirit"), key("SOV_MOD_spirit"));
    let own = frame(vec![list(200, 100)], vec![], vec![]);
    assert_eq!(name(&own, "MOD_spirit"), key("MOD_spirit"));
    assert_eq!(
        name(&own, "MOD_nameless"),
        Ok(IdeaName::Generated("Karl :: Weber".to_owned()))
    );
    let mut cosmetic = own.clone();
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *cosmetic.tables[TableId::CountryAppearance.index()]
    {
        rows[1].cosmetic_tag = b"SOV_red".to_vec();
    }
    assert_eq!(name(&cosmetic, "MOD_nameless"), key("MOD_nameless"));
    let mut stateless = own;
    *stateless.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(vec![
            CountryCounterInputRow {
                country: 100,
                owned_state_count: 0,
                ..Default::default()
            },
            CountryCounterInputRow {
                country: 200,
                owned_state_count: 3,
                ..Default::default()
            },
        ]));
    assert_eq!(name(&stateless, "MOD_spirit"), key("SOV_MOD_spirit"));
    let enemy = frame(vec![list(100, 200)], vec![], vec![war(0)]);
    assert_eq!(name(&enemy, "MOD_spirit"), key("MOD_spirit"));
    let ended = frame(vec![list(100, 200)], vec![], vec![war(1)]);
    assert_eq!(name(&ended, "MOD_spirit"), key("SOV_MOD_spirit"));
    let read = frame(
        vec![list(100, 200)],
        vec![CountryWarRow {
            country: 200,
            can_occupy_non_war: 0,
            enemies_read: 1,
            enemies: vec![100],
            ordered: Default::default(),
        }],
        vec![war(1)],
    );
    assert_eq!(name(&read, "MOD_spirit"), key("MOD_spirit"));
    let mut unread = calm;
    *unread.tables[TableId::CountryCivilWars.index()] = TableView::Unavailable {
        reason: "not sampled".into(),
    };
    let refused = name(&unread, "MOD_spirit").unwrap_err();
    assert!(
        refused.contains("whether it is in civil war")
            && refused.contains("is unavailable: not sampled"),
        "{refused}"
    );
    let mut fixed = unread.clone();
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *fixed.tables[TableId::CountryAppearance.index()]
    {
        rows[1].name_tag.clear();
    }
    assert_eq!(name(&fixed, "MOD_spirit"), key("SOV_MOD_spirit"));
}

#[test]
fn a_spirit_without_localisation_says_which_key_is_missing() {
    let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "localisation/test_l_english.yml",
            "\u{feff}l_english:\n MOD_named:0 \"Named Spirit\"\n",
        ),
        (
            "common/defines/00_defines.lua",
            "NDefines = { NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = { \"theorist\" } } }",
        ),
        (
            "common/idea_tags/00.txt",
            "idea_categories = { country = { type = national_spirit } }",
        ),
        (
            "common/ideas/mod.txt",
            "ideas = { country = { MOD_named = { } MOD_bare = { } MOD_relabelled = { name = MOD_label } } }",
        ),
    ]);
    let content = PoliticsContent::load(Arc::new(catalogue)).unwrap();
    let country = IdeaCountry {
        tag: "GER",
        slot: 0,
        look: Some(CountryLook {
            name_tag: "",
            original_or_own_tag: Ok("GER"),
            cosmetic_tag: "",
            civil_war: Ok(false),
        }),
    };
    let (named, _) = content.idea_artwork("MOD_named", None, country);
    assert_eq!(named.name.as_deref(), Some("Named Spirit"));
    assert_eq!(content.idea_name_missing("MOD_named", None, country, None), None);
    for (token, key) in [("MOD_bare", "MOD_bare"), ("MOD_relabelled", "MOD_label")] {
        let (art, _) = content.idea_artwork(token, None, country);
        assert_eq!(art.name, None, "{token}");
        assert_eq!(
            content.idea_name_missing(token, None, country, None),
            Some(format!("localisation {key} is missing")),
            "{token}"
        );
    }
    let unplaced = IdeaCountry { look: None, ..country };
    assert!(
        content
            .idea_name_missing("MOD_bare", None, unplaced, None)
            .is_some_and(|why| why.contains("name tag of country appearance"))
    );
}
