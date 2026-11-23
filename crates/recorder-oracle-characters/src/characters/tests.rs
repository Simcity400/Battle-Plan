use super::*;
use content_reader_save::{Decimal, GameDate};
use replay_data::{
    rows::ActiveAbilityRow, rows::CharacterRow, rows::Recorded, rows::TableId, rows::TableRows,
    rows::UnitLeaderRow, rows::UnitLeaderTraitRow,
};
use replay_frame::{frame::Metrics, frame::TableView};

const NO_TRAITS: Result<&LeaderTraits, &str> = Err("outside this fixture");

#[test]
fn current_leader_ideology_compares_by_signed_map_order() {
    use replay_data::rows::CountryLeaderRoleRow;
    let (mut frame, mut pack, country) = fixture();
    let manager = pack.manager.as_mut().unwrap();
    manager
        .characters
        .get_mut(&handle(73, u32::MAX))
        .unwrap()
        .country_leaders = vec![
        saved::CountryLeader {
            ideology: b"rewritten".to_vec(),
            id: None,
            traits: Vec::new(),
        },
        saved::CountryLeader {
            ideology: b"second".to_vec(),
            id: None,
            traits: Vec::new(),
        },
    ];
    let links = BTreeMap::from([(102, &country)]);
    let tokens = BTreeMap::from([(41, b"rewritten".as_slice()), (42, b"second".as_slice())]);
    let mut rows = vec![
        CountryLeaderRoleRow {
            character: u32::MAX,
            country: 102,
            ideology: 20,
            current_ideology: Recorded::Value(Some(Some(42))),
        },
        CountryLeaderRoleRow {
            character: u32::MAX,
            country: 102,
            ideology: -7,
            current_ideology: Recorded::Value(Some(Some(41))),
        },
    ];
    *frame.tables[TableId::CountryLeaderRoles.index()] =
        TableView::Complete(TableRows::CountryLeaderRoles(rows.clone()));
    let mut report = Report::default();
    compare_country_leader_roles(&frame, manager, &links, &tokens, &mut report).unwrap();
    assert!(report.differences.is_empty());
    rows[1].current_ideology = Recorded::Value(Some(Some(42)));
    *frame.tables[TableId::CountryLeaderRoles.index()] =
        TableView::Complete(TableRows::CountryLeaderRoles(rows.clone()));
    compare_country_leader_roles(&frame, manager, &links, &tokens, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("/-7.current_ideology"));
    rows[1].current_ideology = Recorded::Value(Some(None));
    *frame.tables[TableId::CountryLeaderRoles.index()] =
        TableView::Complete(TableRows::CountryLeaderRoles(rows.clone()));
    let mut report = Report::default();
    compare_country_leader_roles(&frame, manager, &links, &tokens, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("native null ideology"));
    rows[1].current_ideology = Recorded::Value(None);
    *frame.tables[TableId::CountryLeaderRoles.index()] =
        TableView::Complete(TableRows::CountryLeaderRoles(rows.clone()));
    let mut report = Report::default();
    compare_country_leader_roles(&frame, manager, &links, &tokens, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .contains_key("current country-leader ideology unavailable from native capture")
    );
    rows[1].current_ideology = Recorded::NotRecorded;
    *frame.tables[TableId::CountryLeaderRoles.index()] =
        TableView::Complete(TableRows::CountryLeaderRoles(rows));
    let mut report = Report::default();
    compare_country_leader_roles(&frame, manager, &links, &tokens, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .contains_key("CountryLeaderRoles.current_ideology not recorded")
    );
}

#[test]
fn advisor_tooltip_keys_check_content_without_withholding_existing_fields() {
    use replay_data::{rows::AdvisorRoleRow, rows::CharacterRoleTraitRow};
    let (mut frame, mut pack, country) = fixture();
    let manager = pack.manager.as_mut().unwrap();
    manager
        .characters
        .get_mut(&handle(73, u32::MAX))
        .unwrap()
        .advisors = vec![saved::Advisor {
        slot: None,
        idea: None,
        traits: vec![b"synthetic".to_vec()],
    }];
    let traits = leader_traits(
        "leader_traits={synthetic={custom_modifier_tooltip=SYN_A custom_modifier_tooltip=SYN_B}}",
    );
    let tokens = BTreeMap::from([(31, b"synthetic".as_slice())]);
    let links = BTreeMap::from([(102, &country)]);
    let mut row = AdvisorRoleRow {
        character: u32::MAX,
        country: 102,
        tooltip_keys: Recorded::Value(Some(vec![b"SYN_B".to_vec(), b"SYN_A".to_vec()])),
        ..Default::default()
    };
    *frame.tables[TableId::CharacterRoleTraits.index()] =
        TableView::Complete(TableRows::CharacterRoleTraits(vec![
            CharacterRoleTraitRow {
                character: u32::MAX,
                kind: CharacterRoleTraitRow::ADVISOR,
                role: 0,
                tokens: vec![31],
                definitions: vec![1],
            },
        ]));
    let run = |frame: &DecodedFrame| {
        let mut report = Report::default();
        compare_advisors(frame, manager, &links, &tokens, Ok(&traits), &mut report).unwrap();
        report
    };
    *frame.tables[TableId::AdvisorRoles.index()] =
        TableView::Complete(TableRows::AdvisorRoles(vec![row.clone()]));
    let report = run(&frame);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(
        report.compared_tables["AdvisorRoles.tooltip_keys (content membership)"],
        2
    );
    row.tooltip_keys = Recorded::Value(Some(vec![b"unknown".to_vec()]));
    *frame.tables[TableId::AdvisorRoles.index()] =
        TableView::Complete(TableRows::AdvisorRoles(vec![row.clone()]));
    assert_eq!(run(&frame).differences.len(), 1);
    *frame.tables[TableId::CharacterRoleTraits.index()] = TableView::Unavailable {
        reason: "unreadable".into(),
    };
    let report = run(&frame);
    assert!(report.differences.is_empty());
    assert_eq!(report.compared_tables["AdvisorRoles"], 1);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("lacks CharacterRoleTraits"))
    );
}

#[test]
fn advisor_modifier_cache_distinguishes_not_recorded_from_native_unavailable() {
    use replay_data::{rows::AdvisorModifierCache, rows::AdvisorRoleRow};
    let (mut frame, mut pack, country) = fixture();
    let manager = pack.manager.as_mut().unwrap();
    let character = manager.characters.get_mut(&handle(73, u32::MAX)).unwrap();
    character.advisors = vec![saved::Advisor {
        slot: None,
        idea: None,
        traits: Vec::new(),
    }];
    *frame.tables[TableId::AdvisorRoles.index()] =
        TableView::Complete(TableRows::AdvisorRoles(vec![AdvisorRoleRow {
            character: u32::MAX,
            country: 102,
            ..Default::default()
        }]));
    let links = BTreeMap::from([(102, &country)]);
    for (cache, reason) in [
        (
            Recorded::Value(Some(AdvisorModifierCache::default())),
            "the advisor save writer saves traits and the own modifier block, not the independently rebuilt modifier cache",
        ),
        (
            Recorded::Value(None),
            "AdvisorRoles.modifier_cache is explicitly unavailable",
        ),
        (
            Recorded::NotRecorded,
            "AdvisorRoles.modifier_cache not recorded",
        ),
    ] {
        let TableView::Complete(TableRows::AdvisorRoles(rows)) =
            &mut *frame.tables[TableId::AdvisorRoles.index()]
        else {
            unreachable!()
        };
        rows[0].modifier_cache = cache;
        let mut report = Report::default();
        compare_advisors(
            &frame,
            manager,
            &links,
            &BTreeMap::new(),
            NO_TRAITS,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables["AdvisorRoles"], 1);
        assert_eq!(report.limitations[reason].len(), 1);
    }
}

#[test]
fn operative_codename_oracle_checks_both_hosts_and_saved_omissions() {
    use content_reader_save::{DivisionName, intel};
    use replay_data::{rows::CharacterOperativeRow, rows::NameOperands};
    let (mut frame, _, _) = fixture();
    let naming = NameOperands {
        kind: 2,
        ordered: 1,
        group: b"SYNTHETIC_GROUP".to_vec(),
        ..Default::default()
    };
    *frame.tables[TableId::CharacterOperatives.index()] =
        TableView::Complete(TableRows::CharacterOperatives(vec![
            CharacterOperativeRow {
                character: 7,
                reference: handle(4713, 9),
                resolved: true,
                category: 3,
                codename: Recorded::Value(Some(naming.clone())),
                name: Recorded::Value(Some(b"unsaved name".to_vec())),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![UnitLeaderRow {
            reference_type: 4713,
            id: 9,
            kind: 3,
            codename: Recorded::Value(Some(naming)),
            ..Default::default()
        }]));
    let mut pack = intel::Pack::default();
    pack.countries.insert(
        b"XXX".to_vec(),
        intel::Country {
            agency: Some(intel::Agency {
                name: None,
                created: false,
                upgrade_progress: Decimal::parse(b"0").unwrap(),
                upgrades: Vec::new(),
                defense: Decimal::parse(b"0").unwrap(),
                active_count: 1,
                unknown_f0: 0,
                admission_ceiling: 1,
                operatives: vec![intel::OperativeEntry {
                    id: handle(4713, 9),
                    codename: Some(DivisionName {
                        kind: 2,
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
                captured: Vec::new(),
                recruitment: None,
                cryptology: None,
            }),
            ..Default::default()
        },
    );
    let mut report = Report::default();
    compare_codenames(&frame, &pack, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.scalar_checks, 8);
    assert_eq!(report.compared_tables["CharacterOperatives codenames"], 1);
    assert_eq!(report.compared_tables["UnitLeaders codenames"], 1);
    assert!(report.limitations.keys().any(|reason| {
        reason.contains("the unit-leader writer saves the stored name")
            && reason.contains("localized string")
    }));
    let change = |pack: &mut intel::Pack, value| {
        pack.countries
            .get_mut(b"XXX".as_slice())
            .unwrap()
            .agency
            .as_mut()
            .unwrap()
            .operatives[0]
            .codename = value;
    };
    change(
        &mut pack,
        Some(DivisionName {
            kind: 2,
            order: Some(-1),
            ..Default::default()
        }),
    );
    let mut report = Report::default();
    compare_codenames(&frame, &pack, &mut report).unwrap();
    assert_eq!(report.differences.len(), 2);
    change(
        &mut pack,
        Some(DivisionName {
            kind: 2,
            custom: Some(b"player rename".to_vec()),
            ..Default::default()
        }),
    );
    let mut report = Report::default();
    compare_codenames(&frame, &pack, &mut report).unwrap();
    assert_eq!(report.differences.len(), 2);
    change(&mut pack, None);
    let mut report = Report::default();
    compare_codenames(&frame, &pack, &mut report).unwrap();
    assert_eq!(report.differences.len(), 2);
}

#[test]
fn operative_name_and_codename_compatibility_gaps_are_named_and_counted() {
    use replay_data::rows::CharacterOperativeRow;
    let (mut frame, _, _) = fixture();
    *frame.tables[TableId::CharacterOperatives.index()] =
        TableView::Complete(TableRows::CharacterOperatives(
            (1..=2)
                .map(|id| CharacterOperativeRow {
                    character: id,
                    reference: handle(4713, id),
                    resolved: true,
                    category: 3,
                    name: Recorded::NotRecorded,
                    codename: Recorded::NotRecorded,
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::UnitLeaders.index()] = TableView::Complete(TableRows::UnitLeaders(
        (1..=2)
            .map(|id| UnitLeaderRow {
                reference_type: 4713,
                id,
                kind: 3,
                codename: Recorded::NotRecorded,
                ..Default::default()
            })
            .collect(),
    ));
    for whole_frame in [false, true] {
        let mut report = Report::default();
        if whole_frame {
            report.record_tables(&frame);
        }
        compare_codenames(&frame, &content_reader_save::intel::Pack::default(), &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.scalar_checks, 0);
        assert_eq!(report.compared_tables["CharacterOperatives codenames"], 0);
        assert_eq!(report.compared_tables["UnitLeaders codenames"], 0);
        for field in [
            "CharacterOperatives.name",
            "CharacterOperatives.codename",
            "UnitLeaders.codename",
        ] {
            let subjects = &report.limitations[&format!("{field} not recorded")];
            if whole_frame {
                assert_eq!(subjects, &[format!("{field}: 2 rows not recorded")]);
            } else {
                assert_eq!(subjects.len(), 2);
            }
        }
        assert!(
            !report
                .limitations
                .keys()
                .any(|reason| reason.contains("explicitly unavailable")
                    || reason.contains("localized string"))
        );
    }
}

#[test]
fn operative_name_and_codename_native_absence_is_named_separately() {
    use replay_data::rows::CharacterOperativeRow;
    let (mut frame, _, _) = fixture();
    *frame.tables[TableId::CharacterOperatives.index()] =
        TableView::Complete(TableRows::CharacterOperatives(vec![
            CharacterOperativeRow {
                character: 7,
                reference: handle(4713, 9),
                resolved: true,
                category: 3,
                name: Recorded::Value(None),
                codename: Recorded::Value(None),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![UnitLeaderRow {
            reference_type: 4713,
            id: 9,
            kind: 3,
            codename: Recorded::Value(None),
            ..Default::default()
        }]));
    let mut report = Report::default();
    compare_codenames(&frame, &content_reader_save::intel::Pack::default(), &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.scalar_checks, 0);
    for field in [
        "CharacterOperatives.name",
        "CharacterOperatives.codename",
        "UnitLeaders.codename",
    ] {
        assert_eq!(
            report.limitations[&format!("{field} is explicitly unavailable")].len(),
            1
        );
    }
    assert!(
        !report
            .limitations
            .keys()
            .any(|reason| reason.ends_with("not recorded"))
    );
}

fn leader_traits(text: &str) -> LeaderTraits {
    let root = std::env::temp_dir().join(format!(
        "oracle_leader_traits_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let file = root.join("common/country_leader/traits.txt");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, text).unwrap();
    let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let (traits, _) = content_reader_military::leader_traits::read(&layers).unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    traits
}

fn fixture() -> (DecodedFrame, saved::Pack, CountryRow) {
    let character = handle(73, u32::MAX);
    let leader = handle(4713, u32::MAX);
    let date = GameDate::parse(b"1936.1.1.22").unwrap();
    let mut frame = DecodedFrame {
        branch: 1,
        hour: date.engine_hour().unwrap(),
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index]))
        })
        .map(Into::into),
        metrics: Metrics::default(),
    };
    *frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![CharacterRow {
            id: u32::MAX,
            country: 102,
            name_token_id: 5,
            roster: 1,
            ordinal: 0,
            name: b"unsaved slot".to_vec(),
            token: b"saved literal name".to_vec(),
            template_present: 1,
            template_tag: 8,
            template_file: b"common/characters/GER.txt".to_vec(),
            template_line: 3,
            template_generated: 0,
            gender: 0,
        }]));
    *frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![UnitLeaderRow {
            reference_type: 4713,
            id: u32::MAX,
            character_type: 73,
            character_id: u32::MAX,
            category: 2,
            experience: -9_007_199_254_740_993,
            kind: 2,
            skill_status: 2,
            skill_valid: 1,
            skill_level: i32::MAX,
            components: [i32::MIN, 3, 2, 1],
            owner: 102,
            ..Default::default()
        }]));
    *frame.tables[TableId::UnitLeaderTraits.index()] =
        TableView::Complete(TableRows::UnitLeaderTraits(vec![UnitLeaderTraitRow {
            leader_reference: leader,
            ordinal: 0,
            token_id: 6,
            token: b"separate unsaved token bytes".to_vec(),
            ..Default::default()
        }]));
    *frame.tables[TableId::UnitLeaderTraitExperience.index()] =
        TableView::Complete(TableRows::UnitLeaderTraitExperience(vec![
            replay_data::rows::UnitLeaderTraitExperienceRow {
                list: 0,
                leader_reference: leader,
                ordinal: 0,
                token_id: 7,
                definition_ordinal: 0,
                experience: 1,
            },
        ]));
    let pack = saved::Pack {
        portraits: Default::default(),
        manager: Some(saved::Manager {
            characters: BTreeMap::from([(
                character,
                saved::Character {
                    portraits: None,
                    gender: Some(b"undefined".to_vec()),
                    id: character,
                    country: Some(b"GER".to_vec()),
                    roster: 1,
                    ordinal: 0,
                    token: b"script_token".to_vec(),
                    template: Some(b"GER_template".to_vec()),
                    flags: None,
                    variables: None,
                    name: Some(b"saved literal name".to_vec()),
                    country_leaders: Vec::new(),
                    advisors: Vec::new(),
                    scientist: None,
                    operative: None,
                },
            )]),
            unit_leaders: BTreeMap::from([(
                leader,
                saved::UnitLeader {
                    id: leader,
                    character,
                    category: 2,
                    name: None,
                    skill: i32::MAX,
                    experience: Decimal::parse(b"-90071992547.40993").unwrap(),
                    components: [i32::MIN, 3, 2, 1],
                    traits: vec![b"assigned".to_vec()],
                    trait_factors: Vec::new(),
                    trait_experience: vec![saved::TraitExperience {
                        token: b"progress".to_vec(),
                        experience: Decimal::parse(b"0.00001").unwrap(),
                    }],
                    timed_traits: Vec::new(),
                    captured: false,
                    government_in_exile_tag: None,
                },
            )]),
        }),
        abilities: BTreeMap::from([(b"GER".to_vec(), Vec::new())]),
        appointed_advisors: BTreeMap::from([(b"GER".to_vec(), Vec::new())]),
        character_status: BTreeMap::from([(
            b"GER".to_vec(),
            vec![saved::CharacterStatus {
                character,
                advisor: None,
                unit_leader: Some(true),
            }],
        )]),
        retired_character_status: BTreeMap::from([(b"GER".to_vec(), Vec::new())]),
    };
    let country = CountryRow {
        id: 102,
        tag: b"GER".to_vec(),
        ..Default::default()
    };
    (frame, pack, country)
}

#[test]
fn character_tables_are_compared_at_every_hour() {
    for table in [
        TableId::Characters,
        TableId::CountryLeaderRoles,
        TableId::AdvisorRoles,
        TableId::UnitLeaders,
        TableId::UnitLeaderTraits,
    ] {
        assert_eq!(
            replay_data::cadence::cadence(table),
            replay_data::cadence::Cadence::Hourly,
            "{table:?}"
        );
    }
    let (mut frame, pack, country) = fixture();
    let links = BTreeMap::from([(102, &country)]);
    let tokens = BTreeMap::from([
        (5, b"script_token".as_slice()),
        (6, b"assigned"),
        (7, b"progress"),
        (8, b"GER_template"),
    ]);
    let TableView::Complete(TableRows::UnitLeaders(rows)) =
        &mut *frame.tables[TableId::UnitLeaders.index()]
    else {
        unreachable!()
    };
    rows[0].skill_level -= 1;
    for hour in [1, 21] {
        frame.hour = hour;
        let mut report = Report::default();
        report.record_tables(&frame);
        compare(&frame, &pack, &links, &tokens, NO_TRAITS, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1, "hour {hour}");
    }
}

#[test]
fn gender_oracle_rejects_mismatch_and_absent_writer_field() {
    let (mut frame, mut pack, country) = fixture();
    let links = BTreeMap::from([(102, &country)]);
    let tokens = BTreeMap::from([
        (5, b"script_token".as_slice()),
        (6, b"assigned"),
        (7, b"progress"),
        (8, b"GER_template"),
    ]);
    if let TableView::Complete(TableRows::Characters(rows)) =
        &mut *frame.tables[TableId::Characters.index()]
    {
        rows[0].gender = 2;
    }
    let mut report = Report::default();
    compare(&frame, &pack, &links, &tokens, NO_TRAITS, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    pack.manager
        .as_mut()
        .unwrap()
        .characters
        .values_mut()
        .next()
        .unwrap()
        .gender = None;
    let mut report = Report::default();
    compare(&frame, &pack, &links, &tokens, NO_TRAITS, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
}

#[test]
fn leader_oracle_checks_full_backlinks_and_independent_traits() {
    let (mut frame, pack, country) = fixture();
    let links = BTreeMap::from([(102, &country)]);
    let tokens = BTreeMap::from([
        (5, b"script_token".as_slice()),
        (6, b"assigned"),
        (7, b"progress"),
        (8, b"GER_template"),
    ]);
    let mut report = Report::default();
    compare(&frame, &pack, &links, &tokens, NO_TRAITS, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    let TableView::Complete(TableRows::UnitLeaders(rows)) =
        &mut *frame.tables[TableId::UnitLeaders.index()]
    else {
        unreachable!()
    };
    rows[0].character_type ^= 1;
    let TableView::Complete(TableRows::UnitLeaderTraits(rows)) =
        &mut *frame.tables[TableId::UnitLeaderTraits.index()]
    else {
        unreachable!()
    };
    let trait_row = &mut rows[0];
    trait_row.token_id = 7;
    let mut report = Report::default();
    compare(&frame, &pack, &links, &tokens, NO_TRAITS, &mut report).unwrap();
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
    *frame.tables[TableId::UnitLeaderTraits.index()] = TableView::Unavailable {
        reason: "read fault".into(),
    };
    assert!(
        compare(
            &frame,
            &pack,
            &links,
            &tokens,
            NO_TRAITS,
            &mut Report::default()
        )
        .is_err()
    );
}

#[test]
fn role_traits_and_operative_references_compare_with_the_save() {
    use replay_data::{rows::CharacterOperativeRow, rows::CharacterRoleTraitRow as Row};
    let (mut frame, mut pack, country) = fixture();
    let links = BTreeMap::from([(102, &country)]);
    let tokens = BTreeMap::from([
        (5, b"script_token".as_slice()),
        (6, b"assigned"),
        (7, b"progress"),
        (8, b"GER_template"),
        (20, b"despotism"),
        (21, b"rich_sultan"),
        (22, b"captain"),
        (23, b"inventor"),
    ]);
    let character = handle(73, u32::MAX);
    let saved = pack
        .manager
        .as_mut()
        .unwrap()
        .characters
        .get_mut(&character)
        .unwrap();
    saved.country_leaders = vec![saved::CountryLeader {
        ideology: b"despotism".to_vec(),
        id: None,
        traits: vec![b"rich_sultan".to_vec()],
    }];
    saved.advisors = vec![saved::Advisor {
        slot: None,
        idea: None,
        traits: vec![b"captain".to_vec()],
    }];
    saved.scientist = Some(saved::Scientist {
        injury_days: None,
        active: false,
        traits: Some(vec![b"inventor".to_vec()]),
        skills: Vec::new(),
    });
    saved.operative = Some((77 << 32) | 4713);
    let row = |kind, role, token| Row {
        character: u32::MAX,
        kind,
        role,
        tokens: vec![token],
        definitions: if kind == Row::SCIENTIST {
            Vec::new()
        } else {
            vec![1]
        },
    };
    *frame.tables[TableId::CharacterRoleTraits.index()] =
        TableView::Complete(TableRows::CharacterRoleTraits(vec![
            row(Row::COUNTRY_LEADER, 20, 21),
            row(Row::ADVISOR, 0, 22),
            row(Row::SCIENTIST, 0, 23),
        ]));
    *frame.tables[TableId::CountryLeaderRoles.index()] =
        TableView::Complete(TableRows::CountryLeaderRoles(vec![
            replay_data::rows::CountryLeaderRoleRow {
                character: u32::MAX,
                country: 102,
                ideology: 20,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::AdvisorRoles.index()] =
        TableView::Complete(TableRows::AdvisorRoles(vec![
            replay_data::rows::AdvisorRoleRow {
                character: u32::MAX,
                country: 102,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::CharacterOperatives.index()] =
        TableView::Complete(TableRows::CharacterOperatives(vec![
            CharacterOperativeRow {
                character: u32::MAX,
                reference: (77 << 32) | 4713,
                resolved: true,
                ..Default::default()
            },
        ]));
    let traits = leader_traits("leader_traits = { captain = { } }");
    let mut report = Report::default();
    compare(&frame, &pack, &links, &tokens, Ok(&traits), &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["CharacterRoleTraits"], 3);
    let TableView::Complete(TableRows::CharacterRoleTraits(rows)) =
        &mut *frame.tables[TableId::CharacterRoleTraits.index()]
    else {
        unreachable!()
    };
    rows[0].tokens = vec![22];
    rows.pop();
    let TableView::Complete(TableRows::CharacterOperatives(rows)) =
        &mut *frame.tables[TableId::CharacterOperatives.index()]
    else {
        unreachable!()
    };
    rows[0].reference ^= 1 << 32;
    let mut report = Report::default();
    compare(&frame, &pack, &links, &tokens, Ok(&traits), &mut report).unwrap();
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
}

#[test]
fn advisor_traits_compare_by_the_content_key_at_their_ordinal() {
    use replay_data::rows::CharacterRoleTraitRow as Row;
    let (mut frame, mut pack, country) = fixture();
    let links = BTreeMap::from([(102, &country)]);
    let saved = pack
        .manager
        .as_mut()
        .unwrap()
        .characters
        .get_mut(&handle(73, u32::MAX))
        .unwrap();
    saved.advisors = vec![saved::Advisor {
        slot: None,
        idea: None,
        traits: vec![b"SWI_fascist".to_vec(), b"1st".to_vec()],
    }];
    *frame.tables[TableId::AdvisorRoles.index()] =
        TableView::Complete(TableRows::AdvisorRoles(vec![
            replay_data::rows::AdvisorRoleRow {
                character: u32::MAX,
                country: 102,
                ..Default::default()
            },
        ]));
    let set = |frame: &mut DecodedFrame, tokens: Vec<u32>, definitions: Vec<u32>| {
        *frame.tables[TableId::CharacterRoleTraits.index()] =
            TableView::Complete(TableRows::CharacterRoleTraits(vec![Row {
                character: u32::MAX,
                kind: Row::ADVISOR,
                role: 0,
                tokens,
                definitions,
            }]));
    };
    let tokens = BTreeMap::from([
        (5, b"script_token".as_slice()),
        (8, b"GER_template"),
        (30, b"SWI_FASCIST"),
        (31, b"other"),
    ]);
    let traits = leader_traits("leader_traits = { other = { } SWI_fascist = { } 1st = { } }");
    let run = |frame: &DecodedFrame, content| {
        let mut report = Report::default();
        compare(frame, &pack, &links, &tokens, content, &mut report).unwrap();
        report
    };
    set(&mut frame, vec![30, 0], vec![2, 3]);
    let report = run(&frame, Ok(&traits));
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["CharacterRoleTraits"], 1);
    let report = run(&frame, NO_TRAITS);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["CharacterRoleTraits"], 0);
    for (tokens, definitions) in [(vec![31, 0], vec![2, 3]), (vec![30, 31], vec![2, 3])] {
        set(&mut frame, tokens, definitions);
        let differences = run(&frame, Ok(&traits)).differences;
        assert!(
            differences.len() == 1 && differences[0].contains("definition"),
            "{differences:?}"
        );
    }
    set(&mut frame, vec![31, 0], vec![1, 3]);
    assert_eq!(run(&frame, Ok(&traits)).differences.len(), 1);
    set(&mut frame, vec![30, 0], vec![2, 9]);
    let report = run(&frame, Ok(&traits));
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["CharacterRoleTraits"], 0);
}

#[test]
fn missing_extra_and_byte_different_role_traits_are_differences() {
    use replay_data::rows::CharacterRoleTraitRow as Row;
    let (mut frame, mut pack, country) = fixture();
    let links = BTreeMap::from([(102, &country)]);
    let character = handle(73, u32::MAX);
    let saved = pack
        .manager
        .as_mut()
        .unwrap()
        .characters
        .get_mut(&character)
        .unwrap();
    saved.country_leaders = vec![saved::CountryLeader {
        ideology: b"rewritten".to_vec(),
        id: None,
        traits: vec![b"\xff".to_vec()],
    }];
    *frame.tables[TableId::CountryLeaderRoles.index()] =
        TableView::Complete(TableRows::CountryLeaderRoles(vec![
            replay_data::rows::CountryLeaderRoleRow {
                character: u32::MAX,
                country: 102,
                ideology: 20,
                ..Default::default()
            },
        ]));
    let leader = Row {
        character: u32::MAX,
        kind: Row::COUNTRY_LEADER,
        role: 20,
        tokens: vec![21],
        definitions: vec![1],
    };
    let run = |frame: &DecodedFrame, name: &'static [u8]| {
        let tokens = BTreeMap::from([
            (5, b"script_token".as_slice()),
            (6, b"assigned"),
            (7, b"progress"),
            (8, b"GER_template"),
            (21, name),
        ]);
        let mut report = Report::default();
        compare(frame, &pack, &links, &tokens, NO_TRAITS, &mut report).unwrap();
        report.differences
    };
    let set = |frame: &mut DecodedFrame, rows: Vec<Row>| {
        *frame.tables[TableId::CharacterRoleTraits.index()] =
            TableView::Complete(TableRows::CharacterRoleTraits(rows));
    };
    set(&mut frame, vec![leader.clone()]);
    assert!(run(&frame, b"\xff").is_empty());
    assert_eq!(run(&frame, b"\xfe").len(), 1);
    set(&mut frame, Vec::new());
    let missing = run(&frame, b"\xff");
    assert!(
        missing.len() == 1 && missing[0].contains("membership"),
        "{missing:?}"
    );
    set(
        &mut frame,
        vec![
            leader.clone(),
            Row {
                kind: Row::ADVISOR,
                role: 0,
                ..leader
            },
        ],
    );
    let extra = run(&frame, b"\xff");
    assert!(
        extra.len() == 1 && extra[0].contains("membership"),
        "{extra:?}"
    );
}

#[test]
fn character_template_tag_compares_with_the_saved_template() {
    let (frame, mut pack, country) = fixture();
    let links = BTreeMap::from([(102, &country)]);
    let tokens = BTreeMap::from([
        (5, b"script_token".as_slice()),
        (6, b"assigned"),
        (7, b"progress"),
        (8, b"GER_template"),
        (9, b"other"),
    ]);
    let run = |frame: &DecodedFrame, pack: &saved::Pack| {
        let mut report = Report::default();
        compare(frame, pack, &links, &tokens, NO_TRAITS, &mut report).unwrap();
        report
    };
    let edit = |edit: &dyn Fn(&mut CharacterRow)| {
        let mut frame = frame.clone();
        let TableView::Complete(TableRows::Characters(rows)) =
            &mut *frame.tables[TableId::Characters.index()]
        else {
            unreachable!()
        };
        edit(&mut rows[0]);
        frame
    };
    let report = run(&frame, &pack);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("runtime-only"))
    );
    let other = run(&edit(&|row| row.template_tag = 9), &pack).differences;
    assert!(
        other.len() == 1 && other[0].contains("template"),
        "{other:?}"
    );
    let absent = edit(&|row| {
        row.template_present = 0;
        row.template_tag = 0;
    });
    let presence = run(&absent, &pack).differences;
    assert!(
        presence.len() == 1 && presence[0].contains("template presence"),
        "{presence:?}"
    );
    pack.manager
        .as_mut()
        .unwrap()
        .characters
        .values_mut()
        .next()
        .unwrap()
        .template = None;
    assert!(run(&absent, &pack).differences.is_empty());
}

#[test]
fn leader_owner_and_captured_byte_compare_with_the_save() {
    let (frame, mut pack, country) = fixture();
    let links = BTreeMap::from([(102, &country)]);
    let tokens = BTreeMap::from([
        (5, b"script_token".as_slice()),
        (6, b"assigned"),
        (7, b"progress"),
        (8, b"GER_template"),
    ]);
    let run = |frame: &DecodedFrame, pack: &saved::Pack| {
        let mut report = Report::default();
        compare(frame, pack, &links, &tokens, NO_TRAITS, &mut report).unwrap();
        report.differences
    };
    let edit = |edit: &dyn Fn(&mut UnitLeaderRow)| {
        let mut frame = frame.clone();
        let TableView::Complete(TableRows::UnitLeaders(rows)) =
            &mut *frame.tables[TableId::UnitLeaders.index()]
        else {
            unreachable!()
        };
        edit(&mut rows[0]);
        frame
    };
    assert!(run(&frame, &pack).is_empty());
    let unowned = edit(&|row| row.owner = 0);
    let other = run(&unowned, &pack);
    assert!(other.len() == 1 && other[0].contains("owner"), "{other:?}");
    let owners = |frame: &DecodedFrame, pack: &saved::Pack| {
        let mut report = Report::default();
        compare_leaders(
            frame,
            pack.manager.as_ref().unwrap(),
            &pack.character_status,
            &links,
            &mut report,
        )
        .unwrap();
        report
    };
    let rostered = pack.character_status[b"GER".as_slice()].clone();
    let mut off_roster = pack.clone();
    let character = off_roster
        .manager
        .as_mut()
        .unwrap()
        .characters
        .values_mut()
        .next()
        .unwrap();
    character.country = Some(b"LPR".to_vec());
    off_roster.character_status =
        BTreeMap::from([(b"GER".to_vec(), Vec::new()), (b"LPR".to_vec(), Vec::new())]);
    off_roster.retired_character_status = BTreeMap::from([(b"LPR".to_vec(), rostered.clone())]);
    let report = owners(&unowned, &off_roster);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    let owned = owners(&frame, &off_roster).differences;
    assert!(owned.len() == 1 && owned[0].contains("owner"), "{owned:?}");
    let mut on_roster = off_roster.clone();
    on_roster.character_status = BTreeMap::from([(b"LPR".to_vec(), rostered.clone())]);
    let missing = owners(&unowned, &on_roster).differences;
    assert!(
        missing.len() == 1 && missing[0].contains("owner: replay None"),
        "{missing:?}"
    );
    let mut run_time = off_roster.clone();
    run_time
        .manager
        .as_mut()
        .unwrap()
        .characters
        .values_mut()
        .next()
        .unwrap()
        .country = Some(b"GER".to_vec());
    let report = owners(&frame, &run_time);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("off-roster character's country"))
    );
    let mut both = pack.clone();
    both.character_status.insert(b"LPR".to_vec(), rostered);
    let report = owners(&unowned, &both);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("several countries' rosters"))
    );
    let navy = run(&edit(&|row| row.captured = 1), &pack);
    assert!(navy.len() == 1 && navy[0].contains("captured"), "{navy:?}");
    let captured = edit(&|row| {
        row.kind = engine_native::cunit_leader::KIND_ARMY;
        row.captured = 1;
    });
    let free = run(&captured, &pack);
    assert!(free.len() == 1 && free[0].contains("captured"), "{free:?}");
    let leader = pack
        .manager
        .as_mut()
        .unwrap()
        .unit_leaders
        .values_mut()
        .next()
        .unwrap();
    leader.captured = true;
    assert!(run(&captured, &pack).is_empty());
}

#[test]
fn exile_country_tag_compares_exactly_and_nonpositive_omission_is_limited() {
    let (mut frame, mut pack, country) = fixture();
    let links = BTreeMap::from([(102, &country)]);
    let set = |frame: &mut DecodedFrame, exile_country| {
        let TableView::Complete(TableRows::UnitLeaders(rows)) =
            &mut *frame.tables[TableId::UnitLeaders.index()]
        else {
            unreachable!()
        };
        rows[0].exile_country = exile_country;
    };
    let run = |frame: &DecodedFrame, pack: &saved::Pack| {
        let mut report = Report::default();
        compare_leaders(
            frame,
            pack.manager.as_ref().unwrap(),
            &pack.character_status,
            &links,
            &mut report,
        )
        .unwrap();
        report
    };
    set(&mut frame, replay_data::rows::Recorded::Value(102));
    let report = run(&frame, &pack);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("government_in_exile_tag"));
    pack.manager
        .as_mut()
        .unwrap()
        .unit_leaders
        .values_mut()
        .next()
        .unwrap()
        .government_in_exile_tag = Some(b"GER".to_vec());
    assert!(run(&frame, &pack).differences.is_empty());
    set(&mut frame, replay_data::rows::Recorded::Value(-1));
    pack.manager
        .as_mut()
        .unwrap()
        .unit_leaders
        .values_mut()
        .next()
        .unwrap()
        .government_in_exile_tag = None;
    let report = run(&frame, &pack);
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .keys()
            .any(|line| line.contains("nonpositive exile-country"))
    );
}

#[test]
fn ability_cost_omission_does_not_certify_zero_and_caster_type_is_exact() {
    let (mut frame, mut pack, country) = fixture();
    let date = GameDate::parse(b"1936.1.1.1").unwrap();
    let leader = handle(4713, u32::MAX);
    *frame.tables[TableId::ActiveAbilities.index()] =
        TableView::Complete(TableRows::ActiveAbilities(vec![ActiveAbilityRow {
            country: 102,
            scope_country: u32::MAX,
            token: b"ability".to_vec(),
            caster: Some(leader),
            strategic_region: Some(u32::MAX),
            division_count: i32::MIN,
            activation_hour: date.engine_hour().unwrap(),
            phase: 128,
            allocate_cost: 1,
            allocated_command_power: -1,
            ..Default::default()
        }]));
    pack.abilities
        .get_mut(b"GER".as_slice())
        .unwrap()
        .push(saved::Ability {
            token: b"ability".to_vec(),
            start_date: date,
            country: None,
            leader: Some(leader),
            num_units: Some(i32::MIN),
            strategic_region: Some(-1),
            cost: None,
        });
    let links = BTreeMap::from([(102, &country)]);
    let mut report = Report::default();
    compare_abilities(&frame, &pack, &links, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    let TableView::Complete(TableRows::ActiveAbilities(rows)) =
        &mut *frame.tables[TableId::ActiveAbilities.index()]
    else {
        unreachable!()
    };
    let ability = &mut rows[0];
    ability.caster = Some(leader ^ 1);
    ability.allocated_command_power = 1;
    let mut report = Report::default();
    compare_abilities(&frame, &pack, &links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 2);
}

#[test]
fn native_identity_and_string_omission_do_not_use_display_fallbacks() {
    assert_ne!(handle(73, u32::MAX), handle(4713, u32::MAX));
    let mut report = Report::default();
    optional_text(&mut report, "name", b"exact\0suffix", Some(b"exact"));
    assert!(report.differences.is_empty());
    assert_eq!(report.limitations.len(), 1);
    optional_text(&mut report, "name", b"", None);
    optional_text(&mut report, "name", b"", Some(b""));
    optional_text(&mut report, "name", b"visible", None);
    assert_eq!(report.differences.len(), 2);
    let tokens = BTreeMap::from([(u32::MAX, b"negative token".as_slice())]);
    let mut report = Report::default();
    token_value(&mut report, "token", &tokens, -1, b"negative token");
    assert!(report.differences.is_empty());
    token_value(&mut report, "token", &tokens, 73, b"unknown");
    assert!(report.differences.is_empty());
    assert_eq!(report.limitations.len(), 1);
}

#[test]
fn nonempty_dictionary_checks_character_and_trait_spellings_and_preserves_missing_negative_ids() {
    let (mut frame, pack, country) = fixture();
    let links = BTreeMap::from([(102, &country)]);
    let tokens = BTreeMap::from([
        (5, b"script_token\0unsaved".as_slice()),
        (6, b"unproved assigned\0unsaved"),
        (7, b"unproved progress\0unsaved"),
        (8, b"GER_template"),
    ]);
    let mut report = Report::default();
    compare(&frame, &pack, &links, &tokens, NO_TRAITS, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(
        report.limitations["text serialization stops at the first NUL; captured suffix bytes are not comparable"].len(),
        1
    );
    assert_eq!(
        report.limitations["trait lexer literal contains an interior NUL; the text stream's projection is unproved, so this token spelling is unverified"].len(),
        2
    );
    let wrong = BTreeMap::from([
        (5, b"wrong character".as_slice()),
        (6, b"wrong assigned"),
        (7, b"wrong progress"),
    ]);
    let mut report = Report::default();
    compare(&frame, &pack, &links, &wrong, NO_TRAITS, &mut report).unwrap();
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
    let TableView::Complete(TableRows::Characters(rows)) =
        &mut *frame.tables[TableId::Characters.index()]
    else {
        unreachable!()
    };
    rows[0].name_token_id = -1;
    let mut report = Report::default();
    compare(&frame, &pack, &links, &tokens, NO_TRAITS, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    let missing = report
        .limitations
        .iter()
        .find(|(reason, _)| reason.contains("lexer literal is not recorded"))
        .unwrap()
        .1;
    assert_eq!(missing.len(), 1);
    assert!(missing[0].contains("native token -1"));
}

#[test]
fn reserved_trait_formatter_fails_traits_after_independent_leader_comparison() {
    let (mut frame, pack, country) = fixture();
    let TableView::Complete(TableRows::Characters(rows)) =
        &mut *frame.tables[TableId::Characters.index()]
    else {
        unreachable!()
    };
    rows[0].name_token_id = 0x167;
    let TableView::Complete(TableRows::UnitLeaderTraits(rows)) =
        &mut *frame.tables[TableId::UnitLeaderTraits.index()]
    else {
        unreachable!()
    };
    rows[0].token_id = 0x167;
    let TableView::Complete(TableRows::UnitLeaders(rows)) =
        &mut *frame.tables[TableId::UnitLeaders.index()]
    else {
        unreachable!()
    };
    rows[0].skill_level -= 1;
    let mut report = Report::default();
    assert!(
        compare(
            &frame,
            &pack,
            &BTreeMap::from([(102, &country)]),
            &BTreeMap::from([(0x167, b"script_token".as_slice())]),
            NO_TRAITS,
            &mut report,
        )
        .is_err()
    );
    assert_eq!(report.failed_comparisons.len(), 1);
    assert!(
        report.failed_comparisons["UnitLeaderTraits"].contains("fixed15 formatter token 0x167")
    );
    assert_eq!(report.compared_tables["UnitLeaderTraits"], 1);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("skill"));
}

#[test]
fn missing_dictionary_keeps_position_census_and_raw_value_comparisons() {
    let (mut frame, pack, country) = fixture();
    let links = BTreeMap::from([(102, &country)]);
    let mut report = Report::default();
    compare(
        &frame,
        &pack,
        &links,
        &BTreeMap::new(),
        NO_TRAITS,
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    let missing_names = report
        .limitations
        .iter()
        .find(|(reason, _)| reason.contains("lexer literal is not recorded"))
        .unwrap()
        .1;
    assert_eq!(missing_names.len(), 4);
    let TableView::Complete(TableRows::UnitLeaders(rows)) =
        &mut *frame.tables[TableId::UnitLeaders.index()]
    else {
        unreachable!()
    };
    rows[0].skill_level -= 1;
    let mut report = Report::default();
    compare(
        &frame,
        &pack,
        &links,
        &BTreeMap::new(),
        NO_TRAITS,
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(
        !report
            .differences
            .iter()
            .any(|difference| difference.contains("membership"))
    );
}

#[test]
fn unavailable_advisors_and_traits_preserve_later_table_results_and_errors() {
    let (mut frame, mut pack, country) = fixture();
    *frame.tables[TableId::AdvisorRoles.index()] = TableView::Unavailable {
        reason: "advisor role map key order invalid".into(),
    };
    *frame.tables[TableId::UnitLeaderTraits.index()] = TableView::Unavailable {
        reason: "assigned trait read fault".into(),
    };
    let TableView::Complete(TableRows::UnitLeaders(rows)) =
        &mut *frame.tables[TableId::UnitLeaders.index()]
    else {
        unreachable!()
    };
    rows[0].skill_level -= 1;
    pack.abilities
        .get_mut(b"GER".as_slice())
        .unwrap()
        .push(saved::Ability {
            token: b"missing ability".to_vec(),
            start_date: GameDate::parse(b"1936.1.1.1").unwrap(),
            country: None,
            leader: None,
            num_units: None,
            strategic_region: None,
            cost: None,
        });
    let mut report = Report::default();
    assert!(
        compare(
            &frame,
            &pack,
            &BTreeMap::from([(102, &country)]),
            &BTreeMap::new(),
            NO_TRAITS,
            &mut report
        )
        .is_err()
    );
    assert!(
        report.failed_comparisons["AdvisorRoles"].contains("advisor role map key order invalid")
    );
    assert!(report.failed_comparisons["UnitLeaderTraits"].contains("assigned trait read fault"));
    assert_eq!(report.compared_tables["UnitLeaders"], 1);
    for table in ["UnitLeaders", "ActiveAbilities"] {
        assert_eq!(report.difference_groups[table], 1);
    }
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
}

#[test]
fn unavailable_leader_categories_do_not_invent_extra_trait_owners() {
    let (mut frame, pack, country) = fixture();
    *frame.tables[TableId::UnitLeaders.index()] = TableView::Unavailable {
        reason: "unit leader read fault".into(),
    };
    let TableView::Complete(TableRows::UnitLeaderTraits(rows)) =
        &mut *frame.tables[TableId::UnitLeaderTraits.index()]
    else {
        unreachable!()
    };
    rows[0].token_id = 7;
    rows.push(UnitLeaderTraitRow {
        leader_reference: handle(4713, 1),
        ordinal: 0,
        token_id: 6,
        ..Default::default()
    });
    let mut report = Report::default();
    assert!(
        compare(
            &frame,
            &pack,
            &BTreeMap::from([(102, &country)]),
            &BTreeMap::from([(6, b"assigned".as_slice()), (7, b"progress")]),
            NO_TRAITS,
            &mut report
        )
        .is_err()
    );
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("token"));
    assert_eq!(report.compared_tables["UnitLeaderTraits"], 1);
    assert!(report.compared_tables.contains_key("ActiveAbilities"));
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("native category cannot establish"))
    );
}

#[test]
fn experience_factor_oracle_compares_repeated_override_positions_and_raw_values() {
    let (mut frame, mut pack, _) = fixture();
    let manager = pack.manager.as_mut().unwrap();
    let (&id, leader) = manager.unit_leaders.iter_mut().next().unwrap();
    leader.trait_factors = vec![
        saved::TraitExperience {
            token: b"progress".to_vec(),
            experience: Decimal::parse(b"-0.25").unwrap()
        };
        2
    ];
    let TableView::Complete(TableRows::UnitLeaderTraitExperience(rows)) =
        &mut *frame.tables[TableId::UnitLeaderTraitExperience.index()]
    else {
        panic!()
    };
    for ordinal in 0..2 {
        rows.push(replay_data::rows::UnitLeaderTraitExperienceRow {
            leader_reference: id,
            list: 1,
            ordinal,
            token_id: 7,
            definition_ordinal: 0,
            experience: -25_000,
        });
    }
    let tokens = BTreeMap::from([(7, b"progress".as_slice())]);
    let mut report = Report::default();
    compare_trait_progress(&frame, manager, &tokens, &mut report).unwrap();
    assert!(report.differences.is_empty());
    let TableView::Complete(TableRows::UnitLeaderTraitExperience(rows)) =
        &mut *frame.tables[TableId::UnitLeaderTraitExperience.index()]
    else {
        panic!()
    };
    rows.last_mut().unwrap().experience -= 1;
    let mut report = Report::default();
    compare_trait_progress(&frame, manager, &tokens, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("experience"));
}

const UNNAMED_ROSTER: &[u8] =
    br#"countries={ANT={characters={character_status={character={id=2 type=73} advisor=yes}}}}
character_manager={historical={
character={id={id=1 type=73} token="ANJ_air_chief" gender=male}
character={id={id=2 type=73} token="ANT_air_chief" name="Kenny Bousquet" country="ANT" gender=male}
character={id={id=3 type=73} token="FEZ_air_chief" gender=male}
}}"#;

fn unnamed_roster_row(id: u32, ordinal: u32) -> CharacterRow {
    let (country, name_token_id, token) = match id {
        1 => (0, 11, &b""[..]),
        2 => (7, 12, &b"Kenny Bousquet"[..]),
        _ => (0, 13, &b""[..]),
    };
    CharacterRow {
        id,
        country,
        name_token_id,
        gender: 1,
        roster: 0,
        ordinal,
        token: token.to_vec(),
        ..Default::default()
    }
}

#[test]
fn a_saved_character_no_country_lists_is_compared_as_the_memory_that_wrote_or_loaded_the_save() {
    let document = content_reader_parser::parser::parse(UNNAMED_ROSTER).unwrap();
    let pack = saved::parse(&document.items).unwrap();
    let country = CountryRow {
        id: 7,
        tag: b"ANT".to_vec(),
        ..Default::default()
    };
    let links = BTreeMap::from([(7, &country)]);
    let tokens = BTreeMap::from([
        (11, b"ANJ_air_chief".as_slice()),
        (12, b"ANT_air_chief"),
        (13, b"FEZ_air_chief"),
    ]);
    let run = |ids: &[(u32, u32)], load_hour: bool| {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 1,
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
            metrics: Metrics::default(),
        };
        *frame.tables[TableId::Characters.index()] = TableView::Complete(TableRows::Characters(
            ids.iter()
                .map(|&(id, ordinal)| unnamed_roster_row(id, ordinal))
                .collect(),
        ));
        let mut manager = pack.manager.clone().unwrap();
        let roster = fit_saved_roster(&frame, &mut manager, load_hour);
        let mut report = Report::default();
        compare_characters(&frame, &manager, &links, &tokens, &mut report).unwrap();
        (roster, report.differences)
    };
    let written = [(1, 0), (2, 1), (3, 2)];
    assert_eq!(
        run(&written, false),
        (Some(SavedRoster::Written { unloadable: 2 }), Vec::new())
    );
    for load_hour in [false, true] {
        assert_eq!(
            run(&[(2, 0)], load_hour),
            (Some(SavedRoster::Loaded { unloadable: 2 }), Vec::new())
        );
    }
    assert_eq!(
        run(&[(1, 0), (2, 1)], false),
        (
            Some(SavedRoster::Written { unloadable: 2 }),
            vec![format!(
                "character {} membership: replay false, save true",
                handle(73, 3)
            )]
        )
    );
    let (roster, differences) = run(&written, true);
    assert_eq!(roster, Some(SavedRoster::Loaded { unloadable: 2 }));
    assert_eq!(
        differences,
        [
            format!(
                "character {} membership: replay true, save false",
                handle(73, 1)
            ),
            format!("character {}.ordinal: replay 1, save 0", handle(73, 2)),
            format!(
                "character {} membership: replay true, save false",
                handle(73, 3)
            ),
        ]
    );
}
