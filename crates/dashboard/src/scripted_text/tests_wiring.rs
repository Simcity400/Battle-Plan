use super::*;
use replay::rows::*;

fn frame() -> DecodedFrame {
    let mut frame = crate::scope::tests::frame();
    for id in TableId::ALL {
        if matches!(&*frame.tables[id.index()], TableView::Unavailable { .. }) {
            *frame.tables[id.index()] = TableView::Complete(TableRows::empty(id));
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
        0,
    );
    crate::catalogue::localised_in(&catalogue.localisation, key, &[], &context)
}

#[test]
fn first_party_leaders_and_custom_party_names_use_the_real_localiser() {
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
                ..Default::default()
            },
            PartyLeaderRow {
                country: 100,
                party: 9,
                ordinal: 1,
                character: Some(8),
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
    // A group the content spells in another case: the leader's lexer lookup
    // folds it, the party's name comparison does not.
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
            .contains("PartyLeaders")
    );
}

#[test]
fn genders_manpower_and_agency_preserve_native_empty_and_wrapping_rules() {
    let (catalogue, _folder) = tests::catalogue(
        "",
        concat!(
            " EV_SHE:0 \"she\"\n EV_HE:0 \"he\"\n EV_HER:0 \"her\"\n EV_HERS:0 \"hers\"\n EV_HERSELF:0 \"herself\"\n",
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
    // Every country holds an agency object: without a row the name is unknown.
    assert!(
        rendered(&catalogue, &f, "TEST")
            .unwrap_err()
            .contains("no IntelligenceAgencies row")
    );
    let agency = |name: &[u8], created| IntelligenceAgencyRow {
        country: 100,
        name: name.to_vec(),
        created,
        ..Default::default()
    };
    // Not created: the executable's key, whatever name is already stored.
    set::<tables::IntelligenceAgencies>(&mut f, vec![agency(b"Abwehr", 0)]);
    assert_eq!(
        rendered(&catalogue, &f, "TEST").unwrap(),
        "She|her|her|hers|Herself|-2147483647|No agency"
    );
    // Created: the stored bytes, empty included, never localised again.
    set::<tables::IntelligenceAgencies>(&mut f, vec![agency(b"", 1)]);
    assert!(rendered(&catalogue, &f, "TEST").unwrap().ends_with("647|"));
    set::<tables::IntelligenceAgencies>(&mut f, vec![agency(b"AGENCY_NAME", 1)]);
    assert!(
        rendered(&catalogue, &f, "TEST")
            .unwrap()
            .ends_with("|AGENCY_NAME")
    );
    // A controlled state without provinces may be missing from the array.
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
    // Empty text and keys the viewer localises are the same in every language.
    set::<tables::IntelligenceAgencies>(&mut f, vec![agency(b"", 1)]);
    assert_eq!(rendered(&catalogue, &f, "TEST").unwrap(), "");
    set::<tables::IntelligenceAgencies>(&mut f, vec![agency(b"Abwehr", 0)]);
    assert_eq!(rendered(&catalogue, &f, "TEST").unwrap(), "No agency");
}

#[test]
fn old_names_copy_the_retained_caches_or_the_current_name() {
    let (catalogue, _folder) = tests::catalogue(
        "",
        " TEST:0 \"[ROOT.GetOldName]|[ROOT.GetOldNameDefCap]|[ROOT.GetOldAdjective]|[ROOT.GetOldAdjectiveCap]\"\n",
    );
    let mut f = frame();
    let mut appearance = f.complete::<tables::CountryAppearance>().unwrap().to_vec();
    appearance[0].old_name = Some(b"Weimar Republic".to_vec());
    appearance[0].old_definite_name = Some(b"the Weimar Republic".to_vec());
    // An empty cache takes the current text, here in the viewer's language.
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
            .contains("GetOldAdjective: the old adjective (CCountry+0xB0) was unreadable")
    );
}

#[test]
fn date_election_and_support_formats_are_native_not_ui_formats() {
    let (catalogue, _folder) = tests::catalogue(
        "",
        concat!(
            " February:0 \"Mod-month\"\n",
            " TEST:0 \"[GetDateText]|[GetDate]|[GetYear]|[GetMonth]|[ROOT.GetLastElection]|[ROOT.GetPartySupport]\"\n"
        ),
    );
    let mut f = frame();
    f.hour = engine::cdate::HOUR_EPOCH + 1936 * 8760 + 31 * 24 + 23;
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
    assert!(
        content
            .environment
            .set(Ok(entries::Environment::synthetic()))
            .is_ok()
    );
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
    let (catalogue, _folder) = tests::catalogue(
        "",
        " MOD_STATE:0 \"A $PART$\"\n PART:0 \"state\"\n TEST:0 \"[ROOT.Capital.GetName]\"\n",
    );
    let mut environment = entries::Environment::synthetic();
    environment.state_names.insert(64, Ok("MOD_STATE".into()));
    assert!(
        catalogue
            .scripted_content()
            .unwrap()
            .environment
            .set(Ok(environment))
            .is_ok()
    );
    let mut f = frame();
    assert_eq!(rendered(&catalogue, &f, "TEST").unwrap(), "A state");
    // The owner name, then the custom name, take precedence as stored.
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
            daily_flags: engine::modifier_definition::CLAMP_MAXIMUM,
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
        0,
    );
    assert_eq!(
        at.expand("[ROOT.GetBopTrendTextIcon]").unwrap(),
        "\u{13}BoP_right_texticon "
    );
}

#[test]
fn character_pronouns_and_literal_names_differ_from_party_display_names() {
    let (catalogue, _folder) = tests::catalogue(
        "",
        concat!(
            " EV_HE:0 \"he\"\n EV_HIM:0 \"him\"\n EV_HIS:0 \"his\"\n EV_HIMSELF:0 \"himself\"\n",
            " TEST:0 \"[ROOT.GER_erwin_rommel.GetName]|[ROOT.GER_erwin_rommel.GetSheHeCap]|[ROOT.GER_erwin_rommel.GetHerHimCap]|[ROOT.GER_erwin_rommel.GetHerHisCap]|[ROOT.GER_erwin_rommel.GetHersHisCap]|[ROOT.GER_erwin_rommel.GetHerselfHimself]\"\n"
        ),
    );
    let mut f = frame();
    // Unknown enum values take the non-female branch, exactly like gender 1.
    let mut characters = f.complete::<tables::Characters>().unwrap().to_vec();
    characters[0].gender = u32::MAX;
    set::<tables::Characters>(&mut f, characters.clone());
    assert_eq!(
        rendered(&catalogue, &f, "TEST").unwrap(),
        "|He|Him|His|His|himself"
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
    let (catalogue, _folder) = tests::catalogue(
        r#"entry = { name = Country text = { trigger = { tag = SOV ROOT = { tag = GER } FROM = { tag = GER } PREV = { tag = GER } } localization_key = YES } }
           entry = { name = State text = { trigger = { ROOT = { tag = GER } } localization_key = YES } }"#,
        " YES:0 \"pass\"\n TEST:0 \"[SOV.Country]|[ROOT.Capital.State]\"\n",
    );
    assert!(
        catalogue
            .scripted_content()
            .unwrap()
            .environment
            .set(Ok(entries::Environment::synthetic()))
            .is_ok()
    );
    assert_eq!(rendered(&catalogue, &frame(), "TEST").unwrap(), "pass|pass");
}

/// GER (slot 0), SOV (1) and ENG (2), each owning a state.
fn raid_frame() -> DecodedFrame {
    let mut f = frame();
    let countries = [(100, 0, "GER"), (200, 1, "SOV"), (300, 2, "ENG")];
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
    f
}

/// A viewer (GER), a raiding country (SOV) and its target (ENG): the raid's
/// scope is SOV with FROM a default scope of ENG (140A8D3F0 -> 141580600).
#[test]
fn raid_text_follows_the_active_event_scope_through_chains_and_entries() {
    let (catalogue, _folder) = tests::catalogue(
        r#"entry = { name = Side text = { trigger = { tag = SOV ROOT = { tag = SOV } FROM = { tag = ENG } PREV = { tag = SOV } } localization_key = YES } text = { localization_key = NO } }
           entry = { name = Target text = { trigger = { tag = ENG ROOT = { tag = ENG } FROM = { tag = ENG } PREV = { tag = ENG } } localization_key = YES } text = { localization_key = NO } }
           entry = { name = Named text = { trigger = { tag = ENG ROOT = { tag = SOV } FROM = { tag = ENG } PREV = { tag = SOV } } localization_key = YES } text = { localization_key = NO } }
           entry = { name = Echo text = { localization_key = ECHO } }"#,
        " YES:0 \"yes\"\n NO:0 \"no\"\n ECHO:0 \"[THIS.GetTag]/[ROOT.GetTag]/[PREV.GetTag]\"\n",
    );
    let content = catalogue.scripted_content().unwrap();
    assert!(
        content
            .environment
            .set(Ok(entries::Environment::synthetic()))
            .is_ok()
    );
    let f = raid_frame();
    let text = ScriptedText::new(&f, &catalogue.localisation, content, 0);
    let raid = EventScopes::new(vec![
        EventScope {
            from: Link::Scope(1),
            ..EventScope::of(EventObject::Country(1))
        },
        EventScope::of(EventObject::Country(2)),
    ]);
    let expand = |text_: &str, scopes: &EventScopes| text.expand_in(text_, scopes);
    // Each keyword moves the active scope (141A81250); each bracket starts
    // again from the caller's (1410D0900).
    assert_eq!(
        expand(
            "[THIS.GetTag] [ROOT.GetTag] [FROM.GetTag] [FROM.ROOT.GetTag] [ROOT.FROM.PREV.GetTag] [PREV.GetTag]",
            &raid
        )
        .as_deref(),
        Ok("SOV SOV ENG ENG ENG SOV")
    );
    // Entries see the active scope, not the viewer: in place (1410CE090),
    // after a keyword, and bound as THIS with PREV the active scope
    // (1410CDDF0).
    assert_eq!(
        expand("[Side] [FROM.Target] [ENG.Named]", &raid).as_deref(),
        Ok("yes yes yes")
    );
    assert_eq!(text.expand("[Side]").as_deref(), Ok("no"));
    // An entry's own brackets expand in the scope it was evaluated in
    // (1410CE090 -> 1402F6340 -> 1410CF560), not the caller's.
    assert_eq!(
        expand("[Echo] [FROM.Echo] [ENG.Echo]", &raid).as_deref(),
        Ok("SOV/SOV/SOV ENG/ENG/ENG ENG/SOV/SOV")
    );
    // A scope whose own links the caller does not know stops a chain.
    let nested = EventScopes::new(vec![
        EventScope {
            this: EventObject::Country(1),
            root: Link::Scope(1),
            from: Link::Scope(2),
            prev: Link::Scope(1),
        },
        EventScope {
            this: EventObject::Country(1),
            root: Link::Unknown,
            from: Link::Unknown,
            prev: Link::Unknown,
        },
        EventScope {
            this: EventObject::Country(2),
            root: Link::Unknown,
            from: Link::Unknown,
            prev: Link::Unknown,
        },
    ]);
    assert_eq!(expand("[FROM.GetTag]", &nested).as_deref(), Ok("ENG"));
    let refused = expand("[FROM.ROOT.GetTag]", &nested).unwrap_err();
    assert!(refused.contains("Root link is not known"), "{refused}");
    let refused = expand("[FROM.Target]", &nested).unwrap_err();
    assert!(refused.contains("ROOT is not known"), "{refused}");
}

/// Scopes the caller cannot fully describe refuse only what reads them: an
/// unknown link only in a condition that reads it (`always` reads none,
/// 1403CE520), a state or character scope's missing country only where
/// 1410CF9C0 would search it, and a unit leader only through its character.
#[test]
fn unknown_scope_parts_refuse_only_where_they_are_read() {
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
    assert!(
        content
            .environment
            .set(Ok(entries::Environment::synthetic()))
            .is_ok()
    );
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
    let text = ScriptedText::new(&f, &catalogue.localisation, content, 0);
    let unknown = |this| {
        EventScopes::new(vec![EventScope {
            this,
            root: Link::Unknown,
            from: Link::Unknown,
            prev: Link::Unknown,
        }])
    };
    let sov = unknown(EventObject::Country(1));
    assert_eq!(
        text.expand_in("[Always] [ThisOnly] [SOV.ThisOnly]", &sov)
            .as_deref(),
        Ok("yes yes yes")
    );
    let refused = text.expand_in("[RootToo]", &sov).unwrap_err();
    assert!(refused.contains("ROOT is not known"), "{refused}");
    // The caller's script variables stay set for the whole expansion.
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
    // A state scope holds no country, so a number goes on to a state id.
    let state = unknown(EventObject::State(64));
    assert_eq!(
        text.expand_in("[THIS.64.GetID]", &state).as_deref(),
        Ok("64")
    );
    assert!(text.expand_in("[THIS.rommel.GetID]", &state).is_ok());
    // A unit leader's scope reads as its character (140529310).
    let leader = unknown(EventObject::UnitLeader(9 << 32 | 3));
    assert_eq!(
        text.expand_in("[THIS.GetName] [THIS.64.GetID]", &leader)
            .as_deref(),
        Ok("Erwin Rommel 64")
    );
    // A promoted country is bound with clearing (1410CDDF0 -> 14052F3F0), so
    // it carries no character and a character trigger is false whatever its
    // PREV (140542CA0 reads THIS alone).
    assert_eq!(text.expand_in("[SOV.Choice]", &sov).as_deref(), Ok("no"));
    assert_eq!(
        text.expand_in("[THIS.Holder]", &leader).as_deref(),
        Ok("no")
    );
    let refused = text
        .expand_in("[THIS.GetName]", &unknown(EventObject::Province(5)))
        .unwrap_err();
    assert!(refused.contains("province 5"), "{refused}");
    // A unit leader's scope holds no country (14052EFD0), so a number is a
    // state id without the character inventory (1410CF9C0).
    *f.tables[TableId::Characters.index()] = TableView::Unavailable {
        reason: "test gap".into(),
    };
    let text = ScriptedText::new(&f, &catalogue.localisation, content, 0);
    assert_eq!(
        text.expand_in("[THIS.64.GetID]", &leader).as_deref(),
        Ok("64")
    );
}

/// A promoted character (1410CDDF0) is THIS in a character scope: without
/// a unit leader `tag` compares the character's own country and `skill` is
/// 0; `has_trait` of a name no trait database holds is false; a country
/// trigger is false there (its scope mask rejects a character scope,
/// 140542CA0), so no alternative passes.
#[test]
fn promoted_character_entry_evaluates_in_a_character_scope() {
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
    assert!(
        catalogue
            .scripted_content()
            .unwrap()
            .environment
            .set(Ok(entries::Environment::synthetic()))
            .is_ok()
    );
    assert_eq!(
        rendered(&catalogue, &frame(), "TEST").unwrap(),
        "pass|fail|fail"
    );
    assert_eq!(rendered(&catalogue, &frame(), "OTHER").unwrap(), "");
}
