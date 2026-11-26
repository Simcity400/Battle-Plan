
use super::*;
use crate::script_test_game as game;
use crate::triggers::inputs::{
    Key,
    tests::{
        Change, changes_each_key_reads, check_mode_reports_each_wrong_declaration,
        tables_each_key_reads,
    },
};
use replay_data::{
    rows::CharacterRow, rows::CountryAppearanceRow, rows::FactionMemberRow, rows::FactionRow,
    rows::IntelligenceAgencyRow, rows::LexicalTokenRow, rows::LocalPlayerRow, rows::PartyLeaderRow,
    rows::PoliticalPartyRow, rows::PoliticsRow, rows::ProvinceRow, rows::Recorded,
    rows::ReservedCountryNamingRow, rows::TableId, rows::TableRows, rows::UnitLeaderRow,
};

fn set(frame: &mut DecodedFrame, rows: TableRows) {
    let table = rows.table();
    *frame.tables[table.index()] = TableView::Complete(rows);
}

fn text_frame() -> DecodedFrame {
    let mut frame = super::tests::complete_frame();
    set(
        &mut frame,
        TableRows::Politics(vec![
            PoliticsRow {
                country: 100,
                capital: 64,
                ruling_party: Some(3),
                last_election: 5,
                ..Default::default()
            },
            PoliticsRow {
                country: 200,
                capital: 64,
                ruling_party: Some(1),
                last_election: 7,
                ..Default::default()
            },
        ]),
    );
    set(
        &mut frame,
        TableRows::CountryAppearance(
            [(100, 0), (200, 100)]
                .map(|(country, overlord)| CountryAppearanceRow {
                    country,
                    overlord,
                    original_tag_reference: Recorded::Value(Some(0)),
                    original_tag: Recorded::Value(Some(Vec::new())),
                    old_name: Some(Vec::new()),
                    old_definite_name: Some(Vec::new()),
                    old_adjective: Some(Vec::new()),
                    ..Default::default()
                })
                .into(),
        ),
    );
    set(
        &mut frame,
        TableRows::Characters(vec![
            CharacterRow {
                id: 7,
                country: 100,
                name_token_id: 9000,
                name: b"Rommel".to_vec(),
                token: b"Erwin Rommel".to_vec(),
                gender: 1,
                ..Default::default()
            },
            CharacterRow {
                id: 8,
                country: 200,
                name_token_id: 9001,
                name: b"Zhukov".to_vec(),
                ..Default::default()
            },
        ]),
    );
    set(
        &mut frame,
        TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 9000,
                literal: b"GER_erwin_rommel".to_vec(),
            },
            LexicalTokenRow {
                token: 9001,
                literal: b"SOV_georgy_zhukov".to_vec(),
            },
        ]),
    );
    set(
        &mut frame,
        TableRows::PoliticalParties(vec![
            PoliticalPartyRow {
                country: 100,
                id: 3,
                popularity: 60,
                is_ruling: 1,
                token: b"fascism".to_vec(),
                tag: 100,
                name_default: 1,
                ..Default::default()
            },
            PoliticalPartyRow {
                country: 100,
                id: 4,
                popularity: 40,
                token: b"democratic".to_vec(),
                tag: 100,
                name_default: 1,
                ..Default::default()
            },
            PoliticalPartyRow {
                country: 200,
                id: 1,
                popularity: 100,
                is_ruling: 1,
                token: b"communism".to_vec(),
                long_name: b"SOV".to_vec(),
                ..Default::default()
            },
        ]),
    );
    set(
        &mut frame,
        TableRows::PartyLeaders(
            [(100, 3, 7), (200, 1, 8)]
                .map(|(country, party, character)| PartyLeaderRow {
                    country,
                    party,
                    ordinal: 0,
                    character: Some(character),
                    role: Some(1),
                    leader_id: 2,
                })
                .into(),
        ),
    );
    set(
        &mut frame,
        TableRows::Provinces(vec![ProvinceRow {
            id: 5,
            state: 64,
            owner: 100,
            controller: 100,
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::Factions(
            [(11, "Axis"), (12, "Comintern")]
                .map(|(id, name)| FactionRow {
                    id,
                    name: name.into(),
                    ..Default::default()
                })
                .into(),
        ),
    );
    set(
        &mut frame,
        TableRows::FactionMembers(
            [(100, 11), (200, 12)]
                .map(|(country, faction)| FactionMemberRow {
                    country,
                    faction,
                    ordinal: 0,
                })
                .into(),
        ),
    );
    set(
        &mut frame,
        TableRows::IntelligenceAgencies(
            [(100, "Abwehr"), (200, "NKVD")]
                .map(|(country, name)| IntelligenceAgencyRow {
                    country,
                    created: 1,
                    name: name.into(),
                    ..Default::default()
                })
                .into(),
        ),
    );
    set(
        &mut frame,
        TableRows::LocalPlayer(vec![LocalPlayerRow {
            country: 100,
            observer_country: 0,
        }]),
    );
    set(
        &mut frame,
        TableRows::UnitLeaders(vec![UnitLeaderRow {
            id: 1,
            reference_type: 4713,
            character_id: 7,
            owner: 100,
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::ReservedCountryNaming(vec![ReservedCountryNamingRow {
            tag: b"ZZ0".to_vec(),
            original_tag_reference: Recorded::Value(Some(0)),
            original_tag: Recorded::Value(Some(Vec::new())),
            faction: Recorded::Value(Some(0)),
            built: 7,
            ..Default::default()
        }]),
    );
    frame
}

fn declared_keys() -> Vec<Key> {
    let leader = EventObject::UnitLeader(1 << 32 | 4713);
    let texts = [
        TextQuery::CountryName(1, false),
        TextQuery::CountryName(2, true),
        TextQuery::CountryTag(1),
        TextQuery::CountryTag(0),
        TextQuery::CharacterProperty(7, "GetName".into()),
        TextQuery::CharacterProperty(7, "GetSheHe".into()),
        TextQuery::CharacterProperty(7, "GetIdeology".into()),
        TextQuery::CharacterName(8),
        TextQuery::Capital(1, false),
        TextQuery::Capital(1, true),
        TextQuery::Overlord(2),
        TextQuery::StateRecord(64),
        TextQuery::StateCountry(64, true),
        TextQuery::StateCountry(64, false),
        TextQuery::CharacterCountry(7),
        TextQuery::EventObject(leader, "THIS".into()),
        TextQuery::EventObject(EventObject::Country(1), "THIS".into()),
        TextQuery::LocalPlayer,
        TextQuery::RulingParty(1, "GER".into(), false),
        TextQuery::RulingParty(2, "SOV".into(), true),
        TextQuery::FactionName(1),
        TextQuery::FactionName(0),
        TextQuery::FactionNameById(11),
        TextQuery::CharacterNamed(
            Object::Country(1),
            "GER_erwin_rommel".into(),
            EventObject::Country(1),
        ),
        TextQuery::StateNameFor(64, false),
        TextQuery::StateNameFor(64, true),
        TextQuery::ProvinceDisplayNamePrimary(5),
        TextQuery::ProvinceDisplayName(5),
        TextQuery::ProvinceNameOverride(5),
        TextQuery::ProvinceVictoryPointName(5),
        TextQuery::PartySupport(1),
        TextQuery::PartyLeader(1, None),
        TextQuery::PartyLeader(1, Some("democratic".into())),
        TextQuery::NamedPartyName(1, "fascism".into(), "GER".into()),
        TextQuery::LastElection(1),
        TextQuery::Manpower(1),
        TextQuery::Agency(1),
        TextQuery::OldName(1, "GetOldName".into(), String::new()),
        TextQuery::OldName(2, "GetOldNameDef".into(), "_DEF".into()),
        TextQuery::CharacterGender(8),
        TextQuery::CountryPronoun(1, "GetSheHe".into()),
        TextQuery::DateProperty("GetFoo".into()),
        TextQuery::CountriesGap,
    ]
    .map(Key::Text);
    let names = [
        recorded_names::Query::Operands(1),
        recorded_names::Query::Name(1),
        recorded_names::Query::NameWith(2, "_DEF".into()),
        recorded_names::Query::RulingGroup(1),
        recorded_names::Query::ReservedBuilt,
    ]
    .map(Key::Names);
    texts.into_iter().chain(names).collect()
}

const CONSTANT: [&str; 3] = [
    "Text(CharacterProperty(7, \"GetIdeology\"))",
    "Text(EventObject(Country(1), \"THIS\"))",
    "Text(DateProperty(\"GetFoo\"))",
];

fn with_texts(check: impl FnOnce(&dyn Fn(&DecodedFrame, &[Key]) -> Vec<Option<u64>>)) {
    game::enter();
    let (catalogue, _folder) = super::tests::catalogue(
        "defined_text = { name = GetModText text = { localization_key = GER } }",
        "",
    );
    let content = catalogue.scripted_content().unwrap();
    let digests = |frame: &DecodedFrame, keys: &[Key]| -> Vec<Option<u64>> {
        let text = ScriptedText::new(frame, &catalogue.localisation, content, 1);
        keys.iter().map(|key| text.reread(key)).collect()
    };
    check(&digests);
}

#[test]
fn a_declared_text_or_name_read_reads_no_table_it_does_not_declare() {
    with_texts(|digests| {
        let keys = declared_keys();
        let tables = tables_each_key_reads(&text_frame(), &keys, digests);
        let constant: Vec<String> = keys
            .iter()
            .enumerate()
            .filter(|(at, _)| !tables.contains_key(at))
            .map(|(_, key)| format!("{key:?}"))
            .collect();
        assert_eq!(constant, CONSTANT);
    });
}

#[test]
fn a_declared_text_or_name_read_reads_no_field_or_row_it_does_not_declare() {
    use TableId as T;
    with_texts(|digests| {
        let changes = vec![
            Change::new(
                T::Countries,
                &["political_power_raw", "stability_raw"],
                None,
                |rows| {
                    if let TableRows::Countries(rows) = rows {
                        for row in rows {
                            row.political_power_raw = Some(4);
                            row.stability_raw = 5;
                        }
                    }
                },
            ),
            Change::new(
                T::PoliticalParties,
                &["popularity"],
                Some(200 << 32 | 1),
                |rows| {
                    if let TableRows::PoliticalParties(rows) = rows {
                        rows[2].popularity = 7;
                    }
                },
            ),
            Change::new(
                T::Politics,
                &["last_election", "capital"],
                Some(200),
                |rows| {
                    if let TableRows::Politics(rows) = rows {
                        rows[1].last_election = 9;
                        rows[1].capital = 9;
                    }
                },
            ),
            Change::new(
                T::PartyLeaders,
                &["character"],
                Some(200 << 64 | 1 << 32),
                |rows| {
                    if let TableRows::PartyLeaders(rows) = rows {
                        rows[1].character = Some(7);
                    }
                },
            ),
            Change::new(
                T::CountryAppearance,
                &["old_name", "red"],
                Some(200),
                |rows| {
                    if let TableRows::CountryAppearance(rows) = rows {
                        rows[1].old_name = Some(b"Old".to_vec());
                        rows[1].red = 9;
                    }
                },
            ),
            Change::new(T::IntelligenceAgencies, &["name"], Some(200), |rows| {
                if let TableRows::IntelligenceAgencies(rows) = rows {
                    rows[1].name = b"Cheka".to_vec();
                }
            }),
            Change::new(T::FactionMembers, &["faction"], Some(200), |rows| {
                if let TableRows::FactionMembers(rows) = rows {
                    rows[1].faction = 11;
                }
            }),
            Change::new(
                T::Characters,
                &["roster", "ordinal", "template_line"],
                None,
                |rows| {
                    if let TableRows::Characters(rows) = rows {
                        for row in rows {
                            row.roster = 1;
                            row.ordinal = 2;
                            row.template_line = 3;
                        }
                    }
                },
            ),
            Change::new(T::Characters, &["gender"], Some(8), |rows| {
                if let TableRows::Characters(rows) = rows {
                    rows[1].gender = 2;
                }
            }),
            Change::new(T::Factions, &["name"], Some(12), |rows| {
                if let TableRows::Factions(rows) = rows {
                    rows[1].name = b"Other".to_vec();
                }
            }),
            Change::new(T::States, &["movement_infrastructure"], None, |rows| {
                if let TableRows::States(rows) = rows {
                    rows[0].movement_infrastructure = 4;
                }
            }),
            Change::new(T::Provinces, &["victory_points", "owner"], None, |rows| {
                if let TableRows::Provinces(rows) = rows {
                    rows[0].victory_points = 4;
                    rows[0].owner = 200;
                }
            }),
            Change::new(T::UnitLeaders, &["skill_level", "owner"], None, |rows| {
                if let TableRows::UnitLeaders(rows) = rows {
                    rows[0].skill_level = 4;
                    rows[0].owner = 200;
                }
            }),
            Change::new(
                T::ReservedCountryNaming,
                &["dynamic_tag_boundary"],
                None,
                |rows| {
                    if let TableRows::ReservedCountryNaming(rows) = rows {
                        rows[0].dynamic_tag_boundary = Recorded::Value(Some(4));
                    }
                },
            ),
            Change::new(T::Politics, &["ruling_party"], Some(100), |rows| {
                if let TableRows::Politics(rows) = rows {
                    rows[0].ruling_party = Some(4);
                }
            }),
        ];
        let differed = changes_each_key_reads(&text_frame(), &declared_keys(), digests, &changes);
        assert!(differed > 0, "a read field changes some read");
    });
}

#[test]
fn check_mode_reports_a_wrong_declaration_of_every_text_and_name_read() {
    with_texts(|digests| {
        let frame = text_frame();
        let keys = declared_keys();
        let tables = tables_each_key_reads(&frame, &keys, digests);
        assert_eq!(tables.len(), keys.len() - CONSTANT.len());
        check_mode_reports_each_wrong_declaration(&frame, &keys, &tables, digests);
    });
}
