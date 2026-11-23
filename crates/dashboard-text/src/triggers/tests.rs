
use super::*;
use content_reader_key_names::{buildings::Buildings, tag_aliases::TagAliases};
use content_reader_script::{key_names::KeyNames, trigger_keys::TriggerNames};
use replay_data::{
    rows::CountryAppearanceRow, rows::CountryCounterInputRow, rows::CountryLinkRow,
    rows::CountryRow, rows::FactionMemberRow, rows::FocusRow, rows::LexicalTokenRow,
    rows::Recorded, rows::ReservedCountryNamingRow, rows::ScriptFlagRow, rows::TableId,
    rows::TableRows, rows::tables,
};
use replay_frame::{frame::DecodedFrame, frame::Metrics, frame::TableView};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[path = "event_scopes/tests.rs"]
mod event_scopes;

#[path = "carriers/tests.rs"]
mod carriers;

#[path = "organisations/tests.rs"]
mod organisations;

#[path = "flag_tests.rs"]
mod flags;

#[path = "country_leader_tests.rs"]
mod country_leaders;

#[path = "collections/tests.rs"]
mod collections;

#[path = "temp_tests.rs"]
mod temp_variables;

#[path = "map_script_tests.rs"]
mod map_scripts;

#[path = "dynamic_modifier_tests.rs"]
mod dynamic_modifiers;

#[path = "faction_zero_tests.rs"]
mod faction_zero;

fn frame(countries: &[(u32, u32, &str, i32, &str, &str)], flagged: &[u32]) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 1,
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index]))
        })
        .map(Into::into),
        metrics: Metrics::default(),
    };
    let mut country_tags = BTreeMap::new();
    for &(raw, slot, tag, ..) in countries {
        country_tags.entry(tag).or_insert((raw, slot));
    }
    let mut set = |id: TableId, rows: TableRows| {
        *frame.tables[id.index()] = TableView::Complete(rows);
    };
    set(
        TableId::Countries,
        TableRows::from_rows::<tables::Countries>(
            countries
                .iter()
                .map(|(id, slot, tag, ..)| CountryRow {
                    id: *id,
                    canonical_slot: *slot,
                    tag: tag.as_bytes().to_vec(),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        TableId::CountryLinks,
        crate::test_country_links::rows(
            countries
                .iter()
                .map(|(id, slot, ..)| CountryLinkRow {
                    id: *id,
                    canonical_slot: *slot,
                }),
        ),
    );
    set(
        TableId::CountryCounterInputs,
        TableRows::from_rows::<tables::CountryCounterInputs>(
            countries
                .iter()
                .map(|(id, _, _, owned, ..)| CountryCounterInputRow {
                    country: *id,
                    owned_state_count: *owned,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        TableId::CountryAppearance,
        TableRows::from_rows::<tables::CountryAppearance>(
            countries
                .iter()
                .map(|(id, _, _, _, original, cosmetic)| CountryAppearanceRow {
                    country: *id,
                    name_tag: original.as_bytes().to_vec(),
                    original_tag_reference: Recorded::Value(Some(if original.is_empty() {
                        0
                    } else {
                        country_tags
                            .get(original)
                            .map(|&(raw, _)| raw as i32)
                            .unwrap_or(1)
                    })),
                    original_tag: Recorded::Value(Some(original.as_bytes().to_vec())),
                    cosmetic_tag: cosmetic.as_bytes().to_vec(),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    let mut buckets =
        vec![Vec::new(); countries.iter().map(|row| row.1).max().unwrap_or(0) as usize + 1];
    for &(raw, _, _, _, original, _) in countries {
        if let Some(&(_, slot)) = country_tags.get(original) {
            buckets[slot as usize].push(raw);
        }
    }
    set(
        TableId::ReservedCountryNaming,
        TableRows::ReservedCountryNaming(vec![ReservedCountryNamingRow {
            tag: b"ZZ0".to_vec(),
            original_tag_reference: Recorded::Value(Some(0)),
            original_tag: Recorded::Value(Some(Vec::new())),
            original_tag_buckets: Recorded::Value(Some(buckets)),
            country_array: Recorded::Value(Some(
                std::iter::once(0)
                    .chain(countries.iter().map(|row| row.0))
                    .collect(),
            )),
            ..Default::default()
        }]),
    );
    set(
        TableId::Focus,
        TableRows::from_rows::<tables::Focus>(
            countries
                .iter()
                .map(|(id, _, tag, ..)| FocusRow {
                    country: *id,
                    tree: (*tag == "FRA").then(|| b"vichy_french_focus".to_vec()),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        TableId::LexicalTokens,
        TableRows::from_rows::<tables::LexicalTokens>(vec![LexicalTokenRow {
            token: 900,
            literal: b"x".to_vec(),
        }]),
    );
    set(
        TableId::ScriptFlags,
        TableRows::from_rows::<tables::ScriptFlags>(
            flagged
                .iter()
                .map(|country| ScriptFlagRow {
                    scope_kind: 1,
                    scope_id: *country,
                    token: 900,
                    value: 1,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    frame
}

fn check(frame: &DecodedFrame, aliases: &str, slot: u32, trigger: &str) -> Eval {
    with_world(frame, aliases, Err("none"), |world| {
        let items = items(trigger);
        world.evaluate(&items, Scopes::of(Scope::Country(slot)))
    })
}

fn with_world<T>(
    frame: &DecodedFrame,
    aliases: &str,
    roles: Result<&LeaderRoles, &str>,
    run: impl FnOnce(&World) -> T,
) -> T {
    with_world_traits(frame, aliases, roles, Err("none"), run)
}

fn with_world_traits<T>(
    frame: &dyn replay_frame::frame::DecodedSource,
    aliases: &str,
    roles: Result<&LeaderRoles, &str>,
    traits: Result<
        (
            &content_reader_key_names::unit_leader::Traits,
            &content_reader_military::leader_traits::LeaderTraits,
        ),
        &str,
    >,
    run: impl FnOnce(&World) -> T,
) -> T {
    let aliases: Arc<TagAliases> = Arc::new(content_reader_key_names::tag_aliases::parse(
        &[("common/country_tag_aliases/a.txt", aliases.as_bytes())],
        &names(),
    ));
    let empty = BTreeMap::new();
    let none = BTreeSet::new();
    let impassable = BTreeSet::new();
    let names = TriggerNames::default();
    let buildings = Buildings::default();
    let world = World::read(
        frame,
        ContentFacts {
            state_provinces: &empty,
            state_content: Ok(()),
            continents: Err("none"),
            ideologies: Err("none"),
            impassable: &impassable,
            neighbor_states: Err("none"),
            terrain: None,
            dlcs: &none,
            triggers: Ok(&names),
            character_traits: traits,
            tag_aliases: &aliases,
            buildings: Ok(&buildings),
            resource_bonuses: Err("none"),
            leader_roles: roles,
            invasion_prepare_days: Err("none"),
            script_constants: Err("none"),
        },
    );
    run(&world)
}

#[allow(clippy::field_reassign_with_default)]
fn names() -> KeyNames {
    let folded = |names: &[&str]| {
        names
            .iter()
            .map(|name| name.to_ascii_lowercase().into_bytes())
            .collect()
    };
    let mut names = KeyNames::default();
    names.countries = [b"GER".to_vec(), b"AAA".to_vec()].into();
    names.aliases = folded(&[
        "ALI", "ANY", "BLI", "NON", "PRV", "SB1", "SEL", "SOU", "TGT", "VIC",
    ]);
    names.characters = folded(&["GER_rommel", "GER_other"]);
    names
}

fn items(text: &str) -> Vec<content_reader_parser::parser::Item> {
    content_reader_script::triggers::read(text.as_bytes(), &names()).items
}

#[test]
fn faction_leader_scope_and_boolean_trigger_share_the_first_member() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut frame = frame(
        &[
            (100, 1, "AAA", 1, "", ""),
            (200, 2, "GER", 1, "", ""),
            (300, 3, "FRA", 1, "", ""),
        ],
        &[],
    );
    *frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![
            FactionMemberRow {
                country: 200,
                faction: 5,
                ordinal: 1,
            },
            FactionMemberRow {
                country: 100,
                faction: 5,
                ordinal: 0,
            },
        ]));
    assert_eq!(
        check(&frame, "", 2, "faction_leader = { tag = AAA }"),
        Eval::True
    );
    assert_eq!(
        check(&frame, "", 3, "faction_leader = { always = yes }"),
        Eval::False
    );
    for (slot, yes, no) in [
        (1, Eval::True, Eval::False),
        (2, Eval::False, Eval::True),
        (3, Eval::False, Eval::True),
    ] {
        assert_eq!(check(&frame, "", slot, "is_faction_leader = yes"), yes);
        assert_eq!(check(&frame, "", slot, "is_faction_leader = no"), no);
    }
    with_world(&frame, "", Err("none"), |world| {
        assert_eq!(
            world.evaluate(
                &items("is_faction_leader = yes"),
                Scopes::of(Scope::State(7))
            ),
            Eval::False
        );
    });
}

#[test]
fn has_elections_reads_the_recorded_test_and_refuses_older_recordings_by_name() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay_data::rows::PoliticsRow;
    let mut frame = frame(
        &[(100, 1, "GER", 1, "", ""), (200, 2, "SOV", 1, "", "")],
        &[],
    );
    let politics = |first: Recorded<bool>| {
        TableView::Complete(TableRows::Politics(vec![
            PoliticsRow {
                country: 100,
                has_elections: first,
                ..Default::default()
            },
            PoliticsRow {
                country: 200,
                has_elections: Recorded::Value(false),
                ..Default::default()
            },
        ]))
    };
    *frame.tables[TableId::Politics.index()] = politics(Recorded::Value(true));
    for (slot, yes, no) in [(1, Eval::True, Eval::False), (2, Eval::False, Eval::True)] {
        assert_eq!(check(&frame, "", slot, "has_elections = yes"), yes);
        assert_eq!(check(&frame, "", slot, "has_elections = no"), no);
    }
    *frame.tables[TableId::Politics.index()] = politics(Recorded::NotRecorded);
    assert!(
        check(&frame, "", 1, "has_elections = yes")
            .decided()
            .unwrap_err()
            .contains("has elections of politics not recorded")
    );
    assert_eq!(check(&frame, "", 2, "has_elections = no"), Eval::True);
}

#[test]
fn a_country_trigger_reading_no_map_is_answered_without_the_map() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay_data::rows::PoliticsRow;
    let mut frame = frame(&[(100, 1, "GER", 1, "", "")], &[]);
    *frame.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(vec![PoliticsRow {
            country: 100,
            has_elections: Recorded::Value(true),
            ..Default::default()
        }]));
    for table in [TableId::Provinces, TableId::States] {
        *frame.tables[table.index()] = TableView::Unavailable {
            reason: "map refused".into(),
        };
    }
    assert_eq!(check(&frame, "", 1, "has_elections = yes"), Eval::True);
    *frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
        reason: "links refused".into(),
    };
    assert!(
        check(&frame, "", 1, "has_elections = yes")
            .decided()
            .is_err()
    );
}

#[test]
fn ruling_leader_traits_use_role_key_and_definition_identity() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay_data::{
        rows::CharacterRoleTraitRow, rows::CountryLeaderRoleRow, rows::PartyLeaderRow,
        rows::PoliticalPartyRow,
    };
    let root = std::env::temp_dir().join(format!("dashboard-ruling-trait-{}", std::process::id()));
    let file = root.join("common/country_leader/traits.txt");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "leader_traits = { same = {} SAME = {} }").unwrap();
    let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let traits = content_reader_military::leader_traits::read(&layers).unwrap().0;
    std::fs::remove_dir_all(root).unwrap();
    let units = content_reader_key_names::unit_leader::Traits::default();
    let mut frame = frame(&[(100, 0, "AAA", 1, "AAA", "")], &[]);
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 900,
            literal: b"same".to_vec(),
        }]));
    *frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
            country: 100,
            id: 3,
            is_ruling: 1,
            ..Default::default()
        }]));
    *frame.tables[TableId::PartyLeaders.index()] =
        TableView::Complete(TableRows::PartyLeaders(vec![PartyLeaderRow {
            country: 100,
            party: 3,
            character: Some(1),
            role: Some(-7),
            ..Default::default()
        }]));
    *frame.tables[TableId::CountryLeaderRoles.index()] =
        TableView::Complete(TableRows::CountryLeaderRoles(vec![
            CountryLeaderRoleRow {
                character: 1,
                country: 100,
                ideology: 3,
                current_ideology: replay_data::rows::Recorded::Value(Some(Some(99))),
            },
            CountryLeaderRoleRow {
                character: 1,
                country: 100,
                ideology: -7,
                current_ideology: replay_data::rows::Recorded::Value(Some(Some(99))),
            },
        ]));
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
            country: 100,
            leader_ideology: b"changed".to_vec(),
            leader_character: Some(1),
            ..Default::default()
        }]));
    let row = |role, definition| CharacterRoleTraitRow {
        character: 1,
        kind: CharacterRoleTraitRow::COUNTRY_LEADER,
        role,
        tokens: vec![900],
        definitions: vec![definition],
    };
    let evaluate = |frame: &DecodedFrame| {
        with_world_traits(frame, "", Err("none"), Ok((&units, &traits)), |world| {
            world.evaluate(
                &items("has_country_leader_with_trait = same"),
                Scopes::of(Scope::Country(0)),
            )
        })
    };
    *frame.tables[TableId::CharacterRoleTraits.index()] =
        TableView::Complete(TableRows::CharacterRoleTraits(vec![
            row(3, 1),
            row((-7_i32) as u32, 2),
        ]));
    assert_eq!(evaluate(&frame), Eval::False);
    *frame.tables[TableId::CharacterRoleTraits.index()] = TableView::Complete(
        TableRows::CharacterRoleTraits(vec![row((-7_i32) as u32, 1)]),
    );
    assert_eq!(evaluate(&frame), Eval::True);
    *frame.tables[TableId::CharacterRoleTraits.index()] =
        TableView::Complete(TableRows::CharacterRoleTraits(Vec::new()));
    assert_eq!(evaluate(&frame), Eval::False);
    *frame.tables[TableId::CharacterRoleTraits.index()] = TableView::Unavailable {
        reason: "trait read failed".into(),
    };
    assert!(matches!(evaluate(&frame), Eval::Unknown(_)));
}

#[test]
fn default_event_scope_and_country_targets_follow_scope_masks() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let frame = frame(&[(100, 1, "AAA", 1, "AAA", "costume")], &[]);
    let roles = LeaderRoles::new(Vec::new());
    with_world(&frame, "", Ok(&roles), |world| {
        let check = |text: &str| world.evaluate(&items(text), Scopes::empty());
        assert_eq!(check("has_cosmetic_tag = costume"), Eval::False);
        assert_eq!(check("has_government = democracy"), Eval::False);
        assert_eq!(check("AAA = { has_cosmetic_tag = costume }"), Eval::True);
        assert_eq!(check("AAA = { has_cosmetic_tag = other }"), Eval::False);
        assert_eq!(check("ZZZ = { always = no }"), Eval::True);
        for scope in ["root", "this", "from", "prev"] {
            assert_eq!(check(&format!("{scope} = {{}}")), Eval::False);
            assert_eq!(check(&format!("{scope} = {{ always = yes }}")), Eval::False);
            assert_eq!(
                check(&format!("AAA = {{ {scope} = {{ always = yes }} }}")),
                if scope == "this" {
                    Eval::True
                } else {
                    Eval::False
                }
            );
        }
        assert_eq!(
            check(
                "AND = { NOT = { root = {} has_cosmetic_tag = costume } OR = { from = {} AAA = { has_cosmetic_tag = costume } } }"
            ),
            Eval::True
        );
        assert_eq!(
            check("NOT = { AAA = { has_cosmetic_tag = costume } root = {} }"),
            Eval::False
        );
        assert_eq!(
            check("OR = { AAA = { has_cosmetic_tag = other } root = {} }"),
            Eval::False
        );
    });
}

#[test]
fn unknown_pool_inputs_keep_dashboard_boolean_rules() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut frame = frame(&[(100, 1, "AAA", 1, "AAA", "costume")], &[]);
    *frame.tables[TableId::CountryAppearance.index()] = TableView::Unavailable {
        reason: "test missing appearance".into(),
    };
    with_world(&frame, "", Err("none"), |world| {
        let check = |text: &str| world.evaluate(&items(text), Scopes::empty());
        let unknown = check("AAA = { has_cosmetic_tag = costume }");
        assert!(
            matches!(unknown, Eval::Unknown(ref reasons) if reasons.iter().any(|r| r.contains("CountryAppearance")))
        );
        assert_eq!(
            check("OR = { AAA = { has_cosmetic_tag = costume } always = yes }"),
            Eval::True
        );
        assert_eq!(
            check("AND = { AAA = { has_cosmetic_tag = costume } always = no }"),
            Eval::False
        );
        assert!(matches!(
            check("NOT = { AAA = { has_cosmetic_tag = costume } }"),
            Eval::Unknown(_)
        ));
    });
    *frame.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "test missing country catalogue".into(),
    };
    with_world(&frame, "", Err("none"), |world| {
        let unknown = world.evaluate(&items("AAA = { always = yes }"), Scopes::empty());
        assert!(
            matches!(unknown, Eval::Unknown(ref reasons) if reasons.iter().any(|r| r.names("Countries"))),
            "{unknown:?}"
        );
    });
}

#[test]
fn only_loaded_children_count() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let frame = frame(&countries(), &[]);
    with_world(&frame, "", Err("none"), |world| {
        assert_eq!(world.has_children(&items("")), Ok(false));
        assert_eq!(
            world.has_children(&items("unregistered_key = yes")),
            Ok(false)
        );
        assert_eq!(
            world.has_children(&items("unregistered_key = yes always = no")),
            Ok(true)
        );
        let scopes = Scopes::of(Scope::Country(0));
        assert_eq!(
            world.evaluate(
                &items(
                    "if = { limit = { always = no } } unregistered_key = yes else = { always = no }"
                ),
                scopes
            ),
            Eval::False
        );
        assert_eq!(
            world.evaluate(
                &items("unregistered_key = yes else = { always = no }"),
                scopes
            ),
            Eval::True
        );
        assert!(
            world
                .has_children(&items("GER_rommel = { always = no }"))
                .is_err()
        );
    });
}

#[test]
fn a_check_all_alias_needs_the_whole_roster() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut frame = frame(&countries(), &[]);
    let aliases = "SEL = { always = yes }";
    assert_eq!(
        check(&frame, aliases, 0, "SEL = { always = yes }"),
        Eval::True
    );
    *frame.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "not sampled".into(),
    };
    let result = check(&frame, aliases, 0, "SEL = { always = yes }");
    assert!(
        result
            .clone()
            .decided()
            .is_err_and(|why| why.contains("the country roster is not complete")),
        "{result:?}"
    );
}

#[test]
fn a_set_target_carries_nothing_it_was_entered_from() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let frame = frame(&original_countries(), &[]);
    with_world(&frame, "", Err("none"), |world| {
        let leader = Scopes::of(Scope::Leader((7 << 32) | 1));
        let body = items("NOT = { GER = { has_trait = some_trait } }");
        assert_eq!(world.evaluate(&body, leader), Eval::True);
        let country = Scopes {
            root: Scope::Leader((7 << 32) | 1),
            ..Scopes::of(Scope::Country(1))
        };
        assert!(
            world
                .evaluate(&items("THIS = { has_trait = some_trait }"), country)
                .decided()
                .is_err()
        );
    });
}

#[test]
fn a_character_target_reads_only_this_country() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let frame = frame(&countries(), &[]);
    let roles = LeaderRoles::new(vec![content_reader_key_names::characters::LeaderTemplate {
        path: "common/characters/GER.txt".into(),
        tag: "GER_rommel".into(),
        line: None,
        role: Ok(None),
    }]);
    with_world(&frame, "", Ok(&roles), |world| {
        let leader = Scope::Leader((7 << 32) | 1);
        let scopes = Scopes {
            root: leader,
            prev: Some(leader),
            ..Scopes::of(Scope::Country(0))
        };
        assert_eq!(
            world.evaluate(&items("GER_rommel = { always = yes }"), scopes),
            Eval::False
        );
    });
}

#[test]
fn a_fallback_cycle_is_refused_by_name() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let frame = frame(&original_countries(), &[]);
    for aliases in [
        "ALI = { original_tag = GER always = no fallback = ALI }",
        "ALI = { original_tag = GER always = no fallback = BLI }          BLI = { original_tag = SOV always = no fallback = ALI }",
    ] {
        let why = check(&frame, aliases, 1, "tag = ALI")
            .decided()
            .unwrap_err();
        assert!(why.contains("evaluated inside itself"), "{aliases}: {why}");
    }
}

#[test]
fn an_endless_alias_cycle_refuses_every_enclosing_evaluation() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let frame = frame(&original_countries(), &[]);
    for (aliases, trigger) in [
        ("ALI = { variable = ALI.id }", "tag = ALI"),
        (
            "ALI = { variable = BLI.id } BLI = { variable = ALI.id }",
            "tag = ALI",
        ),
        (
            "ALI = { original_tag = GER always = no fallback = ALI }",
            "OR = { tag = ALI always = yes }",
        ),
    ] {
        let why = check(&frame, aliases, 1, trigger)
            .decided()
            .unwrap_err();
        assert!(why.contains("evaluated inside itself"), "{aliases}: {why}");
    }
}

#[test]
fn later_candidates_are_read_only_when_reached() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let frame = frame(&original_countries(), &[]);
    let aliases = "ALI = { original_tag = GER country = SB1 } SB1 = { variable = global.foo }";
    assert_eq!(check(&frame, aliases, 1, "tag = ALI"), Eval::True);
    let aliases = "ALI = { original_tag = GER targets_dynamic = yes }";
    assert_eq!(check(&frame, aliases, 1, "tag = ALI"), Eval::True);
    let aliases = "ALI = { original_tag = GER targets_dynamic = yes always = no }";
    let why = check(&frame, aliases, 1, "tag = ALI")
        .decided()
        .unwrap_err();
    assert!(why.contains("targets_dynamic"), "{why}");
    let aliases = "ALI = { original_tag = GER original_tag = SOV }";
    let why = check(&frame, aliases, 1, "tag = ALI")
        .decided()
        .unwrap_err();
    assert!(
        why.contains(REFUSED) && why.contains("original_tag 2 times"),
        "{why}"
    );
    let aliases =
        "ALI = { original_tag = GER country = SB1 always = no } SB1 = { variable = global.foo }";
    assert!(check(&frame, aliases, 1, "tag = ALI").decided().is_err());
}

#[test]
fn a_candidate_scope_is_fresh() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let frame = frame(&original_countries(), &[]);
    with_world(
        &frame,
        "ALI = { original_tag = GER PREV = { tag = SOV } }",
        Err("none"),
        |world| {
            let caller = Scopes {
                unknown: UnknownLinks {
                    prev: true,
                    ..UnknownLinks::default()
                },
                ..Scopes::of(Scope::Country(4))
            };
            assert_eq!(
                world.evaluate(&items("ALI = { tag = GER }"), caller),
                Eval::True
            );
        },
    );
}

#[test]
fn conditionals_load_limits_and_alternative_chains() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let frame = frame(&countries(), &[]);
    with_world(&frame, "", Err("none"), |world| {
        let scopes = Scopes::of(Scope::Country(0));
        for (body, expected) in [
            (
                "if = { limit = { always = yes } limit = { always = no } always = no } else = { always = yes }",
                Eval::False,
            ),
            (
                "if = { limit = { unregistered_key = yes } limit = { always = no } always = no } else = { always = yes }",
                Eval::True,
            ),
            (
                "if = { limit = { always = no } } else = { limit = { always = no } always = no }",
                Eval::True,
            ),
            (
                "if = { limit = { always = no } else = { always = no } }",
                Eval::False,
            ),
            (
                "if = { limit = { always = yes } if = { limit = { always = no } } else = { always = no } }",
                Eval::False,
            ),
            (
                "if = { limit = { always = yes } if = { limit = { always = no } } else = { limit = { always = no } } else = { always = no } }",
                Eval::False,
            ),
            (
                "if = { limit = { always = no } } else = { limit = { always = no } } else = { always = no }",
                Eval::True,
            ),
        ] {
            assert_eq!(world.evaluate(&items(body), scopes), expected, "{body}");
        }
    });
}

fn colour_refusals(eval: &Evaluator, scopes: Scopes, body: &str, inputs: &[&str]) {
    let Eval::Unknown(reasons) = eval.evaluate_colour(&items(body), scopes, &[]) else {
        panic!("colour should refuse {inputs:?}: {body}");
    };
    assert_eq!(reasons.len(), inputs.len(), "{reasons:?}");
    for input in inputs {
        assert_eq!(
            reasons.iter().filter(|why| why.names(input)).count(),
            1,
            "{reasons:?}"
        );
    }
}

#[test]
fn colour_callbacks_refuse_unknown_conditions_even_without_temp_writes() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let f = variables_frame();
    with_world(&f, "", Err("none"), |world| {
        let eval = Evaluator::new(world, None);
        let scopes = Scopes::of(Scope::Country(1));
        for body in [
            "set_temp_variable = { alpha = 1 } if = { limit = { has_stability > 0.5 } set_temp_variable = { red = 1 } }",
            "set_temp_variable = { alpha = 1 } if = { limit = { has_stability > 0.5 } always = yes }",
            "OR = { has_stability > 0.5 always = yes } set_temp_variable = { alpha = 0 }",
            "has_stability > 0.5 always = no",
        ] {
            colour_refusals(&eval, scopes, body, &["CountryStabilityInputs"]);
        }
        assert_eq!(eval.evaluate_colour(&items("if = { limit = { always = no } has_stability > 0.5 } set_temp_variable = { alpha = 1 }"), scopes, &[]), Eval::True);
        assert_eq!(
            eval.evaluate(&items("OR = { has_stability > 0.5 always = yes }"), scopes),
            Eval::True
        );
    });
}

#[test]
fn colour_callbacks_refuse_uncertain_scratch_even_after_known_overwrites() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let f = variables_frame();
    with_world(&f, "", Err("none"), |world| {
        let eval = Evaluator::new(world, None);
        let scopes = Scopes::of(Scope::Country(1));
        colour_refusals(
            &eval,
            scopes,
            r#"
            if = { limit = { is_in_tech_sharing_group = mod_group }
                set_temp_variable = { scratch = 1 }
                add_to_temp_variable = { scratch = 1 }
                set_temp_variable = { red = 0 }
            }
            set_temp_variable = { red = 1 }
            set_temp_variable = { green = 0 }
            set_temp_variable = { blue = 0 }
            set_temp_variable = { alpha = 1 }
            set_temp_variable = { highlighted = 0 }
        "#,
            &["ResearchModifiers"],
        );
        assert_eq!(
            eval.evaluate_colour(
                &items("set_temp_variable = { alpha = 1 } set_temp_variable = { red = 1 }"),
                scopes,
                &[]
            ),
            Eval::True
        );
    });
}

#[test]
fn colour_callbacks_name_each_missing_input_once_independently_of_destinations() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let f = variables_frame();
    with_world(&f, "", Err("none"), |world| {
        let eval = Evaluator::new(world, None);
        let scopes = Scopes::of(Scope::Country(1));
        for destination in ["red", "blue"] {
            colour_refusals(
                &eval,
                scopes,
                &format!(
                    r#"
                set_temp_variable = {{ alpha = 1 }}
                if = {{ limit = {{ has_stability > 0.5 }} set_temp_variable = {{ red = 1 }} }}
                if = {{ limit = {{ is_in_tech_sharing_group = mod_group }} set_temp_variable = {{ {destination} = 1 }} }}
            "#
                ),
                &["CountryStabilityInputs", "ResearchModifiers"],
            );
        }
        colour_refusals(
            &eval,
            scopes,
            r#"
            set_temp_variable = { alpha = 1 }
            if = { limit = { has_stability > 0.5 }
                set_temp_variable = { red = 1 } set_temp_variable = { blue = 1 }
            }
            if = { limit = { has_stability > 0.7 } set_temp_variable = { green = 1 } }
            set_temp_variable = { red = stability }
        "#,
            &["CountryStabilityInputs"],
        );
    });
}

#[test]
fn colour_callbacks_keep_a_later_failure_after_an_alias_observed_another_input() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let f = positive_variables_frame();
    let aliases =
        "ALI = { original_tag = GER OR = { is_in_tech_sharing_group = mod_group always = yes } }";
    with_world(&f, aliases, Err("none"), |world| {
        colour_refusals(
            &Evaluator::new(world, None),
            Scopes::of(Scope::Country(1)),
            "set_temp_variable = { red = ALI.stability } set_temp_variable = { alpha = 1 }",
            &["ResearchModifiers", "CountryStabilityInputs"],
        );
    });
}

#[test]
fn colour_callbacks_preserve_input_identity_through_an_unresolved_alias() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let f = positive_variables_frame();
    with_world(
        &f,
        "ALI = { original_tag = GER is_in_tech_sharing_group = mod_group }",
        Err("none"),
        |world| {
            colour_refusals(
                &Evaluator::new(world, None),
                Scopes::of(Scope::Country(1)),
                "set_temp_variable = { red = ALI.stability }",
                &["ResearchModifiers"],
            );
        },
    );
}

#[test]
fn calculated_stability_and_its_modifier_callback_share_the_missing_field_identity() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let mut f = variables_frame();
    let row = replay_data::rows::CountryStabilityInputRow {
        country: 101,
        stability_factor: None,
        party_popularity_stability_factor: Some(0),
        war_stability_factor: Some(0),
        offensive_war_stability_factor: Some(0),
        defensive_war_stability_factor: Some(0),
        war_support_factor: Some(0),
        ..Default::default()
    };
    *f.tables[TableId::CountryStabilityInputs.index()] =
        TableView::Complete(TableRows::CountryStabilityInputs(vec![row.clone()]));
    with_world(&f, "", Err("none"), |world| {
        let eval = Evaluator::new(world, None);
        let (_, failures) = super::refusal::capture(|| {
            assert!(eval.country_stability_value(1, false).is_err());
            assert!(eval.stability_modifier_input(&row, 1, 0).is_err());
        });
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(
            failures
                .first()
                .unwrap()
                .names("CountryStabilityInputs.stability_factor")
        );
    });
}

#[test]
fn original_tag_alias_and_direct_scope_read_share_the_failed_appearance_input() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let mut f = positive_variables_frame();
    *f.tables[TableId::CountryAppearance.index()] = TableView::Unavailable {
        reason: "synthetic appearance acquisition failed".into(),
    };
    with_world(
        &f,
        "ALI = { original_tag = GER always = yes }",
        Err("none"),
        |world| {
            colour_refusals(
                &Evaluator::new(world, None),
                Scopes::of(Scope::Country(1)),
                "OR = { tag = ALI always = yes } OR = { is_subject = yes always = yes } set_temp_variable = { alpha = 1 }",
                &["CountryAppearance"],
            );
        },
    );
}

#[test]
fn cached_core_gaps_and_direct_table_reads_keep_one_source_identity() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let mut f = positive_variables_frame();
    *f.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
        reason: "synthetic missing links".into(),
    };
    with_world(&f, "", Err("none"), |world| {
        for _ in 0..2 {
            let (_, failures) = super::refusal::capture(|| {
                world
                    .core_gap()
                    .unwrap()
                    .context(|why| format!("core preflight: {why}"));
                world.table_gap("CountryLinks").unwrap();
                assert!(super::world::table::<tables::CountryLinks>(world.tables(), 0).is_err());
            });
            assert_eq!(failures.len(), 1, "{failures:?}");
            assert!(failures.first().unwrap().names("CountryLinks"));
        }
    });
}

#[test]
fn colour_callbacks_tag_alias_context_preserves_the_one_source_failure() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let f = positive_variables_frame();
    with_world(
        &f,
        "ALI = { original_tag = GER is_in_tech_sharing_group = mod_group }",
        Err("none"),
        |world| {
            colour_refusals(
                &Evaluator::new(world, None),
                Scopes::of(Scope::Country(1)),
                "tag = ALI",
                &["ResearchModifiers"],
            );
        },
    );
}

#[test]
fn colour_callbacks_sharing_lookup_keeps_lexical_failure_global_across_scopes() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let mut f = positive_variables_frame();
    *f.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
        reason: "synthetic missing dictionary".into(),
    };
    with_world(&f, "", Err("none"), |world| {
        let eval = Evaluator::new(world, None);
        for body in [
            "is_in_tech_sharing_group = mod_group",
            "is_in_tech_sharing_group = mod_group GER = { is_in_tech_sharing_group = mod_group }",
            "GER = { is_in_tech_sharing_group = mod_group } is_in_tech_sharing_group = mod_group",
            "GER = { set_temp_variable = { red = 0 } is_in_tech_sharing_group = mod_group }",
        ] {
            colour_refusals(
                &eval,
                Scopes::of(Scope::Country(2)),
                body,
                &["LexicalTokens"],
            );
        }
    });
}

#[test]
fn colour_callbacks_global_table_failures_have_no_country_scope_even_when_cached() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let mut f = positive_variables_frame();
    *f.tables[TableId::SimulationFeatures.index()] = TableView::Unavailable {
        reason: "synthetic missing global inputs".into(),
    };
    with_world(&f, "", Err("none"), |world| {
        let eval = Evaluator::new(world, None);
        for _ in 0..2 {
            colour_refusals(
                &eval,
                Scopes::of(Scope::Country(2)),
                "set_temp_variable = { red = global.states^num } GER = { set_temp_variable = { blue = global.states^num } }",
                &["SimulationFeatures"],
            );
        }
    });
}

#[test]
fn colour_callbacks_identify_roster_failures_across_iterators_and_builtin_arrays() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let mut f = positive_variables_frame();
    if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *f.tables[TableId::CountryCounterInputs.index()]
    {
        for row in rows {
            row.owned_states = None.into();
            row.controlled_states = None.into();
        }
    }
    with_world(&f, "", Err("none"), |world| {
        let eval = Evaluator::new(world, None);
        let scopes = Scopes::of(Scope::Country(2));
        let mut previous = None;
        for body in [
            "OR = { any_owned_state = { always = yes } always = yes } set_temp_variable = { red = owned_states^num }",
            "set_temp_variable = { red = owned_states^num } OR = { any_owned_state = { always = yes } always = yes }",
        ] {
            colour_refusals(&eval, scopes, body, &["CountryCounterInputs.owned_states"]);
            let result = eval.evaluate_colour(&items(body), scopes, &[]);
            if let Some(previous) = previous {
                assert_eq!(result, previous);
            }
            previous = Some(result);
        }
        colour_refusals(
            &eval,
            scopes,
            "OR = { any_owned_state = { always = yes } always = yes } set_temp_variable = { red = controlled_states^num }",
            &[
                "CountryCounterInputs.owned_states",
                "CountryCounterInputs.controlled_states",
            ],
        );
        let Eval::Unknown(reasons) = eval.evaluate_colour(&items(
            "OR = { any_owned_state = { always = yes } always = yes } GER = { OR = { any_owned_state = { always = yes } always = yes } }"
        ), scopes, &[]) else { panic!("both country rosters must refuse") };
        assert_eq!(reasons.len(), 2, "{reasons:?}");
        assert!(
            reasons
                .iter()
                .any(|why| why.names("CountryCounterInputs.owned_states (Country(1))")),
            "{reasons:?}"
        );
        assert!(
            reasons
                .iter()
                .any(|why| why.names("CountryCounterInputs.owned_states (Country(2))")),
            "{reasons:?}"
        );
    });
}

#[test]
fn unavailable_roster_order_keeps_its_field_identity_through_the_iterator_fallback() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let mut f = positive_variables_frame();
    if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *f.tables[TableId::CountryCounterInputs.index()]
    {
        rows.iter_mut()
            .for_each(|row| row.owned_states = Recorded::NotRecorded);
    }
    with_world(&f, "", Err("none"), |world| {
        colour_refusals(
            &Evaluator::new(world, None),
            Scopes::of(Scope::Country(1)),
            "OR = { any_owned_state = { set_temp_variable = { red = 1 } } always = yes } set_temp_variable = { blue = owned_states^num }",
            &["CountryCounterInputs.owned_states"],
        );
    });
}

#[test]
fn colour_callbacks_refuse_optional_destination_insertion_without_a_store() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let f = variables_frame();
    with_world(&f, "", Err("none"), |world| {
        let eval = Evaluator::new(world, None);
        let scopes = Scopes::of(Scope::Country(1));
        for operation in ["divide_temp_variable", "modulo_temp_variable"] {
            colour_refusals(
                &eval,
                scopes,
                &format!(
                    r#"
                set_temp_variable = {{ alpha = 1 }}
                if = {{ limit = {{ has_stability > 0.5 }}
                    {operation} = {{ var = red?1 value = 0 }}
                }}
            "#
                ),
                &["CountryStabilityInputs"],
            );
        }
    });
}

#[test]
fn colour_callbacks_refuse_uncertain_iterator_stop_and_scope_rosters() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let roles = LeaderRoles::new(Vec::new());
    let mut f = variables_frame();
    *f.tables[TableId::States.index()] = TableView::Complete(TableRows::States(
        [64, 65]
            .into_iter()
            .map(|id| replay_data::rows::StateRow {
                id,
                owner: 101,
                ..Default::default()
            })
            .collect(),
    ));
    if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *f.tables[TableId::CountryCounterInputs.index()]
    {
        rows.iter_mut()
            .find(|row| row.country == 101)
            .unwrap()
            .owned_states = Some(vec![64, 65]).into();
    }
    with_world(&f, "", Ok(&roles), |world| {
        let eval = Evaluator::new(world, None);
        colour_refusals(
            &eval,
            Scopes::of(Scope::Country(1)),
            r#"
            set_temp_variable = { alpha = 1 }
            any_owned_state = {
                if = { limit = { state = 64 } ROOT = { has_stability > 0.5 } }
                if = { limit = { state = 65 } set_temp_variable = { red = 1 } }
            }
        "#,
            &["CountryStabilityInputs"],
        );
    });
    if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *f.tables[TableId::CountryCounterInputs.index()]
    {
        rows.iter_mut()
            .find(|row| row.country == 101)
            .unwrap()
            .owned_states = None.into();
    }
    with_world(&f, "", Ok(&roles), |world| {
        colour_refusals(
            &Evaluator::new(world, None),
            Scopes::of(Scope::Country(1)),
            r#"
            OR = { any_owned_state = { always = yes } always = yes }
            set_temp_variable = { alpha = 1 } set_temp_variable = { red = 1 }
        "#,
            &["CountryCounterInputs.owned_states"],
        );
    });
}

#[test]
fn a_character_scope_is_named_by_the_templates() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let frame = frame(&countries(), &[]);
    let roles = LeaderRoles::new(vec![content_reader_key_names::characters::LeaderTemplate {
        path: "common/characters/GER.txt".into(),
        tag: "GER_rommel".into(),
        line: None,
        role: Ok(None),
    }]);
    with_world(&frame, "", Ok(&roles), |world| {
        let scopes = Scopes::of(Scope::Country(0));
        for key in ["GER_rommel", "ger_ROMMEL"] {
            let body = items(&format!("{key} = {{ tag = GER }}"));
            assert_eq!(world.evaluate(&body, scopes), Eval::False, "{key}");
            let body = items(&format!("{key} = {{ always = yes }}"));
            assert_eq!(world.evaluate(&body, scopes), Eval::False, "{key}");
        }
        assert_eq!(
            world.evaluate(&items("GER_other = { always = no }"), scopes),
            Eval::True
        );
    });
}

#[test]
fn a_link_through_a_link_is_its_scope_s_own() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let frame = frame(&original_countries(), &[]);
    with_world(&frame, "", Err("none"), |world| {
        let scopes = Scopes {
            from: Some(Scope::Country(2)),
            ..Scopes::of(Scope::Country(1))
        };
        assert_eq!(
            world.evaluate(&items("FROM = { tag = D01 }"), scopes),
            Eval::True
        );
        assert_eq!(
            world.evaluate(&items("GER = { FROM = { tag = D01 } }"), scopes),
            Eval::True
        );
        for body in [
            "FROM = { FROM = { tag = D01 } }",
            "FROM.FROM = { tag = D01 }",
            "ROOT = { FROM = { tag = D01 } }",
        ] {
            assert!(
                world.evaluate(&items(body), scopes).decided().is_err(),
                "{body}"
            );
        }
        assert_eq!(
            world.evaluate(
                &items("FROM = { FROM.FROM = { always = yes } }"),
                Scopes::combat()
            ),
            Eval::True
        );
    });
}

#[test]
fn combat_reset_clears_country_carrier_but_tag_reads_the_combat_country() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    struct Side;
    impl Combat for Side {
        fn country(&self) -> Result<u32, crate::triggers::Failure> {
            Ok(4)
        }
        fn trigger(
            &self,
            _: &str,
            _: &content_reader_parser::parser::Item,
            _: Scope,
            _: &Evaluator,
        ) -> Option<Eval> {
            None
        }
        fn category_share(&self, _: u64) -> Result<i64, crate::triggers::Failure> {
            Err("unused combat equipment count".into())
        }
    }
    let frame = frame(&countries(), &[104]);
    with_world(&frame, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, Some(&Side));
        assert_eq!(
            evaluator.evaluate(
                &items("has_country_flag = x"),
                Scopes::of(Scope::Country(4))
            ),
            Eval::True
        );
        for trigger in ["has_country_flag = x", "has_tactic = absent"] {
            assert_eq!(
                evaluator.evaluate(&items(trigger), Scopes::combat()),
                Eval::False,
                "{trigger}"
            );
        }
        assert_eq!(
            evaluator.evaluate(&items("tag = FRA"), Scopes::combat()),
            Eval::True
        );
    });
}

fn countries() -> Vec<(u32, u32, &'static str, i32, &'static str, &'static str)> {
    vec![
        (100, 0, "GER", 2, "", ""),
        (101, 1, "D01", 1, "GER", ""),
        (102, 2, "D02", 1, "GER", ""),
        (103, 3, "SOV", 5, "", "SOV_left_opposition"),
        (104, 4, "FRA", 3, "", ""),
    ]
}

fn original_countries() -> Vec<(u32, u32, &'static str, i32, &'static str, &'static str)> {
    countries()
        .into_iter()
        .map(|(raw, slot, tag, owned, original, cosmetic)| {
            (raw, slot + 1, tag, owned, original, cosmetic)
        })
        .collect()
}

#[test]
fn original_predicate_selects_both_countries_and_compares_raw_aliases() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut f = frame(
        &[(100, 1, "GER", 1, "SOV", ""), (200, 2, "SOV", 1, "", "")],
        &[],
    );
    assert_eq!(check(&f, "", 2, "original_tag = GER"), Eval::True);
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *f.tables[TableId::CountryLinks.index()]
    {
        rows.push(CountryLinkRow {
            id: 300,
            canonical_slot: 2,
        });
    }
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *f.tables[TableId::CountryAppearance.index()]
    {
        rows[0].original_tag_reference = Recorded::Value(Some(300));
        rows[0].original_tag = Recorded::Value(None);
        rows[1].original_tag = Recorded::NotRecorded;
    }
    assert_eq!(check(&f, "", 1, "original_tag = SOV"), Eval::True);
    with_world(&f, "", Err("none"), |world| {
        assert_eq!(world.same_tag(-1, -1), Ok(true));
        assert_eq!(world.same_tag(-1, 0), Ok(false));
        assert!(
            world
                .same_tag(-1, 100)
                .unwrap_err()
                .contains("raw reference 4294967295")
        );
    });
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *f.tables[TableId::CountryAppearance.index()]
    {
        rows[0].original_tag_reference = Recorded::NotRecorded;
    }
    assert!(
        check(&f, "", 1, "original_tag = SOV")
            .decided()
            .unwrap_err()
            .contains("original tag reference of country appearance not recorded")
    );
}

#[test]
fn original_predicate_uses_static_substitution_and_alias_dispatch() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let f = frame(
        &[
            (100, 1, "GER", 0, "SOV", ""),
            (101, 2, "D01", 1, "GER", ""),
            (200, 3, "SOV", 1, "", ""),
        ],
        &[],
    );
    assert_eq!(check(&f, "", 2, "original_tag = GER"), Eval::True);
    assert_eq!(
        check(&f, "ALI = { variable = THIS }", 2, "original_tag = ALI"),
        Eval::True
    );
}

#[test]
fn original_predicate_resolves_a_known_null_operand_through_reserved_original() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut f = frame(&original_countries(), &[]);
    if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *f.tables[TableId::ReservedCountryNaming.index()]
    {
        rows[0].original_tag_reference = Recorded::Value(Some(100));
        rows[0].original_tag = Recorded::Value(None);
    }
    assert_eq!(
        check(&f, "ALI = { always = no }", 1, "original_tag = ALI"),
        Eval::True
    );
    if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *f.tables[TableId::ReservedCountryNaming.index()]
    {
        rows[0].original_tag_reference = Recorded::NotRecorded;
    }
    assert!(
        check(&f, "ALI = { always = no }", 1, "original_tag = ALI")
            .decided()
            .unwrap_err()
            .contains("original tag reference of reserved country naming not recorded")
    );
}

#[test]
fn reserved_world_owned_count_preserves_signed_values_and_independent_refusals() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut f = frame(&original_countries(), &[]);
    for count in [
        Recorded::Value(Some(4)),
        Recorded::Value(Some(0)),
        Recorded::Value(Some(-7)),
        Recorded::Value(None),
        Recorded::NotRecorded,
    ] {
        if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
            &mut *f.tables[TableId::ReservedCountryNaming.index()]
        {
            rows[0].owned_state_count = count;
            rows[0].original_tag_buckets = Recorded::Value(None);
            rows[0].country_array = Recorded::NotRecorded;
        }
        with_world(&f, "", Err("none"), |world| {
            assert_eq!(world.owned_state_count(1), Ok(2));
            match count {
                Recorded::Value(Some(raw)) => {
                    assert_eq!(world.owned_state_count(0), Ok(raw));
                    assert_eq!(world.exists(0), Ok(raw > 0));
                }
                _ => assert!(
                    world
                        .owned_state_count(0)
                        .unwrap_err()
                        .contains("owned state count of reserved country naming")
                ),
            }
        });
    }
}

#[test]
fn original_iterators_keep_bucket_order_duplicates_and_skip_resolved_zero() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut f = frame(
        &[(100, 1, "GER", 1, "GER", ""), (200, 2, "SOV", 1, "", "")],
        &[],
    );
    if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *f.tables[TableId::ReservedCountryNaming.index()]
    {
        rows[0].original_tag_buckets = Recorded::Value(Some(vec![vec![], vec![0, 100], vec![]]));
    }
    assert_eq!(
        check(
            &f,
            "",
            1,
            "any_country_with_original_tag = { original_tag_to_check = GER count = 2 always = yes }"
        ),
        Eval::True
    );
    assert_eq!(
        check(
            &f,
            "",
            1,
            "any_country_with_original_tag = { original_tag_to_check = GER count = 3 always = yes }"
        ),
        Eval::False
    );
    if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *f.tables[TableId::ReservedCountryNaming.index()]
    {
        rows[0].original_tag_buckets = Recorded::Value(Some(vec![vec![], vec![300], vec![]]));
    }
    assert!(
        check(
            &f,
            "",
            1,
            "any_country_with_original_tag = { original_tag_to_check = GER always = yes }"
        )
        .decided()
        .unwrap_err()
        .contains("raw reference 300")
    );
}

#[test]
fn original_aliases_and_null_iterators_read_later_membership_only_when_reached() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut f = frame(&original_countries(), &[]);
    *f.tables[TableId::ReservedCountryNaming.index()] = TableView::Unavailable {
        reason: "later membership not sampled".into(),
    };
    assert_eq!(
        check(&f, "ALI = { original_tag = GER }", 1, "tag = ALI"),
        Eval::True
    );
    assert!(
        check(
            &f,
            "ALI = { original_tag = GER always = no }",
            1,
            "tag = ALI"
        )
        .decided()
        .unwrap_err()
        .contains("native lists are unavailable")
    );
    *f.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                tag: b"GER".to_vec(),
                original_tag_reference: Recorded::Value(Some(-7)),
                original_tag: Recorded::NotRecorded,
                original_tag_buckets: Recorded::Value(None),
                ..Default::default()
            },
        ]));
    *f.tables[TableId::CountryAppearance.index()] = TableView::Unavailable {
        reason: "unrelated positive countries".into(),
    };
    for (trigger, expected) in [
        (
            "any_country_with_original_tag = { original_tag_to_check = THIS always = yes }",
            Eval::False,
        ),
        (
            "all_country_with_original_tag = { original_tag_to_check = THIS always = no }",
            Eval::True,
        ),
    ] {
        assert_eq!(check(&f, "", 0, trigger), expected);
    }
    if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *f.tables[TableId::ReservedCountryNaming.index()]
    {
        rows[0].original_tag_reference = Recorded::NotRecorded;
    }
    assert!(
        check(
            &f,
            "",
            0,
            "any_country_with_original_tag = { original_tag_to_check = THIS always = yes }"
        )
        .decided()
        .unwrap_err()
        .contains("original tag reference of reserved country naming not recorded")
    );
}

#[test]
fn an_alias_is_the_first_candidate_whose_triggers_pass() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let aliases = "ALI = { original_tag = GER has_country_flag = x } \
                   PRV = { original_tag = GER PREV = { tag = SOV } }";
    let flagged = frame(&original_countries(), &[101]);
    assert_eq!(
        check(&flagged, aliases, 4, "ALI = { tag = D01 }"),
        Eval::True
    );
    assert_eq!(check(&flagged, aliases, 2, "tag = ALI"), Eval::True);
    assert_eq!(check(&flagged, aliases, 1, "tag = ALI"), Eval::False);
    let both = frame(&original_countries(), &[100, 101]);
    assert_eq!(check(&both, aliases, 1, "tag = ALI"), Eval::True);
    assert_eq!(check(&both, aliases, 4, "PRV = { tag = GER }"), Eval::True);
    assert_eq!(check(&both, aliases, 5, "PRV = { tag = GER }"), Eval::False);
    let none = frame(&original_countries(), &[]);
    assert_eq!(
        check(&none, aliases, 1, "ALI = { always = yes }"),
        Eval::False
    );
}

#[test]
fn aliases_use_exact_bucket_and_physical_array_order_with_first_tie() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut f = frame(&original_countries(), &[101, 102]);
    let original = "ALI = { original_tag = GER has_country_flag = x }";
    assert_eq!(check(&f, original, 2, "tag = ALI"), Eval::True);
    if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *f.tables[TableId::ReservedCountryNaming.index()]
    {
        rows[0].original_tag_buckets = Recorded::Value(Some(vec![
            vec![],
            vec![102, 101, 102],
            vec![],
            vec![],
            vec![],
            vec![],
        ]));
        rows[0].country_array =
            Recorded::Value(Some(vec![0, u32::MAX, 102, 100, 101, 102, 103, 104]));
    }
    assert_eq!(check(&f, original, 3, "tag = ALI"), Eval::True);
    assert_eq!(
        check(&f, "ALI = { has_country_flag = x }", 3, "tag = ALI"),
        Eval::True
    );
    assert_eq!(
        check(
            &f,
            "ALI = { has_country_flag = x country_score = { base = 1 } }",
            3,
            "tag = ALI"
        ),
        Eval::True
    );
    if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *f.tables[TableId::ReservedCountryNaming.index()]
    {
        rows[0].country_array = Recorded::NotRecorded;
    }
    assert!(
        check(&f, "ALI = { always = yes }", 3, "tag = ALI")
            .decided()
            .unwrap_err()
            .contains("country array of reserved country naming not recorded")
    );
}

#[test]
fn scores_fallbacks_and_every_country() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let aliases = "SOU = { original_tag = SOV country_score = { base = 1 \
                     modifier = { add = 2 has_cosmetic_tag = SOV_left_opposition exists = yes } } } \
                   VIC = { original_tag = FRA has_focus_tree = vichy_french_focus } \
                   NON = { original_tag = FRA has_focus_tree = other fallback = GER } \
                   ANY = { has_country_flag = x } \
                   TGT = { targets = { SOV D02 GER } has_country_flag = x }";
    let frame = frame(&original_countries(), &[102, 104]);
    assert_eq!(check(&frame, aliases, 4, "tag = SOU"), Eval::True);
    assert_eq!(check(&frame, aliases, 5, "tag = VIC"), Eval::True);
    assert_eq!(check(&frame, aliases, 1, "tag = NON"), Eval::True);
    assert_eq!(check(&frame, aliases, 3, "tag = ANY"), Eval::True);
    assert_eq!(check(&frame, aliases, 3, "tag = TGT"), Eval::True);
}

#[test]
fn an_event_target_alias_is_refused_by_name() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let aliases = "SB1 = { global_event_target = appeased }";
    let mut frame = frame(&countries(), &[]);
    *frame.tables[TableId::GlobalSavedEventTargets.index()] = TableView::Unavailable {
        reason: "not sampled".into(),
    };
    match check(&frame, aliases, 0, "SB1 = { always = yes }") {
        Eval::Unknown(reasons) => assert!(
            reasons
                .iter()
                .any(|why| why.contains("tag alias SB1") && why.contains("saved event target")),
            "{reasons:?}"
        ),
        other => panic!("{other:?}"),
    }
}

type Store<'a> = (u8, u32, &'a [(&'a str, i64)]);

fn with_variables(mut frame: DecodedFrame, stores: &[Store<'_>]) -> DecodedFrame {
    use replay_data::{rows::ScriptVariableEntry, rows::ScriptVariablesRow, rows::StateRow};
    let mut rows: Vec<ScriptVariablesRow> = stores
        .iter()
        .map(|(scope_kind, scope_id, entries)| {
            let mut entries: Vec<_> = entries
                .iter()
                .map(|(name, value_raw)| ScriptVariableEntry {
                    name: name.as_bytes().to_vec(),
                    value_raw: *value_raw,
                })
                .collect();
            entries.sort_by(|a, b| a.name.cmp(&b.name));
            ScriptVariablesRow {
                scope_kind: *scope_kind,
                scope_id: *scope_id,
                seed: replay_data::rows::Recorded::Value(None),
                entries,
            }
        })
        .collect();
    rows.sort_by_key(|row| (row.scope_kind, row.scope_id));
    *frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::from_rows::<tables::ScriptVariables>(rows));
    *frame.tables[TableId::States.index()] =
        TableView::Complete(TableRows::from_rows::<tables::States>(vec![StateRow {
            id: 64,
            owner: 100,
            controller: 100,
            ..Default::default()
        }]));
    frame
}

const ONE: i64 = 100_000;
const GER: i64 = 0x4052_4547;

fn variables_frame() -> DecodedFrame {
    variables_for(frame(&countries(), &[]))
}

fn positive_variables_frame() -> DecodedFrame {
    variables_for(frame(&original_countries(), &[]))
}

fn variables_for(frame: DecodedFrame) -> DecodedFrame {
    let state = (0xffff_ffff_c000_0000_u64 | 64) as i64;
    with_variables(
        frame,
        &[
            (0, 0, &[("foo", 3 * ONE), ("owner", GER)]),
            (
                1,
                100,
                &[
                    ("x", 5 * ONE),
                    ("yger", 7 * ONE),
                    ("z^1", ONE),
                    ("arr^num", 2 * ONE),
                    ("arr^0", state),
                    ("arr^1", 65 * ONE),
                ],
            ),
            (1, 101, &[]),
            (1, 102, &[]),
            (1, 103, &[("x", ONE)]),
            (1, 104, &[]),
            (2, 64, &[("s", 2 * ONE)]),
        ],
    )
}

#[test]
fn variables_read_the_recorded_stores_as_the_game_reads_them() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let frame = variables_frame();
    let decided = |slot, trigger: &str| check(&frame, "", slot, trigger).decided();
    for trigger in [
        "check_variable = { x = 5 }",
        "check_variable = { x > 4.5 }",
        "check_variable = { var = x value = 6 compare = less_than }",
        "check_variable = { var = x value = 5 }",
        "check_variable = { global.foo = 3 }",
        "check_variable = { SOV.x = 1 }",
        "check_variable = { missing < 1 }",
        "check_variable = { missing?2 = 2 }",
        "check_variable = { y@ROOT = 7 }",
        "check_variable = { z^1 = 1 }",
        "check_variable = { 64.s = 2 }",
        "check_variable = { x = var:SOV.y?5 }",
        "has_variable = x",
        "has_variable = ROOT.z^1",
    ] {
        assert_eq!(decided(0, trigger), Ok(true), "{trigger}");
    }
    for trigger in [
        "check_variable = { x = 4 }",
        "has_variable = missing",
        "has_variable = temp_var:x",
    ] {
        assert_eq!(decided(0, trigger), Ok(false), "{trigger}");
    }
    assert_eq!(decided(3, "check_variable = { ROOT.x = 1 }"), Ok(true));
    with_world(&frame, "", Err("none"), |world| {
        assert_eq!(
            world
                .evaluate(
                    &items("check_variable = { s = 2 }"),
                    Scopes::of(Scope::State(64))
                )
                .decided(),
            Ok(true)
        );
        let null = Scopes::of(Scope::State(NULL_STATE));
        assert_eq!(
            world
                .evaluate(&items("check_variable = { foo = 3 }"), null)
                .decided(),
            Ok(true)
        );
    });
}

#[test]
fn stored_scopes_decode_where_a_scope_is_read() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let frame = positive_variables_frame();
    let aliases = "SB1 = { variable = global.owner }";
    for trigger in [
        "var:global.owner = { tag = GER }",
        "tag = global.owner",
        "check_variable = { global.owner = ROOT }",
        "tag = SB1",
        "any_state_in = { array = arr always = yes }",
        "any_state_in = { array = arr count = 2 always = yes }",
    ] {
        assert_eq!(
            check(&frame, aliases, 1, trigger).decided(),
            Ok(true),
            "{trigger}"
        );
    }
    assert_eq!(
        check(
            &frame,
            "",
            1,
            "any_state_in = { array = arr count = 2 is_owned_by = GER }"
        )
        .decided(),
        Ok(false)
    );
    assert_eq!(
        check(&frame, "", 4, "tag = global.owner").decided(),
        Ok(false)
    );
}

#[test]
fn recorded_builtin_opinion_uses_the_callback_scope_and_directed_sparse_relations() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay_data::rows::RelationOpinionRow;

    let tags: Vec<_> = (0..5000)
        .map(|id| {
            format!(
                "{}{}{}",
                char::from(b'A' + (id / 676) as u8),
                char::from(b'A' + (id / 26 % 26) as u8),
                char::from(b'A' + (id % 26) as u8)
            )
        })
        .filter(|tag| tag != "GER" && tag != "FRA")
        .collect();
    let mut countries = vec![
        (100, 11, "GER", 3, "GER", ""),
        (101, 12, "FRA", 1, "FRA", ""),
    ];
    countries.extend(tags.iter().enumerate().map(|(index, tag)| {
        (
            index as u32 + 200,
            index as u32 + 13,
            tag.as_str(),
            0,
            tag.as_str(),
            "",
        )
    }));
    assert_eq!(countries.len(), 5000);
    let mut f = with_variables(
        frame(&countries, &[]),
        &[
            (0, 0, &[("target", 0x4041_5246)]),
            (1, 100, &[("opinionfra", 99 * ONE)]),
        ],
    );
    crate::test_country_links::extend(
        &mut f,
        [CountryLinkRow {
            id: 103,
            canonical_slot: 12,
        }],
    );
    *f.tables[TableId::RelationOpinions.index()] =
        TableView::Complete(TableRows::RelationOpinions(
            [(100, 103, -37), (101, 100, 12), (101, 101, 23)]
                .into_iter()
                .map(
                    |(owner_country, target_country, opinion)| RelationOpinionRow {
                        owner_country,
                        target_country,
                        opinion,
                    },
                )
                .collect(),
        ));
    for trigger in [
        "check_variable = { THIS.opinion@FRA = -37 }",
        "check_variable = { THIS.opinion@var:global.target = -37 }",
        "check_variable = { FRA.opinion@ROOT = 12 }",
        "check_variable = { FRA.opinion@THIS = 23 }",
        "check_variable = { THIS.opinion@GER?99 = 0 }",
        "has_variable = opinion@FRA",
    ] {
        assert_eq!(check(&f, "", 11, trigger).decided(), Ok(true), "{trigger}");
    }
    for (operand, reason) in [
        ("THIS.opinion", "slot zero"),
        ("THIS.opinion@0?99", "slot zero"),
        ("THIS.opinion@FRA^random", "random"),
        ("ROOT.opinion@ROOT", "ROOT"),
    ] {
        let why = check(&f, "", 11, &format!("check_variable = {{ {operand} = 0 }}"))
            .decided()
            .unwrap_err();
        assert!(why.contains(reason), "{why}");
    }
    *f.tables[TableId::RelationOpinions.index()] = TableView::Unavailable {
        reason: "not sampled".into(),
    };
    let why = check(&f, "", 11, "check_variable = { THIS.opinion@GER?99 = 0 }")
        .decided()
        .unwrap_err();
    assert!(why.contains("RelationOpinions"), "{why}");
}

#[test]
fn recorded_builtin_faction_leader_encodes_first_member_and_requires_membership() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut f = frame(
        &[
            (100, 1, "AAA", 1, "", ""),
            (200, 2, "BBB", 1, "", ""),
            (300, 3, "CCC", 1, "", ""),
        ],
        &[],
    );
    *f.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![
            FactionMemberRow {
                country: 100,
                faction: 5,
                ordinal: 1,
            },
            FactionMemberRow {
                country: 200,
                faction: 5,
                ordinal: 0,
            },
        ]));
    for (slot, trigger, expected) in [
        (1, "tag = faction_leader", false),
        (2, "tag = AAA.faction_leader", true),
        (2, "tag = THIS.faction_leader@random", true),
        (3, "check_variable = { faction_leader = 0 }", true),
    ] {
        assert_eq!(
            check(&f, "", slot, trigger).decided(),
            Ok(expected),
            "{trigger}",
        );
    }
    let why = check(&f, "", 1, "tag = THIS.faction_leader^random")
        .decided()
        .unwrap_err();
    assert!(why.contains("random"), "{why}");
    *f.tables[TableId::FactionMembers.index()] = TableView::Unavailable {
        reason: "synthetic gap".into(),
    };
    let why = check(&f, "", 3, "check_variable = { faction_leader = 0 }")
        .decided_input()
        .unwrap_err();
    assert!(why.names("FactionMembers"), "{why}");
}

#[test]
fn recorded_builtin_owned_controlled_states_counts_aliases_and_requires_complete_inputs() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay_data::{rows::ProvinceRow, rows::StateRow};

    let mut f = with_variables(
        frame(&original_countries(), &[]),
        &[(1, 100, &[("num_owned_controlled_states", 99 * ONE)])],
    );
    let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *f.tables[TableId::CountryLinks.index()]
    else {
        panic!("complete links");
    };
    links.push(CountryLinkRow {
        id: 110,
        canonical_slot: 1,
    });
    *f.tables[TableId::States.index()] = TableView::Complete(TableRows::States(
        [
            (100, 110),
            (110, 100),
            (100, 101),
            (101, 100),
            (0, 100),
            (100, 0),
            (100, 100),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (owner, controller))| StateRow {
            id: index as u32 + 1,
            owner,
            controller,
            ..Default::default()
        })
        .collect(),
    ));
    *f.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(
        (1..=6)
            .map(|id| ProvinceRow {
                id,
                state: id,
                ..Default::default()
            })
            .collect(),
    ));
    *f.tables[TableId::StateCores.index()] = TableView::Unavailable {
        reason: "not needed".into(),
    };
    for (slot, trigger) in [
        (1, "check_variable = { num_owned_controlled_states = 2 }"),
        (
            1,
            "check_variable = { num_owned_controlled_states@FRA^num = 2 }",
        ),
        (2, "check_variable = { num_owned_controlled_states?99 = 0 }"),
        (
            2,
            "check_variable = { GER.num_owned_controlled_states = 2 }",
        ),
    ] {
        assert_eq!(
            check(&f, "", slot, trigger).decided(),
            Ok(true),
            "{trigger}"
        );
    }
    let TableView::Complete(TableRows::States(states)) = &mut *f.tables[TableId::States.index()]
    else {
        panic!("complete states");
    };
    states[0].controller = 999;
    let trigger = "check_variable = { num_owned_controlled_states = 2 }";
    let why = check(&f, "", 1, trigger).decided_input().unwrap_err();
    assert!(why.names("CountryLinks"), "{why}");
    *f.tables[TableId::States.index()] = TableView::Unavailable {
        reason: "not sampled".into(),
    };
    let why = check(&f, "", 1, trigger).decided_input().unwrap_err();
    assert!(why.names("States"), "{why}");
}

#[test]
fn runtime_only_operands_are_refused_by_name() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut frame = variables_frame();
    *frame.tables[TableId::GlobalSavedEventTargets.index()] = TableView::Unavailable {
        reason: "not sampled".into(),
    };
    let why = |trigger: &str| check(&frame, "", 0, trigger).decided().unwrap_err();
    let builtin = check(&frame, "", 0, "check_variable = { stability > 0 }")
        .decided_input()
        .unwrap_err();
    assert!(
        builtin.contains("stability") && builtin.names("CountryStabilityInputs"),
        "{builtin}"
    );
    assert_eq!(
        check(&frame, "", 0, "has_variable = stability").decided(),
        Ok(false)
    );
    assert!(why("check_variable = { event_target:t.x = 0 }").contains("saved event target"));
    assert!(why("check_variable = { random < 1 }").contains("random"));
    let written = why("set_temp_variable_to_random = { var = t } check_variable = { x = 5 }");
    assert!(written.contains("set_temp_variable_to_random"), "{written}");
    with_world(&frame, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        let scopes = Scopes::of(Scope::Country(0));
        let _ = evaluator.evaluate(&items("set_temp_variable = { t = 1 }"), scopes);
        assert_eq!(
            evaluator
                .evaluate(&items("check_variable = { ROOT.x = 5 }"), scopes)
                .decided(),
            Ok(true)
        );
        let stale = Evaluator::new(world, None).with_temps(Temps::Stale);
        assert!(stale.scoped_value(b"x", scopes).is_err());
        assert_eq!(stale.scoped_value(b"ROOT.x", scopes), Ok(5 * ONE));
    });
}

#[test]
fn repeated_mean_time_operands_are_exact_only_for_a_last_literal() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let frame = variables_frame();
    with_world(&frame, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        let scopes = Scopes::of(Scope::Country(0));
        let weight = |fields: &str| {
            let text = format!("test = {{ base = {{ {fields} }} }}");
            let database = content_reader_military::combat_tactics::parse(text.as_bytes(), &names());
            assert!(
                database.unsupported.is_empty(),
                "{:?}",
                database.unsupported
            );
            let base = database.tactics[0].fields[0].value.items().unwrap();
            evaluator.mean_time_to_happen("test", base, scopes)
        };
        assert_eq!(
            weight("modifier = { factor = x always = yes }"),
            Ok(5 * ONE)
        );
        assert_eq!(
            weight("modifier = { factor = temp_var:x factor = 3 always = yes }"),
            Ok(3 * ONE)
        );
        let why = weight("modifier = { factor = temp_var:x factor = x always = yes }").unwrap_err();
        assert!(
            why.contains(REFUSED) && why.contains("factor is assigned 2 times"),
            "{why}"
        );
    });
}

#[test]
fn an_injected_temp_after_an_unported_writer_is_refused() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let frame = variables_frame();
    let scopes = Scopes {
        target_state: Some(64),
        ..Scopes::of(Scope::Country(0))
    };
    with_world(&frame, "", Err("none"), |world| {
        let read = "var:target_state = { always = yes }";
        assert_eq!(world.evaluate(&items(read), scopes).decided(), Ok(true));
        let why = world
            .evaluate(
                &items(&format!(
                    "set_temp_variable_to_random = {{ var = target_state }} {read}"
                )),
                scopes,
            )
            .decided()
            .unwrap_err();
        assert!(why.contains("set_temp_variable_to_random"), "{why}");
    });
}

fn set_saved(f: &mut DecodedFrame, rows: Vec<replay_data::rows::GlobalSavedEventTargetRow>) {
    *f.tables[TableId::GlobalSavedEventTargets.index()] = TableView::Complete(
        TableRows::from_rows::<tables::GlobalSavedEventTargets>(rows),
    );
}

#[test]
fn saved_entries_keep_empty_scope_masks_and_caller_clearing() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay_data::rows::GlobalSavedEventTargetRow as Row;
    let mut f = frame(&original_countries(), &[]);
    set_saved(
        &mut f,
        vec![
            Row {
                name: b"kept".to_vec(),
                live: Row::FACTION,
                ..Default::default()
            },
            Row {
                ordinal: 1,
                name: b"selected".to_vec(),
                country: 100,
                ..Default::default()
            },
            Row {
                ordinal: 2,
                name: b"empty".to_vec(),
                state: NULL_STATE as i32,
                ..Default::default()
            },
        ],
    );
    with_world(&f, "", Err("none"), |world| {
        for name in ["kept", "empty"] {
            for body in ["", "always = yes"] {
                assert_eq!(
                    world.evaluate(
                        &items(&format!("event_target:{name} = {{ {body} }}")),
                        Scopes::empty()
                    ),
                    Eval::False
                );
            }
        }
        assert_eq!(
            world.evaluate(&items("has_event_target = kept"), Scopes::empty()),
            Eval::True
        );
        assert_eq!(
            world.evaluate(
                &items("event_target:selected = { tag = GER }"),
                Scopes::empty()
            ),
            Eval::True
        );
        for bare in [false, true] {
            let scopes = Scopes {
                root: Scope::Leader((9u64 << 32) | 1),
                bare,
                ..Scopes::of(Scope::Country(1))
            };
            for suffix in ["", ".THIS"] {
                let kept = world.evaluate(
                    &items(&format!(
                        "event_target:kept{suffix} = {{ has_character_flag = x }}"
                    )),
                    scopes,
                );
                if bare {
                    assert_eq!(kept, Eval::False);
                } else {
                    assert!(
                        matches!(kept, Eval::Unknown(ref why) if why.iter().any(|why| why.contains("unit leader"))),
                        "{kept:?}"
                    );
                }
                assert_eq!(
                    world.evaluate(
                        &items(&format!(
                            "event_target:selected{suffix} = {{ has_character_flag = x }}"
                        )),
                        scopes
                    ),
                    Eval::False
                );
            }
        }
    });
}

#[test]
fn an_unsaved_first_name_still_resolves_but_does_not_make_a_dead_target_valid() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay_data::rows::GlobalSavedEventTargetRow as Row;
    let mut f = frame(&countries(), &[]);
    for (country, expected) in [(100, Eval::True), (0, Eval::False)] {
        set_saved(
            &mut f,
            vec![Row {
                name: b"first".to_vec(),
                country,
                live: Row::UNSAVED_NAME,
                ..Default::default()
            }],
        );
        for trigger in [
            "has_event_target = first",
            "event_target:first = { always = yes }",
            "tag = event_target:first",
        ] {
            assert_eq!(check(&f, "", 0, trigger), expected, "{trigger}");
        }
    }
}

#[test]
fn saved_globals_decide_scope_operands_aliases_and_cleared_names() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay_data::rows::GlobalSavedEventTargetRow;
    let mut f = with_variables(
        frame(&original_countries(), &[]),
        &[(1, 100, &[("x", 5 * ONE)])],
    );
    let row = GlobalSavedEventTargetRow {
        ordinal: 0,
        name: b"chosen".to_vec(),
        country: 100,
        ..Default::default()
    };
    set_saved(&mut f, vec![row]);
    assert_eq!(
        check(&f, "", 1, "event_target:chosen = { tag = GER }"),
        Eval::True
    );
    assert_eq!(check(&f, "", 1, "tag = event_target:chosen"), Eval::True);
    assert_eq!(
        check(&f, "", 1, "check_variable = { event_target:chosen.x = 5 }"),
        Eval::True
    );
    assert_eq!(
        check(
            &f,
            "",
            1,
            "check_variable = { global_event_target:chosen.x = 5 }"
        ),
        Eval::True
    );
    assert_eq!(
        check(&f, "", 1, "event_target:Chosen = { always = yes }"),
        Eval::False
    );
    assert_eq!(check(&f, "", 1, "has_event_target = chosen"), Eval::True);
    assert_eq!(
        check(&f, "", 1, "tag = global_event_target:chosen"),
        Eval::True
    );
    assert_eq!(check(&f, "", 2, "tag = event_target:chosen"), Eval::False);
    assert_eq!(
        check(&f, "SEL = { event_target = chosen }", 1, "tag = SEL"),
        Eval::True
    );
    assert_eq!(
        check(&f, "SEL = { global_event_target = chosen }", 1, "tag = SEL"),
        Eval::True
    );
    set_saved(&mut f, vec![]);
    assert_eq!(
        check(&f, "", 1, "event_target:chosen = { always = yes }"),
        Eval::False
    );
    assert_eq!(check(&f, "", 1, "has_event_target = chosen"), Eval::False);
    assert_eq!(check(&f, "", 1, "tag = event_target:chosen"), Eval::False);
    *f.tables[TableId::GlobalSavedEventTargets.index()] = TableView::Unavailable {
        reason: "not sampled".into(),
    };
    assert!(
        check(&f, "", 1, "event_target:chosen = { always = yes }")
            .decided()
            .is_err()
    );
}
#[test]
fn saved_target_aliases_append_id_before_parsing_dotted_values() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay_data::rows::GlobalSavedEventTargetRow;
    let mut f = frame(&countries(), &[]);
    set_saved(
        &mut f,
        vec![
            GlobalSavedEventTargetRow {
                ordinal: 0,
                name: b"chosen".to_vec(),
                ..Default::default()
            },
            GlobalSavedEventTargetRow {
                ordinal: 1,
                name: b"cho".to_vec(),
                country: 100,
                ..Default::default()
            },
        ],
    );
    for key in ["event_target", "global_event_target"] {
        let alias = format!("SEL = {{ {key} = chosen.foo }}");
        assert_eq!(check(&f, &alias, 0, "tag = SEL"), Eval::True);
        assert_eq!(check(&f, &alias, 1, "tag = SEL"), Eval::False);
    }
}

#[test]
fn first_duplicate_invalid_record_stops_global_lookup_and_state_wins_context() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay_data::rows::GlobalSavedEventTargetRow;
    let mut f = frame(&countries(), &[]);
    let first = GlobalSavedEventTargetRow {
        ordinal: 0,
        name: b"x".to_vec(),
        character: 42,
        live: 0,
        ..Default::default()
    };
    let second = GlobalSavedEventTargetRow {
        ordinal: 1,
        country: 100,
        ..first.clone()
    };
    set_saved(&mut f, vec![first, second]);
    assert_eq!(
        check(&f, "", 0, "event_target:x = { always = yes }"),
        Eval::False
    );
    set_saved(
        &mut f,
        vec![GlobalSavedEventTargetRow {
            ordinal: 0,
            name: b"x".to_vec(),
            state: 1,
            country: 100,
            ..Default::default()
        }],
    );
    assert_eq!(
        check(&f, "", 0, "event_target:x = { tag = GER }"),
        Eval::False
    );
    assert_eq!(check(&f, "", 0, "tag = event_target:x"), Eval::True);
}

#[test]
fn global_event_target_prefix_is_a_variable_operand_and_invalid_as_a_context_key() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay_data::rows::GlobalSavedEventTargetRow;
    let mut f = frame(&countries(), &[]);
    set_saved(
        &mut f,
        vec![GlobalSavedEventTargetRow {
            name: b"x".to_vec(),
            country: 100,
            ..Default::default()
        }],
    );
    *f.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::from_rows::<
        tables::LexicalTokens,
    >(vec![LexicalTokenRow {
        token: 0x3d94,
        literal: b"global_event_target".to_vec(),
    }]));
    assert_eq!(check(&f, "", 0, "tag = global_event_target:x"), Eval::True);
    assert_eq!(
        check(&f, "", 0, "GLOBAL_EVENT_TARGET:x = { always = yes }"),
        Eval::False
    );
}

#[test]
fn saved_characters_select_leaders_for_contexts_and_characters_for_variables() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    use super::saved_event_targets::{Reader, Selected as SavedSelection};
    use replay_data::rows::GlobalSavedEventTargetRow;
    let mut f = frame(&countries(), &[]);
    let leader = (9u64 << 32) | 1;
    set_saved(
        &mut f,
        vec![GlobalSavedEventTargetRow {
            name: b"x".to_vec(),
            character: 8,
            live: 1,
            character_leader: Some(leader),
            ..Default::default()
        }],
    );
    with_world(&f, "", Err("none"), |world| {
        assert_eq!(
            world.recorded().saved_selection(b"x", Reader::Context),
            Ok(SavedSelection::Leader(leader))
        );
        assert_eq!(
            world.recorded().saved_selection(b"x", Reader::Variable),
            Ok(SavedSelection::Character(8))
        );
    });
}

#[test]
fn format99_has_ideology_reads_current_pointer_independently_of_map_key() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay_data::{rows::CharacterRow, rows::CountryLeaderRoleRow, rows::Recorded};
    let mut frame = frame(&[(100, 1, "AAA", 1, "AAA", "")], &[]);
    *frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![CharacterRow {
            id: 7,
            country: 100,
            ..Default::default()
        }]));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 11,
            literal: b"synthetic_type".to_vec(),
        }]));
    *frame.tables[TableId::CountryLeaderRoles.index()] =
        TableView::Complete(TableRows::CountryLeaderRoles(vec![CountryLeaderRoleRow {
            character: 7,
            country: 100,
            ideology: -7,
            current_ideology: Recorded::Value(Some(Some(11))),
        }]));
    let names = TriggerNames {
        counts: Some(content_reader_script::trigger_keys::CountTriggerNames {
            ideology_types: BTreeSet::from([b"synthetic_type".to_vec()]),
            ..Default::default()
        }),
        ..Default::default()
    };
    let aliases = Arc::new(TagAliases::default());
    let dlcs = BTreeSet::new();
    let evaluate = |frame: &DecodedFrame| {
        let world = World::read(frame, ContentFacts::content_only(&dlcs, &names, &aliases));
        world.evaluate(
            &items("has_ideology=synthetic_type"),
            Scopes::of(Scope::Character(7)),
        )
    };
    assert_eq!(evaluate(&frame), Eval::True);
    let TableView::Complete(TableRows::CountryLeaderRoles(rows)) =
        &mut *frame.tables[TableId::CountryLeaderRoles.index()]
    else {
        unreachable!()
    };
    rows[0].current_ideology = Recorded::Value(Some(Some(12)));
    assert_eq!(evaluate(&frame), Eval::False);
    let TableView::Complete(TableRows::CountryLeaderRoles(rows)) =
        &mut *frame.tables[TableId::CountryLeaderRoles.index()]
    else {
        unreachable!()
    };
    rows[0].current_ideology = Recorded::Value(Some(None));
    assert_eq!(evaluate(&frame), Eval::False);
    let TableView::Complete(TableRows::CountryLeaderRoles(rows)) =
        &mut *frame.tables[TableId::CountryLeaderRoles.index()]
    else {
        unreachable!()
    };
    rows[0].current_ideology = Recorded::Value(None);
    assert!(matches!(evaluate(&frame), Eval::Unknown(reasons)
        if reasons.iter().any(|reason| reason.names("CountryLeaderRoles.current_ideology (Character(7))")
            && reason.contains("not captured"))));
    let TableView::Complete(TableRows::CountryLeaderRoles(rows)) =
        &mut *frame.tables[TableId::CountryLeaderRoles.index()]
    else {
        unreachable!()
    };
    rows[0].current_ideology = Recorded::NotRecorded;
    assert!(matches!(evaluate(&frame), Eval::Unknown(reasons)
        if reasons.iter().any(|reason| reason.names("CountryLeaderRoles.current_ideology (Character(7))")
            && reason.contains("not recorded"))));
}

#[test]
fn in_array_uses_exact_fixed5_and_truncated_wrapping_length() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    for (length, wanted, expected) in [
        (190_000, "5", false),
        (200_000, "5", true),
        (200_000, "5.00001", false),
        (429_496_729_700_000, "7.00001", true),
        (214_748_364_800_000, "7.00001", false),
        (-100_000, "7.00001", false),
    ] {
        let f = with_variables(
            frame(&countries(), &[]),
            &[(
                1,
                100,
                &[("arr^num", length), ("arr^0", 700_001), ("arr^1", 500_000)],
            )],
        );
        for body in [
            format!("array = arr value = {wanted}"),
            format!("arr = {wanted}"),
        ] {
            assert_eq!(
                check(&f, "", 0, &format!("is_in_array = {{ {body} }}")).decided(),
                Ok(expected),
                "{length} {body}"
            );
        }
    }
}

#[test]
fn in_array_preserves_unused_index_fields_and_reads_tooltip_shorthand() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let f = with_variables(
        frame(&countries(), &[]),
        &[(
            1,
            100,
            &[
                ("arr^num", 100_000),
                ("arr^0", 700_000),
                ("tooltip^num", 100_000),
                ("tooltip^0", 800_000),
                ("not_tooltip^num", 100_000),
                ("not_tooltip^0", 900_000),
            ],
        )],
    );
    for (body, expected) in [
        ("array=arr value=7 size=1", true),
        ("array=arr value=7 index=1", true),
        ("array=arr value=7 size=2 index=3", true),
        ("tooltip=8", true),
        ("not_tooltip=9", true),
    ] {
        assert_eq!(
            check(&f, "", 0, &format!("is_in_array={{{body}}}")).decided(),
            Ok(expected),
            "{body}"
        );
    }
    for operand in ["tooltip", "not_tooltip"] {
        let why = check(
            &f,
            "",
            0,
            &format!("is_in_array={{array=arr value=7 {operand}=7}}"),
        )
        .decided()
        .unwrap_err();
        assert!(
            why.contains(&format!("repeated variable operand {operand}")),
            "{why}"
        );
    }
}

#[test]
fn country_flag_and_completed_focus_require_only_their_recorded_inputs() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay_data::rows::CompletedFocusRow;
    let mut healthy = frame(&countries(), &[100]);
    *healthy.tables[TableId::CompletedFocuses.index()] =
        TableView::Complete(TableRows::CompletedFocuses(vec![CompletedFocusRow {
            country: 100,
            token: b"mod_focus".to_vec(),
            ..Default::default()
        }]));
    for unrelated_gap in [false, true] {
        let mut f = healthy.clone();
        if unrelated_gap {
            *f.tables[TableId::Provinces.index()] = TableView::Unavailable {
                reason: "unrelated map gap".into(),
            };
        }
        for body in ["has_country_flag=x", "has_completed_focus=mod_focus"] {
            assert_eq!(check(&f, "", 0, body), Eval::True, "{body}");
            assert_eq!(check(&f, "", 1, body), Eval::False, "{body}");
        }
    }
    for (body, required) in [
        (
            "has_country_flag=x",
            &[
                TableId::Countries,
                TableId::CountryLinks,
                TableId::LexicalTokens,
                TableId::ScriptFlags,
            ][..],
        ),
        (
            "has_completed_focus=mod_focus",
            &[
                TableId::Countries,
                TableId::CountryLinks,
                TableId::CompletedFocuses,
            ][..],
        ),
    ] {
        for &table in required {
            let mut missing = healthy.clone();
            *missing.tables[table.index()] = TableView::Unavailable {
                reason: "required predicate input missing".into(),
            };
            let why = check(&missing, "", 0, body).decided_input().unwrap_err();
            assert!(why.names(&format!("{table:?}")), "{body}: {why}");
        }
        let mut missing_link = healthy.clone();
        *missing_link.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![]));
        assert!(
            check(&missing_link, "", 0, body)
                .decided_input()
                .unwrap_err()
                .names("CountryLinks")
        );
        let mut missing_country = healthy.clone();
        *missing_country.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![]));
        assert!(
            check(&missing_country, "", 0, body)
                .decided()
                .unwrap_err()
                .contains("Countries")
        );
    }
}

#[test]
fn a_state_trigger_reading_no_provinces_is_answered_without_them() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay_data::rows::StateRow;
    let mut f = frame(&countries(), &[]);
    let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *f.tables[TableId::ReservedCountryNaming.index()]
    else {
        panic!("the fixture records reserved country naming");
    };
    rows[0].owned_state_count = Recorded::Value(Some(2));
    *f.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![StateRow {
        id: 64,
        owner: 100,
        controller: 104,
        ..Default::default()
    }]));
    *f.tables[TableId::ScriptFlags.index()] =
        TableView::Complete(TableRows::ScriptFlags(vec![ScriptFlagRow {
            scope_kind: 2,
            scope_id: 64,
            token: 900,
            value: 1,
            ..Default::default()
        }]));
    *f.tables[TableId::Provinces.index()] = TableView::Unavailable {
        reason: "unrelated province gap".into(),
    };
    let dlcs = BTreeSet::new();
    let names = TriggerNames::default();
    let aliases = Arc::new(TagAliases::default());
    let evaluate = |frame: &DecodedFrame, body: &str| {
        World::read(frame, ContentFacts::content_only(&dlcs, &names, &aliases))
            .evaluate(&items(body), Scopes::of(Scope::State(64)))
            .decided()
    };
    for (body, expected) in [
        ("state = 64", true),
        ("is_owned_by = GER", true),
        ("is_controlled_by = GER", false),
        ("is_controlled_by = FRA", true),
        ("is_owned_and_controlled_by = GER", false),
        ("has_state_flag = x", true),
    ] {
        assert_eq!(evaluate(&f, body), Ok(expected), "{body}");
    }
    assert!(
        evaluate(&f, "is_fully_controlled_by = FRA")
            .unwrap_err()
            .contains("unrelated province gap")
    );
    *f.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
        reason: "links refused".into(),
    };
    assert!(evaluate(&f, "is_owned_by = GER").is_err());
    *f.tables[TableId::States.index()] = TableView::Unavailable {
        reason: "states refused".into(),
    };
    assert!(
        evaluate(&f, "has_state_flag = x")
            .unwrap_err()
            .contains("states refused")
    );
}

#[test]
fn continent_reads_the_retained_token_and_names_missing_operands() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay_data::{rows::Recorded, rows::StateRow};
    let mut f = frame(&countries(), &[]);
    *f.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![StateRow {
        id: 64,
        continent: Recorded::Value(Some(901)),
        ..Default::default()
    }]));
    *f.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 901,
            literal: b"second_mod_continent".to_vec(),
        }]));
    *f.tables[TableId::Provinces.index()] = TableView::Unavailable {
        reason: "unrelated province gap".into(),
    };
    let dlcs = BTreeSet::new();
    let names = TriggerNames::default();
    let aliases = Arc::new(TagAliases::default());
    let continents = vec!["first_mod_continent".into(), "second_mod_continent".into()];
    for (operand, wanted, expected) in [
        (
            Recorded::Value(Some(901)),
            "SECOND_MOD_CONTINENT",
            Some(true),
        ),
        (
            Recorded::Value(Some(901)),
            "first_mod_continent",
            Some(false),
        ),
        (
            Recorded::Value(Some(0)),
            "second_mod_continent",
            Some(false),
        ),
        (Recorded::Value(None), "second_mod_continent", None),
        (Recorded::NotRecorded, "second_mod_continent", None),
        (Recorded::Value(Some(901)), "undefined_continent", None),
    ] {
        let TableView::Complete(TableRows::States(states)) =
            &mut *f.tables[TableId::States.index()]
        else {
            unreachable!()
        };
        states[0].continent = operand;
        let world = World::read(
            &f,
            ContentFacts {
                continents: Ok(&continents),
                ..ContentFacts::content_only(&dlcs, &names, &aliases)
            },
        );
        let result = world
            .evaluate(
                &items(&format!("is_on_continent = {wanted}")),
                Scopes::of(Scope::State(64)),
            )
            .decided();
        match expected {
            Some(expected) => assert_eq!(result, Ok(expected)),
            None => {
                let why = result.unwrap_err();
                assert!(
                    why.contains("continent") || why.contains("continent of states"),
                    "{why}"
                );
            }
        }
    }
    let world = World::read(&f, ContentFacts::content_only(&dlcs, &names, &aliases));
    assert!(
        world
            .evaluate(
                &items("is_on_continent = first_mod_continent"),
                Scopes::of(Scope::State(64))
            )
            .decided()
            .unwrap_err()
            .contains("content")
    );
    *f.tables[TableId::States.index()] = TableView::Unavailable {
        reason: "continent own table gap".into(),
    };
    let world = World::read(
        &f,
        ContentFacts {
            continents: Ok(&continents),
            ..ContentFacts::content_only(&dlcs, &names, &aliases)
        },
    );
    assert!(
        world
            .evaluate(
                &items("is_on_continent = first_mod_continent"),
                Scopes::of(Scope::State(64))
            )
            .decided_input()
            .unwrap_err()
            .names("States")
    );
}

#[test]
fn country_leader_ideology_reads_the_recorded_first_leader_without_a_character() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (cat, _folder) = crate::catalogue::tests::folder_catalogue(&[(
        "common/ideologies/synthetic.txt",
        "ideologies={mod_group={types={First_Subtype={} visible_subtype={}}}}",
    )]);
    let ideologies = content_reader_key_names::ideologies::read(&cat.content.layers)
        .unwrap()
        .0;
    let mut f = frame(&countries(), &[]);
    let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *f.tables[TableId::CountryAppearance.index()]
    else {
        unreachable!()
    };
    rows[0].leader_ideology = b"first_subtype".to_vec();
    rows[0].leader_character = None;
    *f.tables[TableId::Provinces.index()] = TableView::Unavailable {
        reason: "unrelated map gap".into(),
    };
    let dlcs = BTreeSet::new();
    let names = TriggerNames::default();
    let aliases = Arc::new(TagAliases::default());
    let world = World::read(
        &f,
        ContentFacts {
            ideologies: Ok(&ideologies),
            ..ContentFacts::content_only(&dlcs, &names, &aliases)
        },
    );
    for (slot, wanted, expected) in [
        (0, "FIRST_SUBTYPE", true),
        (0, "visible_subtype", false),
        (1, "first_subtype", false),
    ] {
        assert_eq!(
            world
                .evaluate(
                    &items(&format!("has_country_leader_ideology = {wanted}")),
                    Scopes::of(Scope::Country(slot))
                )
                .decided(),
            Ok(expected)
        );
    }
    let unknown = world
        .evaluate(
            &items("has_country_leader_ideology = mod_group"),
            Scopes::of(Scope::Country(0)),
        )
        .decided()
        .unwrap_err();
    assert!(
        unknown.contains("mod_group") && unknown.contains("declaration"),
        "{unknown}"
    );
}

#[test]
fn in_array_stops_before_a_second_alias_index_and_evaluates_length_once() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let f = with_variables(
        frame(&original_countries(), &[]),
        &[(
            1,
            100,
            &[("arr^num", 200_000), ("arr^0", 700_000), ("arr^1", 500_000)],
        )],
    );
    with_world(
        &f,
        "ALI={original_tag=GER add_to_temp_variable={calls=1}}",
        Err("none"),
        |world| {
            assert_eq!(
                world
                    .evaluate(
                        &items("is_in_array={array=ALI.arr value=7} check_variable={calls=2}"),
                        Scopes::of(Scope::Country(0))
                    )
                    .decided(),
                Ok(true)
            );
        },
    );
}

#[test]
fn country_government_and_numeric_original_keep_local_table_admission() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay_data::{rows::PoliticalPartyRow, rows::PoliticsRow};
    let (cat, _folder) = crate::catalogue::tests::folder_catalogue(&[(
        "common/ideologies/synthetic.txt",
        "ideologies={mod_group={} other_group={}}",
    )]);
    let ideologies = content_reader_key_names::ideologies::read(&cat.content.layers)
        .unwrap()
        .0;
    let dlcs = BTreeSet::new();
    let names = TriggerNames::default();
    let aliases = Arc::new(TagAliases::default());
    let evaluate = |frame: &DecodedFrame, text: &str, scopes: Scopes| {
        let world = World::read(
            frame,
            ContentFacts {
                ideologies: Ok(&ideologies),
                ..ContentFacts::content_only(&dlcs, &names, &aliases)
            },
        );
        world.evaluate(&items(text), scopes)
    };
    let mut healthy = with_variables(
        frame(&original_countries(), &[]),
        &[(1, 100, &[("chosen_group", 3_000_001)])],
    );
    *healthy.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![
            PoliticalPartyRow {
                country: 100,
                id: 20,
                token: b"mod_group".to_vec(),
                is_ruling: 1,
                ..Default::default()
            },
            PoliticalPartyRow {
                country: 100,
                id: 30,
                token: b"other_group".to_vec(),
                ..Default::default()
            },
            PoliticalPartyRow {
                country: 101,
                id: 30,
                token: b"other_group".to_vec(),
                is_ruling: 1,
                ..Default::default()
            },
        ]));
    *healthy.tables[TableId::Politics.index()] = TableView::Complete(TableRows::Politics(vec![
        PoliticsRow {
            country: 100,
            ruling_party: Some(20),
            ..Default::default()
        },
        PoliticsRow {
            country: 101,
            ruling_party: Some(30),
            ..Default::default()
        },
    ]));
    *healthy.tables[TableId::Provinces.index()] = TableView::Unavailable {
        reason: "unrelated map gap".into(),
    };
    let scopes = Scopes {
        root: Scope::Country(2),
        from: Some(Scope::Country(2)),
        prev: Some(Scope::Country(1)),
        ..Scopes::of(Scope::Country(1))
    };
    for (operand, expected) in [
        ("MOD_GROUP", true),
        ("other_group", false),
        ("THIS", true),
        ("PREV", true),
        ("ROOT", false),
        ("FROM", false),
        ("GER", true),
        ("var:20.99999", true),
        ("var:30", false),
        ("var:chosen_group", false),
        ("var:4294967316", true),
    ] {
        let body = format!("has_government={operand}");
        assert_eq!(
            evaluate(&healthy, &body, scopes).decided(),
            Ok(expected),
            "{body}"
        );
        if matches!(operand, "ROOT" | "FROM" | "other_group" | "var:30") {
            let mut scope = scopes;
            scope.this = Scope::Country(2);
            assert_eq!(
                evaluate(&healthy, &body, scope).decided(),
                Ok(true),
                "{body}"
            );
        }
    }
    for invalid in ["var:0", "20", "chosen_group", "VAR:20"] {
        assert!(
            evaluate(&healthy, &format!("has_government={invalid}"), scopes)
                .decided()
                .unwrap_err()
                .contains("null-object"),
            "{invalid}"
        );
    }
    assert_eq!(check(&healthy, "", 1, "original_tag=THIS "), Eval::True);
    for required in [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::PoliticalParties,
    ] {
        let mut missing = healthy.clone();
        *missing.tables[required.index()] = TableView::Unavailable {
            reason: "required government input missing".into(),
        };
        assert!(
            evaluate(&missing, "has_government=mod_group", scopes)
                .decided()
                .unwrap_err()
                .contains(&format!("{required:?}"))
        );
    }
    let mut missing = healthy.clone();
    *missing.tables[TableId::Politics.index()] = TableView::Unavailable {
        reason: "scoped ruling read missing".into(),
    };
    assert_eq!(
        evaluate(&missing, "has_government=mod_group", scopes),
        Eval::True
    );
    assert!(
        evaluate(&missing, "has_government=FROM", scopes)
            .decided()
            .unwrap_err()
            .contains("Politics")
    );
    let absent = World::read(
        &healthy,
        ContentFacts::content_only(&dlcs, &names, &aliases),
    );
    assert!(
        absent
            .evaluate(&items("has_government=mod_group"), scopes)
            .decided()
            .unwrap_err()
            .contains("ideolog")
    );
    let TableView::Complete(rows) = &*healthy.tables[TableId::Countries.index()] else {
        unreachable!()
    };
    let rows = rows.clone();
    *healthy.tables[TableId::Countries.index()] = TableView::Truncated { rows, missing: 1 };
    assert!(
        check(&healthy, "", 1, "original_tag=THIS")
            .decided()
            .unwrap_err()
            .contains("whether THIS is a country tag")
    );
    assert_eq!(check(&healthy, "", 1, "original_tag=THIS "), Eval::True);
    *healthy.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                original_tag_reference: Recorded::Value(Some(0)),
                ..Default::default()
            },
        ]));
    *healthy.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "positive roster missing".into(),
    };
    *healthy.tables[TableId::CountryAppearance.index()] = TableView::Unavailable {
        reason: "positive appearances missing".into(),
    };
    assert_eq!(check(&healthy, "", 0, "original_tag=THIS "), Eval::True);
    *healthy.tables[TableId::ReservedCountryNaming.index()] = TableView::Unavailable {
        reason: "reserved getter missing".into(),
    };
    assert!(
        check(&healthy, "", 0, "original_tag=THIS ")
            .decided()
            .unwrap_err()
            .contains("reserved country naming")
    );
}

#[test]
fn missing_state_content_refuses_only_state_predicates_that_read_it() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let f = with_variables(frame(&original_countries(), &[]), &[]);
    let dlcs = BTreeSet::new();
    let names = TriggerNames::default();
    let aliases = Arc::new(TagAliases::default());
    let world = World::read(&f, ContentFacts::content_only(&dlcs, &names, &aliases));
    let scopes = Scopes::of(Scope::State(64));
    for body in ["state=64", "is_owned_by=GER", "is_controlled_by=GER"] {
        assert_eq!(world.evaluate(&items(body), scopes), Eval::True, "{body}");
    }
    for body in [
        "is_fully_controlled_by=GER",
        "impassable=no",
        "is_coastal=no",
    ] {
        assert!(
            world
                .evaluate(&items(body), scopes)
                .decided()
                .unwrap_err()
                .contains("content geography"),
            "{body}"
        );
    }
}

#[test]
fn government_id_and_scope_resolution_cover_every_country() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay_data::{rows::PoliticalPartyRow, rows::PoliticsRow};
    let (cat, _folder) = crate::catalogue::tests::folder_catalogue(&[(
        "common/ideologies/synthetic.txt",
        "ideologies={mod_group={} other_group={}}",
    )]);
    let ideologies = content_reader_key_names::ideologies::read(&cat.content.layers)
        .unwrap()
        .0;
    let tags: Vec<_> = (0..24)
        .map(|id| {
            format!(
                "{}{}{}",
                char::from(b'A' + (id / 676) as u8),
                char::from(b'A' + (id / 26 % 26) as u8),
                char::from(b'A' + (id % 26) as u8),
            )
        })
        .collect();
    let countries: Vec<_> = tags
        .iter()
        .enumerate()
        .map(|(id, tag)| {
            (
                id as u32 + 100,
                id as u32 + 1,
                tag.as_str(),
                0,
                tag.as_str(),
                "",
            )
        })
        .collect();
    let mut f = frame(&countries, &[]);
    *f.tables[TableId::Provinces.index()] = TableView::Unavailable {
        reason: "unrelated map gap".into(),
    };
    *f.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(
            countries
                .iter()
                .flat_map(|country| {
                    [
                        PoliticalPartyRow {
                            country: country.0,
                            id: 20,
                            token: b"mod_group".to_vec(),
                            is_ruling: 1,
                            ..Default::default()
                        },
                        PoliticalPartyRow {
                            country: country.0,
                            id: 30,
                            token: b"other_group".to_vec(),
                            ..Default::default()
                        },
                    ]
                })
                .collect(),
        ));
    *f.tables[TableId::Politics.index()] = TableView::Complete(TableRows::Politics(
        countries
            .iter()
            .map(|country| PoliticsRow {
                country: country.0,
                ruling_party: Some(20),
                ..Default::default()
            })
            .collect(),
    ));
    let dlcs = BTreeSet::new();
    let names = TriggerNames::default();
    let aliases = Arc::new(TagAliases::default());
    let world = World::read(
        &f,
        ContentFacts {
            ideologies: Ok(&ideologies),
            ..ContentFacts::content_only(&dlcs, &names, &aliases)
        },
    );
    let numeric = items("has_government=var:20");
    let scope = items("has_government=THIS");
    let other = items("has_government=var:30");
    for country in &countries {
        let scopes = Scopes::of(Scope::Country(country.1));
        assert_eq!(world.evaluate(&numeric, scopes), Eval::True);
        assert_eq!(world.evaluate(&scope, scopes), Eval::True);
        assert_eq!(world.evaluate(&other, scopes), Eval::False);
    }
}

#[test]
fn roster_visibility_keeps_country_and_selected_character_without_changing_other_callers() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay_data::{rows::CharacterRow, rows::UnitLeaderRow};
    let mut frame = frame(&countries(), &[101]);
    let row = UnitLeaderRow {
        id: 5,
        reference_type: 4713,
        owner: 101,
        character_id: 9,
        visibility_choice: 1,
        ..Default::default()
    };
    *frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![row.clone()]));
    *frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![CharacterRow {
            id: 9,
            country: 101,
            template_present: 1,
            template_tag: 901,
            template_file: b"common/characters/mod.txt".to_vec(),
            template_line: 4,
            ..Default::default()
        }]));
    let TableView::Complete(TableRows::LexicalTokens(tokens)) =
        &mut *frame.tables[TableId::LexicalTokens.index()]
    else {
        panic!("tokens");
    };
    tokens.push(LexicalTokenRow {
        token: 901,
        literal: b"mod_commander".to_vec(),
    });
    let TableView::Complete(TableRows::ScriptFlags(flags)) =
        &mut *frame.tables[TableId::ScriptFlags.index()]
    else {
        panic!("flags");
    };
    flags.push(ScriptFlagRow {
        scope_kind: world::CHARACTER_FLAGS,
        scope_id: 9,
        token: 900,
        value: 1,
        ..Default::default()
    });
    let roles = LeaderRoles::new(vec![content_reader_key_names::characters::LeaderTemplate {
        path: "common/characters/mod.txt".into(),
        tag: "mod_commander".into(),
        line: Some(4),
        role: Ok(Some(items("has_country_flag = x has_character_flag = x"))),
    }]);
    with_world(&frame, "", Ok(&roles), |world| {
        let evaluator = Evaluator::new(world, None);
        assert_eq!(evaluator.leader_roster_visible(&row), Eval::True);
        assert_eq!(evaluator.leader_visible(&row), Eval::False);
    });
    *frame.tables[TableId::ScriptFlags.index()] = TableView::Unavailable {
        reason: "flag read refused".into(),
    };
    with_world(&frame, "", Ok(&roles), |world| {
        assert!(
            Evaluator::new(world, None)
                .leader_roster_visible(&row)
                .decided()
                .is_err()
        );
    });
}

#[test]
fn leader_visibility_steps_decide_as_the_roster_check() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    use content_reader_key_names::characters::LeaderTemplate;
    use replay_data::{rows::CharacterRow, rows::UnitLeaderRow};
    let path = "common/characters/mod.txt";
    let mut frame = frame(&countries(), &[101]);
    *frame.tables[TableId::Characters.index()] = TableView::Complete(TableRows::Characters(
        [
            (1, 901, 4, 0),
            (2, 902, 8, 0),
            (3, 903, 12, 0),
            (4, 901, 4, 1),
        ]
        .into_iter()
        .map(|(id, tag, line, generated)| CharacterRow {
            id,
            template_present: 1,
            template_tag: tag,
            template_file: path.as_bytes().to_vec(),
            template_line: line,
            template_generated: generated,
            ..Default::default()
        })
        .collect(),
    ));
    let TableView::Complete(TableRows::LexicalTokens(tokens)) =
        &mut *frame.tables[TableId::LexicalTokens.index()]
    else {
        panic!("tokens");
    };
    for (token, literal) in [
        (901, "mod_commander"),
        (902, "mod_plain"),
        (903, "mod_advisor"),
    ] {
        tokens.push(LexicalTokenRow {
            token,
            literal: literal.as_bytes().to_vec(),
        });
    }
    let template = |tag: &str, line, role| LeaderTemplate {
        path: path.into(),
        tag: tag.into(),
        line: Some(line),
        role: Ok(role),
    };
    let roles = LeaderRoles::new(vec![
        template("mod_commander", 4, Some(items("has_country_flag = x"))),
        template("MOD_PLAIN", 8, Some(Vec::new())),
        template("mod_advisor", 12, None),
    ]);
    let leader = |id, owner, visibility_choice| UnitLeaderRow {
        id,
        owner,
        character_id: id,
        visibility_choice,
        template_visibility: 1,
        ..Default::default()
    };
    let refused = |id, why: &str| {
        (
            leader(id, 101, 1),
            LeaderVisibility::Decided(Eval::unknown(why)),
            Eval::unknown(why),
        )
    };
    let unlinked = "unit leaders names a country that no recorded country link holds";
    with_world(&frame, "", Ok(&roles), |world| {
        for (row, visibility, verdict) in [
            (
                leader(1, 101, 1),
                LeaderVisibility::Conditioned(0),
                Eval::True,
            ),
            (
                leader(1, 102, 1),
                LeaderVisibility::Conditioned(0),
                Eval::False,
            ),
            (
                leader(2, 101, 1),
                LeaderVisibility::Unconditioned,
                Eval::True,
            ),
            (
                leader(2, 999, 1),
                LeaderVisibility::Unconditioned,
                Eval::unknown(Failure::input(
                    TableId::CountryLinks,
                    "canonical_slot",
                    None,
                    unlinked,
                )),
            ),
            (
                leader(3, 101, 0),
                LeaderVisibility::Decided(Eval::True),
                Eval::True,
            ),
            refused(
                3,
                "unit leader 3 has its own role choice, but its character's template has no unit leader role",
            ),
            refused(
                4,
                "unit leader 4's character template was made by generate_character, which is not read",
            ),
            refused(5, "unit leader 5's character 5 is not recorded"),
        ] {
            assert_eq!(world.decoded_leader_visibility(&row), visibility);
            assert_eq!(world.leader_visibility(&row), visibility);
            assert_eq!(
                Evaluator::new(world, None).decide_leader_visibility(&row, visibility, true),
                verdict
            );
            assert_eq!(
                Evaluator::new(world, None).leader_roster_visible(&row),
                verdict
            );
        }
    });
}

#[test]
fn hours_reading_one_lexer_table_share_its_folds() {
    let first = frame(&[(100, 7, "GER", 0, "GER", "GER")], &[100]);
    let mut next = first.clone();
    next.hour = 2;
    let names = TriggerNames::default();
    let aliases = Arc::new(TagAliases::default());
    let dlcs = BTreeSet::new();
    let read = |frame| World::read(frame, ContentFacts::content_only(&dlcs, &names, &aliases));
    let folds = |world: &World| match world.names().unwrap() {
        world::Names::Own(folds) => Arc::as_ptr(folds),
    };
    let (earlier, later) = (read(&first), read(&next));
    assert_eq!(folds(&earlier), folds(&later));
    assert_eq!(later.token_of(b"X"), Ok(Some(900)));
    let mut reread = first.clone();
    let _ = &mut *reread.tables[TableId::LexicalTokens.index()];
    assert_ne!(folds(&read(&reread)), folds(&earlier));
}

#[test]
fn claimed_by_on_a_warmed_world_records_the_claims_it_read() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::triggers::inputs::{Key, capture};
    use replay_data::rows::StateCoreRow;
    let with_claims = |claims: Vec<u32>| {
        let mut f = with_variables(frame(&original_countries(), &[]), &[]);
        *f.tables[TableId::StateCores.index()] =
            TableView::Complete(TableRows::StateCores(vec![StateCoreRow {
                state: 64,
                cores: vec![104],
                claims: claims.into(),
                ..Default::default()
            }]));
        f
    };
    let (before, after) = (with_claims(vec![104]), with_claims(Vec::new()));
    let dlcs = BTreeSet::new();
    let names = TriggerNames::default();
    let aliases = Arc::new(TagAliases::default());
    let world = World::read(&before, ContentFacts::content_only(&dlcs, &names, &aliases));
    let body = items("is_claimed_by = FRA");
    let scopes = || Scopes::of(Scope::State(64));
    assert_eq!(world.evaluate(&body, scopes()), Eval::True);
    let (eval, mut inputs) = capture(&before, || world.evaluate(&body, scopes()));
    assert_eq!(eval, Eval::True);
    let later = World::read(&after, ContentFacts::content_only(&dlcs, &names, &aliases));
    let reread = |world: &World<'_>, key: &Key| match key {
        Key::World(query) => Some(world.reread(query)),
        Key::Scopes(query) => Some(world.recorded().reread(query)),
        _ => None,
    };
    assert!(inputs.confirm(&before, |key| reread(&world, key)));
    assert!(!inputs.confirm(&after, |key| reread(&later, key)));
}

#[test]
fn invasion_preparation_on_a_warmed_world_records_its_table() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::triggers::inputs::capture;
    use replay_data::rows::InvasionPreparationRow;
    let with_progress = |progress| {
        let mut f = with_variables(frame(&original_countries(), &[]), &[]);
        *f.tables[TableId::InvasionPreparations.index()] =
            TableView::Complete(TableRows::InvasionPreparations(vec![
                InvasionPreparationRow {
                    owner_group: 7,
                    instance: 1,
                    progress,
                    ..Default::default()
                },
            ]));
        f
    };
    let (before, after) = (with_progress(5), with_progress(6));
    let dlcs = BTreeSet::new();
    let names = TriggerNames::default();
    let aliases = Arc::new(TagAliases::default());
    let world = World::read(&before, ContentFacts::content_only(&dlcs, &names, &aliases));
    let _ = world.invasion_preparation((7, 1));
    let (_, mut inputs) = capture(&before, || world.invasion_preparation((7, 1)));
    assert!(inputs.confirm(&before, |_| None));
    assert!(!inputs.confirm(&after, |_| None));
}

#[test]
fn claimed_by_tests_the_states_recorded_claims_by_canonical_slot() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay_data::{rows::Recorded, rows::StateCoreRow};
    let mut f = with_variables(frame(&original_countries(), &[]), &[]);
    let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *f.tables[TableId::CountryLinks.index()]
    else {
        panic!("complete links");
    };
    links.push(CountryLinkRow {
        id: 110,
        canonical_slot: 1,
    });
    let claims = |f: &mut replay_frame::frame::DecodedFrame, claims: Recorded<Vec<u32>>| {
        *f.tables[TableId::StateCores.index()] =
            TableView::Complete(TableRows::StateCores(vec![StateCoreRow {
                state: 64,
                cores: vec![104],
                claims,
                ..Default::default()
            }]));
    };
    claims(&mut f, vec![110].into());
    let state = |f: &replay_frame::frame::DecodedFrame, body: &str| {
        let dlcs = BTreeSet::new();
        let names = TriggerNames::default();
        let aliases = Arc::new(TagAliases::default());
        let world = World::read(f, ContentFacts::content_only(&dlcs, &names, &aliases));
        world.evaluate(&items(body), Scopes::of(Scope::State(64)))
    };
    assert_eq!(state(&f, "is_claimed_by = GER"), Eval::True);
    assert_eq!(state(&f, "is_claimed_by = FRA"), Eval::False);
    assert_eq!(state(&f, "is_core_of = FRA"), Eval::True);
    claims(&mut f, Recorded::NotRecorded);
    assert!(
        state(&f, "is_claimed_by = GER")
            .decided()
            .unwrap_err()
            .contains("claims of state cores is not recorded")
    );
}

#[path = "refusal_consumption_tests.rs"]
mod refusal_consumption;

#[test]
fn shared_trigger_kit_keeps_unit_scope_and_country_ace_operands() {
    assert!(super::RECORDED_TABLES.contains(&replay_data::rows::TableId::Divisions));
    assert!(super::RECORDED_TABLES.contains(&replay_data::rows::TableId::CountryCommandPower));
}

#[test]
fn new_assignment_triggers_refuse_unrecorded_predicates_in_their_accepted_scopes() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let frame = frame(&[(100, 1, "AAA", 1, "", "")], &[]);
    with_world(&frame, "", Err("none"), |world| {
        for (name, operand, scope) in [
            ("is_fighting_in_strategic_region", "37", Scope::Combat),
            ("is_deployed", "yes", Scope::Character(7)),
            ("is_deployed", "no", Scope::Character(7)),
        ] {
            let trigger = items(&format!("{name} = {operand}"));
            let why = world
                .evaluate(&trigger, Scopes::of(scope))
                .decided()
                .unwrap_err();
            assert!(why.contains(name), "{why}");
            assert!(why.contains("trigger "), "{why}");
            assert_eq!(
                world.evaluate(&trigger, Scopes::of(Scope::Country(1))),
                Eval::False,
                "{name} must retain its executable scope mask"
            );
        }
    });
}

#[test]
fn volunteer_presence_trigger_refuses_its_unrecorded_predicate_in_country_scope() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let frame = frame(&[(100, 1, "AAA", 1, "", "")], &[]);
    with_world(&frame, "", Err("none"), |world| {
        for operand in ["AAA", "THIS"] {
            let trigger = items(&format!("has_volunteers = {operand}"));
            let why = world
                .evaluate(&trigger, Scopes::of(Scope::Country(1)))
                .decided()
                .unwrap_err();
            assert!(why.contains("has_volunteers"), "{why}");
            assert!(why.contains("trigger "), "{why}");
            assert_eq!(
                world.evaluate(&trigger, Scopes::of(Scope::Character(7))),
                Eval::False,
                "has_volunteers must retain its executable scope mask"
            );
        }
    });
}

#[path = "recorded_source_tests.rs"]
mod recorded_source;
