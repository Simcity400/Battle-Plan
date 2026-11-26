use super::*;
use crate::script_test_game as game;
use replay_data::rows::*;

fn frame() -> DecodedFrame {
    let mut frame = crate::scripted_text::tests::complete_frame();
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *frame.tables[TableId::CountryAppearance.index()]
    {
        for row in rows {
            row.original_tag_reference = Recorded::Value(Some(0));
            row.original_tag = Recorded::Value(Some(Vec::new()));
        }
    }
    frame
}
fn set<T: TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
    *frame.tables[T::ID.index()] = TableView::Complete(T::wrap(rows));
}
fn rendered(
    catalogue: &crate::catalogue::Catalogue,
    frame: &DecodedFrame,
    key: &str,
) -> Result<String, String> {
    let context = ScriptedText::new(
        frame,
        &catalogue.localisation,
        catalogue.scripted_content()?,
        1,
    );
    crate::catalogue::localised_in(&catalogue.localisation, key, &[], &context)
}

#[test]
fn entry_invocations_detect_cycles_and_render_finite_scope_changes() {
    game::enter();
    let (catalogue, _folder) = tests::catalogue(
        r#"entry = { name = Walk text = { trigger = { tag = GER } localization_key = DONE } text = { localization_key = NEXT } }
           entry = { name = SelfLoop text = { localization_key = SELF } }
           entry = { name = Loop text = { localization_key = LOOP } }
           entry = { name = Other text = { localization_key = OTHER } }"#,
        " DONE:0 \"done\"\n NEXT:0 \"step [FROM.Walk]\"\n SELF:0 \"[SelfLoop]\"\n LOOP:0 \"[Other]\"\n OTHER:0 \"[Loop]\"\n",
    );
    let content = catalogue.scripted_content().unwrap();
    content.set_synthetic_environment(Default::default());
    let f = frame();
    let text = ScriptedText::new(&f, &catalogue.localisation, content, 1);
    for entry in ["SelfLoop", "Loop"] {
        let refusal = text.expand(&format!("[{entry}]")).unwrap_err();
        assert!(
            refusal.contains(&format!("{entry}: cyclic entry invocation")),
            "{refusal}"
        );
    }
    let count = usize::try_from(content.recursion).unwrap() + 10;
    let scopes = EventScopes::new(
        (0..count)
            .map(|at| EventScope {
                from: if at + 1 == count {
                    Link::Itself
                } else {
                    Link::Scope(at + 1)
                },
                ..EventScope::of(EventObject::Country(if at + 1 == count { 1 } else { 2 }))
            })
            .collect(),
    );
    let expected = format!("{}done", "step ".repeat(count - 1));
    assert_eq!(
        text.expand_in("[Walk] [Walk]", &scopes),
        Ok(format!("{expected} {expected}"))
    );
    let mut shorter = scopes;
    shorter.scopes[0].from = Link::Scope(2);
    assert_eq!(
        text.expand_in("[Walk]", &shorter),
        Ok(format!("{}done", "step ".repeat(count - 2)))
    );
}

#[test]
fn entry_invocations_refuse_growing_history_at_the_backstop_without_stack_overflow() {
    game::enter();
    let (catalogue, _folder) = tests::catalogue(
        "entry = { name = Grow text = { localization_key = GROW } }",
        " GROW:0 \"[GER.Grow]\"\n",
    );
    let f = frame();
    let text = ScriptedText::new(
        &f,
        &catalogue.localisation,
        catalogue.scripted_content().unwrap(),
        2,
    );
    let unknown = EventScopes::new(vec![EventScope {
        prev: Link::Unknown,
        ..EventScope::of(EventObject::Country(1))
    }]);
    for result in [text.expand("[Grow]"), text.expand_in("[Grow]", &unknown)] {
        let refusal = result.unwrap_err();
        assert!(refusal.contains(
            "scripted localisation nesting the game could never finish (native stack exhaustion): Grow"
        ));
        assert!(refusal.contains(&format!("beyond {MAX_SCRIPTED_NESTING} levels")));
        assert!(!refusal.contains("cyclic entry invocation"));
    }
}

#[test]
fn entry_invocations_guard_fresh_balance_contexts() {
    game::enter();
    let (catalogue, _folder) = tests::catalogue(
        "entry = { name = Cross text = { localization_key = CROSS } }",
        " CROSS:0 \"[ROOT.GetActiveRangeName]\"\n RANGE:0 \"[Cross]\"\n SELF_RANGE:0 \"[ROOT.GetActiveRangeName]\"\n",
    );
    let mut f = frame();
    set::<tables::PowerBalanceMembers>(
        &mut f,
        vec![PowerBalanceMemberRow {
            country: 100,
            ..Default::default()
        }],
    );
    for (range, text, reason) in [
        (
            b"RANGE".as_slice(),
            "[Cross]",
            "Cross: cyclic entry invocation",
        ),
        (
            b"SELF_RANGE".as_slice(),
            "[ROOT.GetActiveRangeName]",
            "native stack exhaustion): GetActiveRangeName",
        ),
    ] {
        set::<tables::PowerBalances>(
            &mut f,
            vec![PowerBalanceRow {
                active_range: range.to_vec(),
                ..Default::default()
            }],
        );
        let refusal = ScriptedText::new(
            &f,
            &catalogue.localisation,
            catalogue.scripted_content().unwrap(),
            1,
        )
        .expand(text)
        .unwrap_err();
        assert!(refusal.contains(reason), "{refusal}");
    }
}

#[test]
fn province_names_use_recorded_owner_original_tag_and_capture_failures() {
    game::enter();
    let (catalogue, _folder) = tests::catalogue(
        "",
        " GER_VICTORY_POINTS_5:0 \"Owner\"\n SOV_VICTORY_POINTS_5:0 \"Controller\"\n VICTORY_POINTS_5:0 \"Generic\"\n OLD_VICTORY_POINTS_5:0 \"Original\"\n OR0_VICTORY_POINTS_5:0 \"Reserved state owner\"\n OR0_VICTORY_POINTS_6:0 \"Reserved original\"\n RENAME:0 \"Renamed\"\n",
    );
    let content = catalogue.scripted_content().unwrap();
    let mut f = frame();
    set::<tables::Provinces>(
        &mut f,
        vec![
            ProvinceRow {
                id: 5,
                state: 64,
                owner: 100,
                controller: 200,
                ..Default::default()
            },
            ProvinceRow {
                id: 6,
                ..Default::default()
            },
        ],
    );
    let name = |f: &DecodedFrame, id| {
        ScriptedText::new(f, &catalogue.localisation, content, 1).province_display_name_primary(id)
    };
    assert_eq!(name(&f, 5).unwrap().as_deref(), Some("Owner"));
    assert!(name(&f, 6).unwrap_err().contains("reserved country naming"));
    set::<tables::ReservedCountryNaming>(
        &mut f,
        vec![ReservedCountryNamingRow {
            tag: b"ZZ0".to_vec(),
            original_tag_reference: Recorded::Value(Some(0)),
            original_tag: Recorded::Value(Some(Vec::new())),
            ..Default::default()
        }],
    );
    assert_eq!(name(&f, 6), Ok(None));
    if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *f.tables[TableId::ReservedCountryNaming.index()]
    {
        rows[0].original_tag_reference = Recorded::Value(Some(1));
        rows[0].original_tag = Recorded::Value(Some(b"OR0".to_vec()));
    }
    assert_eq!(name(&f, 6).unwrap().as_deref(), Some("Reserved original"));
    let mut state = f.complete::<tables::States>().unwrap()[0].clone();
    state.owner = 0;
    set::<tables::States>(&mut f, vec![state.clone()]);
    assert_eq!(
        name(&f, 5).unwrap().as_deref(),
        Some("Reserved state owner")
    );
    state.owner = 300;
    set::<tables::States>(&mut f, vec![state.clone()]);
    assert!(
        name(&f, 5)
            .unwrap_err()
            .contains("state owner's link 300 resolves to no country")
    );
    *f.tables[TableId::States.index()] = TableView::Unavailable {
        reason: "state owner not sampled".into(),
    };
    assert!(name(&f, 5).unwrap_err().contains("state owner not sampled"));
    state.owner = 100;
    set::<tables::States>(&mut f, vec![state]);
    set::<tables::ProvinceNames>(
        &mut f,
        vec![ProvinceNameRow {
            province: 5,
            name: b"RENAME".to_vec(),
        }],
    );
    assert_eq!(name(&f, 5).unwrap().as_deref(), Some("Renamed"));
    set::<tables::ProvinceNames>(&mut f, Vec::new());
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *f.tables[TableId::CountryAppearance.index()]
    {
        rows[0].original_tag_reference = Recorded::Value(Some(1));
        rows[0].original_tag = Recorded::Value(Some(b"OLD".to_vec()));
    }
    assert_eq!(name(&f, 5).unwrap().as_deref(), Some("Original"));
    for (original_tag, reason) in [
        (
            Recorded::NotRecorded,
            "original tag of country appearance not recorded",
        ),
        (Recorded::Value(None), "capture refused"),
    ] {
        if let TableView::Complete(TableRows::CountryAppearance(rows)) =
            &mut *f.tables[TableId::CountryAppearance.index()]
        {
            rows[0].original_tag = original_tag;
        }
        assert!(name(&f, 5).unwrap_err().contains(reason));
    }
    *f.tables[TableId::ProvinceNames.index()] = TableView::Unavailable {
        reason: "test gap".into(),
    };
    assert!(name(&f, 5).unwrap_err().contains("province names"));
}

#[test]
fn player_text_uses_recorded_perspective_and_reserved_zero() {
    game::enter();
    let (catalogue, _folder) = tests::catalogue("", "");
    let content = catalogue.scripted_content().unwrap();
    let mut f = frame();
    set::<tables::ReservedCountryNaming>(
        &mut f,
        vec![ReservedCountryNamingRow {
            tag: b"ZZ0".to_vec(),
            ..Default::default()
        }],
    );
    for (country, observer_country, expected) in [
        (0, 200, "SOV/GER"),
        (100, 200, "GER/GER"),
        (0, 100, "GER/GER"),
        (0, 0, "ZZ0/GER"),
    ] {
        set::<tables::LocalPlayer>(
            &mut f,
            vec![LocalPlayerRow {
                country,
                observer_country,
            }],
        );
        let text = ScriptedText::new(&f, &catalogue.localisation, content, 1);
        assert_eq!(
            text.expand("[Player.GetTag]/[ROOT.GetTag]").as_deref(),
            Ok(expected)
        );
        let chosen = ScriptedText::new(&f, &catalogue.localisation, content, 1).viewing(2);
        assert_eq!(chosen.expand("[Player.GetTag]/[ROOT.GetTag]").as_deref(), Ok("SOV/GER"));
        assert_eq!(chosen.for_root(2).expand("[Player.GetTag]").as_deref(), Ok("SOV"));
    }
}

#[test]
fn saved_globals_promote_text_and_missing_names_leave_its_context_unchanged() {
    game::enter();
    let (catalogue, _folder) = tests::catalogue("", " NAME:0 \"Saved character\"\n");
    let mut f = frame();
    set::<tables::Characters>(
        &mut f,
        vec![CharacterRow {
            id: 7,
            name: b"Saved character".to_vec(),
            ..Default::default()
        }],
    );
    set::<tables::GlobalSavedEventTargets>(
        &mut f,
        vec![
            GlobalSavedEventTargetRow {
                ordinal: 0,
                name: b"chosen".to_vec(),
                country: 100,
                ..Default::default()
            },
            GlobalSavedEventTargetRow {
                ordinal: 1,
                name: b"place".to_vec(),
                state: 64,
                ..Default::default()
            },
            GlobalSavedEventTargetRow {
                ordinal: 2,
                name: b"person".to_vec(),
                character: 7,
                live: 1,
                ..Default::default()
            },
        ],
    );
    let content = catalogue.scripted_content().unwrap();
    let text = ScriptedText::new(&f, &catalogue.localisation, content, 1);
    assert_eq!(
        text.expand("[chosen.GetTag]/[place.GetID]/[person.GetName]")
            .as_deref(),
        Ok("GER/64/Saved character")
    );
    assert_eq!(
        text.expand("[missing.GetID]/[Chosen.GetID]").as_deref(),
        Ok("/")
    );
    set::<tables::GlobalSavedEventTargets>(&mut f, Vec::new());
    let text = ScriptedText::new(&f, &catalogue.localisation, content, 1);
    assert_eq!(text.expand("[chosen.GetTag]").as_deref(), Ok(""));
}

#[test]
fn random_lists_use_raw_fixed5_seed_scope_identity_and_fast_paths() {
    game::enter();
    let (catalogue, _folder) = tests::catalogue(
        r#"
        a = { name = Default text = { random_list = { 30 = { localization_key = A } 10 = { localization_key = B } 20 = { localization_key = C } } } }
        a = { name = Seeded text = { random_list = { seed = 7 30 = { localization_key = A } 10 = { localization_key = B } 20 = { localization_key = C } } } }
        a = { name = Single text = { random_list = { seed = unknown 0 = { localization_key = A modifier = { factor = random } } } } }
        a = { name = Zero text = { random_list = { seed = unknown 0 = { localization_key = A } 0 = { localization_key = B } } } text = { localization_key = C } }
        a = { name = Empty text = { random_list = {} } }
        a = { name = Ordinary text = { localization_key = A } }
        a = { name = Promoted text = { random_list = { 56 = { localization_key = A } 4 = { localization_key = B } } } }
    "#,
        " A:0 \"A\"\n B:0 \"B\"\n C:0 \"C\"\n",
    );
    let content = catalogue.scripted_content().unwrap();
    content.set_synthetic_environment(Default::default());
    let f = frame();
    let text = ScriptedText::new(&f, &catalogue.localisation, content, 1);
    assert_eq!(
        text.expand("[Default]|[Seeded]|[SOV.Seeded]|[Single]|[Zero]|[Empty]|[Ordinary]")
            .as_deref(),
        Ok("C|B|B|A|||A")
    );
    assert_eq!(
        text.expand("[Promoted]|[SOV.Promoted]").as_deref(),
        Ok("A|B")
    );
    let seeded = EventScopes::new(vec![EventScope {
        random_seed: 2,
        ..EventScope::of(EventObject::Country(1))
    }]);
    assert_eq!(text.expand_in("[Default]", &seeded).as_deref(), Ok("A"));
    let missing = EventScopes::new(vec![EventScope {
        random_seed_refusal: Some("decision random-count is unrecorded"),
        ..EventScope::of(EventObject::Country(1))
    }]);
    assert_eq!(
        text.expand_in("[Default]", &missing),
        Err("[Default]: scripted localisation Default: decision random-count is unrecorded".into())
    );
    assert_eq!(
        text.expand_in("[Seeded]|[Single]|[Zero]|[Empty]|[Ordinary]", &missing)
            .as_deref(),
        Ok("B|A|||A")
    );
    assert_eq!(
        text.expand_in("[Promoted]", &missing),
        Err(
            "[Promoted]: scripted localisation Promoted: decision random-count is unrecorded"
                .into()
        )
    );
    assert_eq!(
        text.expand_in("[SOV.Promoted]", &missing).as_deref(),
        Ok("B")
    );
}

#[test]
fn recorded_factory_counts_wrap_convert_and_drive_a_seeded_pick() {
    game::enter();
    let (catalogue, _folder) = tests::catalogue(
        "a = { name = Pick text = { trigger = {} random_list = { seed = num_of_controlled_factories 30 = { localization_key = A } 10 = { localization_key = B } 20 = { localization_key = C } } } }",
        " A:0 \"A\"\n B:0 \"B\"\n C:0 \"C\"\n",
    );
    let content = catalogue.scripted_content().unwrap();
    content.set_synthetic_environment(Default::default());
    let mut f = frame();
    set::<tables::CountryFactories>(
        &mut f,
        vec![CountryFactoriesRow {
            country: 200,
            military_levels: i32::MAX,
            naval_levels: i32::MAX,
            civilian_levels: 9,
            military_total_raw: i64::MIN,
            naval_total_raw: i64::MAX,
            civilian_total_raw: -100_000,
            ..Default::default()
        }],
    );
    let text = ScriptedText::new(&f, &catalogue.localisation, content, 2);
    let evaluator = crate::triggers::Evaluator::new(text.world().unwrap(), None);
    let scopes = crate::triggers::Scopes::of(crate::triggers::Scope::Country(2));
    assert_eq!(
        evaluator.scoped_value(b"num_of_controlled_factories", scopes),
        Ok(700_000)
    );
    assert_eq!(text.expand("[Pick]").as_deref(), Ok("B"));
    drop(text);
    set::<tables::CountryFactories>(&mut f, Vec::new());
    let text = ScriptedText::new(&f, &catalogue.localisation, content, 2);
    assert!(
        text.expand("[Pick]")
            .unwrap_err()
            .contains("country 200 has no country factories row")
    );
    drop(text);
    *f.tables[TableId::CountryFactories.index()] = TableView::Unavailable {
        reason: "capture failed".into(),
    };
    let text = ScriptedText::new(&f, &catalogue.localisation, content, 2);
    assert!(
        text.expand("[Pick]")
            .unwrap_err()
            .contains("CountryFactories is unavailable: capture failed")
    );
}

#[test]
fn scriptable_random_weights_use_the_unified_operands_and_ignore_modifier_conditions() {
    game::enter();
    let (catalogue, _folder) = tests::catalogue("", "");
    let content = catalogue.scripted_content().unwrap();
    content.set_synthetic_environment(Default::default());
    let f = frame();
    let text = ScriptedText::new(&f, &catalogue.localisation, content, 1);
    let evaluator = crate::triggers::Evaluator::new(text.world().unwrap(), None);
    let scopes = crate::triggers::Scopes::of(crate::triggers::Scope::Country(1));
    let member =
        |source: &str| content_reader_scripted_localisation::scripted_localisation::RandomMember {
            weight: 1_000_000,
            key: Vec::new(),
            modifiers: content_reader_parser::parser::parse(source.as_bytes())
                .unwrap()
                .items
                .into_iter()
                .map(|item| item.value.items().unwrap().to_vec())
                .collect(),
        };
    assert_eq!(evaluator.localisation_weight(&member("modifier = { always = no add = 1 factor = 2 } modifier = { always = no add = 2 factor = 0.5 }"), scopes), Ok(13));
    assert_eq!(
        evaluator.localisation_weight(
            &member("modifier = { factor = 0 } modifier = { factor = random }"),
            scopes
        ),
        Ok(0)
    );
    assert_eq!(
        evaluator.localisation_weight(&member("modifier = { add = -10.5 }"), scopes),
        Ok(-1)
    );
    let why = evaluator
        .localisation_weight(
            &member("modifier = { factor = num_of_controlled_factories }"),
            scopes,
        )
        .unwrap_err();
    assert!(why.contains("country factories row"), "{why}");
}

#[test]
fn seeded_random_lists_use_canonical_slots_across_five_thousand_countries() {
    game::enter();
    let (catalogue, _folder) = tests::catalogue(
        "a = { name = Pick text = { random_list = { seed = 7 30 = { localization_key = A } 10 = { localization_key = B } 20 = { localization_key = C } } } }",
        " A:0 \"A\"\n B:0 \"B\"\n C:0 \"C\"\n",
    );
    let content = catalogue.scripted_content().unwrap();
    content.set_synthetic_environment(Default::default());
    let mut f = frame();
    set::<tables::CountryLinks>(
        &mut f,
        (1..=5_000)
            .map(|slot| CountryLinkRow {
                id: 10_000 + slot,
                canonical_slot: slot,
            })
            .collect(),
    );
    set::<tables::Countries>(
        &mut f,
        (1..=5_000)
            .map(|slot| CountryRow {
                id: 10_000 + slot,
                canonical_slot: slot,
                tag: vec![
                    b'A' + (slot / 676) as u8,
                    b'A' + (slot / 26 % 26) as u8,
                    b'A' + (slot % 26) as u8,
                ],
                ..Default::default()
            })
            .collect(),
    );
    let text = ScriptedText::new(&f, &catalogue.localisation, content, 1);
    let mut counts = [0; 3];
    for slot in 1..=5_000 {
        let scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(slot))]);
        let result = text.expand_in("[Pick]", &scopes).unwrap();
        counts[match result.as_str() {
            "A" => 0,
            "B" => 1,
            "C" => 2,
            other => panic!("unexpected selection at slot {slot}: {other}"),
        }] += 1;
    }
    assert_eq!(counts, [2_494, 802, 1_704]);
}

#[test]
fn first_party_leaders_and_custom_party_names_use_the_real_localiser() {
    game::enter();
    let (catalogue, _folder) = tests::catalogue(
        "",
        concat!(
            " LEADER:0 \"Mod $NAME$\"\n NAME:0 \"Leader\"\n",
            " PARTY:0 \"The $NAME$ Party\"\n",
            " TEST:0 \"[ROOT.GetLeader]|[ROOT.GetFascistLeader]|[ROOT.GetDemocraticLeader]|[ROOT.GetCommunistLeader]|[ROOT.GetNeutralLeader]|[ROOT.GetDemocraticParty]\"\n",
            " FOLD:0 \"[ROOT.GetFascistLeader]|[ROOT.GetFascistParty]\"\n"
        ),
    );
    let mut f = frame();
    set::<tables::Politics>(
        &mut f,
        vec![PoliticsRow {
            country: 100,
            ruling_party: Some(9),
            ..Default::default()
        }],
    );
    set::<tables::PoliticalParties>(
        &mut f,
        ["fascism", "democratic", "communism", "neutrality"]
            .into_iter()
            .enumerate()
            .map(|(i, token)| PoliticalPartyRow {
                country: 100,
                id: 9 + i as u32,
                token: token.into(),
                long_name: b"PARTY".to_vec(),
                ..Default::default()
            })
            .collect(),
    );
    set::<tables::Characters>(
        &mut f,
        vec![
            CharacterRow {
                id: 7,
                country: 100,
                token: b"LEADER".to_vec(),
                ..Default::default()
            },
            CharacterRow {
                id: 8,
                name: b"Second".to_vec(),
                ..Default::default()
            },
        ],
    );
    set::<tables::PartyLeaders>(
        &mut f,
        vec![
            PartyLeaderRow {
                country: 100,
                party: 9,
                ordinal: 0,
                character: Some(7),
                role: Some(9),
                ..Default::default()
            },
            PartyLeaderRow {
                country: 100,
                party: 9,
                ordinal: 1,
                character: Some(8),
                role: Some(9),
                ..Default::default()
            },
            PartyLeaderRow {
                country: 100,
                party: 10,
                ordinal: 0,
                character: None,
                ..Default::default()
            },
            PartyLeaderRow {
                country: 100,
                party: 10,
                ordinal: 1,
                character: Some(8),
                role: Some(9),
                ..Default::default()
            },
        ],
    );
    assert_eq!(
        rendered(&catalogue, &f, "TEST").unwrap(),
        "Mod Leader|Mod Leader||||The Leader Party"
    );
    assert_eq!(
        rendered(&catalogue, &f, "FOLD").unwrap(),
        "Mod Leader|The Leader Party"
    );
    let mut parties = f.complete::<tables::PoliticalParties>().unwrap().to_vec();
    parties[0].token = b"Fascism".to_vec();
    set::<tables::PoliticalParties>(&mut f, parties.clone());
    assert_eq!(rendered(&catalogue, &f, "FOLD").unwrap(), "Mod Leader|");
    parties[0].token = b"fascism".to_vec();
    set::<tables::PoliticalParties>(&mut f, parties);
    *f.tables[TableId::PartyLeaders.index()] = TableView::Truncated {
        rows: TableRows::PartyLeaders(Vec::new()),
        missing: 4,
    };
    assert!(
        rendered(&catalogue, &f, "TEST")
            .unwrap_err()
            .contains("party leaders")
    );
}

#[test]
fn genders_manpower_and_agency_preserve_empty_and_wrapping_rules() {
    game::enter();
    let (catalogue, _folder) = tests::catalogue(
        "",
        concat!(
            " EV_SHE:0 \"f-subject\"\n EV_HE:0 \"m-subject\"\n EV_HER:0 \"f-object\"\n EV_HERS:0 \"f-possessive\"\n EV_HERSELF:0 \"f-reflexive\"\n",
            " AGENCY_NAME:0 \"No agency\"\n",
            " TEST:0 \"[ROOT.GetSheHeCap]|[ROOT.GetHerHim]|[ROOT.GetHerHis]|[ROOT.GetHersHis]|[ROOT.GetHerselfHimselfCap]|[ROOT.GetManpower]|[ROOT.GetAgency]\"\n"
        ),
    );
    let mut f = frame();
    set::<tables::Politics>(
        &mut f,
        vec![PoliticsRow {
            country: 100,
            ruling_party: Some(9),
            ..Default::default()
        }],
    );
    set::<tables::PoliticalParties>(
        &mut f,
        vec![PoliticalPartyRow {
            country: 100,
            id: 9,
            ..Default::default()
        }],
    );
    set::<tables::PartyLeaders>(
        &mut f,
        vec![PartyLeaderRow {
            country: 100,
            party: 9,
            character: Some(7),
            role: Some(9),
            ..Default::default()
        }],
    );
    set::<tables::Characters>(
        &mut f,
        vec![CharacterRow {
            id: 7,
            gender: 2,
            ..Default::default()
        }],
    );
    let mut countries = f.complete::<tables::Countries>().unwrap().to_vec();
    countries[0].manpower_residual = i32::MAX;
    set::<tables::Countries>(&mut f, countries);
    set::<tables::States>(
        &mut f,
        vec![
            StateRow {
                id: 1,
                owner: 200,
                controller: 100,
                manpower_available: 2,
                ..Default::default()
            },
            StateRow {
                id: 2,
                owner: 100,
                controller: 200,
                manpower_available: 99,
                ..Default::default()
            },
        ],
    );
    set::<tables::Provinces>(
        &mut f,
        [1, 2]
            .map(|state| ProvinceRow {
                id: state,
                state,
                ..Default::default()
            })
            .to_vec(),
    );
    assert!(
        rendered(&catalogue, &f, "TEST")
            .unwrap_err()
            .contains("no intelligence agencies row")
    );
    let agency = |name: &[u8], created| IntelligenceAgencyRow {
        country: 100,
        name: name.to_vec(),
        created,
        ..Default::default()
    };
    set::<tables::IntelligenceAgencies>(&mut f, vec![agency(b"Abwehr", 0)]);
    assert_eq!(
        rendered(&catalogue, &f, "TEST").unwrap(),
        "F-subject|f-object|f-object|f-possessive|F-reflexive|-2147483647|No agency"
    );
    set::<tables::IntelligenceAgencies>(&mut f, vec![agency(b"", 1)]);
    assert!(rendered(&catalogue, &f, "TEST").unwrap().ends_with("647|"));
    set::<tables::IntelligenceAgencies>(&mut f, vec![agency(b"AGENCY_NAME", 1)]);
    assert!(
        rendered(&catalogue, &f, "TEST")
            .unwrap()
            .ends_with("|AGENCY_NAME")
    );
    set::<tables::Provinces>(
        &mut f,
        vec![ProvinceRow {
            id: 2,
            state: 2,
            ..Default::default()
        }],
    );
    assert!(
        rendered(&catalogue, &f, "TEST")
            .unwrap_err()
            .contains("no province names state 1")
    );
}

#[test]
fn stored_text_needs_the_recording_language() {
    game::enter();
    let (mut catalogue, _folder) = tests::catalogue(
        "",
        " AGENCY_NAME:0 \"No agency\"\n TEST:0 \"[ROOT.GetAgency]\"\n",
    );
    catalogue.recorded_language = "german".into();
    let mut f = frame();
    let agency = |name: &[u8], created| IntelligenceAgencyRow {
        country: 100,
        name: name.to_vec(),
        created,
        ..Default::default()
    };
    set::<tables::IntelligenceAgencies>(&mut f, vec![agency(b"Abwehr", 1)]);
    let error = rendered(&catalogue, &f, "TEST").unwrap_err();
    assert!(
        error.contains("stored in german") && error.contains("viewer's language is english"),
        "{error}"
    );
    set::<tables::IntelligenceAgencies>(&mut f, vec![agency(b"", 1)]);
    assert_eq!(rendered(&catalogue, &f, "TEST").unwrap(), "");
    set::<tables::IntelligenceAgencies>(&mut f, vec![agency(b"Abwehr", 0)]);
    assert_eq!(rendered(&catalogue, &f, "TEST").unwrap(), "No agency");
}

#[test]
fn stored_character_names_need_the_recording_language() {
    game::enter();
    for language in ["english", "german"] {
        let (mut catalogue, _folder) = tests::catalogue("", "");
        catalogue.recorded_language = language.into();
        let content = catalogue.scripted_content().unwrap();
        content.set_synthetic_environment(Default::default());
        let mut f = raid_frame();
        set::<tables::Characters>(
            &mut f,
            vec![CharacterRow {
                id: 44,
                country: 100,
                name: b"Erwin Rommel".to_vec(),
                ..Default::default()
            }],
        );
        set::<tables::UnitLeaders>(
            &mut f,
            vec![UnitLeaderRow {
                id: 9,
                reference_type: 3,
                character_id: 44,
                ..Default::default()
            }],
        );
        let text = ScriptedText::new(&f, &catalogue.localisation, content, 1);
        let leader = EventScopes::new(vec![EventScope {
            this: EventObject::UnitLeader(9 << 32 | 3),
            root: Link::Unknown,
            from: Link::Unknown,
            prev: Link::Unknown,
            random_seed: 1,
            random_counter: 0x5ea6_ba9e,
            random_seed_refusal: None,
            clock_seed: None,
        }]);
        let name = text.expand_in("[THIS.GetName]", &leader);
        if language == "english" {
            assert_eq!(name.as_deref(), Ok("Erwin Rommel"));
        } else {
            let refused = name.unwrap_err();
            assert!(
                refused.contains("stored in german") && refused.contains("character 44 name"),
                "{refused}"
            );
        }
    }
}

#[test]
fn old_names_copy_the_retained_caches_or_the_current_name() {
    game::enter();
    let (catalogue, _folder) = tests::catalogue(
        "",
        " TEST:0 \"[ROOT.GetOldName]|[ROOT.GetOldNameDefCap]|[ROOT.GetOldAdjective]|[ROOT.GetOldAdjectiveCap]\"\n",
    );
    let mut f = frame();
    let mut appearance = f.complete::<tables::CountryAppearance>().unwrap().to_vec();
    appearance[0].old_name = Some(b"Weimar Republic".to_vec());
    appearance[0].old_definite_name = Some(b"the Weimar Republic".to_vec());
    appearance[0].old_adjective = Some(Vec::new());
    set::<tables::CountryAppearance>(&mut f, appearance.clone());
    assert_eq!(
        rendered(&catalogue, &f, "TEST").unwrap(),
        "Weimar Republic|The Weimar Republic|german|German"
    );
    appearance[0].old_adjective = None;
    set::<tables::CountryAppearance>(&mut f, appearance);
    assert!(
        rendered(&catalogue, &f, "TEST")
            .unwrap_err()
            .contains("GetOldAdjective: the old adjective was unreadable")
    );
}

#[test]
fn date_election_and_support_formats_preserve_game_text() {
    game::enter();
    let (catalogue, _folder) = tests::catalogue(
        "",
        concat!(
            " February:0 \"Mod-month\"\n",
            " TEST:0 \"[GetDateText]|[GetDate]|[GetYear]|[GetMonth]|[ROOT.GetLastElection]|[ROOT.GetPartySupport]\"\n"
        ),
    );
    let mut f = frame();
    f.hour = game_model::calendar::HOUR_EPOCH + 1936 * 8760 + 31 * 24 + 23;
    let last_election = f.hour - 23;
    set::<tables::Politics>(
        &mut f,
        vec![PoliticsRow {
            country: 100,
            ruling_party: Some(9),
            last_election,
            ..Default::default()
        }],
    );
    set::<tables::PoliticalParties>(
        &mut f,
        vec![
            PoliticalPartyRow {
                country: 100,
                id: 9,
                popularity: 250_000,
                ..Default::default()
            },
            PoliticalPartyRow {
                country: 100,
                id: 10,
                popularity: 750_000,
                ..Default::default()
            },
        ],
    );
    assert_eq!(
        rendered(&catalogue, &f, "TEST").unwrap(),
        " 24:00, 1 Mod-month, 1936|1936.2.1.24|1936|Mod-month| 1:00, 1 Mod-month, 1936|25"
    );
}

#[test]
fn scripted_entries_order_triggers_replacements_and_gaps() {
    game::enter();
    let scripts = r#"entry = { name = Choose text = { localization_key = OLD } }
        entry = { name = Choose text = { trigger = { always = yes } }
            text = { trigger = { tag = SOV } localization_key = WRONG }
            text = { trigger = { ROOT = { tag = GER } NOT = { tag = SOV } } localization_key = PICK }
            text = { localization_key = WRONG } }
        entry = { name = None text = { trigger = { always = no } localization_key = WRONG } }
        entry = { name = Gap text = { trigger = { has_country_flag = unseen } localization_key = PICK } text = { localization_key = WRONG } }"#;
    let (catalogue, _folder) = tests::catalogue(
        scripts,
        " PICK:0 \"$PART$ [ROOT.GetTag]\"\n PART:0 \"Selected\"\n TEST:0 \"[Choose]|[None]\"\n GAP:0 \"[Gap]\"\n",
    );
    let content = catalogue.scripted_content().unwrap();
    content.set_synthetic_environment(Default::default());
    let mut f = frame();
    assert_eq!(rendered(&catalogue, &f, "TEST").unwrap(), "Selected GER|");
    set::<tables::LexicalTokens>(
        &mut f,
        vec![LexicalTokenRow {
            token: 1,
            literal: b"unseen".to_vec(),
        }],
    );
    *f.tables[TableId::ScriptFlags.index()] = TableView::Unavailable {
        reason: "test gap".into(),
    };
    let error = rendered(&catalogue, &f, "GAP").unwrap_err();
    assert!(
        error.contains("Gap") && error.contains("test gap"),
        "{error}"
    );
}

#[test]
fn state_names_prefer_the_stored_custom_and_owner_names_to_the_template() {
    game::enter();
    let (catalogue, _folder) = tests::catalogue(
        "",
        " MOD_STATE:0 \"A $PART$\"\n PART:0 \"state\"\n TEST:0 \"[ROOT.Capital.GetName]\"\n",
    );
    let mut environment = entries::EnvironmentInputs::synthetic();
    environment.state_names.insert(64, Ok("MOD_STATE".into()));
    assert!(
        catalogue
            .scripted_content()
            .unwrap()
            .environment()
            .unwrap()
            .inputs
            .set(Ok(environment))
            .is_ok()
    );
    let mut f = frame();
    assert_eq!(rendered(&catalogue, &f, "TEST").unwrap(), "A state");
    let mut states = f.complete::<tables::States>().unwrap().to_vec();
    states[0].owner_name = "Niederösterreich".into();
    set::<tables::States>(&mut f, states.clone());
    assert_eq!(
        rendered(&catalogue, &f, "TEST").unwrap(),
        "Niederösterreich"
    );
    states[0].custom_name = b"Ostmark".to_vec();
    set::<tables::States>(&mut f, states);
    assert_eq!(rendered(&catalogue, &f, "TEST").unwrap(), "Ostmark");
}

#[test]
fn balance_names_selection_change_binding_and_icon_direction() {
    game::enter();
    let (catalogue, _folder) = tests::catalogue(
        "",
        " MOD_BALANCE:0 \"Balance\"\n MOD_LEFT:0 \"Left\"\n MOD_RIGHT:0 \"Right\"\n MOD_RANGE:0 \"[ROOT.GetTag] range\"\n BOP_CHANGE_RATE_DAILY:0 \"Daily $VALUE|2$\"\n BOP_CHANGE_RATE_WEEKLY:0 \"Weekly $VALUE|2$\"\n TEST:0 \"[ROOT.GetPowerBalanceName]|[ROOT.GetLeftSideName]|[ROOT.GetRightSideName]|[ROOT.GetActiveSideName]|[ROOT.GetTrendingSideName]|[ROOT.GetActiveRangeName]|[ROOT.GetChangeRateDesc]\"\n",
    );
    let mut f = frame();
    set::<tables::PowerBalances>(
        &mut f,
        vec![PowerBalanceRow {
            ordinal: 0,
            value_raw: -1,
            balance: b"MOD_BALANCE".to_vec(),
            left: b"MOD_LEFT".to_vec(),
            right: b"MOD_RIGHT".to_vec(),
            trending: b"MOD_RIGHT".to_vec(),
            active_range: b"MOD_RANGE".to_vec(),
        }],
    );
    set::<tables::PowerBalanceMembers>(
        &mut f,
        vec![PowerBalanceMemberRow {
            country: 100,
            daily_raw: 150_000,
            daily_present: true,
            daily_flags: engine_native_parts::modifier_definition::CLAMP_MAXIMUM,
            weekly_raw: -350_000,
            weekly_present: true,
            ..Default::default()
        }],
    );
    assert_eq!(
        rendered(&catalogue, &f, "TEST").unwrap(),
        "Balance|Left|Right|Left|Right|GER range|Daily 0.50"
    );
    let at = ScriptedText::new(
        &f,
        &catalogue.localisation,
        catalogue.scripted_content().unwrap(),
        1,
    );
    assert_eq!(
        at.expand("[ROOT.GetBopTrendTextIcon]").unwrap(),
        "\u{13}BoP_right_texticon "
    );
}

#[test]
fn character_pronouns_and_literal_names_differ_from_party_display_names() {
    game::enter();
    let (catalogue, _folder) = tests::catalogue(
        "",
        concat!(
            " EV_HE:0 \"m-subject\"\n EV_HIM:0 \"m-object\"\n EV_HIS:0 \"m-possessive\"\n EV_HIMSELF:0 \"m-reflexive\"\n",
            " TEST:0 \"[ROOT.GER_erwin_rommel.GetName]|[ROOT.GER_erwin_rommel.GetSheHeCap]|[ROOT.GER_erwin_rommel.GetHerHimCap]|[ROOT.GER_erwin_rommel.GetHerHisCap]|[ROOT.GER_erwin_rommel.GetHersHisCap]|[ROOT.GER_erwin_rommel.GetHerselfHimself]\"\n"
        ),
    );
    let mut f = frame();
    let mut characters = f.complete::<tables::Characters>().unwrap().to_vec();
    characters[0].gender = u32::MAX;
    set::<tables::Characters>(&mut f, characters.clone());
    assert_eq!(
        rendered(&catalogue, &f, "TEST").unwrap(),
        "|M-subject|M-object|M-possessive|M-possessive|m-reflexive"
    );
    characters[0].name = b"Literal Name".to_vec();
    set::<tables::Characters>(&mut f, characters);
    assert!(
        rendered(&catalogue, &f, "TEST")
            .unwrap()
            .starts_with("Literal Name|")
    );
}

#[test]
fn promoted_entry_binds_this_and_preserves_root_and_from() {
    game::enter();
    let (catalogue, _folder) = tests::catalogue(
        r#"entry = { name = Country text = { trigger = { tag = SOV ROOT = { tag = GER } FROM = { tag = GER } PREV = { tag = GER } } localization_key = YES } }
           entry = { name = State text = { trigger = { ROOT = { tag = GER } } localization_key = YES } }"#,
        " YES:0 \"pass\"\n TEST:0 \"[SOV.Country]|[ROOT.Capital.State]\"\n",
    );
    catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    assert_eq!(rendered(&catalogue, &frame(), "TEST").unwrap(), "pass|pass");
}

fn raid_frame() -> DecodedFrame {
    let mut f = frame();
    let countries = [(100, 1, "GER"), (200, 2, "SOV"), (300, 3, "ENG")];
    set::<tables::CountryLinks>(
        &mut f,
        countries
            .iter()
            .map(|&(id, canonical_slot, _)| CountryLinkRow { id, canonical_slot })
            .collect(),
    );
    set::<tables::Countries>(
        &mut f,
        countries
            .iter()
            .map(|&(id, canonical_slot, tag)| CountryRow {
                id,
                canonical_slot,
                tag: tag.into(),
                ..Default::default()
            })
            .collect(),
    );
    set::<tables::CountryCounterInputs>(
        &mut f,
        countries
            .iter()
            .map(|&(id, ..)| CountryCounterInputRow {
                country: id,
                owned_state_count: 1,
                ..Default::default()
            })
            .collect(),
    );
    set::<tables::ScriptVariables>(
        &mut f,
        std::iter::once((0, 0))
            .chain(countries.iter().map(|&(id, ..)| (1, id)))
            .map(|(scope_kind, scope_id)| ScriptVariablesRow {
                scope_kind,
                scope_id,
                seed: replay_data::rows::Recorded::Value(None),
                entries: Vec::new(),
            })
            .collect(),
    );
    f
}

#[test]
fn raid_text_follows_the_active_event_scope_through_chains_and_entries() {
    game::enter();
    let (catalogue, _folder) = tests::catalogue(
        r#"entry = { name = Side text = { trigger = { tag = SOV ROOT = { tag = SOV } FROM = { tag = ENG } PREV = { tag = SOV } } localization_key = YES } text = { localization_key = NO } }
           entry = { name = Target text = { trigger = { tag = ENG ROOT = { tag = ENG } FROM = { tag = ENG } PREV = { tag = ENG } } localization_key = YES } text = { localization_key = NO } }
           entry = { name = Named text = { trigger = { tag = ENG ROOT = { tag = SOV } FROM = { tag = ENG } PREV = { tag = SOV } } localization_key = YES } text = { localization_key = NO } }
           entry = { name = Echo text = { localization_key = ECHO } }"#,
        " YES:0 \"yes\"\n NO:0 \"no\"\n ECHO:0 \"[THIS.GetTag]/[ROOT.GetTag]/[PREV.GetTag]\"\n",
    );
    let content = catalogue.scripted_content().unwrap();
    content.set_synthetic_environment(Default::default());
    let f = raid_frame();
    let text = ScriptedText::new(&f, &catalogue.localisation, content, 1);
    let raid = EventScopes::new(vec![
        EventScope {
            from: Link::Scope(1),
            ..EventScope::of(EventObject::Country(2))
        },
        EventScope::of(EventObject::Country(3)),
    ]);
    let expand = |text_: &str, scopes: &EventScopes| text.expand_in(text_, scopes);
    assert_eq!(
        expand(
            "[THIS.GetTag] [ROOT.GetTag] [FROM.GetTag] [FROM.ROOT.GetTag] [ROOT.FROM.PREV.GetTag] [PREV.GetTag]",
            &raid
        )
        .as_deref(),
        Ok("SOV SOV ENG ENG ENG SOV")
    );
    assert_eq!(
        expand("[Side] [FROM.Target] [ENG.Named]", &raid).as_deref(),
        Ok("yes yes yes")
    );
    assert_eq!(text.expand("[Side]").as_deref(), Ok("no"));
    assert_eq!(
        expand("[Echo] [FROM.Echo] [ENG.Echo]", &raid).as_deref(),
        Ok("SOV/SOV/SOV ENG/ENG/ENG ENG/SOV/SOV")
    );
    let nested = EventScopes::new(vec![
        EventScope {
            this: EventObject::Country(2),
            root: Link::Scope(1),
            from: Link::Scope(2),
            prev: Link::Scope(1),
            random_seed: 1,
            random_counter: 0x5ea6_ba9e,
            random_seed_refusal: None,
            clock_seed: None,
        },
        EventScope {
            this: EventObject::Country(2),
            root: Link::Unknown,
            from: Link::Unknown,
            prev: Link::Unknown,
            random_seed: 1,
            random_counter: 0x5ea6_ba9e,
            random_seed_refusal: None,
            clock_seed: None,
        },
        EventScope {
            this: EventObject::Country(3),
            root: Link::Unknown,
            from: Link::Unknown,
            prev: Link::Unknown,
            random_seed: 1,
            random_counter: 0x5ea6_ba9e,
            random_seed_refusal: None,
            clock_seed: None,
        },
    ]);
    assert_eq!(expand("[FROM.GetTag]", &nested).as_deref(), Ok("ENG"));
    let refused = expand("[FROM.ROOT.GetTag]", &nested).unwrap_err();
    assert!(refused.contains("Root link is not known"), "{refused}");
    let refused = expand("[FROM.Target]", &nested).unwrap_err();
    assert!(refused.contains("ROOT is not known"), "{refused}");
}

#[test]
fn unknown_scope_parts_refuse_only_where_they_are_read() {
    game::enter();
    let (catalogue, _folder) = tests::catalogue(
        r#"entry = { name = Always text = { trigger = { always = yes } localization_key = YES } text = { localization_key = NO } }
           entry = { name = ThisOnly text = { trigger = { tag = SOV } localization_key = YES } text = { localization_key = NO } }
           entry = { name = RootToo text = { trigger = { tag = SOV ROOT = { tag = SOV } } localization_key = YES } text = { localization_key = NO } }
           entry = { name = Targeted text = { trigger = { var:target_state = { always = yes } } localization_key = YES } text = { localization_key = NO } }
           entry = { name = Choice text = { trigger = { has_character_flag = x } localization_key = YES } text = { localization_key = NO } }
           entry = { name = Holder text = { localization_key = HOLDER } }"#,
        " YES:0 \"yes\"\n NO:0 \"no\"\n HOLDER:0 \"[SOV.Choice]\"\n",
    );
    let content = catalogue.scripted_content().unwrap();
    content.set_synthetic_environment(Default::default());
    let mut f = raid_frame();
    set::<tables::LexicalTokens>(
        &mut f,
        vec![LexicalTokenRow {
            token: 7,
            literal: b"rommel".to_vec(),
        }],
    );
    set::<tables::Characters>(
        &mut f,
        vec![CharacterRow {
            id: 44,
            country: 100,
            name_token_id: 7,
            name: b"Erwin Rommel".to_vec(),
            ..Default::default()
        }],
    );
    set::<tables::UnitLeaders>(
        &mut f,
        vec![UnitLeaderRow {
            id: 9,
            reference_type: 3,
            character_id: 44,
            ..Default::default()
        }],
    );
    let text = ScriptedText::new(&f, &catalogue.localisation, content, 1);
    let unknown = |this| {
        EventScopes::new(vec![EventScope {
            this,
            root: Link::Unknown,
            from: Link::Unknown,
            prev: Link::Unknown,
            random_seed: 1,
            random_counter: 0x5ea6_ba9e,
            random_seed_refusal: None,
            clock_seed: None,
        }])
    };
    let sov = unknown(EventObject::Country(2));
    assert_eq!(
        text.expand_in("[Always] [ThisOnly] [SOV.ThisOnly]", &sov)
            .as_deref(),
        Ok("yes yes yes")
    );
    let refused = text.expand_in("[RootToo]", &sov).unwrap_err();
    assert!(refused.contains("ROOT is not known"), "{refused}");
    assert_eq!(text.expand_in("[Targeted]", &sov).as_deref(), Ok("no"));
    let targeted = EventScopes {
        variables: ScopeVariables {
            target_state: Some(64),
            ..ScopeVariables::default()
        },
        ..sov.clone()
    };
    assert_eq!(
        text.expand_in("[Targeted] [SOV.Targeted]", &targeted)
            .as_deref(),
        Ok("yes yes")
    );
    let state = unknown(EventObject::State(64));
    assert_eq!(
        text.expand_in("[THIS.64.GetID]", &state).as_deref(),
        Ok("64")
    );
    assert!(text.expand_in("[THIS.rommel.GetID]", &state).is_ok());
    let leader = unknown(EventObject::UnitLeader(9 << 32 | 3));
    assert_eq!(
        text.expand_in("[THIS.GetName] [THIS.64.GetID]", &leader)
            .as_deref(),
        Ok("Erwin Rommel 64")
    );
    assert_eq!(text.expand_in("[SOV.Choice]", &sov).as_deref(), Ok("no"));
    assert_eq!(
        text.expand_in("[THIS.Holder]", &leader).as_deref(),
        Ok("no")
    );
    let refused = text
        .expand_in("[THIS.GetName]", &unknown(EventObject::Province(5)))
        .unwrap_err();
    assert!(refused.contains("province 5"), "{refused}");
    *f.tables[TableId::Characters.index()] = TableView::Unavailable {
        reason: "test gap".into(),
    };
    let text = ScriptedText::new(&f, &catalogue.localisation, content, 1);
    assert_eq!(
        text.expand_in("[THIS.64.GetID]", &leader).as_deref(),
        Ok("64")
    );
}

#[test]
fn promoted_character_entry_evaluates_in_a_character_scope() {
    game::enter();
    let (catalogue, _folder) = tests::catalogue(
        r#"entry = { name = Char text = { trigger = { tag = GER skill = 0 NOT = { tag = SOV } ROOT = { tag = GER } } localization_key = YES } text = { localization_key = NO } }
           entry = { name = Trait text = { trigger = { has_trait = unknown_trait } localization_key = YES } text = { localization_key = NO } }
           entry = { name = Flag text = { trigger = { has_character_flag = unset } localization_key = YES } text = { localization_key = NO } }
           entry = { name = Other text = { trigger = { has_government = fascism } localization_key = YES } }"#,
        " YES:0 \"pass\"
 NO:0 \"fail\"
 TEST:0 \"[ROOT.GER_erwin_rommel.Char]|[ROOT.GER_erwin_rommel.Trait]|[ROOT.GER_erwin_rommel.Flag]\"
 OTHER:0 \"[ROOT.GER_erwin_rommel.Other]\"
",
    );
    catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    assert_eq!(
        rendered(&catalogue, &frame(), "TEST").unwrap(),
        "pass|fail|fail"
    );
    assert_eq!(rendered(&catalogue, &frame(), "OTHER").unwrap(), "");
}

#[test]
fn ability_text_promotes_character_but_keeps_country_for_named_lookup_and_entries() {
    game::enter();
    let (catalogue, _folder) = tests::catalogue(
        r#"entry={name=Mixed text={trigger={has_country_flag=x has_character_flag=x tag=SOV}
            localization_key=YES} text={localization_key=NO}}"#,
        " YES:0 \"both\"\n NO:0 \"missing\"\n",
    );
    let content = catalogue.scripted_content().unwrap();
    content.set_synthetic_environment(Default::default());
    let mut f = frame();
    set::<tables::LexicalTokens>(
        &mut f,
        vec![
            LexicalTokenRow {
                token: 8,
                literal: b"peer".to_vec(),
            },
            LexicalTokenRow {
                token: 900,
                literal: b"x".to_vec(),
            },
        ],
    );
    set::<tables::Characters>(
        &mut f,
        vec![
            CharacterRow {
                id: 44,
                country: 100,
                name: b"Ability Leader".to_vec(),
                ..Default::default()
            },
            CharacterRow {
                id: 45,
                country: 100,
                name_token_id: 8,
                name: b"Country Peer".to_vec(),
                ..Default::default()
            },
        ],
    );
    set::<tables::UnitLeaders>(
        &mut f,
        vec![UnitLeaderRow {
            id: 9,
            reference_type: 3,
            character_id: 44,
            character_type: engine_native::ccharacter::REFERENCE_TYPE,
            owner: 200,
            ..Default::default()
        }],
    );
    set::<tables::ScriptFlags>(
        &mut f,
        vec![
            ScriptFlagRow {
                scope_kind: 1,
                scope_id: 100,
                token: 900,
                value: 1,
                ..Default::default()
            },
            ScriptFlagRow {
                scope_kind: 3,
                scope_id: 44,
                token: 900,
                value: 1,
                ..Default::default()
            },
        ],
    );
    let text = ScriptedText::new(&f, &catalogue.localisation, content, 1);
    let graph = EventScopes::new(vec![EventScope::of(EventObject::CountryUnitLeader {
        country: 1,
        leader: 9 << 32 | 3,
    })]);
    assert_eq!(
        text.expand_in(
            "[ROOT.GetName]/[FROM.GetName]/[PREV.GetName]/[ROOT.peer.GetName]",
            &graph
        )
        .as_deref(),
        Ok("Ability Leader/Ability Leader/Ability Leader/Country Peer")
    );
    assert_eq!(
        text.expand_in("[Mixed]/[GER.Mixed]", &graph).as_deref(),
        Ok("both/missing")
    );
}
