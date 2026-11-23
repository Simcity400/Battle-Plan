use super::*;
use replay_data::rows::{
    CharacterRow, CountryCharacterRow, PartyLeaderRow, PoliticalPartyRow, PoliticsRow,
    UnitLeaderRow,
};

fn leader_frame() -> DecodedFrame {
    let mut frame = frame(
        &[
            (100, 1, "AAA", 1, "AAA", ""),
            (101, 1, "AAA", 1, "AAA", ""),
            (200, 2, "GER", 1, "GER", ""),
        ],
        &[],
    );
    *frame.tables[TableId::Characters.index()] = TableView::Complete(TableRows::Characters(
        [7, 8, 9, 10]
            .into_iter()
            .map(|id| CharacterRow {
                id,
                country: if id == 10 { 200 } else { 101 },
                name_token_id: 900 + id as i32,
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::Politics.index()] = TableView::Complete(TableRows::Politics(
        [100, 200]
            .into_iter()
            .map(|country| PoliticsRow {
                country,
                ruling_party: Some(20),
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(
            [(100, 10, 0), (100, 20, 1), (200, 20, 1)]
                .into_iter()
                .map(|(country, id, is_ruling)| PoliticalPartyRow {
                    country,
                    id,
                    is_ruling,
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::PartyLeaders.index()] = TableView::Complete(TableRows::PartyLeaders(
        [
            (100, 10, 0, 9),
            (100, 20, 0, 7),
            (100, 20, 1, 8),
            (200, 20, 0, 10),
        ]
        .into_iter()
        .map(|(country, party, ordinal, character)| PartyLeaderRow {
            country,
            party,
            ordinal,
            character: Some(character),
            role: Some(party as i32),
            ..Default::default()
        })
        .collect(),
    ));
    frame
}

#[test]
fn is_country_leader_compares_only_the_ruling_partys_first_leader() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let frame = leader_frame();
    with_world(&frame, "", Err("none"), |world| {
        for (character, leader) in [(7, true), (8, false), (9, false)] {
            for (operand, expected) in [("yes", leader), ("no", !leader)] {
                assert_eq!(
                    world.evaluate(
                        &items(&format!("is_country_leader = {operand}")),
                        Scopes::of(Scope::Character(character)),
                    ),
                    Eval::from_bool(expected),
                    "character {character}, {operand}"
                );
            }
        }
    });
}

#[test]
fn is_country_leader_is_false_for_absent_or_retired_characters_for_both_operands() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut frame = leader_frame();
    *frame.tables[TableId::CountryRetiredCharacters.index()] =
        TableView::Complete(TableRows::CountryRetiredCharacters(
            [7, 8]
                .into_iter()
                .enumerate()
                .map(|(ordinal, character)| CountryCharacterRow {
                    country: 100,
                    ordinal: ordinal as u32,
                    character: Some(character),
                })
                .collect(),
        ));
    *frame.tables[TableId::PartyLeaders.index()] = TableView::Unavailable {
        reason: "unreached party leaders".into(),
    };
    with_world(&frame, "", Err("none"), |world| {
        for character in [0, 7, 8] {
            for operand in ["yes", "no"] {
                assert_eq!(
                    world.evaluate(
                        &items(&format!("is_country_leader = {operand}")),
                        Scopes::of(Scope::Character(character)),
                    ),
                    Eval::False,
                    "character {character}, {operand}"
                );
            }
        }
    });
}

#[test]
fn is_country_leader_is_false_without_a_ruling_leader_for_both_operands() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    for no_ruling_party in [false, true] {
        let mut frame = leader_frame();
        if no_ruling_party {
            let TableView::Complete(TableRows::PoliticalParties(rows)) =
                &mut *frame.tables[TableId::PoliticalParties.index()]
            else {
                unreachable!()
            };
            for row in rows.iter_mut().filter(|row| row.country == 100) {
                row.is_ruling = 0;
            }
        } else {
            let TableView::Complete(TableRows::PartyLeaders(rows)) =
                &mut *frame.tables[TableId::PartyLeaders.index()]
            else {
                unreachable!()
            };
            rows.retain(|row| row.country != 100 || row.party != 20);
        }
        with_world(&frame, "", Err("none"), |world| {
            for operand in ["yes", "no"] {
                assert_eq!(
                    world.evaluate(
                        &items(&format!("is_country_leader = {operand}")),
                        Scopes::of(Scope::Character(7)),
                    ),
                    Eval::False,
                    "no ruling party {no_ruling_party}, {operand}"
                );
            }
        });
    }
}

#[test]
fn is_country_leader_uses_the_characters_canonical_country_over_the_scope_country() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut frame = leader_frame();
    let leader = (1u64 << 32) | 1;
    *frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![UnitLeaderRow {
            id: 1,
            reference_type: 1,
            owner: 200,
            character_id: 7,
            ..Default::default()
        }]));
    with_world(&frame, "", Err("none"), |world| {
        for scopes in [
            Scopes::of(Scope::Country(2)).with_this(Scope::Character(7)),
            Scopes::of(Scope::Leader(leader)),
            Scopes::of(Scope::CountryLeader { country: 2, leader }),
        ] {
            assert_eq!(
                world.evaluate(&items("is_country_leader = yes"), scopes),
                Eval::True
            );
            assert_eq!(
                world.evaluate(&items("is_country_leader = no"), scopes),
                Eval::False
            );
        }
    });
}

#[test]
fn is_country_leader_refuses_each_unrecorded_input_by_name() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    for (table, name) in [
        (TableId::Characters, "Characters"),
        (
            TableId::CountryRetiredCharacters,
            "CountryRetiredCharacters",
        ),
        (TableId::PoliticalParties, "PoliticalParties"),
        (TableId::PartyLeaders, "PartyLeaders"),
    ] {
        let mut frame = leader_frame();
        *frame.tables[table.index()] = TableView::Unavailable {
            reason: format!("{name} synthetic gap"),
        };
        with_world(&frame, "", Err("none"), |world| {
            for operand in ["yes", "no"] {
                let result = world.evaluate(
                    &items(&format!("is_country_leader = {operand}")),
                    Scopes::of(Scope::Character(7)),
                );
                assert!(
                    matches!(&result, Eval::Unknown(reasons) if reasons.iter().any(|reason|
                        reason.contains("is_country_leader")
                            && reason.contains(&format!("{name} synthetic gap")))),
                    "{name}, {operand}: {result:?}"
                );
            }
        });
    }
}

#[test]
fn is_country_leader_refuses_missing_or_incomplete_politics_with_the_country() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    for politics in [
        TableView::Unavailable {
            reason: "synthetic politics gap".into(),
        },
        TableView::Truncated {
            rows: TableRows::Politics(vec![PoliticsRow {
                country: 100,
                ruling_party: Some(20),
                ..Default::default()
            }]),
            missing: 1,
        },
    ] {
        let mut frame = leader_frame();
        *frame.tables[TableId::Politics.index()] = politics;
        with_world(&frame, "", Err("none"), |world| {
            for operand in ["yes", "no"] {
                let result = world.evaluate(
                    &items(&format!("is_country_leader = {operand}")),
                    Scopes::of(Scope::Country(2)).with_this(Scope::Character(7)),
                );
                assert!(
                    matches!(&result, Eval::Unknown(reasons) if reasons.iter().any(|reason|
                        reason.contains("is_country_leader")
                            && reason.contains("Politics")
                            && reason.contains("country slot 1"))),
                    "{operand}: {result:?}"
                );
            }
        });
    }
}

#[test]
fn is_country_leader_refuses_a_missing_politics_row_for_its_canonical_country() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut frame = leader_frame();
    for table in [
        TableId::Politics,
        TableId::PoliticalParties,
        TableId::PartyLeaders,
    ] {
        match &mut *frame.tables[table.index()] {
            TableView::Complete(TableRows::Politics(rows)) => {
                rows.retain(|row| row.country == 200);
            }
            TableView::Complete(TableRows::PoliticalParties(rows)) => {
                rows.retain(|row| row.country == 200);
            }
            TableView::Complete(TableRows::PartyLeaders(rows)) => {
                rows.retain(|row| row.country == 200);
            }
            _ => unreachable!(),
        }
    }
    with_world(&frame, "", Err("none"), |world| {
        for operand in ["yes", "no"] {
            let result = world.evaluate(
                &items(&format!("is_country_leader = {operand}")),
                Scopes::of(Scope::Country(2)).with_this(Scope::Character(7)),
            );
            assert!(
                matches!(&result, Eval::Unknown(reasons) if reasons.iter().any(|reason|
                    reason.contains("is_country_leader")
                        && reason.contains("Politics lacks country slot 1"))),
                "{operand}: {result:?}"
            );
        }
    });
}

#[test]
fn is_country_leader_is_false_for_a_recorded_none_ruling_party_for_both_operands() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut frame = leader_frame();
    let TableView::Complete(TableRows::Politics(rows)) =
        &mut *frame.tables[TableId::Politics.index()]
    else {
        unreachable!()
    };
    rows[0].ruling_party = None;
    *frame.tables[TableId::PartyLeaders.index()] = TableView::Unavailable {
        reason: "unreached party leaders".into(),
    };
    with_world(&frame, "", Err("none"), |world| {
        for operand in ["yes", "no"] {
            assert_eq!(
                world.evaluate(
                    &items(&format!("is_country_leader = {operand}")),
                    Scopes::of(Scope::Character(7)),
                ),
                Eval::False,
                "{operand}"
            );
        }
    });
}

#[test]
fn is_country_leader_reevaluates_when_only_the_recorded_ruling_party_changes() {
    use super::super::inputs::{Key, Since, TableChange, TableChanges, TableRead, capture};

    struct PoliticsChanged(u64);
    impl TableChanges for PoliticsChanged {
        fn table(&self, table: TableId) -> TableChange<'_> {
            if table == TableId::Politics {
                TableChange::Fields(self.0)
            } else {
                TableChange::Same
            }
        }
    }

    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let ruling_read = TableRead::fields(TableId::Politics, &["ruling_party"]);
    for initially_ruling in [false, true] {
        let mut first = leader_frame();
        let TableView::Complete(TableRows::Politics(rows)) =
            &mut *first.tables[TableId::Politics.index()]
        else {
            unreachable!()
        };
        rows[0].ruling_party = initially_ruling.then_some(20);
        let mut second = first.clone();
        second.hour += 1;
        let TableView::Complete(TableRows::Politics(rows)) =
            &mut *second.tables[TableId::Politics.index()]
        else {
            unreachable!()
        };
        rows[0].ruling_party = (!initially_ruling).then_some(20);
        let changes = PoliticsChanged(ruling_read.fields);
        let since = Since::hour_before(&second, &changes).unwrap();
        for (character, operand) in [(7, "yes"), (8, "no")] {
            let trigger = items(&format!("is_country_leader = {operand}"));
            let scopes = Scopes::of(Scope::Character(character));
            let (value, mut inputs) = with_world(&first, "", Err("none"), |world| {
                capture(&first, || world.evaluate(&trigger, scopes))
            });
            assert_eq!(value, Eval::from_bool(initially_ruling));
            assert!(
                inputs
                    .keys()
                    .filter_map(Key::dependence)
                    .flatten()
                    .any(|read| read == ruling_read)
            );
            assert!(!inputs.unchanged(&since));
            let mut rereads = 0;
            with_world(&second, "", Err("none"), |world| {
                assert!(!inputs.confirm_since(&second, Some(&since), |key| {
                    match key {
                        Key::World(query) => {
                            if matches!(query, super::super::world::Query::RulingParty(1)) {
                                rereads += 1;
                            }
                            Some(world.reread(query))
                        }
                        _ => None,
                    }
                }));
                assert_eq!(
                    world.evaluate(&trigger, scopes),
                    Eval::from_bool(!initially_ruling)
                );
            });
            assert_eq!(rereads, 1);
        }
    }
}

#[test]
fn is_country_leader_refuses_operands_other_than_yes_or_no() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let frame = leader_frame();
    with_world(&frame, "", Err("none"), |world| {
        for operand in ["maybe", "YES", "THIS", "{}"] {
            let result = world.evaluate(
                &items(&format!("is_country_leader = {operand}")),
                Scopes::of(Scope::Character(8)),
            );
            assert!(
                matches!(&result, Eval::Unknown(reasons) if reasons.iter().any(|reason|
                    reason.contains("is_country_leader needs yes or no"))),
                "{operand}: {result:?}"
            );
        }
    });
}

#[test]
fn ita_advisor_availability_requires_a_nonleader_without_the_new_duce_flag() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut key_names = names();
    key_names
        .scripted
        .insert(b"ita_is_advisor_available".to_vec());
    let names = TriggerNames {
        scripted: content_reader_script::scripted_trigger_grammar::ScriptedTriggers::parse(
            &[(
                "common/scripted_triggers/synthetic.txt",
                b"ITA_is_advisor_available = { is_country_leader = no NOT = { has_character_flag = ITA_has_new_duce_flag } }",
            )],
            &key_names,
        ),
        ..Default::default()
    };
    let trigger =
        content_reader_script::triggers::read(b"ITA_is_advisor_available = yes", &key_names).items;
    let dlcs = BTreeSet::new();
    let aliases = Arc::new(TagAliases::default());
    for flagged in [false, true] {
        let mut frame = leader_frame();
        let TableView::Complete(TableRows::LexicalTokens(tokens)) =
            &mut *frame.tables[TableId::LexicalTokens.index()]
        else {
            unreachable!()
        };
        tokens.push(LexicalTokenRow {
            token: 901,
            literal: b"ITA_has_new_duce_flag".to_vec(),
        });
        if flagged {
            *frame.tables[TableId::ScriptFlags.index()] =
                TableView::Complete(TableRows::ScriptFlags(
                    [7, 8, 9]
                        .into_iter()
                        .map(|scope_id| ScriptFlagRow {
                            scope_kind: world::CHARACTER_FLAGS,
                            scope_id,
                            token: 901,
                            value: 1,
                            ..Default::default()
                        })
                        .collect(),
                ));
        }
        let world = World::read(&frame, ContentFacts::content_only(&dlcs, &names, &aliases));
        for character in [7, 8, 9] {
            assert_eq!(
                world.evaluate(&trigger, Scopes::of(Scope::Character(character))),
                Eval::from_bool(character != 7 && !flagged),
                "character {character}, flagged {flagged}"
            );
        }
    }
}
