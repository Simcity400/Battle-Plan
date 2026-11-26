use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

#[test]
fn loader_warnings_are_only_nonpositive_and_duplicate_id_skips() {
    content_reader_script::test_game::enter();
    use crate::DiagnosticKind;
    struct Folder(std::path::PathBuf);
    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let folder = Folder(std::env::temp_dir().join(format!(
        "states-warning-kinds-{}-{nonce}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    for (path, text) in [
        (
            "history/states/0-discarded.txt",
            "state={id=0} state={id=-2}",
        ),
        (
            "history/states/1-kept.txt",
            "@a bad state={id=1 impassable=maybe resources={unknown=2} state_startup_effect={free_operative={operative=authored}}}",
        ),
        (
            "history/states/2-duplicate.txt",
            "state={id=1 name=DISCARDED}",
        ),
    ] {
        let path = folder.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let layers = ResolvedLayers::from_folders(&[(&folder.0, &[][..])]).unwrap();
    let defines = Defines::new().unwrap();
    defines
        .load(
            "test.lua",
            b"NDefines={NBuildings={MAX_BUILDING_LEVELS=13}}",
        )
        .unwrap();
    let (states, notes) = read(&layers, &defines, &BTreeMap::new()).unwrap();
    assert_eq!(states.definitions().len(), 1);
    assert_eq!(states.source(1), Some(&script_files::ScriptPath::from("history/states/1-kept.txt")));
    let warnings: Vec<_> = notes
        .iter()
        .filter(|(kind, _)| *kind == DiagnosticKind::Warning)
        .map(|(_, message)| message.as_str())
        .collect();
    assert_eq!(
        warnings,
        [
            "history/states/0-discarded.txt state id 0 is not positive; the game discards the template; the state follows the game's reading",
            "history/states/0-discarded.txt state id -2 is not positive; the game discards the template; the state follows the game's reading",
            "state 1 is defined by both history/states/1-kept.txt and history/states/2-duplicate.txt; the game keeps the first",
        ]
    );
    assert!(
        notes
            .iter()
            .any(|(kind, _)| *kind == DiagnosticKind::Misread),
        "{notes:?}"
    );
    assert!(
        notes
            .iter()
            .any(|(kind, _)| *kind == DiagnosticKind::Unsupported),
        "{notes:?}"
    );
}

#[test]
fn state_field_recovery_causes_keep_offline_notes() {
    content_reader_script::test_game::enter();
    let input = b"@bad nope orphan=yes state={id=1 impassable=yes impassable=maybe provinces={1 100} resources={unknown=2} history={victory_points={1} buildings=4}}";
    let parsed = attributed(input);
    let state = &parsed.states[0].0;
    assert!(state.impassable);
    assert_eq!(state.provinces, [1]);
    assert_eq!(state.resources, Ok(Vec::new()));
    assert!(state.victory_points.is_empty());
    assert!(state.buildings.is_empty());
    assert_eq!(
        parsed
            .diagnostics
            .iter()
            .filter_map(|note| note.cause)
            .collect::<Vec<_>>(),
        [
            MisreadCause::InvalidImpassable,
            MisreadCause::OffMapProvince,
            MisreadCause::NonStrategicResource,
            MisreadCause::InvalidVictoryPoints,
            MisreadCause::NonBlockBuildings,
        ]
    );
    assert!(
        parsed
            .diagnostics
            .iter()
            .all(|note| note.kind == crate::DiagnosticKind::Misread)
    );
    assert!(parsed.diagnostics.iter().any(|note| note.cause.is_none()));
    let (_, offline) = parse_states(
        input,
        &Context {
            resource: &|_| Resource::Null,
            max_building_levels: 13,
            province_count: 100,
            names: &KeyNames::default(),
        },
    )
    .unwrap();
    assert_eq!(
        offline,
        parsed
            .diagnostics
            .into_iter()
            .map(Diagnostic::into_diagnostic)
            .collect::<Vec<_>>()
    );
}

fn attributed(input: &[u8]) -> ParsedStates {
    parse_states_attributed(
        input,
        &Context {
            resource: &|_| Resource::Null,
            max_building_levels: 13,
            province_count: 100,
            names: &KeyNames::default(),
        },
    )
    .unwrap()
}

#[test]
fn states_deferred_readers_affect_their_template_and_every_later_template() {
    content_reader_script::test_game::enter();
    let parsed = attributed(b"state = { id = 1 provinces = { 1 } } state = { id = 2 state_startup_effect = { free_operative = { operative = authored } } } state = { id = 3 }");
    assert_eq!(parsed.states.len(), 3);
    assert!(parsed.states[0].1.is_empty());
    assert_eq!(parsed.states[0].0.provinces, [1]);
    assert!(
        parsed.states[1].1.iter().any(|note| {
            note.contains("Deferred effect `free_operative`") && note.contains("count: 1")
        }),
        "{:?}",
        parsed.states[1].1
    );
    assert_eq!(
        parsed.states[2].1,
        [format!(
            "after state 2's unported reader: {}",
            parsed.states[1].1[0]
        )]
    );
}

#[test]
fn states_settled_diagnostics_inside_one_template_affect_only_that_state() {
    content_reader_script::test_game::enter();
    let parsed = attributed(b"state = { id = 1 impassable = maybe } state = { id = 2 }");
    assert!(parsed.states[0].1[0].contains("not yes or no"));
    assert!(parsed.states[1].1.is_empty());
}

#[test]
fn states_repeated_deferred_diagnostics_keep_each_templates_count() {
    content_reader_script::test_game::enter();
    let parsed = attributed(b"state = { id = 1 state_startup_effect = { free_operative = { operative = first } free_operative = { operative = second } } } state = { id = 2 history = { free_operative = { operative = third } } } state = { id = 3 }");
    assert_eq!(parsed.states[0].1.len(), 1, "{:?}", parsed.states[0].1);
    assert!(parsed.states[0].1[0].contains("count: 2"));
    assert_eq!(parsed.states[1].1.len(), 2, "{:?}", parsed.states[1].1);
    assert_eq!(
        parsed.states[1].1[0],
        format!("after state 1's unported reader: {}", parsed.states[0].1[0])
    );
    assert!(parsed.states[1].1[1].contains("count: 1"));
    assert_eq!(
        parsed.states[2].1,
        [
            parsed.states[1].1[0].clone(),
            format!("after state 2's unported reader: {}", parsed.states[1].1[1]),
        ]
    );
    assert_eq!(parsed.diagnostics.len(), 1, "{:?}", parsed.diagnostics);
    assert!(parsed.diagnostics[0].message.contains("count: 3"));
}

#[test]
fn states_unattributable_diagnostics_affect_every_template_in_the_file() {
    content_reader_script::test_game::enter();
    let parsed = attributed(b"state = { id = 1 } orphan tail state = { id = 2 }");
    assert_eq!(parsed.states.len(), 2);
    for (_, notes) in parsed.states {
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(
            notes[0].contains("key `orphan` has no operator"),
            "{notes:?}"
        );
    }
}

#[test]
fn states_variable_definition_diagnostics_affect_every_template() {
    content_reader_script::test_game::enter();
    for input in [
        b"@a bad state = { id = 1 } state = { id = 2 }".as_slice(),
        b"@a = 1 @a = 2 state = { id = 1 } state = { id = 2 }".as_slice(),
    ] {
        let parsed = attributed(input);
        assert_eq!(parsed.states.len(), 2);
        for (_, notes) in parsed.states {
            assert!(
                notes.iter().any(|note| {
                    note.contains("definition `@a` has no `=`")
                        || note.contains("variable `@a` is already taken")
                }),
                "{notes:?}"
            );
        }
    }
    let parsed =
        attributed(b"state = { id = 1 @a = 1 @a = 2 provinces = { 1 } } state = { id = 2 }");
    assert_eq!(parsed.states.len(), 2);
    assert!(
        parsed.states[0]
            .1
            .iter()
            .any(|note| { note.contains("variable `@a` is already taken") })
    );
    assert!(
        parsed.states[1]
            .1
            .iter()
            .any(|note| { note.contains("variable `@a` is already taken") })
    );
    let parsed = attributed(
        b"state = { id = 3 } state = { id = 0 @a bad provinces = { 1 } } state = { id = 4 name = @a }",
    );
    assert_eq!(parsed.states.len(), 2);
    for (_, notes) in parsed.states {
        assert!(
            notes
                .iter()
                .any(|note| note.contains("definition `@a` has no `=`")),
            "{notes:?}"
        );
    }
}

#[test]
fn states_file_diagnostics_without_kept_templates_remain_notes() {
    content_reader_script::test_game::enter();
    let parsed = attributed(b"@a bad orphan = yes");
    assert!(parsed.states.is_empty());
    assert_eq!(parsed.diagnostics.len(), 1);
    assert!(
        parsed.diagnostics[0]
            .message
            .contains("definition `@a` has no `=`")
    );
    let parsed = attributed(b"@a bad state = { id = 0 }");
    assert!(parsed.states.is_empty());
    assert_eq!(parsed.diagnostics.len(), 2);
}

const RESOURCES: [&str; 3] = ["chromium", "steel", "sparkstone"];

#[test]
fn resource_definitions_read_their_own_fields() {
    let mut resources = Resources::default();
    read_resources(
        b"resources = { Oil = { icon_frame = 1 cic = 0.125 CONVOYS = 0.1 odd = { fake = { } } } \
              steel = { modifier = 2 } \"quoted\" = { \"cic\" = { inner = { } } } }",
        "a.txt",
        &mut resources,
    );
    assert_eq!(
        resources.tokens,
        [
            Token::register(b"oil"),
            Token::register(b"steel"),
            Token::register(b"quoted")
        ]
    );
    assert_eq!(resources.find(b"OIL"), Resource::Found);
    assert_eq!(resources.find(b"later"), Resource::Null);
    read_resources(
        b"resources = { oil = { modifier = { x = 1 } } later = { } }",
        "b.txt",
        &mut resources,
    );
    assert_eq!(resources.find(b"quoted"), Resource::Found);
    let Resource::Unknown(why) = resources.find(b"missing") else {
        panic!("{:?}", resources.find(b"missing"));
    };
    assert!(
            why.starts_with("`missing` names no strategic resource read, and b.txt line 1: modifier has a block value, which modifier value reader reads"),
            "{why}"
        );
}

#[test]
fn resources_resolve_by_token() {
    content_reader_script::test_game::enter();
    let mut resources = Resources::default();
    read_resources(
        b"resources = { 1st = { } Steel = { } }",
        "a.txt",
        &mut resources,
    );
    assert_eq!(resources.find(b"STEEL\0junk"), Resource::Found);
    assert_eq!(resources.find(b"-2"), Resource::Found);
    assert!(matches!(resources.find(b"bogus"), Resource::Unknown(_)));
    let context = Context {
        resource: &|name| resources.find(name),
        max_building_levels: 13,
        province_count: 10_000,
        names: &KeyNames::default(),
    };
    let (states, _) = parse_states(
        b"state = { id = 1 name = A resources = { steel = 2 9 = 3 } }
              state = { id = 2 name = B resources = { bogus = 4 } }",
        &context,
    )
    .unwrap();
    assert_eq!(
        states[0].resources,
        Ok(vec![("steel".to_owned(), 2), ("9".to_owned(), 3)])
    );
    assert_eq!(
        states[1].resources.as_ref().unwrap_err(),
        "state 2: `bogus` is no strategic resource's name, and a resource holds token 0 (a name empty or starting with a digit or `-`): token lookup finds that resource unless the lexer holds a token for `bogus`, which depends on every name the game registered"
    );
    let error = parse_states(
        b"state = { id = 1 name = A }
              state = { id = 3 name = C resources = { bogus = { x = 1 } } }
              state = { id = 4 name = D }",
        &context,
    )
    .unwrap_err()
    .to_string();
    assert!(
            error.starts_with("line 2: `bogus` is no strategic resource's name")
                && error.contains(
                    "the state being read there (id 3 so far) and every state after it in the file are unknown; the state inventory is unavailable"
                ),
            "{error}"
        );
}

fn parse(input: &[u8]) -> io::Result<(Vec<StateDefinition>, Vec<String>)> {
    parse_states(
        input,
        &Context {
            resource: &|name| {
                if RESOURCES
                    .iter()
                    .any(|known| name.eq_ignore_ascii_case(known.as_bytes()))
                {
                    Resource::Found
                } else {
                    Resource::Null
                }
            },
            max_building_levels: 13,
            province_count: 10_000,
            names: &KeyNames::default(),
        },
    )
    .map(|(states, notes)| {
        (
            states,
            notes.into_iter().map(|(_, message)| message).collect(),
        )
    })
}

const ORIGINAL_STATE: &[u8] = br#"
state = {
    id = 29
    name = "ORIGINAL_RIDGE"
    manpower = 4207
    resources = { sparkstone = 7.000 }
    state_category = test_hamlet
    state_category = test_market
    history = {
        owner = MOD_A
        buildings = {
            workshop = 4
            landing_strip = 2
            18 = {
                pier = 3
                control_room = { level = 5 allowed = { always = yes } }
            }
        }
        1942.6.3 = { buildings = { storage = 6 } }
        if = { limit = { } }
        victory_points = { 18 6 }
        victory_points = { 11 4 }
    }
    provinces = { 11 16 18 }
    local_supplies = 1.25
}
"#;

#[test]
fn a_state_keeps_its_provinces_points_buildings_and_repeated_category() {
    content_reader_script::test_game::enter();
    let (mut states, misreads) = parse(ORIGINAL_STATE).unwrap();
    assert!(misreads.is_empty(), "{misreads:?}");
    let state = states.remove(0);
    assert_eq!(state.id, 29);
    assert_eq!(state.name_key.as_deref(), Ok("ORIGINAL_RIDGE"));
    assert_eq!(state.category.as_deref(), Some("test_market"));
    assert_eq!(state.manpower, Some(4_207));
    assert_eq!(state.provinces, vec![11, 16, 18]);
    assert_eq!(state.resources, Ok(vec![("sparkstone".to_owned(), 7)]));
    assert_eq!(state.victory_points, vec![(18, 6), (11, 4)]);
    assert_eq!(
        state.buildings["workshop"],
        BuildingLevel {
            level: 4,
            conditional: false
        }
    );
    assert_eq!(state.buildings["landing_strip"].level, 2);
    let province = &state.province_buildings[&18];
    assert_eq!(province["pier"].level, 3);
    assert_eq!(
        province["control_room"],
        BuildingLevel {
            level: 5,
            conditional: true
        }
    );
    assert!(!state.impassable);
    assert_eq!(state.skipped_history, vec!["1942.6.3", "if"]);
    assert!(!state.buildings.contains_key("storage"));
}

#[test]
fn an_impassable_state_without_history_buildings_still_parses() {
    content_reader_script::test_game::enter();
    let state = parse(
        br#"state = { id = 1022 name = "STATE_1022" impassable = yes
                 resources = { } history = { owner = INS } provinces = { 1230 } }"#,
    )
    .unwrap()
    .0
    .remove(0);
    assert!(state.impassable);
    assert_eq!(state.category, None);
    assert_eq!(state.manpower, None);
    assert!(state.buildings.is_empty());
    assert!(state.victory_points.is_empty());
    assert!(state.skipped_history.is_empty());
}

#[test]
fn every_state_and_every_provinces_block_is_read_in_order() {
    content_reader_script::test_game::enter();
    let (states, _) = parse(
        br#"state = { id = 1 name = "A" provinces = { 3 1 } PROVINCES = { 2 1 } }
                STATE = { id = 2 name = "B" provinces = { 9 } }"#,
    )
    .unwrap();
    assert_eq!(states.len(), 2);
    assert_eq!(states[0].provinces, vec![3, 1, 2, 1]);
    assert_eq!(states[1].id, 2);
}

#[test]
fn names_read_in_keyword_order_through_the_files_variables() {
    content_reader_script::test_game::enter();
    let (states, misreads) = parse(
        br#"@n = "STATE_KEY"
                @p = 5
                state = { id = 1 name = "A" NAME = "B" "name" = "C" provinces = { @p 6 } }
                state = { @m = OTHER id = 2 name = @n }
                state = { id = 3 history = { @late = LATE owner = X } name = @late }
                state = { id = 4 name = @missing }
                "state" = { id = 9 name = "Q" }"#,
    )
    .unwrap();
    let names: Vec<_> = states
        .iter()
        .map(|state| (state.id, state.name_key.clone()))
        .collect();
    assert_eq!(
        names,
        [
            (1, Ok("B".to_owned())),
            (2, Ok("STATE_KEY".to_owned())),
            (3, Ok("LATE".to_owned())),
            (4, Ok("@missing".to_owned())),
        ]
    );
    assert_eq!(states[0].provinces, [5, 6]);
    assert!(misreads.is_empty(), "{misreads:?}");
}

#[test]
fn names_follow_the_games_reading_when_the_text_leaves_the_tree() {
    content_reader_script::test_game::enter();
    let (states, misreads) = parse(b"@a = 1 @a = 2 state = { id = 1 name = A }").unwrap();
    assert_eq!(states[0].name_key.as_deref(), Ok("A"));
    assert!(misreads[0].contains("variable `@a` is already taken"));
    let (states, misreads) =
        parse(b"state = { id = 1 name = { x = 1 name = B } id = 2 } state = { id = 3 name = C }")
            .unwrap();
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].name_key.as_deref(), Ok("B"));
    assert!(misreads[0].starts_with("line 1: field `name` is a block"));
    assert!(misreads[1].contains("key `}` has no operator"));
}

#[test]
fn a_dated_conditional_in_history_reads_without_diagnostics() {
    content_reader_script::test_game::enter();
    let (states, misreads) = parse(
        br#"state = {
                id = 31
                name = "ORIGINAL_PLAIN"
                manpower = 6103
                state_category = test_market
                history = {
                    owner = MOD_A
                    add_core_of = MOD_A
                    1944.2.5 = {
                        IF = {
                            limit = { always = yes }
                            if = {
                                limit = { always = no }
                                transfer_state_to = MOD_B
                            }
                        }
                    }
                    victory_points = { 24 5 }
                    buildings = { workshop = 3 26 = { tower = 1 } }
                }
                provinces = { 23 24 26 }
            }"#,
    )
    .unwrap();
    assert!(misreads.is_empty(), "{misreads:?}");
    assert_eq!(states[0].id, 31);
    assert_eq!(states[0].provinces, vec![23, 24, 26]);
    assert_eq!(states[0].victory_points, vec![(24, 5)]);
}

#[test]
fn conditional_history_bodies_preserve_following_fields_and_next_state() {
    content_reader_script::test_game::enter();
    let (states, misreads) = parse(
        br#"state = {
            id = 7 name = "SYNTHETIC_FIRST"
            history = {
                owner = MOD_A
                if = {
                    limit = { always = yes }
                    if = {
                        limit = { always = no }
                        transfer_state_to = MOD_B
                    }
                }
                1936.1.1 = {
                    if = { limit = { always = no } add_core_of = MOD_A }
                }
                victory_points = { 11 3 }
                buildings = { infrastructure = 2 13 = { bunker = 4 } }
            }
            provinces = { 11 12 13 }
        }
        state = {
            id = 8 name = "SYNTHETIC_SECOND"
            history = {
                if = { limit = { always = no } transfer_state_to = MOD_A }
                victory_points = { 21 5 }
                buildings = { infrastructure = 3 22 = { bunker = 1 } }
            }
            provinces = { 21 22 }
        }"#,
    )
    .unwrap();
    assert!(misreads.is_empty(), "{misreads:?}");
    assert_eq!(states.len(), 2);
    let first = &states[0];
    assert_eq!(first.id, 7);
    assert_eq!(first.name_key.as_deref(), Ok("SYNTHETIC_FIRST"));
    assert_eq!(first.provinces, [11, 12, 13]);
    assert_eq!(first.victory_points, [(11, 3)]);
    assert_eq!(first.buildings["infrastructure"].level, 2);
    assert_eq!(first.province_buildings[&13]["bunker"].level, 4);
    let second = &states[1];
    assert_eq!(second.id, 8);
    assert_eq!(second.name_key.as_deref(), Ok("SYNTHETIC_SECOND"));
    assert_eq!(second.provinces, [21, 22]);
    assert_eq!(second.victory_points, [(21, 5)]);
    assert_eq!(second.buildings["infrastructure"].level, 3);
    assert_eq!(second.province_buildings[&22]["bunker"].level, 1);
}

#[test]
fn a_templates_own_readers_consume_what_the_game_consumes() {
    content_reader_script::test_game::enter();
    let (states, misreads) = parse(
        br#"state = {
                color = { 1 2 3 }
                provinces = { 1 { 2 } -3 .5 x "4" 5abc }
                id = 7 name = N
                impassable_ignored_links = { 8 9 }
                state_startup_effect = { if = { limit = { always = yes } } id = 1 }
                resources = { chromium = 5 CHROMIUM = 1.9 bogus = 3 }
                manpower = 10 manpower = x
            }"#,
    )
    .unwrap();
    let state = &states[0];
    assert_eq!(state.id, 7);
    assert_eq!(state.provinces, vec![1, 2, 5]);
    assert_eq!(
        state.resources,
        Ok(vec![("chromium".to_owned(), 5), ("CHROMIUM".to_owned(), 1)])
    );
    assert_eq!(state.manpower, Some(10));
    assert_eq!(
        misreads,
        [
            "state 7: province -3 is not on the map (10000 provinces); the game reports it and keeps it out of the state",
            "state 7: province 0 is not on the map (10000 provinces); the game reports it and keeps it out of the state",
            "state 7: `bogus` is no strategic resource; the game reports it and keeps no amount",
        ],
    );
}

#[test]
fn history_commands_keep_what_the_game_keeps() {
    content_reader_script::test_game::enter();
    let (states, misreads) = parse(
        br#"state = { id = 1 name = N history = {
                5 = { owner = X } -1 = { } 0 = { }
                victory_points = { 010 0x10 }
                victory_points = { 1 }
                victory_points = { a = 1 }
                victory_points = { 3 x }
                buildings = { Infrastructure = 99 +12 = { Naval_Base = { level = 2 } } 0 = 3
                    infrastructure = { level = x } }
                buildings = 4
            } }"#,
    )
    .unwrap();
    let state = &states[0];
    assert_eq!(state.skipped_history, vec!["5", "-1"]);
    assert_eq!(state.victory_points, vec![(8, 16), (3, 0)]);
    assert_eq!(
        state.buildings["infrastructure"],
        BuildingLevel {
            level: 0,
            conditional: false
        }
    );
    assert_eq!(state.buildings["0"].level, 3);
    assert_eq!(state.province_buildings[&12]["naval_base"].level, 2);
    assert_eq!(
        misreads,
        [
            "state 1: victory_points holds 1 values, not a province and a value; the game records an error and keeps none",
            "state 1: victory_points holds 3 values, not a province and a value; the game records an error and keeps none",
            "state 1: buildings is not a block; the game reports it and reads no buildings",
        ]
    );
    let (states, _) =
        parse(b"state = { id = 1 name = N history = { buildings = { infrastructure = 99 } } }")
            .unwrap();
    assert_eq!(states[0].buildings["infrastructure"].level, 13);
}

#[test]
fn reported_keys_skip_their_blocks() {
    content_reader_script::test_game::enter();
    let (states, misreads) = parse(
        b"wrapper = { state = { id = 5 name = X } } state = { id = 6 name = Y
                resources = { bogus = { name = Z provinces = { 7 } } steel = 2 }
                history = { buildings = { dockyard = { odd = { level = 9 } level = 3 } } } }",
    )
    .unwrap();
    assert_eq!(states.iter().map(|state| state.id).collect::<Vec<_>>(), [6]);
    let state = &states[0];
    assert_eq!(state.name_key.as_deref(), Ok("Y"));
    assert!(state.provinces.is_empty());
    assert_eq!(state.resources, Ok(vec![("steel".to_owned(), 2)]));
    assert_eq!(state.buildings["dockyard"].level, 3);
    assert_eq!(
        misreads,
        ["state 6: `bogus` is no strategic resource; the game reports it and keeps no amount"]
    );
    let (states, _) = parse(
            b"state = { id = 1 name = N history = { owner = X buildings { level = 2 } } provinces = { 4 } }",
        )
        .unwrap();
    assert_eq!(states[0].provinces, [4]);
    assert!(states[0].buildings.is_empty());
}

#[test]
fn an_unsettled_state_is_not_published() {
    content_reader_script::test_game::enter();
    let error = parse(
            b"state = { id = 1 name = A }
              state = { id = 2 name = B state_startup_effect = { free_operative = { x @y = 1 } } provinces = { 3 } }
              state = { id = 4 name = D }",
        )
        .unwrap_err()
        .to_string();
    assert!(
        error.starts_with(
            "line 2: variable @y is in a trigger or effect block that also holds anonymous values"
        ),
        "{error}"
    );
    assert!(
            error.contains(
                "the state being read there (id 2 so far) and every state after it in the file are unknown; the state inventory is unavailable"
            ),
            "{error}"
        );
    let error = parse(
        b"state = { state_startup_effect = { free_operative = { x @y = 1 } } id = 3 name = C }",
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("(no id read yet)"), "{error}");
}

#[test]
fn only_positive_signed_ids_enter_the_state_database() {
    content_reader_script::test_game::enter();
    let (states, notes) = parse(
        br#"
            state = { id = x name = bad provinces = { 1 } }
            state = { name = absent provinces = { 2 } }
            state = { id = 0 name = zero provinces = { 3 } }
            state = { id = -2 name = negative provinces = { 4 } }
            state = { id = 0x80000000 name = wrapped provinces = { 5 } }
            state = { id = 7 id = bad name = replaced provinces = { 6 } }
            state = { id = 010 }
        "#,
    )
    .unwrap();
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].id, 8);
    assert_eq!(states[0].name_key.as_deref(), Ok(""));
    assert_eq!(notes.len(), 6);
    for (note, id) in notes.iter().zip([-1, -1, 0, -2, i32::MIN, -1]) {
        assert_eq!(
            note,
            &format!("state id {id} is not positive; the game discards the template")
        );
    }
    assert!(parse(b"state = { } ").unwrap().0.is_empty());
    assert!(parse(b"# no templates\n").unwrap().0.is_empty());
}

#[test]
fn state_names_keep_c_string_prefixes_and_line_feed_replacement() {
    content_reader_script::test_game::enter();
    let (states, _) = parse(
        b"@n = \"cut\0\xfe\"
            state = { id = 1 name = \"line\nfeed\" }
            state = { id = 2 name = \"key\0suffix\" }
            state = { id = 3 name = @n }
            state = { id = 4 name = \"\xfe\" }
            state = { id = 5 name = \"\xfe\" name = \"last\0\n\" }
        ",
    )
    .unwrap();
    assert_eq!(states[0].name_key.as_deref(), Ok("Unreadable String"));
    assert_eq!(states[1].name_key.as_deref(), Ok("key"));
    assert_eq!(states[2].name_key.as_deref(), Ok("cut"));
    assert!(
        states[3]
            .name_key
            .as_ref()
            .unwrap_err()
            .contains("not UTF-8")
    );
    assert_eq!(states[4].name_key.as_deref(), Ok("last"));
}

#[test]
fn state_fields_keep_keyword_dispatch_and_failed_boolean_values() {
    content_reader_script::test_game::enter();
    let (states, notes) = parse(b"state = { id = 1 name = N
            state_category = \"\xfe\" STATE_CATEGORY = \"Town\0suffix\" \"state_category\" = ignored
            IMPASSABLE = yes impassable = maybe \"impassable\" = no
            RESOURCES = { steel = 3 } \"resources\" = { steel = 9 }
            HISTORY = { BUILDINGS = { dockyard = 2 } } \"history\" = { buildings = { dockyard = 9 } }
        }
        state = { id = 2 IMPASSABLE = maybe }
        state = { id = 3 impassable = yes impassable = \"no\0suffix\" }
        ").unwrap();
    assert_eq!(states[0].category.as_deref(), Some("Town"));
    assert!(states[0].impassable);
    assert_eq!(states[0].resources, Ok(vec![("steel".to_owned(), 3)]));
    assert_eq!(states[0].buildings["dockyard"].level, 2);
    assert!(!states[1].impassable);
    assert!(!states[2].impassable);
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(notes.iter().all(|note| note.contains("not yes or no")));
}

#[cfg(feature = "test-support")]
#[test]
fn the_loader_reads_resources_and_the_building_cap_from_content() {
    content_reader_script::test_game::enter();
    use std::fs;
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_states_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    for (path, text) in [
        (
            "common/resources/00_resources.txt",
            "resources = { Tungsten = { icon_frame = 1 } }",
        ),
        (
            "history/states/1-a.txt",
            "state = { id = 1 name = A resources = { TUNGSTEN = 4 oil = 2 }
                   history = { buildings = { infrastructure = 12 } } provinces = { 2 3 1 } }",
        ),
        (
            "map/default.map",
            "definitions = \"definition.csv\" provinces = \"provinces.bmp\"
                   terrain = \"terrain.bmp\" rivers = \"rivers.bmp\"
                   heightmap = \"heightmap.bmp\" tree_definition = \"trees.bmp\"",
        ),
        (
            "map/definition.csv",
            "0;0;0;0;land;false;unknown;0\n1;10;0;0;land;false;plains;1\n2;20;0;0;sea;false;ocean;0\n",
        ),
    ] {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let mut bitmap = vec![0_u8; 54];
    bitmap[..2].copy_from_slice(b"BM");
    bitmap[2..6].copy_from_slice(&62_u32.to_le_bytes());
    bitmap[10..14].copy_from_slice(&54_u32.to_le_bytes());
    bitmap[14..18].copy_from_slice(&40_u32.to_le_bytes());
    bitmap[18..22].copy_from_slice(&2_i32.to_le_bytes());
    bitmap[22..26].copy_from_slice(&1_i32.to_le_bytes());
    bitmap[26..28].copy_from_slice(&1_u16.to_le_bytes());
    bitmap[28..30].copy_from_slice(&24_u16.to_le_bytes());
    bitmap[34..38].copy_from_slice(&8_u32.to_le_bytes());
    bitmap.extend_from_slice(&[0, 0, 10, 0, 0, 20, 0, 0]);
    fs::write(root.join("map/provinces.bmp"), bitmap).unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let defines = Defines::new().unwrap();
    defines
        .load(
            "common/defines/00_defines.lua",
            b"NDefines = { NBuildings = { MAX_BUILDING_LEVELS = 10 } }",
        )
        .unwrap();
    let map = crate::map::load(&layers).unwrap();
    let (states, notes) = read(&layers, &defines, &map.definitions).unwrap();
    let state = states.get(1).unwrap();
    assert_eq!(states.source(1), Some(&script_files::ScriptPath::from("history/states/1-a.txt")));
    assert_eq!(states.diagnostics(1).len(), 2);
    assert!(
        states
            .diagnostics(1)
            .iter()
            .all(|state_note| { notes.iter().any(|(_, note)| note.contains(state_note)) })
    );
    assert_eq!(state.resources, Ok(vec![("TUNGSTEN".to_owned(), 4)]));
    assert_eq!(state.buildings["infrastructure"].level, 10);
    assert_eq!(state.provinces, [2, 1]);
    assert_eq!(
        notes
            .iter()
            .map(|(_, note)| note.as_str())
            .collect::<Vec<_>>(),
        [
            "history/states/1-a.txt state 1: province 3 is not on the map (3 provinces); the game reports it and keeps it out of the state; the state follows the game's reading",
            "history/states/1-a.txt state 1: `oil` is no strategic resource; the game reports it and keeps no amount; the state follows the game's reading",
        ]
    );
    for (path, text) in [
        ("2-kept.txt", "state = { id = 2 provinces = { 1 } }"),
        (
            "3-discarded.txt",
            "state = { id = 2 provinces = { 2 } state_startup_effect = { free_operative = { operative = authored } } }",
        ),
        ("4-after.txt", "state = { id = 3 provinces = { 2 } }"),
    ] {
        fs::write(root.join("history/states").join(path), text).unwrap();
    }
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let (states, notes) = read(&layers, &defines, &map.definitions).unwrap();
    assert_eq!(states.get(2).unwrap().provinces, [1]);
    assert_eq!(states.source(2), Some(&script_files::ScriptPath::from("history/states/2-kept.txt")));
    assert!(states.diagnostics(2).is_empty());
    assert!(states.diagnostics(3).is_empty());
    assert!(
        notes
            .iter()
            .any(|(_, note)| note.contains("defined by both"))
    );
    assert!(notes.iter().any(|(_, note)| {
        note.contains("history/states/3-discarded.txt")
            && note.contains("Deferred effect `free_operative`")
    }));
    assert_eq!(notes.len(), 4);
    for (path, text) in [
        ("5-no-state.txt", "@a bad orphan = yes"),
        (
            "6-discarded-file.txt",
            "@a bad state = { id = 2 provinces = { 2 } }",
        ),
    ] {
        fs::write(root.join("history/states").join(path), text).unwrap();
    }
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let (states, notes) = read(&layers, &defines, &map.definitions).unwrap();
    assert_eq!(states.get(2).unwrap().provinces, [1]);
    assert!(states.diagnostics(2).is_empty());
    assert!(states.diagnostics(3).is_empty());
    for path in ["5-no-state.txt", "6-discarded-file.txt"] {
        assert!(notes.iter().any(|(_, note)| {
            note.contains(path) && note.contains("definition `@a` has no `=`")
        }));
    }
    assert_eq!(
        notes
            .iter()
            .filter(|(_, note)| note.contains("defined by both"))
            .count(),
        2
    );
    assert_eq!(notes.len(), 7);
    fs::write(
            root.join("history/states/2-unsettled.txt"),
            "state = { id = 2 state_startup_effect = { free_operative = { x @y = 1 } } provinces = { 1 } }",
        )
        .unwrap();
    fs::write(
        root.join("history/states/3-later.txt"),
        "state = { id = 2 name = LATER provinces = { 2 } }",
    )
    .unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let error = read(&layers, &defines, &map.definitions)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("history/states/2-unsettled.txt: line 1"),
        "{error}"
    );
    assert!(error.contains("state inventory is unavailable"), "{error}");
    fs::remove_dir_all(&root).unwrap();
}
