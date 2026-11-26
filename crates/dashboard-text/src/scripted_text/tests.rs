use super::*;
use crate::script_test_game as game;
pub(super) use super::complete_test_frame as complete_frame;
use crate::scope::tests::positive_frame as frame;
use replay_data::{
    rows::FactionMemberRow, rows::PoliticalPartyRow, rows::TableId, rows::TableRows,
};

#[test]
fn faction_goal_text_keeps_the_explicit_faction_and_selected_country_environment() {
    game::enter();
    let (catalogue, _folder) = catalogue(SCRIPTED, "");
    let mut frame = frame();
    *frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![FactionMemberRow {
            faction: 5,
            country: 100,
            ordinal: 0,
        }]));
    let text = ScriptedText::new(
        &frame,
        &catalogue.localisation,
        catalogue.scripted_content().unwrap(),
        2,
    );
    assert_eq!(
        text.expand_faction("[Faction.Leader.GetTag] / [ROOT.GetTag]", 5)
            .as_deref(),
        Ok("GER / SOV")
    );
}

#[test]
fn reserved_zero_naming_is_separate_from_positive_country_links() {
    game::enter();
    let (catalogue, _folder) = catalogue(SCRIPTED, "");
    let content = catalogue.scripted_content().unwrap();
    let mut frame = frame();
    let text = ScriptedText::new(&frame, &catalogue.localisation, content, 1);
    assert_eq!(text.country_tag(1), Ok("GER"));
    assert!(
        text.country_tag(0)
            .unwrap_err()
            .contains("reserved country naming")
    );
    drop(text);
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            replay_data::rows::ReservedCountryNamingRow {
                tag: b"ZZ0".to_vec(),
                ..Default::default()
            },
        ]));
    let text = ScriptedText::new(&frame, &catalogue.localisation, content, 1);
    assert_eq!(text.country_tag(0), Ok("ZZ0"));
    assert_eq!(text.country_tag(1), Ok("GER"));
    drop(text);
    *frame.tables[TableId::ReservedCountryNaming.index()] = TableView::Unavailable {
        reason: "reserved getter capture failed".into(),
    };
    let text = ScriptedText::new(&frame, &catalogue.localisation, content, 1);
    assert!(
        text.country_tag(0)
            .unwrap_err()
            .contains("reserved getter capture failed")
    );
    assert_eq!(text.country_tag(1), Ok("GER"));
}

#[path = "temp_tests.rs"]
mod temp_variables;

#[path = "faction_zero_tests.rs"]
mod faction_zero;

const LOCALISATION: &str = concat!(
    "\u{feff}l_english:\n",
    " GER:0 \"German Reich\"\n",
    " GER_DEF:0 \"the German Reich\"\n",
    " GER_ADJ:0 \"german\"\n",
    " SOV:0 \"Soviet Union\"\n",
    " SOV_DEF:0 \"the Soviet Union\"\n",
    " SOV_ADJ:0 \"Soviet\"\n",
    " fascism:0 \"Fascism\"\n",
    " fascism_noun:0 \"Fascist\"\n",
    " GER_fascism_party:0 \"Fixture Party\"\n",
    " GER_fascism_party_long:0 \"National Socialist Party\"\n",
    " MOD_idea:0 \"[ROOT.GetAdjectiveCap] spirit of [Root.Overlord.GetNameDef]\"\n",
);

pub(super) fn catalogue(
    scripted: &str,
    extra: &str,
) -> (crate::catalogue::Catalogue, crate::catalogue::tests::Folder) {
    catalogue_with(scripted, extra, &[])
}

pub(super) fn catalogue_with(
    scripted: &str,
    extra: &str,
    files: &[(&str, &str)],
) -> (crate::catalogue::Catalogue, crate::catalogue::tests::Folder) {
    let extra = format!("\u{feff}l_english:\n{extra}");
    let mut all = vec![
        ("localisation/mod_l_english.yml", LOCALISATION),
        ("localisation/replace/extra_l_english.yml", extra.as_str()),
        (
            "common/defines/00_defines.lua",
            "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 } }",
        ),
        ("common/scripted_localisation/mod.txt", scripted),
    ];
    all.extend_from_slice(files);
    crate::catalogue::tests::folder_catalogue(&all)
}

const SCRIPTED: &str = "defined_text = { name = GetModText text = { localization_key = GER } }";

#[test]
fn scripted_map_state_hovers_use_controller_continent_occupied_country_and_cached_vp() {
    game::enter();
    use replay_data::rows::{LexicalTokenRow, ProvinceRow, StateResistanceRow, StateRow};
    let (_, folder) = catalogue_with(
        "",
        concat!(
            " mod_continent:0 \"Generic continent\"\n",
            " GER_mod_continent:0 \"Controller continent\"\n",
            " VICTORY_POINTS_2:0 \"First port\"\n",
            " VICTORY_POINTS_1:0 \"Cached city\"\n",
        ),
        &[
            (
                "map/default.map",
                "definitions = definition.csv provinces = provinces.bmp terrain = terrain.bmp rivers = rivers.bmp heightmap = heightmap.bmp tree_definition = trees.bmp",
            ),
            (
                "map/definition.csv",
                "0;0;0;0;land;false;mod_land;1\n1;1;0;0;land;false;mod_land;1\n2;2;0;0;land;false;mod_land;1\n",
            ),
            (
                "common/defines/map.lua",
                "NDefines.NBuildings = { MAX_BUILDING_LEVELS = 15 }",
            ),
            (
                "history/states/mod.txt",
                "state = { id = 64 name = MOD_STATE provinces = { 2 1 } }",
            ),
        ],
    );
    let mut bmp = vec![0_u8; 58];
    bmp[..2].copy_from_slice(b"BM");
    for (at, value) in [(2, 58_u32), (10, 54), (14, 40), (18, 1), (22, 1)] {
        bmp[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    bmp[26..28].copy_from_slice(&1_u16.to_le_bytes());
    bmp[28..30].copy_from_slice(&24_u16.to_le_bytes());
    bmp[54..57].copy_from_slice(&[0, 0, 1]);
    std::fs::write(folder.0.join("map/provinces.bmp"), bmp).unwrap();
    let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&folder.0, &[][..])]).unwrap();
    let cat = crate::catalogue::tests::layers_catalogue(layers);
    let content = cat.scripted_content().unwrap();
    let mut f = complete_frame();
    *f.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![StateRow {
        id: 64,
        owner: 200,
        controller: 100,
        continent: Some(998).into(),
        best_vp_index: Some(1).into(),
        ..Default::default()
    }]));
    *f.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(vec![
        ProvinceRow {
            id: 1,
            state: 64,
            controller: 100,
            victory_points: 0,
            ..Default::default()
        },
        ProvinceRow {
            id: 2,
            state: 64,
            controller: 100,
            victory_points: 50,
            ..Default::default()
        },
    ]));
    *f.tables[TableId::StateResistance.index()] =
        TableView::Complete(TableRows::StateResistance(vec![StateResistanceRow {
            state: 64,
            country: 200,
            ..Default::default()
        }]));
    f.tables[TableId::StateResistance.index()] = f.tables[TableId::StateResistance.index()]
        .clone()
        .with_sampled_at(Some(f.hour));
    *f.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 998,
            literal: b"mod_continent".to_vec(),
        }]));
    let scopes = EventScopes::new(vec![EventScope::of(EventObject::State(64))]);
    let text = ScriptedText::new(&f, &cat.localisation, content, 1);
    assert_eq!(
        text.expand_in("[Root.GetContinent]", &scopes),
        Ok("Controller continent".into())
    );
    assert_eq!(
        text.expand_in("[Root.GetCapitalVictoryPointName]", &scopes),
        Ok("Cached city".into())
    );
    assert_eq!(
        text.expand_in("[Root.Occupied.GetTag]", &scopes),
        Ok("SOV".into())
    );
    drop(text);
    if let TableView::Complete(TableRows::States(rows)) = &mut *f.tables[TableId::States.index()] {
        rows[0].best_vp_index = Some(-1).into();
    }
    assert_eq!(
        ScriptedText::new(&f, &cat.localisation, content, 1)
            .expand_in("[Root.GetCapitalVictoryPointName]", &scopes),
        Ok("First port".into())
    );
    f.hour += 1;
    assert!(
        ScriptedText::new(&f, &cat.localisation, content, 1)
            .expand_in("[Root.Occupied.GetTag]", &scopes)
            .unwrap_err()
            .contains("not read this hour, so the values carried from an earlier hour cannot answer this exactly")
    );
}

fn with_party(mut frame: DecodedFrame) -> DecodedFrame {
    *frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
            country: 100,
            id: 3,
            token: b"fascism".to_vec(),
            tag: 100,
            name_default: 1,
            ..Default::default()
        }]));
    *frame.tables[TableId::Autonomy.index()] = TableView::Complete(TableRows::Autonomy(Vec::new()));
    frame
}

#[test]
fn a_long_party_name_falling_back_to_the_group_token_is_the_token() {
    game::enter();
    let extra = concat!(
        " communism:0 \"Communism\"\n",
        " communism_long:0 \"Not probed\"\n",
    );
    let (catalogue, _folder) = catalogue(SCRIPTED, extra);
    let mut frame = with_party(frame());
    if let TableView::Complete(TableRows::PoliticalParties(rows)) =
        &mut *frame.tables[TableId::PoliticalParties.index()]
    {
        rows[0].token = b"communism".to_vec();
    }
    if let TableView::Complete(TableRows::Politics(rows)) =
        &mut *frame.tables[TableId::Politics.index()]
    {
        rows[0].ruling_party = Some(3);
    }
    let content = catalogue.scripted_content().unwrap();
    let ger = ScriptedText::new(&frame, &catalogue.localisation, content, 1);
    assert_eq!(
        ger.expand("[ROOT.GetRulingPartyLong]").as_deref(),
        Ok("Communism")
    );
}

#[test]
fn party_original_fallback_uses_the_party_tag_and_keeps_earlier_hits() {
    game::enter();
    use replay_data::rows::Recorded;
    let (catalogue, _folder) = catalogue(
        SCRIPTED,
        concat!(
            " mod_group:0 \"Group fallback\"\n",
            " MOD_original_mod_group_party:0 \"Party-tag original\"\n",
            " MOD_original_mod_group_party_long:0 \"Long party-tag original\"\n",
            " MOD_wrong_mod_group_party:0 \"Wrong roster owner\"\n",
            " _mod_group_party:0 \"Empty original prefix\"\n",
            " _mod_group_party_long:0 \"Long empty original prefix\"\n",
        ),
    );
    let content = catalogue.scripted_content().unwrap();
    let mut frame = frame();
    let mut party = PoliticalPartyRow {
        country: 100,
        tag: 200,
        token: b"mod_group".to_vec(),
        name_default: 1,
        ..Default::default()
    };
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *frame.tables[TableId::CountryAppearance.index()]
    {
        rows[0].original_tag_reference = Recorded::Value(Some(300));
        rows[0].original_tag = Recorded::Value(Some(b"MOD_wrong".to_vec()));
        rows[1].original_tag_reference = Recorded::Value(Some(400));
        rows[1].original_tag = Recorded::Value(Some(b"MOD_original".to_vec()));
    }
    let name = |frame: &DecodedFrame, party: &PoliticalPartyRow, long| {
        ScriptedText::new(frame, &catalogue.localisation, content, 1).party_name(party, "GER", long)
    };
    assert_eq!(name(&frame, &party, false).unwrap(), "Party-tag original");
    assert_eq!(
        name(&frame, &party, true).unwrap(),
        "Long party-tag original"
    );
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *frame.tables[TableId::CountryAppearance.index()]
    {
        rows[1].original_tag = Recorded::Value(Some(Vec::new()));
    }
    assert_eq!(
        name(&frame, &party, false).unwrap(),
        "Empty original prefix"
    );
    assert_eq!(
        name(&frame, &party, true).unwrap(),
        "Long empty original prefix"
    );
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *frame.tables[TableId::CountryAppearance.index()]
    {
        rows[1].original_tag_reference = Recorded::Value(Some(0));
        rows[1].original_tag = Recorded::Value(Some(Vec::new()));
    }
    assert_eq!(name(&frame, &party, true).unwrap(), "Group fallback");
    for nonpositive in [0, -1] {
        if let TableView::Complete(TableRows::CountryAppearance(rows)) =
            &mut *frame.tables[TableId::CountryAppearance.index()]
        {
            rows[1].original_tag_reference = Recorded::Value(Some(nonpositive));
            rows[1].original_tag = Recorded::NotRecorded;
        }
        assert_eq!(name(&frame, &party, true).unwrap(), "Group fallback");
    }
    let appearance = frame
        .complete::<tables::CountryAppearance>()
        .unwrap()
        .to_vec();
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
            Recorded::Value(Some(400)),
            Recorded::NotRecorded,
            "original tag of country appearance not recorded",
        ),
        (
            Recorded::Value(Some(400)),
            Recorded::Value(None),
            "original tag of country appearance capture refused",
        ),
        (
            Recorded::Value(Some(400)),
            Recorded::Value(Some(vec![0xFF])),
            "country original-or-own tag is not UTF-8",
        ),
    ] {
        let mut rows = appearance.clone();
        rows[1].original_tag_reference = reference;
        rows[1].original_tag = unread;
        *frame.tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(rows));
        assert!(name(&frame, &party, false).unwrap_err().contains(reason));
        let own_key = PoliticalPartyRow {
            tag: 100,
            token: b"fascism".to_vec(),
            ..party.clone()
        };
        *frame.tables[TableId::CountryAppearance.index()] = TableView::Unavailable {
            reason: "not sampled".into(),
        };
        assert_eq!(name(&frame, &own_key, false).unwrap(), "Fixture Party");
        let stored = PoliticalPartyRow {
            tag: 999,
            name_default: 0,
            name: b"mod_group".to_vec(),
            ..party.clone()
        };
        assert_eq!(name(&frame, &stored, false).unwrap(), "Group fallback");
    }
    party.tag = 999;
    assert!(
        name(&frame, &party, false)
            .unwrap_err()
            .contains("no recorded country link")
    );
    party.tag = 0;
    assert!(
        name(&frame, &party, false)
            .unwrap_err()
            .contains("reserved country naming")
    );
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            replay_data::rows::ReservedCountryNamingRow {
                tag: b"ZZ0".to_vec(),
                original_tag_reference: Recorded::Value(Some(400)),
                original_tag: Recorded::Value(Some(b"MOD_original".to_vec())),
                ..Default::default()
            },
        ]));
    assert_eq!(name(&frame, &party, false).unwrap(), "Party-tag original");
}

#[test]
fn brackets_evaluate_for_the_country_the_text_is_shown_for() {
    game::enter();
    let (catalogue, _folder) = catalogue(SCRIPTED, "");
    let localisation = &*catalogue.localisation;
    let content = catalogue.scripted_content().unwrap();
    let mut frame = with_party(frame());
    if let TableView::Complete(TableRows::Politics(rows)) =
        &mut *frame.tables[TableId::Politics.index()]
    {
        rows[0].ruling_party = Some(3);
        rows.push(replay_data::rows::PoliticsRow {
            country: 200,
            ..Default::default()
        });
    }
    let sov = ScriptedText::new(&frame, localisation, content, 2);
    let ger = ScriptedText::new(&frame, localisation, content, 1);
    let expand = |text: &str, at: &ScriptedText| at.expand(text);
    assert_eq!(
        crate::catalogue::localised_in(localisation, "MOD_idea", &[], &sov).as_deref(),
        Ok("Soviet spirit of the German Reich")
    );
    assert_eq!(expand("[root.gettag]", &sov).as_deref(), Ok("SOV"));
    assert_eq!(expand("[GER.GetName]", &sov).as_deref(), Ok("German Reich"));
    assert_eq!(
        expand("[GER.GetAdjectiveCap]", &sov).as_deref(),
        Ok("German")
    );
    assert_eq!(expand("[ROOT.Overlord.GetTag]", &ger).as_deref(), Ok("GER"));
    assert_eq!(expand("[ROOT.Capital.GetID]", &ger).as_deref(), Ok("64"));
    assert_eq!(
        expand("[ROOT.Capital.Controller.GetTag]", &ger).as_deref(),
        Ok("SOV")
    );
    assert_eq!(
        expand("[ROOT.GetRulingIdeology]", &ger).as_deref(),
        Ok("fascism")
    );
    assert_eq!(
        expand("[ROOT.GetRulingIdeologyNoun]", &ger).as_deref(),
        Ok("fascist")
    );
    assert_eq!(
        expand("[ROOT.GetRulingParty]", &ger).as_deref(),
        Ok("Fixture Party")
    );
    assert_eq!(
        expand("[ROOT.GetRulingPartyLong]", &ger).as_deref(),
        Ok("National Socialist Party")
    );
    assert_eq!(expand("[ROOT.GetFlag]", &ger).as_deref(), Ok("\u{12}GER"));
    assert_eq!(
        expand("a[ROOT.Nowhere.GetTag]b", &ger).as_deref(),
        Ok("aGERb")
    );
    assert_eq!(expand("a[ROOT.GetNothing]b", &ger).as_deref(), Ok("ab"));
    assert_eq!(expand("[GetName]", &ger).as_deref(), Ok(""));
    assert_eq!(expand("x[ROOT.GetTag] y [z", &ger).as_deref(), Ok("xGER"));
    for (text, reason) in [
        ("[Player.GetName]", "local player"),
        ("[(ROOT.GetName?a:b)]", "the conditional scripted text"),
        (
            "[ROOT.GetOldName]",
            "GetOldName: the old name was unreadable",
        ),
        (
            "[ROOT.Capital.GetName]",
            "GetName: state 64 has no recorded-content template",
        ),
    ] {
        let error = expand(text, &ger).unwrap_err();
        assert!(error.contains(reason), "{text}: {error}");
    }
}

#[test]
fn names_only_expansion_reads_country_names_through_scope_links() {
    game::enter();
    let (catalogue, _folder) = catalogue(
        "defined_text = { name = GetName text = { localization_key = READS_VARIABLE } }",
        " READS_VARIABLE:0 \"[?amount]\"
",
    );
    let content = catalogue.scripted_content().unwrap();
    let mut frame = with_party(frame());
    if let TableView::Complete(TableRows::Politics(rows)) =
        &mut *frame.tables[TableId::Politics.index()]
    {
        rows[0].ruling_party = Some(3);
        rows.push(replay_data::rows::PoliticsRow {
            country: 200,
            ..Default::default()
        });
    }
    let text = ScriptedText::new(&frame, &catalogue.localisation, content, 1);
    let country = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
    let state = EventScopes::new(vec![EventScope::of(EventObject::State(64))]);
    assert_eq!(
        text.expand_names_in("[ROOT.GetNameDefCap] ([THIS.GetTag])", &country, &[])
            .as_deref(),
        Ok("The German Reich (GER)")
    );
    for (expression, scopes) in [
        ("[GetName]", &country),
        ("[ROOT.GetName]", &state),
        ("[?amount]", &country),
        ("[ROOT.Capital.GetName]", &country),
        ("[ROOT.GetRulingParty]", &country),
        ("[(ROOT.GetName?a:b)]", &country),
    ] {
        let error = text.expand_names_in(expression, scopes, &[]).unwrap_err();
        assert!(
            error.contains("reads more than an event scope's country name"),
            "{expression}: {error}"
        );
    }
    let linked = EventScopes::new(vec![
        EventScope {
            from: Link::Scope(1),
            ..EventScope::of(EventObject::Country(1))
        },
        EventScope {
            root: Link::Scope(0),
            ..EventScope::of(EventObject::Country(0))
        },
    ]);
    let names = [
        ScopeName::Hour,
        ScopeName::Unproven("it carries a character".to_owned()),
    ];
    assert_eq!(
        text.expand_names_in("[From.Root.GetTag]", &linked, &names)
            .as_deref(),
        Ok("GER")
    );
    let error = text
        .expand_names_in("[From.GetTag]", &linked, &names)
        .unwrap_err();
    assert!(
        error.contains("event scope 1: it carries a character"),
        "{error}"
    );
    let recorded = [ScopeName::Recorded([
        b"Old Reich".to_vec(),
        b"the Old Reich".to_vec(),
        b"Old German".to_vec(),
    ])];
    assert_eq!(
        text.expand_names_in(
            "[ROOT.GetName] / [ROOT.GetNameDefCap] / [THIS.GetAdjectiveCap] / [ROOT.GetNameWithFlag]",
            &country,
            &recorded
        )
        .as_deref(),
        Ok("Old Reich / The Old Reich / Old German / \u{12}GER The Old Reich")
    );
}

#[test]
fn numeric_brackets_use_the_country_the_text_is_shown_for() {
    game::enter();
    let (catalogue, _folder) = catalogue(SCRIPTED, "");
    let content = catalogue.scripted_content().unwrap();
    content.set_synthetic_environment(Default::default());
    let mut frame = frame();
    *frame.tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(Vec::new()));
    for id in [
        TableId::CountryCharacters,
        TableId::CountryRetiredCharacters,
    ] {
        *frame.tables[id.index()] = TableView::Complete(TableRows::empty(id));
    }
    *frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(
            [(100, 125_000), (200, 275_000)]
                .map(
                    |(scope_id, value_raw)| replay_data::rows::ScriptVariablesRow {
                        scope_kind: 1,
                        scope_id,
                        seed: replay_data::rows::Recorded::Value(None),
                        entries: vec![replay_data::rows::ScriptVariableEntry {
                            name: b"my_var".to_vec(),
                            value_raw,
                        }],
                    },
                )
                .into(),
        ));
    for (country, expected) in [(1, "1.2"), (2, "2.7")] {
        let text = ScriptedText::new(&frame, &catalogue.localisation, content, country);
        let scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(country))]);
        let evaluator = crate::triggers::Evaluator::new(text.world().unwrap(), None);
        assert_eq!(
            text.expand_in_session("[?my_var|1]", &scopes, 0, &evaluator),
            Ok(expected.into())
        );
        assert!(
            text.expand_in_session("[?ROOT.my_var.GetName]", &scopes, 0, &evaluator)
                .unwrap_err()
                .contains("property dispatch")
        );
    }
}

#[test]
fn a_property_no_read_file_defines_is_empty_only_when_every_file_is_known() {
    game::enter();
    let frame = frame();
    let (whole, _folder) = catalogue(SCRIPTED, "");
    let at = ScriptedText::new(
        &frame,
        &whole.localisation,
        whole.scripted_content().unwrap(),
        1,
    );
    assert_eq!(at.expand("a[ROOT.GetNothing]b").as_deref(), Ok("ab"));
    let (partial, _folder) = catalogue(
        &format!(
            "{SCRIPTED} x = {{ name = GetLater text = {{ random_list = {{ 1 = {{ localization_key = {{ key = K }} }} }} }} }}"
        ),
        " MOD_nothing:0 \"a[ROOT.GetNothing]b\"\n",
    );
    let content = partial.scripted_content().unwrap();
    let at = ScriptedText::new(&frame, &partial.localisation, content, 1);
    let error = at.expand("[ROOT.GetNothing]").unwrap_err();
    assert!(
        error.contains("GetNothing: scripted localisation definitions are uncertain")
            && error.contains("common/scripted_localisation/mod.txt is unknown from line 1")
            && error.contains("bound localisation reader is unsupported"),
        "{error}"
    );
    assert!(
        at.expand("[ROOT.GetModText]")
            .unwrap_err()
            .contains("scripted localisation definitions are uncertain")
    );
    assert_eq!(partial.localise_in("MOD_nothing", &at), None);
    let lines = partial.refusal_lines();
    assert!(
        lines.iter().any(|line| line.contains(&format!(
            "localisation MOD_nothing holds {error}; MOD_nothing"
        ))),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.starts_with(
            "Scripted localisation in common/scripted_localisation/mod.txt is unknown"
        )),
        "{lines:?}"
    );
}

#[test]
fn a_tag_alias_is_resolved_or_refused_by_name() {
    game::enter();
    let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
        ("localisation/mod_l_english.yml", LOCALISATION),
        (
            "common/defines/00_defines.lua",
            "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 } }",
        ),
        ("common/scripted_localisation/mod.txt", SCRIPTED),
        (
            "common/country_tag_aliases/mod.txt",
            "SPA = { original_tag = SPR OR = { has_country_flag = x } }                  ALI = { original_tag = GER PREV = { tag = GER } fallback = SOV }",
        ),
    ]);
    let localisation = &*catalogue.localisation;
    let content = catalogue.scripted_content().unwrap();
    assert!(
        content
            .environment()
            .unwrap()
            .inputs
            .set(Ok(entries::EnvironmentInputs::synthetic()
                .with_aliases(std::sync::Arc::clone(&content.tag_aliases))))
            .is_ok()
    );
    let mut whole = frame();
    for id in TableId::ALL {
        if matches!(&*whole.tables[id.index()], TableView::Unavailable { .. }) {
            *whole.tables[id.index()] = TableView::Complete(TableRows::empty(id));
        }
    }
    let mut unrecorded = whole.clone();
    *unrecorded.tables[TableId::LocalPlayer.index()] = TableView::Unavailable {
        reason: "not sampled".into(),
    };
    *whole.tables[TableId::LocalPlayer.index()] =
        TableView::Complete(TableRows::from_rows::<tables::LocalPlayer>(vec![
            replay_data::rows::LocalPlayerRow {
                country: 100,
                observer_country: 0,
            },
        ]));
    let text = ScriptedText::new(&whole, localisation, content, 2);
    let refused = text.expand("[SPA.GetName]").unwrap_err();
    assert!(refused.contains("tag alias SPA: "), "{refused}");
    assert_eq!(text.expand("[GER.GetTag]").as_deref(), Ok("GER"));
    assert_eq!(text.expand("[ALI.GetTag]").as_deref(), Ok("GER"));
    let text = ScriptedText::new(&unrecorded, localisation, content, 2);
    let refused = text.expand("[ALI.GetTag]").unwrap_err();
    assert!(refused.contains("the local player's country"), "{refused}");
}

#[test]
fn promotions_tell_absence_from_a_recording_gap() {
    game::enter();
    let (catalogue, _folder) = catalogue(SCRIPTED, "");
    let localisation = &*catalogue.localisation;
    let content = catalogue.scripted_content().unwrap();
    let expand = |frame: &DecodedFrame, root: u32, text: &str| {
        ScriptedText::new(frame, localisation, content, root).expand(text)
    };
    let whole = frame();
    assert_eq!(
        expand(&whole, 1, "[ROOT.GER_erwin_rommel.GetName]").as_deref(),
        Ok("")
    );
    assert_eq!(
        expand(&whole, 1, "[ROOT.Nobody.GetTag]").as_deref(),
        Ok("GER")
    );
    assert_eq!(
        expand(&whole, 1, "[ROOT.99.GetName]").as_deref(),
        Ok("NULL")
    );
    assert_eq!(expand(&whole, 1, "[ROOT.0.GetTag]").as_deref(), Ok("GER"));
    let refused = |frame: &DecodedFrame, root: u32, text: &str, reason: &str| {
        let error = expand(frame, root, text).unwrap_err();
        assert!(error.contains(reason), "{text}: {error}");
    };
    let mut truncated = frame();
    *truncated.tables[TableId::Characters.index()] = TableView::Truncated {
        rows: TableRows::Characters(Vec::new()),
        missing: 3,
    };
    refused(
        &truncated,
        1,
        "[ROOT.GER_erwin_rommel.GetName]",
        "GER_erwin_rommel: whether it names a character of country slot 1 is unknown: characters: the table is truncated (3 rows missing)",
    );
    let mut unspelled = frame();
    *unspelled.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(Vec::new()));
    refused(
        &unspelled,
        1,
        "[ROOT.Nobody.GetTag]",
        "character 7's script token 9000 is not spelled",
    );
    let mut untagged = frame();
    *untagged.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    refused(
        &untagged,
        1,
        "[ROOT.Nobody.GetTag]",
        "Nobody: whether it is a country tag is unknown: countries: the table is unavailable: fixture",
    );
    let mut unlinked = frame();
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *unlinked.tables[TableId::CountryAppearance.index()]
    {
        rows[1].overlord = 300;
    }
    refused(
        &unlinked,
        2,
        "[ROOT.Overlord.GetTag]",
        "the overlord link 300 of country slot 2 resolves to no country",
    );
    let mut faction = frame();
    *faction.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![FactionMemberRow {
            country: 200,
            faction: 5,
            ordinal: 1,
        }]));
    refused(
        &faction,
        2,
        "[ROOT.FactionLeader.GetTag]",
        "faction 5 has no first member",
    );
    if let TableView::Complete(TableRows::FactionMembers(rows)) =
        &mut *faction.tables[TableId::FactionMembers.index()]
    {
        rows.push(FactionMemberRow {
            country: 100,
            faction: 5,
            ordinal: 0,
        });
    }
    assert_eq!(
        expand(&faction, 2, "[ROOT.FactionLeader.GetTag]").as_deref(),
        Ok("GER")
    );
    let mut no_faction = frame();
    *no_faction.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(Vec::new()));
    assert_eq!(
        expand(&no_faction, 1, "[ROOT.FactionLeader.GetTag]").as_deref(),
        Ok("GER")
    );
    refused(
        &whole,
        1,
        "[ROOT.FactionLeader.GetTag]",
        "the faction of country slot 1 is unknown: faction members: the table is unavailable",
    );
}

#[test]
fn no_faction_localises_its_key() {
    game::enter();
    let mut frame = frame();
    *frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(Vec::new()));
    let extra = concat!(
        " MOD_faction:0 \"[ROOT.GetFactionName]\"\n",
        " NO_FACTION:0 \"No Faction\"\n",
    );
    let (defined, _folder) = catalogue(SCRIPTED, extra);
    let at = ScriptedText::new(
        &frame,
        &defined.localisation,
        defined.scripted_content().unwrap(),
        1,
    );
    assert_eq!(
        defined.localise_in("MOD_faction", &at).as_deref(),
        Some("No Faction")
    );
    let (lacking, _folder) = catalogue(SCRIPTED, " MOD_faction:0 \"[ROOT.GetFactionName]\"\n");
    let at = ScriptedText::new(
        &frame,
        &lacking.localisation,
        lacking.scripted_content().unwrap(),
        1,
    );
    assert_eq!(
        lacking.localise_in("MOD_faction", &at).as_deref(),
        Some("NO_FACTION")
    );
    assert_eq!(lacking.refusal_lines(), Vec::<String>::new());
}

#[test]
fn scripted_faction_names_use_the_recorded_template_and_copy_literals() {
    game::enter();
    use replay_data::{rows::FactionMemberRow, rows::FactionRow, rows::LexicalTokenRow};
    let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "localisation/demo_l_english.yml",
            "\u{feff}l_english:\n DEMO_NAME:0 \"[ROOT.GetTag] Pact $$\"\n Literal:0 \"Do not translate\"\n EMPTY_NAME:0 \"\"\n",
        ),
        (
            "common/factions/templates/demo.txt",
            "demo = { name = DEMO_NAME } empty = {} localised_empty = { name = EMPTY_NAME }",
        ),
        (
            "common/defines/00_defines.lua",
            "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 } }",
        ),
    ]);
    let mut frame = frame();
    *frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![FactionMemberRow {
            country: 100,
            faction: 5,
            ordinal: 0,
        }]));
    *frame.tables[TableId::Factions.index()] =
        TableView::Complete(TableRows::Factions(vec![FactionRow {
            id: 5,
            template_token: Some(17),
            ..Default::default()
        }]));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 17,
            literal: b"demo".to_vec(),
        }]));
    let content = catalogue.scripted_content().unwrap();
    let at = ScriptedText::new(&frame, &catalogue.localisation, content, 1);
    assert_eq!(at.faction_name(1).as_deref(), Ok("[ROOT.GetTag] Pact $"));
    for template in ["empty", "localised_empty"] {
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 17,
                literal: template.as_bytes().to_vec(),
            }]));
        let at = ScriptedText::new(&frame, &catalogue.localisation, content, 1);
        assert_eq!(at.faction_name(1).as_deref(), Ok(""), "{template}");
    }
    if let TableView::Complete(TableRows::Factions(rows)) =
        &mut *frame.tables[TableId::Factions.index()]
    {
        rows[0].name = b"Literal".to_vec();
        rows[0].template_token = Some(17);
    }
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
        reason: "not captured".into(),
    };
    let at = ScriptedText::new(&frame, &catalogue.localisation, content, 1);
    assert_eq!(at.faction_name(1).as_deref(), Ok("Literal"));
    let mut other_language = catalogue.localisation.as_ref().clone();
    other_language.language = "french".into();
    let at = ScriptedText::new(&frame, &other_language, content, 1);
    assert_eq!(
        at.faction_name(1).unwrap_err(),
        "faction name is text the game stored in english without its key, and the viewer's language is french"
    );
}

#[test]
fn a_case_change_leaves_a_byte_above_0x7f() {
    game::enter();
    let extra = concat!(
        " GER_ADJ:0 \"érudit\"\n",
        " SOV_ADJ:0 \"sovié\"\n",
        " fascism:0 \"FAScismÉ\"\n",
    );
    let (catalogue, _folder) = catalogue(SCRIPTED, extra);
    let mut frame = with_party(frame());
    if let TableView::Complete(TableRows::Politics(rows)) =
        &mut *frame.tables[TableId::Politics.index()]
    {
        rows[0].ruling_party = Some(3);
        rows.push(replay_data::rows::PoliticsRow {
            country: 200,
            ..Default::default()
        });
    }
    let at = ScriptedText::new(
        &frame,
        &catalogue.localisation,
        catalogue.scripted_content().unwrap(),
        1,
    );
    for (text, shown) in [
        ("[GER.GetAdjectiveCap]", "érudit"),
        ("[GER.GetNonIdeologyAdjectiveCap]", "érudit"),
        ("[ROOT.GetRulingIdeology]", "fascismÉ"),
    ] {
        assert_eq!(at.expand(text).as_deref(), Ok(shown));
    }
    assert_eq!(at.expand("[GER.GetAdjective]").as_deref(), Ok("érudit"));
    assert_eq!(at.expand("[SOV.GetAdjectiveCap]").as_deref(), Ok("Sovié"));
}

#[test]
fn scripted_text_state_name_failures_keep_caller_text_without_duplicate_refusals() {
    game::enter();
    let (cat, _folder) = catalogue(SCRIPTED, "");
    let state = replay_data::rows::StateRow {
        id: 41,
        ..Default::default()
    };
    let names = StateNames::new();
    for (usage, expected) in [
        (
            StateNameUsage::Occupation,
            "No recorded-content state template",
        ),
        (StateNameUsage::Raid, "state 41 has no content name"),
        (
            StateNameUsage::Place,
            "state 41: No recorded-content state template",
        ),
    ] {
        assert_eq!(
            state_display_name(&state, &cat, Ok(&names), usage).unwrap_err(),
            expected
        );
    }
    assert!(cat.refusal_lines().is_empty());
}

#[test]
fn scripted_text_state_localisation_refusal_is_owned_once_by_the_product() {
    game::enter();
    let (cat, _folder) = catalogue(SCRIPTED, " REFUSED_STATE:0 \"control \u{14}\"\n");
    let state = replay_data::rows::StateRow {
        id: 41,
        ..Default::default()
    };
    let names = StateNames::from([(41, Ok("REFUSED_STATE".into()))]);
    assert_eq!(
        state_display_name(&state, &cat, Ok(&names), StateNameUsage::Raid).unwrap_err(),
        "state 41 name REFUSED_STATE is refused"
    );
    assert!(cat.refusal_lines().is_empty());
}

#[test]
fn predicate_world_keeps_country_facts_when_map_content_or_provinces_are_missing() {
    game::enter();
    use replay_data::{rows::CompletedFocusRow, rows::LexicalTokenRow, rows::ScriptFlagRow};
    let (cat, _folder) = crate::catalogue::tests::folder_catalogue(&[
        ("localisation/mod_l_english.yml", LOCALISATION),
        (
            "common/defines/synthetic.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
        ),
        (
            "common/ideologies/synthetic.txt",
            "ideologies={mod_group={types={first_subtype={}}}}",
        ),
    ]);
    let content = cat.scripted_content().unwrap();
    let mut frame = frame();
    *frame.tables[TableId::Provinces.index()] = TableView::Unavailable {
        reason: "unrelated province read failed".into(),
    };
    *frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
            country: 100,
            token: b"mod_group".to_vec(),
            is_ruling: 1,
            ..Default::default()
        }]));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 901,
            literal: b"occupation_flag".to_vec(),
        }]));
    *frame.tables[TableId::ScriptFlags.index()] =
        TableView::Complete(TableRows::ScriptFlags(vec![ScriptFlagRow {
            scope_kind: 1,
            scope_id: 100,
            token: 901,
            value: 1,
            ..Default::default()
        }]));
    *frame.tables[TableId::CompletedFocuses.index()] =
        TableView::Complete(TableRows::CompletedFocuses(vec![CompletedFocusRow {
            country: 100,
            token: b"occupation_focus".to_vec(),
            ..Default::default()
        }]));
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *frame.tables[TableId::CountryAppearance.index()]
    {
        rows.iter_mut()
            .find(|row| row.country == 100)
            .unwrap()
            .leader_ideology = b"first_subtype".to_vec();
    }
    let text = ScriptedText::new(&frame, &cat.localisation, content, 1);
    assert!(text.world().is_err());
    let world = text.trigger_world().unwrap();
    let parsed = content_reader_script::triggers::read(
        b"has_government=mod_group has_country_flag=occupation_flag has_completed_focus=occupation_focus has_country_leader_ideology=FIRST_SUBTYPE",
        &content_reader_script::key_names::KeyNames::default(),
    );
    assert_eq!(
        world.evaluate(
            &parsed.items,
            crate::triggers::Scopes::of(crate::triggers::Scope::Country(1))
        ),
        crate::triggers::Eval::True
    );
    assert!(world.state_order(64).is_err());
    assert!(text.world().is_err());
}

#[path = "recorded_source_tests.rs"]
mod recorded_source;
