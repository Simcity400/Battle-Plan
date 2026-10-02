use crate::{
    key_names::KeyNames,
    parser::{
        Item,
        native::{self, Read, Shape},
    },
};

fn read(text: &[u8]) -> Read {
    native::read(
        text,
        native::Mode::KeepMark,
        native::Top::File(()),
        Some(&KeyNames::default()),
        &mut |(), field| {
            if field.keyword_is("body") {
                Shape::Effect
            } else {
                Shape::One
            }
        },
    )
}

fn scalar<'a>(items: &'a [Item], key: &str) -> &'a [u8] {
    items
        .iter()
        .find(|item| item.keyword_is(key))
        .unwrap_or_else(|| panic!("missing {key}: {items:?}"))
        .value
        .scalar()
        .unwrap()
}

fn body<'a>(items: &'a [Item], key: &str) -> &'a [Item] {
    items
        .iter()
        .find(|item| item.keyword_is(key))
        .unwrap()
        .value
        .items()
        .unwrap()
}

// 141384F60 / 1413859E0 versus 141385790: legacy events refuse recipient,
// set_from and set_root; the operative event accepts their scalar readers.
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

// 1403A9810 resolves raw tokens without calling the definition reader and
// without writing its retained value slot. Scalar uses the lexer token.
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

// 141384F50 -> CBoundLocalization::vf3 142390E40 / 1423912A0.
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

// 141384F80 / 141385260, with their 141385A10 / 141385E80 key loops.
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

// 1403AC0D0 has an inline key loop and scalar RecordError.
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
            .any(|note| note.what.contains("0x1403AC0D0")),
        "{scalar:?}"
    );
}

// 1403AB2C0 / 1403AB590 / 1403ACCD0 / 1403ACF40: scalar or inline loop.
#[test]
fn scalar_or_inline_value_readers_use_native_fields() {
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

// The five other reached variable builders always call ReadScript, even
// for quoted scalar input (their preceding assert consumes no tokens).
#[test]
fn variable_builder_value_readers_use_native_key_loops() {
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

// 1403A9B40 is a thunk to ReadScript, with a distinct key reader per class.
#[test]
fn thunked_value_reader_uses_own_keys_and_popularity_effect_fallback() {
    for (class, key) in [
        ("add_popularity", "popularity"),
        ("release_autonomy", "freedom_level"),
        ("set_autonomy", "freedom_level"),
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

// 141385770 delegates only its block to each class's key reader.
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

// 14047E6C0 / 14047EB10 -> 140481020 -> 140480E00: any key is an operand.
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

// 14047EF60 / 14047F3E0 -> 140481030: named scalar operands, others error.
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

// 1403A89E0 -> 1403ADE70: the distribution is mutable and case-sensitive.
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
