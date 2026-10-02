use super::{CountryLook, IdeaCountry, IdeaName, PoliticsContent};
use crate::products::country_stability::{BreakdownLine, line_name};
use std::sync::Arc;

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
            .load_recorded_portrait(kind, name.as_bytes())
            .unwrap_err();
        assert!(why.contains(reason), "{why}");
    }
    let why = content
        .sprite_png("gfx/mod/direct.png")
        .unwrap_err()
        .to_string();
    assert!(why.contains(reason), "{why}");
}

/// 140FBCF90 localises the key it selects and nothing else; 140FC16E0
/// takes `token_desc` before the override's. Read from a content folder
/// through the Politics content loader.
#[test]
fn an_idea_name_is_its_selected_key_alone() {
    let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n",
                " MOD_spirit:0 \"Token Name\"\n",
                " MOD_spirit_renamed:0 \"Jos¤é\"\n",
                " MOD_spirit_desc:0 \"Token description\"\n",
                " MOD_spirit_renamed_desc:0 \"Override description\"\n",
                " MOD_other:0 \"Other Token Name\"\n",
                " MOD_other_override_desc:0 \"Override only\"\n",
                " MOD_plain:0 \"Plain Name\"\n",
            ),
        ),
        (
            "common/defines/00_defines.lua",
            "NDefines = { NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = { \"theorist\" } } }",
        ),
        (
            "common/ideas/mod.txt",
            "ideas = { country = { MOD_spirit = { name = MOD_spirit_renamed } MOD_other = { name = MOD_other_override } MOD_plain = { } } }",
        ),
    ]);
    let content = PoliticsContent::load(Arc::new(catalogue)).unwrap();
    let look = CountryLook {
        name_tag: "",
        cosmetic_tag: "",
        civil_war: Ok(false),
    };
    let country = IdeaCountry {
        tag: "MOD",
        slot: 0,
        look: Some(look),
    };
    // A refused override is not replaced by the token's text.
    let (spirit, _) = content.idea_artwork("MOD_spirit", None, country);
    assert_eq!(spirit.name, None);
    assert_eq!(spirit.description.as_deref(), Some("Token description"));
    assert!(
        content
            .catalogue
            .refusal_lines()
            .iter()
            .any(|line| line.contains("MOD_spirit_renamed")),
        "{:?}",
        content.catalogue.refusal_lines()
    );
    // An override content lacks is its own bytes (142230940); the token's
    // text does not stand in.
    let (other, _) = content.idea_artwork("MOD_other", None, country);
    assert_eq!(other.name.as_deref(), Some("MOD_other_override"));
    assert_eq!(other.description.as_deref(), Some("Override only"));
    let (plain, _) = content.idea_artwork("MOD_plain", None, country);
    assert_eq!(
        (plain.name.as_deref(), plain.description),
        (Some("Plain Name"), None)
    );
    // The stability breakdown selects the same key.
    let line = |identity: &str| BreakdownLine {
        line: 0,
        kind: "idea".into(),
        identity: identity.into(),
        identity_id: 0,
        secondary_raw: String::new(),
        value_raw: String::new(),
        text: None,
        label: None,
        group: None,
        group_kind: None,
    };
    assert_eq!(
        line_name(&line("MOD_spirit"), &content, None, None, country),
        None
    );
    assert_eq!(
        line_name(&line("MOD_other"), &content, None, None, country).as_deref(),
        Some("MOD_other_override")
    );
    assert_eq!(
        line_name(&line("MOD_plain"), &content, None, None, country).as_deref(),
        Some("Plain Name")
    );
}

/// 140FBCF90's country steps: `TAG_token` before the token for a
/// country that is not dynamic; a dynamic country, or one the hour does
/// not place, is refused by name, as is the generated advisor name
/// (140FBBEE0) of an idea without keys in a category of type 0.
#[test]
fn an_idea_is_named_for_its_country() {
    let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n",
                " MOD_spirit:0 \"Token Name\"\n",
                " GER_MOD_spirit:0 \"German Name\"\n",
                " POLITICS_ADVISOR_NAME_PATTERN:0 \"$NAME$ $SURNAME$\"\n",
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
        cosmetic_tag: "",
        civil_war: Ok(false),
    };
    let country = |tag, look| IdeaCountry { tag, slot: 3, look };
    let key = |key: &str| Ok(IdeaName::Key(key.to_owned()));
    assert_eq!(
        content.idea_name("MOD_spirit", country("GER", Some(fixed))),
        key("GER_MOD_spirit")
    );
    assert_eq!(
        content.idea_name("MOD_spirit", country("SOV", Some(fixed))),
        key("MOD_spirit")
    );
    // A dynamic country whose civil-war inputs the hour lacks.
    let dynamic = CountryLook {
        name_tag: "GER",
        cosmetic_tag: "",
        civil_war: Err("hour 1 CountryCivilWars is unavailable"),
    };
    let refused = content
        .idea_name("MOD_spirit", country("D01", Some(dynamic)))
        .unwrap_err();
    assert!(
        refused.contains("140D2BD20") && refused.ends_with("CountryCivilWars is unavailable"),
        "{refused}"
    );
    assert!(
        content
            .idea_name("MOD_spirit", country("GER", None))
            .unwrap_err()
            .contains("CountryAppearance.name_tag")
    );
    // No key and a category of type 0: the name 140FBBEE0 generates
    // from GER's names: one name and one surname, so the draw is fixed.
    assert_eq!(
        content.idea_name("MOD_nameless", country("GER", Some(fixed))),
        Ok(IdeaName::Generated("Karl Weber".to_owned()))
    );
    // The seed is +0xAB8 (its database index, 2, from 140A31D80), the
    // slot (3) and the level (2).
    let nameless = content.idea("MOD_nameless").unwrap().unwrap();
    assert_eq!(nameless.position, 2);
    let ita = content.names.for_country("", "ITA");
    let (first, surname) = content.names.draw(ita, 7).unwrap();
    assert_eq!(
        content.idea_name("MOD_nameless", country("ITA", Some(fixed))),
        Ok(IdeaName::Generated(format!(
            "{} {}",
            String::from_utf8_lossy(first),
            String::from_utf8_lossy(surname)
        )))
    );
    // An empty override, or one ReadStringInto cuts at a NUL to empty,
    // goes on down 140FBCF90's ladder (+0xC70 is its length).
    for token in ["MOD_empty", "MOD_cut"] {
        assert!(
            matches!(
                content.idea_name(token, country("GER", Some(fixed))),
                Ok(IdeaName::Generated(_))
            ),
            "{token}"
        );
    }
    // A cosmetic tag the names list is used first.
    let cosmetic = CountryLook {
        name_tag: "",
        cosmetic_tag: "GER_red",
        civil_war: Ok(false),
    };
    assert_eq!(
        content.idea_name("MOD_nameless", country("GER", Some(cosmetic))),
        Ok(IdeaName::Generated("Rosa Rot".to_owned()))
    );
    // A country without names or surnames draws from the default, which
    // has none: empty text.
    assert_eq!(
        content.idea_name("MOD_nameless", country("SOV", Some(fixed))),
        Ok(IdeaName::Generated(String::new()))
    );
}

/// 140FBCF90 for a dynamic country (SOV, named after GER) read from a
/// recorded hour: 140D2BD20 on its CCountry+0xF88 holds when its own
/// civil-war list (+0x12F0, CountryCivilWars) names a country owning a
/// state (+0x484), or an enemy (+0x98: CountryWars where read, else
/// the ongoing WarRelations) owning a state lists it. Then neither
/// `TAG_token` nor the token is asked, and an advisor name is generated
/// only while the cosmetic tag (+0x1488, length +0x1498) is empty.
#[test]
fn a_dynamic_country_in_civil_war_skips_its_keys() {
    use crate::products::identity::NamingCountry;
    use replay::{
        frame::{DecodedFrame, TableView},
        rows::{
            CountryCivilWarRow, CountryCounterInputRow, CountryWarRow, TableId, TableRows,
            WarRelationRow, tables,
        },
    };
    let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n",
                " MOD_spirit:0 \"Token Name\"\n",
                " SOV_MOD_spirit:0 \"Soviet Name\"\n",
                " POLITICS_ADVISOR_NAME_PATTERN:0 \"$NAME$ $SURNAME$\"\n",
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
    // GER (link 100, slot 0) and SOV (link 200, slot 1) each own a
    // state; SOV's original tag is GER.
    let frame = |civil: Vec<CountryCivilWarRow>,
                 wars: Vec<CountryWarRow>,
                 relations: Vec<WarRelationRow>| {
        let mut frame = crate::scope::tests::frame();
        if let TableView::Complete(TableRows::CountryAppearance(rows)) =
            &mut *frame.tables[TableId::CountryAppearance.index()]
        {
            rows[1].name_tag = b"GER".to_vec();
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
    // No civil war: the country's key first.
    let calm = frame(vec![], vec![], vec![]);
    assert_eq!(name(&calm, "MOD_spirit"), key("SOV_MOD_spirit"));
    // Its own list names GER, which owns a state.
    let own = frame(vec![list(200, 100)], vec![], vec![]);
    assert_eq!(name(&own, "MOD_spirit"), key("MOD_spirit"));
    assert_eq!(
        name(&own, "MOD_nameless"),
        Ok(IdeaName::Generated("Karl Weber".to_owned()))
    );
    let mut cosmetic = own.clone();
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *cosmetic.tables[TableId::CountryAppearance.index()]
    {
        rows[1].cosmetic_tag = b"SOV_red".to_vec();
    }
    assert_eq!(name(&cosmetic, "MOD_nameless"), key("MOD_nameless"));
    // A listed country that owns no state does not count.
    let mut stateless = own.clone();
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
    // An enemy's list names SOV: the war is ongoing (the hour is 1),
    // not once it ended; a read enemy vector stands without the war.
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
        }],
        vec![war(1)],
    );
    assert_eq!(name(&read, "MOD_spirit"), key("MOD_spirit"));
    // An input the hour lacks is refused by name.
    let mut unread = calm.clone();
    *unread.tables[TableId::CountryCivilWars.index()] = TableView::Unavailable {
        reason: "not sampled".into(),
    };
    let refused = name(&unread, "MOD_spirit").unwrap_err();
    assert!(
        refused.contains("140D2BD20") && refused.contains("CountryCivilWars is unavailable"),
        "{refused}"
    );
    // A country without an original tag never asks.
    let mut fixed = unread.clone();
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *fixed.tables[TableId::CountryAppearance.index()]
    {
        rows[1].name_tag.clear();
    }
    assert_eq!(name(&fixed, "MOD_spirit"), key("SOV_MOD_spirit"));
}
