use crate::test_game::effect_assertions::{assert_complete_template_read, body, scalar};
use crate::{key_names::KeyNames, native::{self, Read, Shape}, scope::ScopeKind};

fn initialize_tables() {
    crate::test_game::enter();
}

fn read(text: &[u8]) -> Read {
    initialize_tables();
    native::read(
        text,
        native::Mode::KeepMark,
        native::Top::File(()),
        Some(&KeyNames::default()),
        &mut |(), field| {
            if field.keyword_is("body") {
                Shape::EffectIn(ScopeKind::Country.bit())
            } else {
                Shape::One
            }
        },
    )
}

macro_rules! scalar_value_reader_case {
    ($test:ident, $key:literal) => {
        #[test]
        fn $test() {
            let input = format!(
                "body = {{ {} = fixture add_stability = 0.1 }} tail = 3",
                $key
            );
            let result = read(input.as_bytes());
            assert!(result.unsettled.is_none(), "{result:?}");
            assert!(
                result
                    .misreads
                    .iter()
                    .all(|note| !note.what.starts_with("Deferred effect")),
                "{result:?}"
            );
            let effects = body(&result.items, "body");
            assert_eq!(scalar(effects, $key), b"fixture");
            assert_eq!(scalar(effects, "add_stability"), b"0.1");
            assert_eq!(scalar(&result.items, "tail"), b"3");
        }
    };
}
scalar_value_reader_case!(show_ideas_tooltip_scalar, "show_ideas_tooltip");
scalar_value_reader_case!(
    unlock_decision_category_tooltip_scalar,
    "unlock_decision_category_tooltip"
);
scalar_value_reader_case!(activate_mission_scalar, "activate_mission");
scalar_value_reader_case!(remove_mission_scalar, "remove_mission");
scalar_value_reader_case!(retire_character_scalar, "retire_character");
scalar_value_reader_case!(
    show_unit_leaders_tooltip_scalar,
    "show_unit_leaders_tooltip"
);
scalar_value_reader_case!(activate_decision_scalar, "activate_decision");
scalar_value_reader_case!(remove_decision_scalar, "remove_decision");
scalar_value_reader_case!(activate_advisor_scalar, "activate_advisor");
scalar_value_reader_case!(deactivate_advisor_scalar, "deactivate_advisor");
scalar_value_reader_case!(activate_mission_tooltip_scalar, "activate_mission_tooltip");
scalar_value_reader_case!(remove_unit_leader_scalar, "remove_unit_leader");
scalar_value_reader_case!(recruit_character_scalar, "recruit_character");
scalar_value_reader_case!(
    injure_scientist_for_days_scalar,
    "injure_scientist_for_days"
);
scalar_value_reader_case!(add_scientist_trait_scalar, "add_scientist_trait");
scalar_value_reader_case!(capture_by_scalar, "capture_by");
scalar_value_reader_case!(
    release_captured_generals_from_scalar,
    "release_captured_generals_from"
);
scalar_value_reader_case!(remove_scientist_role_scalar, "remove_scientist_role");
scalar_value_reader_case!(set_oob_scalar, "set_oob");
scalar_value_reader_case!(set_naval_oob_scalar, "set_naval_oob");
scalar_value_reader_case!(set_air_oob_scalar, "set_air_oob");

#[test]
fn assignment_key_readers() {
    let result = read(b"body={capture_by={country=fixture province=1 ignored={@lost=1}} release_captured_generals_from={target=fixture province=2 ignored={@lost=2}} remove_scientist_role={character=fixture ignored={@lost=3}}} lost=@lost");
    assert!(result.unsettled.is_none(), "{result:?}");
    assert!(
        result
            .misreads
            .iter()
            .all(|note| !note.what.starts_with("Deferred effect")),
        "{result:?}"
    );
    let effects = body(&result.items, "body");
    assert_eq!(scalar(body(effects, "capture_by"), "country"), b"fixture");
    assert_eq!(
        scalar(body(effects, "release_captured_generals_from"), "target"),
        b"fixture"
    );
    assert_eq!(
        scalar(body(effects, "remove_scientist_role"), "character"),
        b"fixture"
    );
    assert_eq!(scalar(&result.items, "lost"), b"@lost");
}

#[test]
fn remove_unit_leader_error_skips_a_block_value() {
    let result = read(b"body={remove_unit_leader={@lost=3} add_stability=0.1} lost=@lost");
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(scalar(body(&result.items, "body"), "add_stability"), b"0.1");
    assert_eq!(scalar(&result.items, "lost"), b"@lost");
    assert!(
        result
            .misreads
            .iter()
            .all(|note| !note.what.starts_with("Deferred effect"))
    );
}
#[test]
fn event_value_readers_use_their_own_keys() {
    for class in [
        "country_event",
        "news_event",
        "state_event",
        "unit_leader_event",
        "operative_leader_event",
    ] {
        let text = format!(
            "body = {{ {class} = {{ id = event.1 invalid = {{ @lost = 2 }} @seen = 3 days = @seen }} }} lost = @lost seen = @seen"
        );
        let result = read(text.as_bytes());
        assert!(result.unsettled.is_none(), "{class}: {result:?}");
        assert_eq!(scalar(&result.items, "lost"), b"@lost", "{class}");
        assert_eq!(scalar(&result.items, "seen"), b"3", "{class}");
        assert!(
            !result
                .misreads
                .iter()
                .any(|note| note.what.starts_with("Deferred effect")),
            "{result:?}"
        );
    }
    let legacy = read(b"body = { country_event = { recipient = { @lost = 4 } } } lost = @lost");
    assert_eq!(scalar(&legacy.items, "lost"), b"@lost");
    let operative = read(
        b"body = { operative_leader_event = { recipient = { @seen = 4 days = 0 } } seen = @seen",
    );
    assert_eq!(scalar(&operative.items, "seen"), b"4");
    assert_eq!(
        scalar(
            body(body(&operative.items, "body"), "operative_leader_event"),
            "recipient"
        ),
        b"{"
    );
}
#[test]
fn idea_value_reader_is_a_raw_list_or_current_token() {
    for class in ["add_ideas", "remove_ideas"] {
        let text = format!(
            "@alias = idea body = {{ {class} = {{ @lost = 7 some_idea }} {class} = @alias }} lost = @lost"
        );
        let result = read(text.as_bytes());
        assert!(result.unsettled.is_none(), "{result:?}");
        assert_eq!(scalar(&result.items, "lost"), b"@lost");
        let items = body(&result.items, "body");
        let list = items[0].value.items().unwrap();
        assert_eq!(list.len(), 4);
        assert!(list.iter().all(|item| item.key.is_none()));
        assert_eq!(items[1].value.scalar(), Some(b"@alias".as_slice()));
    }
}
#[test]
fn tooltip_value_reader_recursively_binds_localization_arguments() {
    let result = read(b"body = { custom_effect_tooltip = { localization_key = ROOT argument = { localization_key = { @seen = 4 child = KEY } } } seen = @seen");
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(scalar(&result.items, "seen"), b"4");
    let argument = body(
        body(body(&result.items, "body"), "custom_effect_tooltip"),
        "argument",
    );
    assert_eq!(scalar(argument, "localization_key"), b"{");
    assert!(
        !result
            .misreads
            .iter()
            .any(|note| note.what.starts_with("Deferred effect"))
    );
}
#[test]
fn flag_value_readers_skip_unknown_fields_and_keep_one_value_fields() {
    for class in [
        "set_country_flag",
        "set_state_flag",
        "set_global_flag",
        "set_character_flag",
        "set_unit_leader_flag",
        "set_mio_flag",
        "set_project_flag",
        "modify_country_flag",
        "modify_state_flag",
        "modify_global_flag",
        "modify_character_flag",
        "modify_unit_leader_flag",
        "modify_mio_flag",
        "modify_project_flag",
    ] {
        let text = format!(
            "body = {{ {class} = {{ flag = token invalid = {{ @lost = 2 }} value = {{ @seen = 3 days = 1 }} }} lost = @lost seen = @seen"
        );
        let result = read(text.as_bytes());
        assert!(result.unsettled.is_none(), "{class}: {result:?}");
        assert_eq!(scalar(&result.items, "lost"), b"@lost", "{class}");
        assert_eq!(scalar(&result.items, "seen"), b"3", "{class}");
        assert_eq!(
            scalar(body(body(&result.items, "body"), class), "value"),
            b"{",
            "{class}"
        );
    }
}
#[test]
fn swap_ideas_value_reader_dispatches_its_inline_keys() {
    let result = read(b"body = { swap_ideas = { add_idea = mod_idea invalid = { @lost = 2 } @seen = 3 days = @seen } } lost = @lost seen = @seen");
    assert_eq!(scalar(&result.items, "lost"), b"@lost");
    assert_eq!(scalar(&result.items, "seen"), b"3");
    assert!(
        !result
            .misreads
            .iter()
            .any(|note| note.what.starts_with("Deferred effect"))
    );
    let scalar = read(b"body = { swap_ideas = no } final = 1");
    assert!(
        scalar
            .misreads
            .iter()
            .any(|note| note.what.contains("effect `swap_ideas` requires a block")),
        "{scalar:?}"
    );
}
#[test]
fn scalar_or_inline_value_readers_use_accepted_fields() {
    for (class, key) in [
        ("set_character_name", "name"),
        ("set_nationality", "target_country"),
        ("unlock_decision_tooltip", "decision"),
        ("white_peace", "tag"),
        ("puppet", "target"),
        ("load_focus_tree", "tree"),
    ] {
        let text = format!(
            "body = {{ {class} = {{ {key} = authored invalid = {{ @lost = 2 }} @seen = 3 {key} = @seen }} }} lost = @lost seen = @seen"
        );
        let result = read(text.as_bytes());
        assert!(result.unsettled.is_none(), "{result:?}");
        assert_eq!(scalar(&result.items, "lost"), b"@lost", "{class}");
        assert_eq!(scalar(&result.items, "seen"), b"3", "{class}");
    }
}
#[test]
fn variable_builder_value_readers_use_field_loops() {
    for (class, key) in [
        ("get_highest_scored_country", "var"),
        ("get_highest_scored_country_temp", "var"),
        ("get_sorted_scored_countries", "array"),
        ("get_sorted_scored_countries_temp", "array"),
        ("get_supply_vehicles_temp", "var"),
    ] {
        let text = format!(
            "body = {{ {class} = {{ {key} = authored invalid = {{ @lost = 2 }} @seen = 3 {key} = @seen }} }} lost = @lost seen = @seen"
        );
        let result = read(text.as_bytes());
        assert!(result.unsettled.is_none(), "{result:?}");
        assert_eq!(scalar(&result.items, "lost"), b"@lost", "{class}");
        assert_eq!(scalar(&result.items, "seen"), b"3", "{class}");
    }
}
#[test]
fn shared_value_reader_uses_own_keys_and_popularity_effect_fallback() {
    for (class, key) in [
        ("add_popularity", "popularity"),
        ("release_autonomy", "freedom_level"),
        ("set_autonomy", "freedom_level"),
        ("set_autonomy", "keep_subjects"),
        ("send_equipment_fraction", "value"),
    ] {
        let text = format!(
            "body = {{ {class} = {{ {key} = 1 invalid = {{ @lost = 2 }} @seen = 3 {key} = @seen }} }} lost = @lost seen = @seen"
        );
        let result = read(text.as_bytes());
        assert_eq!(scalar(&result.items, "lost"), b"@lost", "{class}");
        assert_eq!(scalar(&result.items, "seen"), b"3", "{class}");
        assert!(
            !result
                .misreads
                .iter()
                .any(|note| note.what.starts_with("Deferred effect")),
            "{result:?}"
        );
    }
    let result = read(b"body = { add_popularity = { country_event = { invalid = { @lost = 2 } @seen = 3 days = @seen } } } lost = @lost seen = @seen");
    assert_eq!(scalar(&result.items, "lost"), b"@lost");
    assert_eq!(scalar(&result.items, "seen"), b"3");
}
#[test]
fn simple_assignment_value_reader_keeps_each_classes_error_policy() {
    for class in [
        "complete_mio_trait",
        "unlock_mio_trait_tooltip",
        "unlock_mio_policy_tooltip",
    ] {
        let text = format!(
            "body = {{ {class} = {{ invalid = {{ @lost = 2 }} @seen = 3 show_modifiers = yes }} }} lost = @lost seen = @seen"
        );
        let result = read(text.as_bytes());
        assert_eq!(scalar(&result.items, "lost"), b"@lost", "{class}");
        assert_eq!(scalar(&result.items, "seen"), b"3", "{class}");
    }
    let result = read(b"body = { country_lock_all_division_template = { invalid = { @seen = 2 is_locked = yes } } seen = @seen");
    assert_eq!(scalar(&result.items, "seen"), b"2");
    assert_eq!(
        scalar(
            body(
                body(&result.items, "body"),
                "country_lock_all_division_template"
            ),
            "invalid"
        ),
        b"{"
    );
    let result = read(b"body = { create_faction_from_template = { color = { 1 2 3 } } } final = 1");
    assert_eq!(
        body(
            body(body(&result.items, "body"), "create_faction_from_template"),
            "color"
        )
        .len(),
        3
    );
    assert!(
        !result
            .misreads
            .iter()
            .any(|note| note.what.starts_with("Deferred effect"))
    );
}
#[test]
fn modulo_value_readers_keep_shorthand_operands_without_block_skip() {
    for class in ["modulo_temp_variable", "modulo_variable"] {
        let text = format!(
            "body = {{ {class} = {{ arbitrary_operand = {{ @seen = 3 value = 2 }} }} seen = @seen"
        );
        let result = read(text.as_bytes());
        assert_eq!(scalar(&result.items, "seen"), b"3");
        assert_eq!(
            scalar(
                body(body(&result.items, "body"), class),
                "arbitrary_operand"
            ),
            b"{"
        );
        assert!(
            !result
                .misreads
                .iter()
                .any(|note| note.what.starts_with("Deferred effect"))
        );
    }
}
#[test]
fn random_assignment_value_readers_skip_unknown_keys() {
    for class in ["set_temp_variable_to_random", "set_variable_to_random"] {
        let text = format!(
            "body = {{ {class} = {{ var = my_var invalid = {{ @lost = 2 }} @seen = 3 min = @seen max = 9 integer = yes }} }} lost = @lost seen = @seen"
        );
        let result = read(text.as_bytes());
        assert_eq!(scalar(&result.items, "lost"), b"@lost", "{class}");
        assert_eq!(scalar(&result.items, "seen"), b"3", "{class}");
        let quoted = read(format!("body = {{ {class} = \"my_var\" }}").as_bytes());
        assert!(
            !quoted
                .misreads
                .iter()
                .any(|note| note.what.starts_with("Deferred effect"))
        );
        let ambiguous = read(format!("body = {{ {class} = authored_name }}").as_bytes());
        assert!(
            ambiguous
                .misreads
                .iter()
                .any(|note| note.what.starts_with("Deferred effect"))
        );
    }
}
#[test]
fn randomize_value_reader_tracks_distribution_in_authored_order() {
    for class in ["randomize_temp_variable", "randomize_variable"] {
        let text = format!(
            "body = {{ {class} = {{ var = my_var max = 2 distribution = poisson max = {{ @lost = 2 }} distribution = Uniform max = {{ @also_lost = 4 }} lambda = {{ @seen = 3 min = 0 }} }} lost = @lost also_lost = @also_lost seen = @seen"
        );
        let result = read(text.as_bytes());
        assert_eq!(scalar(&result.items, "lost"), b"@lost", "{class}");
        assert_eq!(scalar(&result.items, "also_lost"), b"@also_lost", "{class}");
        assert_eq!(scalar(&result.items, "seen"), b"3", "{class}");
    }
}
#[test]
fn value_wargoals_reject_duplicate_generators_after_scalar() {
    for class in ["create_wargoal", "declare_war_on"] {
        let text = format!(
            "body = {{ {class} = {{ generator = ROOT generator = {{ @lost = 2 }} @seen = 3 target = @seen }} }} lost = @lost seen = @seen"
        );
        let result = read(text.as_bytes());
        assert!(result.unsettled.is_none(), "{class}: {result:?}");
        assert_eq!(scalar(&result.items, "lost"), b"@lost", "{class}");
        assert_eq!(scalar(&result.items, "seen"), b"3", "{class}");
        let fields = body(body(&result.items, "body"), class);
        assert_eq!(fields[0].value.scalar(), Some(b"ROOT".as_slice()));
        assert!(fields[1].value.items().unwrap().is_empty(), "{result:?}");
    }
}
#[test]
fn value_wargoals_reject_duplicate_generators_after_array() {
    for class in ["create_wargoal", "declare_war_on"] {
        let text = format!(
            "@alias = 6 body = {{ {class} = {{ generator = {{ @alias 7 }} generator = {{ @lost = 2 }} @seen = 3 target = @seen }} }} lost = @lost seen = @seen"
        );
        let result = read(text.as_bytes());
        assert!(result.unsettled.is_none(), "{class}: {result:?}");
        assert_eq!(scalar(&result.items, "lost"), b"@lost", "{class}");
        assert_eq!(scalar(&result.items, "seen"), b"3", "{class}");
        let fields = body(body(&result.items, "body"), class);
        let entries = fields[0].value.items().unwrap();
        assert_eq!(entries.len(), 2, "{result:?}");
        assert!(entries.iter().all(|item| item.key.is_none()));
        assert_eq!(entries[0].value.scalar(), Some(b"6".as_slice()));
        assert_eq!(entries[1].value.scalar(), Some(b"7".as_slice()));
        assert!(fields[1].value.items().unwrap().is_empty(), "{result:?}");
    }
}
#[test]
fn value_wargoal_arrays_preserve_the_last_value_slot() {
    for class in ["create_wargoal", "declare_war_on"] {
        let text =
            format!("body = {{ {class} = {{ generator = {{ 1 2 }} target }} }} }} final = 1");
        let result = read(text.as_bytes());
        assert!(result.unsettled.is_none(), "{class}: {result:?}");
        assert_eq!(
            scalar(body(body(&result.items, "body"), class), "target"),
            b"2",
            "{class}: {result:?}"
        );
        assert_eq!(scalar(&result.items, "final"), b"1", "{class}");
    }
}
#[test]
fn value_wargoal_arrays_keep_raw_nested_openings() {
    for class in ["create_wargoal", "declare_war_on"] {
        let text = format!(
            "body = {{ {class} = {{ generator = {{ 1 {{ @raw = 2 }} @seen = 4 target = @seen }} }} raw = @raw seen = @seen"
        );
        let result = read(text.as_bytes());
        assert!(result.unsettled.is_none(), "{class}: {result:?}");
        assert_eq!(scalar(&result.items, "raw"), b"@raw", "{class}");
        assert_eq!(scalar(&result.items, "seen"), b"4", "{class}");
        let fields = body(body(&result.items, "body"), class);
        assert_eq!(scalar(fields, "target"), b"4", "{class}");
        let entries = body(fields, "generator");
        assert_eq!(entries.len(), 5, "{class}: {result:?}");
        assert!(entries.iter().all(|item| item.key.is_none()));
        for (entry, expected) in entries.iter().zip([
            b"1".as_slice(),
            b"{".as_slice(),
            b"@raw".as_slice(),
            b"=".as_slice(),
            b"2".as_slice(),
        ]) {
            assert_eq!(entry.value.scalar(), Some(expected), "{class}");
        }
    }
}
#[test]
fn value_wargoal_type_blocks_are_skipped() {
    for class in ["create_wargoal", "declare_war_on", "remove_wargoal"] {
        let text = format!(
            "body = {{ {class} = {{ type = {{ @lost = 2 }} @seen = 3 target = @seen }} }} lost = @lost seen = @seen"
        );
        let result = read(text.as_bytes());
        assert!(result.unsettled.is_none(), "{class}: {result:?}");
        assert_eq!(scalar(&result.items, "lost"), b"@lost", "{class}");
        assert_eq!(scalar(&result.items, "seen"), b"3", "{class}");
        let fields = body(body(&result.items, "body"), class);
        assert!(body(fields, "type").is_empty(), "{class}: {result:?}");
        assert_eq!(scalar(fields, "target"), b"3", "{class}");
    }
}
#[test]
fn value_equipment_old_prioritised_skips_raw_tokens() {
    let result = read(
        b"@open = { } anchor = 0 body = { send_equipment = { old_prioritised = { @open @unresolved @lost = 2 } amount = 3 } } lost = @lost final = 1",
    );
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(scalar(&result.items, "lost"), b"@lost");
    assert_eq!(scalar(&result.items, "final"), b"1");
    let fields = body(body(&result.items, "body"), "send_equipment");
    assert!(body(fields, "old_prioritised").is_empty(), "{result:?}");
    assert_eq!(scalar(fields, "amount"), b"3");
}
#[test]
fn value_transfer_leaders_are_a_raw_integer_array() {
    let result = read(
        b"@alias = 6 body = { transfer_units_fraction = { keep_unit_leaders = { @alias @raw = 2 } size = 0.5 } } raw = @raw final = 1",
    );
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(scalar(&result.items, "raw"), b"@raw");
    assert_eq!(scalar(&result.items, "final"), b"1");
    let fields = body(body(&result.items, "body"), "transfer_units_fraction");
    assert_eq!(scalar(fields, "size"), b"0.5");
    let entries = body(fields, "keep_unit_leaders");
    assert_eq!(entries.len(), 4, "{result:?}");
    assert!(entries.iter().all(|item| item.key.is_none()));
    assert_eq!(entries[0].value.scalar(), Some(b"6".as_slice()));
    assert_eq!(entries[1].value.scalar(), Some(b"@raw".as_slice()));
    assert_eq!(entries[2].value.scalar(), Some(b"=".as_slice()));
    assert_eq!(entries[3].value.scalar(), Some(b"2".as_slice()));
}
#[test]
fn value_transfer_arrays_preserve_the_last_value_slot() {
    let result = read(
        b"body = { transfer_units_fraction = { keep_unit_leaders = { 1 2 } size } } } final = 1",
    );
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(
        scalar(
            body(body(&result.items, "body"), "transfer_units_fraction"),
            "size"
        ),
        b"2"
    );
    assert_eq!(scalar(&result.items, "final"), b"1");
}
#[test]
fn value_transfer_leader_filter_uses_current_names_and_trigger_dispatch() {
    let mut names = KeyNames::default();
    names.countries.insert(b"XYZ".to_vec());
    initialize_tables();
    let result = native::read(
        b"body = { transfer_units_fraction = { keep_unit_leaders_trigger = { XYZ = { invalid_trigger = { @lost = 2 } @seen = 3 always = yes } } size = 0.5 } } lost = @lost seen = @seen final = 1",
        native::Mode::KeepMark,
        native::Top::File(()),
        Some(&names),
        &mut |(), field| {
            if field.keyword_is("body") {
                Shape::Effect
            } else {
                Shape::One
            }
        },
    );
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(scalar(&result.items, "lost"), b"@lost");
    assert_eq!(scalar(&result.items, "seen"), b"3");
    assert_eq!(scalar(&result.items, "final"), b"1");
    let fields = body(body(&result.items, "body"), "transfer_units_fraction");
    assert_eq!(scalar(fields, "size"), b"0.5");
    let country = body(body(fields, "keep_unit_leaders_trigger"), "XYZ");
    assert!(body(country, "invalid_trigger").is_empty(), "{result:?}");
    assert_eq!(scalar(country, "always"), b"yes");
}
#[test]
fn value_resistance_blocks_keep_effect_fallback() {
    for (class, own_key) in [
        ("add_resistance_target", "amount"),
        ("force_enable_resistance", "clear"),
        ("force_disable_resistance", "clear"),
    ] {
        let text = format!(
            "body = {{ {class} = {{ {own_key} = yes remove_ideas = {{ @raw = 2 }} @seen = 3 occupier = @seen }} }} raw = @raw seen = @seen"
        );
        let result = read(text.as_bytes());
        assert!(result.unsettled.is_none(), "{class}: {result:?}");
        assert_eq!(scalar(&result.items, "raw"), b"@raw", "{class}");
        assert_eq!(scalar(&result.items, "seen"), b"3", "{class}");
        let fields = body(body(&result.items, "body"), class);
        assert_eq!(scalar(fields, own_key), b"yes", "{class}");
        assert_eq!(scalar(fields, "occupier"), b"3", "{class}");
        let entries = body(fields, "remove_ideas");
        assert_eq!(entries.len(), 3, "{class}: {result:?}");
        assert!(entries.iter().all(|item| item.key.is_none()));
    }
}
#[test]
fn value_project_iteration_output_uses_statement_callbacks() {
    let result = read(b"body = { complete_special_project = { project = authored iteration_output = { reward reward = option @not_defined = 7 { @hidden = 9 } } invalid = { @unknown = 6 } @seen = 3 state = @seen scientist = somebody show_modifiers = yes } } not_defined = @not_defined hidden = @hidden unknown = @unknown seen = @seen");
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(scalar(&result.items, "not_defined"), b"@not_defined");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    assert_eq!(scalar(&result.items, "unknown"), b"@unknown");
    assert_eq!(scalar(&result.items, "seen"), b"3");
    let statements = body(
        body(body(&result.items, "body"), "complete_special_project"),
        "iteration_output",
    );
    assert!(statements[0].key.is_none(), "{result:?}");
    assert_eq!(statements[0].value.scalar(), Some(b"reward".as_slice()));
    assert_eq!(scalar(statements, "reward"), b"option");
    assert_eq!(scalar(statements, "@not_defined"), b"7");
    assert!(
        !result
            .misreads
            .iter()
            .any(|note| note.what.starts_with("Deferred effect")),
        "{result:?}"
    );
}
#[test]
fn value_project_iteration_assignment_brace_keeps_one_token() {
    let result = read(b"body = { complete_special_project = { project = authored iteration_output = { reward = { @not_defined = 2 } @seen = 3 state = @seen } } not_defined = @not_defined seen = @seen");
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(scalar(&result.items, "not_defined"), b"@not_defined");
    assert_eq!(scalar(&result.items, "seen"), b"3");
    let project = body(body(&result.items, "body"), "complete_special_project");
    assert_eq!(scalar(project, "state"), b"3");
    assert_eq!(scalar(body(project, "iteration_output"), "reward"), b"{");
    assert!(
        !result
            .misreads
            .iter()
            .any(|note| note.what.starts_with("Deferred effect")),
        "{result:?}"
    );
}
#[test]
fn value_project_iteration_scalar_reads_the_statement_tail() {
    let result = read(b"body = { complete_special_project = { project = authored iteration_output = ignored reward = option @not_defined = 2 } @seen = 3 state = @seen } } not_defined = @not_defined seen = @seen");
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(scalar(&result.items, "not_defined"), b"@not_defined");
    assert_eq!(scalar(&result.items, "seen"), b"3");
    let project = body(body(&result.items, "body"), "complete_special_project");
    assert_eq!(scalar(project, "state"), b"3");
    let statements = body(project, "iteration_output");
    assert_eq!(scalar(statements, "reward"), b"option");
    assert_eq!(scalar(statements, "@not_defined"), b"2");
    assert!(
        !result
            .misreads
            .iter()
            .any(|note| note.what.starts_with("Deferred effect")),
        "{result:?}"
    );
}
#[test]
fn value_focus_originator_binds_localization_arguments() {
    let result = read(b"body = { complete_national_focus = { focus = authored originator_name = { localization_key = ROOT argument = { localization_key = { @seen = 4 child = KEY } } } } seen = @seen");
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(scalar(&result.items, "seen"), b"4");
    let focus = body(body(&result.items, "body"), "complete_national_focus");
    assert_eq!(scalar(focus, "focus"), b"authored");
    let argument = body(body(focus, "originator_name"), "argument");
    assert_eq!(scalar(argument, "localization_key"), b"{");
    assert_eq!(scalar(argument, "child"), b"KEY");
    assert!(
        !result
            .misreads
            .iter()
            .any(|note| note.what.starts_with("Deferred effect")),
        "{result:?}"
    );
}
#[test]
fn value_focus_agency_and_doctrine_keep_effect_fallback() {
    for (class, own_fields) in [
        (
            "complete_national_focus",
            "focus = authored use_side_message = yes",
        ),
        (
            "create_intelligence_agency",
            "name = authored icon = mod_icon",
        ),
        (
            "set_sub_doctrine",
            "folder = authored sub_doctrine = authored track = 1",
        ),
    ] {
        let text = format!(
            "body = {{ {class} = {{ {own_fields} country_event = {{ invalid = {{ @lost = 2 }} @seen = 3 days = @seen }} invalid = {{ @also_lost = 4 }} }} }} lost = @lost also_lost = @also_lost seen = @seen"
        );
        let result = read(text.as_bytes());
        assert!(result.unsettled.is_none(), "{class}: {result:?}");
        assert_eq!(scalar(&result.items, "lost"), b"@lost", "{class}");
        assert_eq!(scalar(&result.items, "also_lost"), b"@also_lost", "{class}");
        assert_eq!(scalar(&result.items, "seen"), b"3", "{class}");
        let event = body(body(body(&result.items, "body"), class), "country_event");
        assert_eq!(scalar(event, "days"), b"3", "{class}");
        assert!(
            !result
                .misreads
                .iter()
                .any(|note| note.what.starts_with("Deferred effect")),
            "{class}: {result:?}"
        );
    }
}
#[test]
fn value_doctrine_database_failure_does_not_skip_a_brace() {
    let result =
        read(b"body = { set_sub_doctrine = { folder = { @seen = 3 track = 1 } } seen = @seen");
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(scalar(&result.items, "seen"), b"3");
    let doctrine = body(body(&result.items, "body"), "set_sub_doctrine");
    assert_eq!(scalar(doctrine, "folder"), b"{");
    assert_eq!(scalar(doctrine, "track"), b"1");
    assert!(
        !result
            .misreads
            .iter()
            .any(|note| note.what.starts_with("Deferred effect")),
        "{result:?}"
    );
}
#[test]
fn value_strategic_location_scalar_runs_the_key_loop() {
    let result = read(
        b"body = { strategic_state_location = ignored @seen = 3 arbitrary = 1 } } seen = @seen",
    );
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(scalar(&result.items, "seen"), b"3");
    let location = body(body(&result.items, "body"), "strategic_state_location");
    assert_eq!(scalar(location, "arbitrary"), b"1");
    assert!(
        !result
            .misreads
            .iter()
            .any(|note| note.what.starts_with("Deferred effect")),
        "{result:?}"
    );
}
#[test]
fn value_strategic_location_unknown_brace_is_one_value() {
    let result = read(b"body = { strategic_state_location = { missing_location = { @seen = 3 other = 1 } } seen = @seen");
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(scalar(&result.items, "seen"), b"3");
    let location = body(body(&result.items, "body"), "strategic_state_location");
    assert_eq!(scalar(location, "missing_location"), b"{");
    assert_eq!(scalar(location, "other"), b"1");
    assert!(
        !result
            .misreads
            .iter()
            .any(|note| note.what.starts_with("Deferred effect")),
        "{result:?}"
    );
}
#[test]
fn value_peace_conference_rejects_unknown_fields() {
    let result = read(b"body = { start_peace_conference = { tooltip = authored message = authored tag = ROOT winner_scope = ALL loser_scope = LIMITED_FACTION score_factor = 1 invalid = { @lost = 2 } @seen = 3 score_factor = @seen } } lost = @lost seen = @seen");
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(scalar(&result.items, "lost"), b"@lost");
    assert_eq!(scalar(&result.items, "seen"), b"3");
    let conference = body(body(&result.items, "body"), "start_peace_conference");
    assert_eq!(scalar(conference, "winner_scope"), b"ALL");
    assert_eq!(scalar(conference, "loser_scope"), b"LIMITED_FACTION");
    assert_eq!(scalar(conference, "score_factor"), b"1");
    assert!(
        !result
            .misreads
            .iter()
            .any(|note| note.what.starts_with("Deferred effect")),
        "{result:?}"
    );
}
#[test]
fn value_country_trait_scalar_and_inline_keys() {
    for class in ["add_country_leader_trait", "remove_country_leader_trait"] {
        let text = format!(
            "@alias = authored_trait body = {{ {class} = @alias {class} = {{ ideology = authored_ideology trait = authored_trait invalid = {{ @lost = 2 }} @seen = 3 trait = @seen }} }} lost = @lost seen = @seen"
        );
        let result = read(text.as_bytes());
        assert!(result.unsettled.is_none(), "{class}: {result:?}");
        assert_eq!(scalar(&result.items, "lost"), b"@lost", "{class}");
        assert_eq!(scalar(&result.items, "seen"), b"3", "{class}");
        assert_eq!(
            body(&result.items, "body")[0].value.scalar(),
            Some(b"authored_trait".as_slice())
        );
        assert!(
            !result
                .misreads
                .iter()
                .any(|note| note.what.starts_with("Deferred effect")),
            "{class}: {result:?}"
        );
    }
}
#[test]
fn value_swap_traits_keep_lookup_tokens_and_skip_unknowns() {
    for class in ["swap_ruler_traits", "swap_country_leader_traits"] {
        let text = format!(
            "body = {{ {class} = {{ remove = authored_trait add = authored_trait invalid = {{ @lost = 2 }} @seen = 3 add = @seen }} }} lost = @lost seen = @seen"
        );
        let result = read(text.as_bytes());
        assert!(result.unsettled.is_none(), "{class}: {result:?}");
        assert_eq!(scalar(&result.items, "lost"), b"@lost", "{class}");
        assert_eq!(scalar(&result.items, "seen"), b"3", "{class}");
        assert!(
            !result
                .misreads
                .iter()
                .any(|note| note.what.starts_with("Deferred effect")),
            "{result:?}"
        );
        let one = read(format!("body = {{ {class} = {{ remove = {{ @escaped = 4 add = authored_trait }} }} escaped = @escaped").as_bytes());
        assert!(one.unsettled.is_none(), "{class}: {one:?}");
        assert_eq!(scalar(&one.items, "escaped"), b"4", "{class}");
        assert_eq!(
            scalar(body(body(&one.items, "body"), class), "remove"),
            b"{",
            "{class}"
        );

        let wrong_scalar = read(format!("body = {{ {class} = no }} final = 1").as_bytes());
        assert!(
            wrong_scalar
                .misreads
                .iter()
                .any(|note| note.what.contains("requires a block")),
            "{wrong_scalar:?}"
        );
        assert!(
            !wrong_scalar
                .misreads
                .iter()
                .any(|note| note.what.starts_with("Deferred effect")),
            "{wrong_scalar:?}"
        );
        assert_eq!(scalar(&wrong_scalar.items, "final"), b"1");
    }
}
#[test]
fn value_random_trait_reads_resolved_string_tokens() {
    let result = read(b"@alias = authored_trait body = { add_random_trait = { @lost = 2 @alias } add_random_trait = yes } lost = @lost");
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(scalar(&result.items, "lost"), b"@lost");
    let items = body(&result.items, "body");
    let list = items[0].value.items().unwrap();
    assert_eq!(list.len(), 4, "{result:?}");
    assert!(list.iter().all(|item| item.key.is_none()));
    assert_eq!(list[3].value.scalar(), Some(b"authored_trait".as_slice()));
    assert_eq!(items[1].value.scalar(), Some(b"yes".as_slice()));
    assert!(
        !result
            .misreads
            .iter()
            .any(|note| note.what.starts_with("Deferred effect")),
        "{result:?}"
    );
}
#[test]
fn value_operative_keys_follow_each_class_constructor() {
    for (class, own, rejected) in [
        ("capture_operative", "captured_by", "killed_by"),
        ("kill_operative", "killed_by", "turned_by"),
        ("turn_operative", "turned_by", "captured_by"),
    ] {
        let text = format!(
            "body = {{ {class} = {{ {own} = authored_country operative = authored_operative {rejected} = {{ @lost = 2 }} @seen = 3 {own} = @seen }} }} lost = @lost seen = @seen"
        );
        let result = read(text.as_bytes());
        assert!(result.unsettled.is_none(), "{class}: {result:?}");
        assert_eq!(scalar(&result.items, "lost"), b"@lost", "{class}");
        assert_eq!(scalar(&result.items, "seen"), b"3", "{class}");
        assert!(
            !result
                .misreads
                .iter()
                .any(|note| note.what.starts_with("Deferred effect")),
            "{class}: {result:?}"
        );
        let one = read(format!("body = {{ {class} = {{ {own} = {{ @escaped = 4 operative = authored_operative }} }} escaped = @escaped").as_bytes());
        assert!(one.unsettled.is_none(), "{class}: {one:?}");
        assert_eq!(scalar(&one.items, "escaped"), b"4", "{class}");
        assert_eq!(
            scalar(body(body(&one.items, "body"), class), own),
            b"{",
            "{class}"
        );
    }
    let refused = read(b"body = { free_operative = { operative = authored_operative } }");
    assert!(
        refused
            .misreads
            .iter()
            .any(|note| note.what.starts_with("Deferred effect `free_operative`")),
        "{refused:?}"
    );
}
#[test]
fn value_create_operative_uses_raw_lists_and_gender_skip() {
    let result = read(b"@alias = authored_token body = { create_operative_leader = { nationalities = { @lost = 2 @alias } traits = { @also_lost = 5 @alias } gender = { @gender_lost = 7 } name = authored_name female = yes skill = 1 portrait_path = authored_path picture = authored_picture gfx = authored_gfx bypass_recruitment = yes available_to_spy_master = yes portrait_tag_override = authored_country invalid = { @bad = 9 } @seen = 3 name = @seen } } lost = @lost also_lost = @also_lost gender_lost = @gender_lost bad = @bad seen = @seen");
    assert!(result.unsettled.is_none(), "{result:?}");
    for key in ["lost", "also_lost", "gender_lost", "bad"] {
        assert_eq!(
            scalar(&result.items, key),
            format!("@{key}").as_bytes(),
            "{key}: {result:?}"
        );
    }
    assert_eq!(scalar(&result.items, "seen"), b"3");
    let operative = body(body(&result.items, "body"), "create_operative_leader");
    for key in ["nationalities", "traits"] {
        let list = body(operative, key);
        assert_eq!(list.len(), 4, "{key}: {result:?}");
        assert!(list.iter().all(|item| item.key.is_none()));
        assert_eq!(list[3].value.scalar(), Some(b"authored_token".as_slice()));
    }
    assert!(
        !result
            .misreads
            .iter()
            .any(|note| note.what.starts_with("Deferred effect")),
        "{result:?}"
    );
}
#[test]
fn value_create_operative_traits_loop_even_after_scalar() {
    let result = read(b"body = { create_operative_leader = { traits = authored_trait @lost = 2 name = authored_name } } lost = @lost");
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(scalar(body(&result.items, "body"), "lost"), b"@lost");
    let list = body(
        body(body(&result.items, "body"), "create_operative_leader"),
        "traits",
    );
    assert_eq!(list.len(), 6, "{result:?}");
    assert!(list.iter().all(|item| item.key.is_none()));
    assert!(
        !result
            .misreads
            .iter()
            .any(|note| note.what.starts_with("Deferred effect")),
        "{result:?}"
    );
}
#[test]
fn value_promote_character_uses_inline_keys() {
    let result = read(b"body = { promote_character = authored_character promote_character = { ideology = authored_ideology character = authored_character invalid = { @lost = 2 } @seen = 3 ideology = @seen } } lost = @lost seen = @seen");
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(scalar(&result.items, "lost"), b"@lost");
    assert_eq!(scalar(&result.items, "seen"), b"3");
    assert_eq!(
        body(&result.items, "body")[0].value.scalar(),
        Some(b"authored_character".as_slice())
    );
    assert!(
        !result
            .misreads
            .iter()
            .any(|note| note.what.starts_with("Deferred effect")),
        "{result:?}"
    );
}
#[test]
fn value_remove_role_uses_scalar_or_character_inline_keys() {
    let result = read(b"body = { remove_unit_leader_role = authored_character remove_unit_leader_role = { character = authored_character invalid = { @lost = 2 } @seen = 3 character = @seen } } lost = @lost seen = @seen");
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(scalar(&result.items, "lost"), b"@lost");
    assert_eq!(scalar(&result.items, "seen"), b"3");
    assert_eq!(
        body(&result.items, "body")[0].value.scalar(),
        Some(b"authored_character".as_slice())
    );
    assert!(
        !result
            .misreads
            .iter()
            .any(|note| note.what.starts_with("Deferred effect")),
        "{result:?}"
    );
}

#[test]
fn template_value_reader_starts_its_persistent_loop_after_a_scalar() {
    let result = read(b"body={division_template=none name=T priority=2} add_stability=0} final=9");
    assert_complete_template_read(&result);
    let effects = body(&result.items, "body");
    let template = body(effects, "division_template");
    assert_eq!(scalar(template, "name"), b"T");
    assert_eq!(scalar(template, "priority"), b"2");
    assert_eq!(scalar(effects, "add_stability"), b"0");
    assert_eq!(scalar(&result.items, "final"), b"9");
}
#[test]
fn template_cid_unknown_field_leaves_its_block_as_cid_fields() {
    let result = read(
        br#"
        body={division_template={parent={
            not_a_cid_field={@seen=3 type=1}
            name=T
        } add_stability=0} seen=@seen
    "#,
    );
    assert_complete_template_read(&result);
    let effects = body(&result.items, "body");
    let template = body(effects, "division_template");
    assert_eq!(scalar(template, "name"), b"T");
    assert_eq!(scalar(body(template, "parent"), "type"), b"1");
    assert_eq!(scalar(&result.items, "seen"), b"3");
    assert_eq!(scalar(effects, "add_stability"), b"0");
}
#[test]
fn template_equipment_lists_use_nested_reference_and_cid_grammars() {
    let result = read(
        br#"
        body={division_template={name=T not_allowed_equipment={
            equipment={type=infantry version=1 creator=AAA create_if_missing=yes
                version_name=V license_creator=AAA unknown={@hidden=9}}
            equipment_variant_index={type=1 id=2}
            mystery={@seen=3 equipment={type=armor version=2}}
            priority=4
        } add_stability=0} seen=@seen hidden=@hidden
    "#,
    );
    assert_complete_template_read(&result);
    let effects = body(&result.items, "body");
    let template = body(effects, "division_template");
    let list = body(template, "not_allowed_equipment");
    let reference = body(list, "equipment");
    assert_eq!(scalar(reference, "version"), b"1");
    assert_eq!(scalar(reference, "version_name"), b"V");
    assert_eq!(scalar(body(list, "equipment_variant_index"), "id"), b"2");
    assert_eq!(scalar(list, "mystery"), b"{");
    assert_eq!(scalar(template, "priority"), b"4");
    assert_eq!(scalar(&result.items, "seen"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    assert_eq!(scalar(effects, "add_stability"), b"0");
}
#[test]
fn template_forbidden_equipment_unknown_field_reads_one_value_only() {
    let result = read(
        br#"
        body={division_template={forbidden_equipment_types={
            mystery={@seen=3 token=fixture_equipment}
            name=T
        } add_stability=0} seen=@seen
    "#,
    );
    assert_complete_template_read(&result);
    let effects = body(&result.items, "body");
    let template = body(effects, "division_template");
    let list = body(template, "forbidden_equipment_types");
    assert_eq!(scalar(list, "mystery"), b"{");
    assert_eq!(scalar(list, "token"), b"fixture_equipment");
    assert_eq!(scalar(template, "name"), b"T");
    assert_eq!(scalar(&result.items, "seen"), b"3");
    assert_eq!(scalar(effects, "add_stability"), b"0");
}

fn read_scoped_effect(text: &[u8], scope: Option<u32>, names: &KeyNames) -> Read {
    initialize_tables();
    native::read(
        text,
        native::Mode::KeepMark,
        native::Top::File(()),
        Some(names),
        &mut |(), field| {
            if field.keyword_is("body") {
                scope.map_or(Shape::Effect, Shape::EffectIn)
            } else {
                Shape::One
            }
        },
    )
}

#[test]
fn character_scope_value_readers_skip_different_blocks_from_country_scope() {
    for scope in [ScopeKind::Country.bit(), ScopeKind::Character.bit()] {
        let result = read_scoped_effect(
            b"body={promote_character={character={@promoted=7} ideology=fixture}} spare=0 promoted=@promoted",
            Some(scope), &KeyNames::default(),
        );
        assert_complete_template_read(&result);
        assert_eq!(
            scalar(&result.items, "promoted"),
            if scope == ScopeKind::Character.bit() {
                b"@promoted".as_slice()
            } else {
                b"7"
            }
        );
        let result = read_scoped_effect(
            b"body={remove_unit_leader_role={character=fixture @removed=3 character=fixture}} removed=@removed",
            Some(scope), &KeyNames::default(),
        );
        assert_complete_template_read(&result);
        assert_eq!(
            scalar(&result.items, "removed"),
            if scope == ScopeKind::Character.bit() {
                b"@removed".as_slice()
            } else {
                b"3"
            }
        );
    }
}

#[test]
fn character_role_scalar_yes_is_case_sensitive_and_rejects_current_blocks() {
    let result = read_scoped_effect(
        b"body={add_stability=YES remove_unit_leader_role {@lost=7}} lost=@lost",
        Some(ScopeKind::Character.bit()),
        &KeyNames::default(),
    );
    assert_complete_template_read(&result);
    assert_eq!(scalar(&result.items, "lost"), b"@lost");
    assert!(
        result
            .misreads
            .iter()
            .any(|note| note.what.contains("requires exact yes"))
    );
    let result = read_scoped_effect(
        b"body={add_stability=yes remove_unit_leader_role {@seen=7 add_stability=0} seen=@seen",
        Some(ScopeKind::Character.bit()),
        &KeyNames::default(),
    );
    assert_complete_template_read(&result);
    assert_eq!(scalar(&result.items, "seen"), b"7");
}

#[test]
fn unknown_effect_scope_has_named_counted_refusals_for_sensitive_blocks() {
    let result = read_scoped_effect(
        b"body={remove_unit_leader_role={character=fixture}}",
        None,
        &KeyNames::default(),
    );
    assert!(result.misreads.iter().any(|note| {
        note.what
            .starts_with("Deferred effect `remove_unit_leader_role`")
            && note.count == Some(1)
    }));
    let result = read_scoped_effect(
        b"body={promote_character={ideology=fixture character {@lost=7}}}",
        None,
        &KeyNames::default(),
    );
    assert!(result.unsettled.is_some());
    assert!(result.misreads.iter().any(|note| {
        note.what
            .starts_with("Deferred effect `promote_character.character`")
            && note.count == Some(1)
    }));
}

#[test]
fn target_scope_zero_replaces_character_scope_and_restores_the_parent() {
    for target in [
        "ROOT",
        "THIS",
        "PREV",
        "FROM",
        "event_target:fixture",
        "var:fixture",
    ] {
        let text = format!(
            "body={{{target}={{remove_unit_leader_role={{character=fixture @seen=3 character=fixture}}}} promote_character={{character={{@lost=7}} ideology=fixture}}}} seen=@seen lost=@lost"
        );
        let result = read_scoped_effect(
            text.as_bytes(),
            Some(ScopeKind::Character.bit()),
            &KeyNames::default(),
        );
        assert_complete_template_read(&result);
        assert_eq!(scalar(&result.items, "seen"), b"3", "{target}: {result:?}");
        assert_eq!(
            scalar(&result.items, "lost"),
            b"@lost",
            "{target}: {result:?}"
        );
    }
}

#[test]
fn iterator_scope_force_restores_character_parent_scope() {
    let result = read_scoped_effect(
        b"body={every_country={remove_unit_leader_role={character=fixture @seen=3 character=fixture}} promote_character={character={@lost=7} ideology=fixture}} seen=@seen lost=@lost",
        Some(ScopeKind::Character.bit()), &KeyNames::default(),
    );
    assert_complete_template_read(&result);
    assert_eq!(scalar(&result.items, "seen"), b"3");
    assert_eq!(scalar(&result.items, "lost"), b"@lost");
}

#[test]
fn content_targets_and_registered_effect_precedence_select_effect_scope() {
    let names = KeyNames {
        countries: [b"XYZ".to_vec()].into(),
        characters: [b"fixture_character".to_vec(), b"hidden_effect".to_vec()].into(),
        ..Default::default()
    };
    let result = read_scoped_effect(
        b"body={fixture_character={remove_unit_leader_role={@lost=7 character=fixture}} XYZ={remove_unit_leader_role={@country=3 character=fixture}} hidden_effect={remove_unit_leader_role={@registered=5 character=fixture}}} lost=@lost country=@country registered=@registered",
        Some(4), &names,
    );
    assert_complete_template_read(&result);
    assert_eq!(scalar(&result.items, "lost"), b"@lost");
    assert_eq!(scalar(&result.items, "country"), b"3");
    assert_eq!(scalar(&result.items, "registered"), b"5");
}

#[test]
fn project_iteration_empty_assigned_key_keeps_one_value() {
    let result = read(b"body={complete_special_project={iteration_output={\"\"={@seen=3 reward=fixture} state=1}} seen=@seen");
    assert_complete_template_read(&result);
    assert_eq!(scalar(&result.items, "seen"), b"@seen");
    let iterations = body(
        body(body(&result.items, "body"), "complete_special_project"),
        "iteration_output",
    );
    assert_eq!(iterations[0].key.as_deref(), Some(b"".as_slice()));
    assert_eq!(iterations[0].value.scalar(), Some(b"{".as_slice()));
    assert!(
        iterations
            .iter()
            .any(|item| item.key.as_deref() == Some(b"@seen"))
    );
}
