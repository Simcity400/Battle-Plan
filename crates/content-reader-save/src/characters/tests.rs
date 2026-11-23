use super::*;

fn fixture(input: &[u8]) -> io::Result<Pack> {
    parse(&content_reader_parser::parser::parse(input)?.items)
}

#[test]
fn scientist_active_preserves_saved_boolean_presence() {
    let pack = fixture(
        br#"character_manager={historical={
        character={id={id=1 type=73} token=a scientist={skills={} is_assigned=yes}}
        character={id={id=2 type=73} token=b scientist={skills={}}}
    }} countries={}"#,
    )
    .unwrap();
    let manager = pack.manager.unwrap();
    assert!(
        manager.characters[&((1 << 32) | 73)]
            .scientist
            .as_ref()
            .unwrap()
            .active
    );
    assert!(
        !manager.characters[&((2 << 32) | 73)]
            .scientist
            .as_ref()
            .unwrap()
            .active
    );
    assert!(fixture(br#"character_manager={historical={character={id={id=1 type=73} token=a scientist={skills={} is_assigned=invalid}}}} countries={}"#).is_err());
}

const SCIENTIST: &[u8] = br#"character_manager={ historical={
        character={
            id={ id=265 type=73 } token="AST_mark_oliphant" country="AST"
            scientist={
                traits={ "scientist_trait_nuclear_specialist" }
                skills={
                    specialization_nuclear={ level=2 }
                    specialization_naval={ experience=219.18 }
                    specialization_air={ }
                    specialization_land={ }
                }
            }
        }
        character={
            id={ id=266 type=73 } token="plain"
            scientist={ skills={ specialization_nuclear={ } } }
        }
        character={ id={ id=267 type=73 } token="not_a_scientist" }
    }} countries={}"#;

#[test]
fn shared_character_status_retains_unit_leader_truth_and_absence() {
    let pack = fixture(
        br"countries={AAA={characters={
            character_status={character={id=7 type=73} advisor=yes unit_leader=yes}
            character_status={character={id=8 type=73} unit_leader=no}
            character_status={character={id=9 type=73}}
        }}}",
    )
    .unwrap();
    let statuses = &pack.character_status[b"AAA".as_slice()];
    assert_eq!(statuses[0].advisor, Some(true));
    assert_eq!(statuses[0].unit_leader, Some(true));
    assert_eq!(statuses[1].unit_leader, Some(false));
    assert_eq!(statuses[2].unit_leader, None);
}

#[test]
fn operative_references_and_portrait_categories_do_not_invent_leader_records() {
    let pack=fixture(br#"countries={GER={operative={id={id=8 type=4713} portrait_path="a.dds"}}} character_manager={historical={character={id={id=7 type=73} token=x operative={id=8 type=4713} portraits={operative={large="GFX_a"}}}}}"#).unwrap();
    assert_eq!(pack.portraits.len(), 1);
    assert_eq!(pack.portraits[&((8 << 32) | 4713)][0], b"a.dds");
    assert_eq!(
        pack.manager.unwrap().characters[&((7 << 32) | 73)]
            .portraits
            .as_ref()
            .unwrap()[0]
            .category,
        b"operative"
    );
}

#[test]
fn scientist_roles_keep_trait_lists_ordered_specializations_and_omissions() {
    let pack = fixture(SCIENTIST).unwrap();
    let manager = pack.manager.as_ref().unwrap();
    let first = manager.characters[&((265 << 32) | 73)]
        .scientist
        .as_ref()
        .unwrap();
    assert_eq!(
        first.traits.as_deref(),
        Some([b"scientist_trait_nuclear_specialist".to_vec()].as_slice())
    );
    assert_eq!(first.skills.len(), 4);
    assert_eq!(first.skills[0].name, b"specialization_nuclear");
    assert_eq!(first.skills[0].level, Some(2));
    assert_eq!(first.skills[0].experience, None);
    assert_eq!(
        first.skills[1].experience.as_ref().map(|value| value.raw),
        Some(21_918_000)
    );
    assert_eq!(first.skills[1].level, None);
    assert_eq!(first.skills[3].name, b"specialization_land");
    assert_eq!(first.skills[3].level, None);
    let second = manager.characters[&((266 << 32) | 73)]
        .scientist
        .as_ref()
        .unwrap();
    assert_eq!(second.traits, None);
    assert_eq!(second.skills.len(), 1);
    assert!(manager.characters[&((267 << 32) | 73)].scientist.is_none());
}

#[test]
fn a_scientist_without_skills_or_with_a_nonpositive_level_is_refused() {
    let text = String::from_utf8(SCIENTIST.to_vec()).unwrap();
    for broken in [
        text.replace(
            "scientist={ skills={ specialization_nuclear={ } } }",
            "scientist={ }",
        ),
        text.replace(
            "specialization_nuclear={ level=2 }",
            "specialization_nuclear={ level=0 }",
        ),
        text.replace(
            "specialization_nuclear={ level=2 }",
            "specialization_nuclear={ level=-1 }",
        ),
        text.replace("experience=219.18", "experience=0"),
        text.replace("experience=219.18", "experience=-1"),
    ] {
        assert!(fixture(broken.as_bytes()).is_err());
    }
}

#[test]
fn character_template_and_own_flags_are_optional_and_exact() {
    let pack = fixture(
        br#"countries={} character_manager={historical={
            character={id={id=1 type=73} token=a template="GER_a" flags={
                recently_sick={ value=1 date="1936.1.26.1" days=180 }
                x={ value=-2 date="1936.1.1.1" }
            }}
            character={id={id=2 type=73} token=b}
        }}"#,
    )
    .unwrap();
    let manager = pack.manager.unwrap();
    let first = &manager.characters[&((1_u64 << 32) | 73)];
    assert_eq!(first.template.as_deref(), Some(b"GER_a".as_slice()));
    let flags = first.flags.as_ref().unwrap();
    assert_eq!(flags.len(), 2);
    assert_eq!(
        (
            flags[0].name.as_slice(),
            flags[0].value,
            flags[0].remaining_days
        ),
        (b"recently_sick".as_slice(), 1, Some(180))
    );
    assert_eq!((flags[1].value, flags[1].remaining_days), (-2, None));
    let second = &manager.characters[&((2_u64 << 32) | 73)];
    assert!(second.template.is_none() && second.flags.is_none());
    assert!(fixture(b"countries={} character_manager={historical={character={id={id=1 type=73} token=a flags={}}}}").is_err());
}

#[test]
fn character_rosters_keep_identity_order_names_and_optional_country() {
    let pack = fixture(
        br#"countries={} character_manager={historical={
            character={id={id=4294967295 type=73} token="" name="raw name"}
            character={id={id=1 type=73} token="same" country=GER}
        } dynamic={character={id={id=2 type=73} token="same"}}}"#,
    )
    .unwrap();
    let manager = pack.manager.unwrap();
    let first = &manager.characters[&((u64::from(u32::MAX) << 32) | 73)];
    assert_eq!((first.roster, first.ordinal), (0, 0));
    assert!(first.token.is_empty());
    assert!(first.country.is_none());
    assert_eq!(first.name.as_deref(), Some(b"raw name".as_slice()));
    assert_eq!(
        (
            manager.characters[&((2_u64 << 32) | 73)].roster,
            manager.characters.len()
        ),
        (1, 3)
    );
    assert!(fixture(b"countries={} character_manager={historical={character={id={id=1 type=73} token=a}} dynamic={character={id={id=1 type=73} token=b}}}").is_err());
    assert!(fixture(b"countries={}").unwrap().manager.is_none());
}

#[test]
fn experience_precision_and_independent_trait_order_are_lossless() {
    let pack = fixture(br#"countries={} character_manager={historical={character={
            id={id=8 type=73} token=x
            navy_leader={id={id=4294967295 type=4713} skill=-2147483648 experience=-90071992547.40993
                attack_skill=2147483647 coordination_skill=-1
                traits={b a b} in_progress={a=0.00001 b=-90071992547.40993 a=0} trait_xp_factor={b=-0.00001 b=90071992547.40993 a=0}
                traits_to_remove={sick=-2147483648 b=89 sick=0}}
            advisors={advisor={slot=x idea_token=y dynamic_template={slot=ignored}} advisor={}}
        }}}"#).unwrap();
    let manager = pack.manager.unwrap();
    let leader = manager.unit_leaders.values().next().unwrap();
    assert_eq!(leader.experience.raw, -9_007_199_254_740_993);
    assert_eq!(leader.components, [i32::MAX, 0, 0, -1]);
    assert_eq!(leader.traits, [b"b".to_vec(), b"a".to_vec(), b"b".to_vec()]);
    assert_eq!(
        leader.trait_experience[1].experience.raw,
        -9_007_199_254_740_993
    );
    assert_eq!(leader.trait_experience.len(), 3);
    assert_eq!(leader.trait_factors.len(), 3);
    assert_eq!(leader.trait_factors[0].token, b"b");
    assert_eq!(leader.trait_factors[1].token, b"b");
    assert_eq!(leader.trait_factors[0].experience.raw, -1);
    assert_eq!(
        leader.trait_factors[1].experience.raw,
        9_007_199_254_740_993
    );
    assert_eq!(
        leader
            .timed_traits
            .iter()
            .map(|timed| (timed.token.as_slice(), timed.days))
            .collect::<Vec<_>>(),
        [(b"sick".as_slice(), i32::MIN), (b"b", 89), (b"sick", 0)]
    );
    assert!(
        manager.characters.values().next().unwrap().advisors[1]
            .slot
            .is_none()
    );
}

#[test]
fn country_leader_roles_keep_their_optional_id() {
    let pack = fixture(
        br#"character_manager={ historical={
            character={ id={ id=1 type=73 } token="a"
                country_leaders={
                    country_leader={ ideology=despotism id=12 traits={ rich_sultan b_trait } }
                    country_leader={ ideology=fascism_ideology }
                } }
        } } countries={ }"#,
    )
    .unwrap();
    let roles = &pack.manager.unwrap().characters[&((1 << 32) | 73)].country_leaders;
    assert_eq!(
        roles,
        &[
            CountryLeader {
                ideology: b"despotism".to_vec(),
                id: Some(12),
                traits: vec![b"rich_sultan".to_vec(), b"b_trait".to_vec()],
            },
            CountryLeader {
                ideology: b"fascism_ideology".to_vec(),
                id: None,
                traits: Vec::new(),
            },
        ]
    );
    assert!(
            fixture(
                br#"character_manager={ historical={ character={ id={ id=1 type=73 } token="a"
                country_leaders={ country_leader={ ideology=despotism id=-1 } } } } } countries={ }"#
            )
            .is_err()
        );
}

#[test]
fn advisor_traits_and_the_operative_reference_are_kept() {
    let pack = fixture(
        br#"character_manager={ historical={
            character={ id={ id=1 type=73 } token="a" operative={ id=77 type=4713 }
                advisors={ advisor={ slot=theorist idea_token=a_idea traits={ b_trait a_trait } }
                    advisor={ slot=x } } }
            character={ id={ id=2 type=73 } token="b" }
        } } countries={ }"#,
    )
    .unwrap();
    let characters = &pack.manager.unwrap().characters;
    let first = &characters[&((1 << 32) | 73)];
    assert_eq!(first.operative, Some((77 << 32) | 4713));
    assert_eq!(
        first.advisors[0].traits,
        [b"b_trait".to_vec(), b"a_trait".to_vec()]
    );
    assert!(first.advisors[1].traits.is_empty());
    assert_eq!(characters[&((2 << 32) | 73)].operative, None);
}

#[test]
fn appointed_advisors_keep_written_order_and_omissions_mean_empty() {
    let pack = fixture(
        br#"countries={
            GER={ characters={
                character_status={ character={ id=1 type=73 } advisor=yes }
                appointed_advisors={
                    { slot="political_advisor" character={ id=1482 type=73 } }
                    { slot="" character={ id=1451 type=73 } }
                }
                recruit_scientist={ scientist={ id=9 type=73 } }
            } }
            ENG={ characters={ character_status={ character={ id=2 type=73 } }
                retired_character_status={ character={ id=3 type=73 } } } }
            ITA={ }
        }"#,
    )
    .unwrap();
    let german = &pack.appointed_advisors[b"GER".as_slice()];
    assert_eq!(
        german,
        &[
            AppointedAdvisor {
                slot: b"political_advisor".to_vec(),
                character: (1482 << 32) | 73,
            },
            AppointedAdvisor {
                slot: Vec::new(),
                character: (1451 << 32) | 73,
            },
        ]
    );
    assert!(pack.appointed_advisors[b"ENG".as_slice()].is_empty());
    assert!(pack.appointed_advisors[b"ITA".as_slice()].is_empty());
    assert_eq!(
        pack.character_status[b"GER".as_slice()],
        [CharacterStatus {
            character: (1 << 32) | 73,
            advisor: Some(true),
            unit_leader: None,
        }]
    );
    assert_eq!(
        pack.character_status[b"ENG".as_slice()],
        [CharacterStatus {
            character: (2 << 32) | 73,
            advisor: None,
            unit_leader: None,
        }]
    );
    assert!(pack.character_status[b"ITA".as_slice()].is_empty());
    assert_eq!(
        pack.retired_character_status[b"ENG".as_slice()],
        [CharacterStatus {
            character: (3 << 32) | 73,
            advisor: None,
            unit_leader: None,
        }]
    );
    assert!(pack.retired_character_status[b"GER".as_slice()].is_empty());
    assert!(pack.retired_character_status[b"ITA".as_slice()].is_empty());
    assert!(
        fixture(br#"countries={ GER={ characters={ character_status={ advisor=yes } } } }"#)
            .is_err()
    );
    for broken in [
            br#"countries={ GER={ characters={ appointed_advisors={ } } } }"#.as_slice(),
            br#"countries={ GER={ characters={ appointed_advisors={ advisor={ slot=x character={ id=1 type=73 } } } } } }"#,
            br#"countries={ GER={ characters={ appointed_advisors={ { character={ id=1 type=73 } } } } } }"#,
            br#"countries={ GER={ characters={ appointed_advisors={ { slot=x } } } } }"#,
        ] {
            assert!(fixture(broken).is_err());
        }
}

#[test]
fn ability_optional_operands_follow_serializer_predicates() {
    let pack = fixture(
        br#"countries={GER={active_ability={id=a start_date="1936.1.1.1"
            leader={id=4294967295 type=4713} num_units=-1 strategic_region=-2147483648 cost=0.00001}
            active_ability={id=b start_date="1936.1.1.1"}}}"#,
    )
    .unwrap();
    let rows = &pack.abilities[b"GER".as_slice()];
    assert_eq!(rows[0].leader, Some((u64::from(u32::MAX) << 32) | 4713));
    assert_eq!(rows[0].cost.as_ref().unwrap().raw, 1);
    assert_eq!(rows[0].strategic_region, Some(i32::MIN));
    assert!(rows[1].cost.is_none());
    for invalid in [
        br#"countries={GER={active_ability={id=a start_date="1936.1.1.1" num_units=1}}}"#
            .as_slice(),
        br#"countries={GER={active_ability={id=a start_date="1936.1.1.1" cost=0}}}"#,
    ] {
        assert!(fixture(invalid).is_err());
    }
}

#[test]
fn exile_country_save_tag_is_preserved_with_leader_writer_omission() {
    let pack = fixture(br#"countries={} character_manager={historical={character={id={id=3 type=73} token=synthetic field_marshal={id={id=7 type=4713} skill=1 experience=0 attack_skill=0 defense_skill=0 planning_skill=0 logistics_skill=0 government_in_exile_tag=OTHER}}}}"#).unwrap();
    assert_eq!(
        pack.manager.unwrap().unit_leaders[&((7_u64 << 32) | 4713)]
            .government_in_exile_tag
            .as_deref(),
        Some(b"OTHER".as_slice())
    );
}
