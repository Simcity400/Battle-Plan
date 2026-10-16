use crate::{
    key_names::KeyNames,
    parser::native::{self, Shape},
};

fn read(text: &str, names: &KeyNames) -> native::Read {
    native::read(
        text.as_bytes(),
        native::Mode::KeepMark,
        native::Top::File(()),
        Some(names),
        &mut |_, field| {
            if field.keyword_is("body") {
                Shape::Trigger
            } else {
                Shape::One
            }
        },
    )
}

#[test]
fn jump_table_key_readers_skip_unknown_blocks() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    for class in [
        "has_railway_level",
        "free_building_slots",
        "has_built",
        "has_volunteers_amount_from",
        "has_resources_amount",
        "army_manpower_in_state",
        "divisions_in_border_state",
        "has_mines",
        "has_operation_token",
        "war_length_with",
        "compare_ideology_with_faction",
        "fighting_army_strength_ratio",
        "network_strength",
        "career_profile_check_ratio",
        "career_profile_check_playthrough_ratio",
        "count_in_collection",
    ] {
        let result = read(
            &format!(
                "body = {{ {class} = {{ invalid = {{ @lost = 2 }} @kept = 3 marker = @kept }} }} lost = @lost kept = @kept"
            ),
            &KeyNames::default(),
        );
        assert!(
            result.unsettled.is_none(),
            "{class}: {:?}",
            result.unsettled
        );
        assert!(result.items.len() >= 3, "{class}: {result:?}");
        assert_eq!(
            result.items[1].value.scalar(),
            Some(&b"@lost"[..]),
            "{class}"
        );
        assert_eq!(result.items[2].value.scalar(), Some(&b"3"[..]), "{class}");
    }
}

#[test]
fn scoped_variable_keys_consume_one_value_instead_of_a_body() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    for class in [
        "check_variable",
        "clamp_temp_variable",
        "has_border_war_between",
        "resize_temp_array",
        "career_profile_set_temp_variable",
        "has_game_rule",
        "is_in_array",
    ] {
        let result = read(
            &format!(
                "body = {{ {class} = {{ unknown = {{ @kept = 6 value = @kept }} }} }} kept = @kept"
            ),
            &KeyNames::default(),
        );
        assert!(
            result.unsettled.is_none(),
            "{class}: {:?}",
            result.unsettled
        );
        let entries = result.items[0].value.items().unwrap()[0]
            .value
            .items()
            .unwrap();
        assert_eq!(entries[0].value.scalar(), Some(&b"{"[..]), "{class}");
        assert!(!result.misreads.is_empty(), "{class}");
    }
}

#[test]
fn province_filter_and_building_lookup_use_their_own_readers() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let names = KeyNames {
        province_buildings: [(b"native_building".to_vec(), Some(true))].into(),
        ..KeyNames::default()
    };
    let result = read(
        "body = { any_province_building_level = { province = { id = 1 invalid = { @lost = 2 } @kept = 4 id = @kept } building = native_building level > 0 } } lost = @lost kept = @kept",
        &names,
    );
    assert!(result.unsettled.is_none());
    assert_eq!(result.items[1].value.scalar(), Some(&b"@lost"[..]));
    assert_eq!(result.items[2].value.scalar(), Some(&b"4"[..]));
}

#[test]
fn variable_blocks_use_the_expression_reader() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    for class in [
        "set_temp_variable",
        "add_to_temp_variable",
        "subtract_from_temp_variable",
        "multiply_temp_variable",
        "divide_temp_variable",
    ] {
        let result = read(
            &format!(
                "body = {{ {class} = {{ x = {{ value = 1 @kept = 3 invalid = 0 @lost = 4 add = 1 }} }} }} kept = @kept lost = @lost"
            ),
            &KeyNames::default(),
        );
        assert!(
            result.unsettled.is_none(),
            "{class}: {:?}",
            result.unsettled
        );
        assert_eq!(result.items[1].value.scalar(), Some(&b"3"[..]), "{class}");
        assert_eq!(
            result.items[2].value.scalar(),
            Some(&b"@lost"[..]),
            "{class}"
        );
    }
}

#[test]
fn collection_statements_define_only_inside_the_limit_trigger() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let result = read(
        "body = { collection_size = { input = { input = scope:root operators = { country_and_all_subjects @lost = 2 limit = { invalid = { @hidden = 3 } @kept = 4 always = yes } } } value > 0 } } lost = @lost hidden = @hidden kept = @kept",
        &KeyNames::default(),
    );
    assert!(result.unsettled.is_none(), "{:?}", result.unsettled);
    assert_eq!(result.items[1].value.scalar(), Some(&b"@lost"[..]));
    assert_eq!(result.items[2].value.scalar(), Some(&b"@hidden"[..]));
    assert_eq!(result.items[3].value.scalar(), Some(&b"4"[..]));
}

#[test]
fn railway_and_country_lists_do_not_register_definitions() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    for (class, key) in [
        ("can_build_railway", "path"),
        ("has_railway_connection", "path"),
        ("any_country_of", "target"),
        ("any_country_with_original_tag_of", "target"),
    ] {
        let result = read(
            &format!("body = {{ {class} = {{ {key} = {{ 1 @lost = 2 }} }} }} lost = @lost"),
            &KeyNames::default(),
        );
        assert!(
            result.unsettled.is_none(),
            "{class}: {:?}",
            result.unsettled
        );
        assert_eq!(
            result.items[1].value.scalar(),
            Some(&b"@lost"[..]),
            "{class}"
        );
    }
}

#[test]
fn license_equipment_reference_skips_invalid_child_blocks() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let result = read(
        "body = { has_license = { equipment = { type = native_type invalid = { @lost = 2 } @kept = 3 version_name = @kept } } } lost = @lost kept = @kept",
        &KeyNames::default(),
    );
    assert!(result.unsettled.is_none());
    assert_eq!(result.items[1].value.scalar(), Some(&b"@lost"[..]));
    assert_eq!(result.items[2].value.scalar(), Some(&b"3"[..]));
}

#[test]
fn validated_lists_skip_invalid_opening_entries_without_defining() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    for (class, key) in [
        ("amount_taken_ideas", "categories"),
        ("amount_taken_ideas", "slots"),
        ("count_in_collection", "unit_category"),
    ] {
        let result = read(
            &format!(
                "body = {{ {class} = {{ {key} = {{ {{ @lost = 2 }} token }} @kept = 3 marker = @kept }} }} lost = @lost kept = @kept"
            ),
            &KeyNames::default(),
        );
        assert!(
            result.unsettled.is_none(),
            "{class}: {:?}",
            result.unsettled
        );
        assert_eq!(
            result.items[1].value.scalar(),
            Some(&b"@lost"[..]),
            "{class}"
        );
        assert_eq!(result.items[2].value.scalar(), Some(&b"3"[..]), "{class}");
    }
}

#[test]
fn unreached_key_readers_refuse_by_parent_class_name() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    for class in [
        "career_profile_check_points",
        "career_profile_check_medal",
        "is_licensing_to",
        "faction_goal_fulfillment",
    ] {
        let result = read(
            &format!("body = {{ {class} = {{ value = 1 }} }}"),
            &KeyNames::default(),
        );
        assert!(result.unsettled.unwrap().what.contains(class), "{class}");
    }
}

#[test]
fn invalid_meta_parameters_use_record_errors_skip_after_the_raw_read() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let result = read(
        // Error recovery consumes one closing delimiter; each enclosing loop
        // then needs its own delimiter before the file resumes.
        "body = { meta_trigger = { text = { ] } @lost = 2 debug = yes } } } @kept = 3 marker = @kept lost = @lost kept = @kept",
        &KeyNames::default(),
    );
    assert!(result.unsettled.is_none());
    assert_eq!(
        result
            .items
            .iter()
            .find(|item| item.keyword_is("lost"))
            .unwrap()
            .value
            .scalar(),
        Some(&b"@lost"[..])
    );
    assert_eq!(
        result
            .items
            .iter()
            .find(|item| item.keyword_is("kept"))
            .unwrap()
            .value
            .scalar(),
        Some(&b"3"[..])
    );
    assert!(
        result
            .misreads
            .iter()
            .any(|note| note.what.contains("raw meta text"))
    );
}

#[test]
fn embedded_nul_in_meta_text_is_refused_by_name() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let text = String::from_utf8(
        b"body = { meta_trigger = { text = { "
            .iter()
            .copied()
            .chain([0])
            .chain(b" } } }".iter().copied())
            .collect(),
    )
    .unwrap();
    let result = read(&text, &KeyNames::default());
    assert!(
        result
            .unsettled
            .unwrap()
            .what
            .contains("raw meta text cannot be read: embedded NUL")
    );
}

#[test]
fn meta_text_counts_braces_in_comments_as_raw_characters() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let result = read(
        "body = { meta_trigger = { text = { # }\n @kept = 7 debug = yes } } kept = @kept",
        &KeyNames::default(),
    );
    assert!(result.unsettled.is_none());
    assert_eq!(result.items[1].value.scalar(), Some(&b"7"[..]));
}

#[test]
fn any_state_of_reads_target_arrays_without_defining_their_tokens() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    for operator in ["=", ""] {
        let text = format!(
            "@first = 11 body = {{ any_state_of = {{ target {operator} {{ @first 2 @lost = 4 }} @kept = 7 always = yes }} }} kept = @kept lost = @lost"
        );
        let read = native::read(
            text.as_bytes(),
            native::Mode::KeepMark,
            native::Top::File(()),
            Some(&KeyNames::default()),
            &mut |(), field| {
                if field.keyword_is("body") {
                    Shape::Trigger
                } else {
                    Shape::One
                }
            },
        );
        assert!(read.unsettled.is_none(), "{read:?}");
        assert_eq!(read.items[1].value.scalar(), Some(b"7".as_slice()));
        assert_eq!(read.items[2].value.scalar(), Some(b"@lost".as_slice()));
        let state = read.items[0].value.items().unwrap()[0]
            .value
            .items()
            .unwrap();
        assert_eq!(state.len(), 2, "{state:?}");
        assert!(state[1].keyword_is("always"));
        let target = state[0].value.items().unwrap();
        assert_eq!(target.len(), 5);
        assert!(target.iter().all(|item| item.key.is_none()));
        assert_eq!(target[0].value.scalar(), Some(b"11".as_slice()));
    }
    let read = crate::triggers::read(
        b"any_state_of = { target = { 1 2 } always extra = 8 }",
        &KeyNames::default(),
    );
    assert!(read.unsettled.is_none(), "{read:?}");
    let fields = read.items[0].value.items().unwrap();
    assert!(fields[1].keyword_is("always"));
    assert_eq!(fields[1].value.scalar(), Some(b"2".as_slice()));
}
