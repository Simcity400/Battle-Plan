use crate::{
    effect_keys::tests::{block, read, scalar},
    key_names::KeyNames,
};

#[test]
fn daily_mastery_selectors_are_own_fields() {
    crate::test_game::enter();

    let item = content_reader_parser::parser::parse(b"add_daily_mastery={amount=0.5 name=synthetic days=100 track=mod_track folder=mod_folder grand_doctrine=mod_grand sub_doctrine=mod_sub track_index=2}").unwrap().items.remove(0);
    let fields = crate::effect_keys::description_fields(&item).unwrap();
    assert_eq!(fields.len(), 0);
    for key in [
        b"track".as_slice(),
        b"folder",
        b"grand_doctrine",
        b"sub_doctrine",
        b"track_index",
    ] {
        let token = crate::effect_keys::registered(b"add_daily_mastery").unwrap();
        assert!(crate::effect_keys::is_own_field(token, key));
    }
}

#[test]
fn effect_key_only_skips_rejected_blocks() {
    crate::test_game::enter();
    for effect in [
        "add_opinion_modifier",
        "add_dynamic_modifier",
        "set_party_name",
        "add_equipment_to_stockpile",
    ] {
        let text = format!(
            "body = {{ {effect} = {{ invalid_key = {{ @hidden = 9 }} @seen = 3 marker = 1 }} }} final = @seen hidden = @hidden"
        );
        let result = read(text.as_bytes(), &KeyNames::default());
        assert_eq!(scalar(&result.items, "final"), b"3");
        assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
        assert!(
            result.misreads.iter().all(|note| note.count.is_none()),
            "{result:?}"
        );
    }
}

#[test]
fn effect_key_then_effect_dispatches_the_fallback() {
    crate::test_game::enter();
    for effect in ["add_tech_bonus", "add_timed_idea"] {
        let text = format!(
            "body = {{ {effect} = {{ invalid_key = {{ @hidden = 9 }} hidden_effect = {{ @seen = 3 add_stability = 0 }} }} }} final = @seen hidden = @hidden"
        );
        let result = read(text.as_bytes(), &KeyNames::default());
        assert_eq!(scalar(&result.items, "final"), b"3");
        assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
        assert!(result.misreads.iter().all(|note| note.count.is_none()));
    }
}

#[test]
fn effect_key_dynamic_names_read_one_token() {
    crate::test_game::enter();
    for effect in [
        "set_technology",
        "add_to_array",
        "clamp_temp_variable",
        "clamp_variable",
    ] {
        let text = format!(
            "body = {{ {effect} = {{ arbitrary_name = {{ @seen = 3 marker = 1 }} final = @seen"
        );
        let result = read(text.as_bytes(), &KeyNames::default());
        assert_eq!(scalar(block(&result.items, "body"), "final"), b"3");
        assert_eq!(
            scalar(
                block(block(&result.items, "body"), effect),
                "arbitrary_name"
            ),
            b"{"
        );
        assert!(
            result.misreads.iter().all(|note| note.count.is_none()),
            "{result:?}"
        );
    }
}

#[test]
fn effect_key_variables_select_expressions_and_keep_scalar_var() {
    crate::test_game::enter();
    for effect in [
        "set_variable",
        "set_temp_variable",
        "add_to_temp_variable",
        "subtract_from_temp_variable",
        "multiply_temp_variable",
        "divide_temp_variable",
    ] {
        let text = format!(
            "body = {{ {effect} = {{ var = x value = {{ @seen = 3 value = @seen add = 2 }} }} }} final = @seen"
        );
        let result = read(text.as_bytes(), &KeyNames::default());
        assert!(result.unsettled.is_none(), "{result:?}");
        assert_eq!(scalar(&result.items, "final"), b"3");
        assert!(result.misreads.iter().all(|note| note.count.is_none()));
    }
    let result = read(
        b"body = { set_variable = { x = { value = 2 } value = 3 } }",
        &KeyNames::default(),
    );
    assert!(
        result
            .unsettled
            .unwrap()
            .what
            .contains("scalar value after an expression")
    );
}

#[test]
fn effect_key_divide_temp_if_zero_is_one_scoped_value() {
    crate::test_game::enter();
    let result = read(
        b"body = { divide_temp_variable = { if_zero = { @seen = 3 value = 1 } final = @seen",
        &KeyNames::default(),
    );
    assert_eq!(scalar(block(&result.items, "body"), "final"), b"3");
    assert_eq!(
        scalar(
            block(block(&result.items, "body"), "divide_temp_variable"),
            "if_zero"
        ),
        b"{"
    );
    assert!(result.misreads.iter().all(|note| note.count.is_none()));
}

#[test]
fn effect_key_iterators_read_limits_and_raw_priorities() {
    crate::test_game::enter();
    for effect in ["every_country", "every_state", "random_state"] {
        let text = format!(
            "body = {{ {effect} = {{ tooltip = {{ arg = {{ @tooltip = 2 marker = 1 }} }} limit = {{ invalid_trigger = {{ @hidden = 9 }} always = yes }} limit = {{ @duplicate = 4 }} invalid_effect = {{ @invalid = 5 }} }} }} final = @tooltip hidden = @hidden duplicate = @duplicate invalid = @invalid"
        );
        let result = read(text.as_bytes(), &KeyNames::default());
        assert_eq!(scalar(&result.items, "final"), b"2");
        for key in ["hidden", "duplicate", "invalid"] {
            assert_eq!(scalar(&result.items, key), format!("@{key}").as_bytes());
        }
        assert!(
            result.misreads.iter().all(|note| note.count.is_none()),
            "{result:?}"
        );
    }
    let result = read(
        b"body = { random_state = { prioritize = { @hidden = 9 } } } final = @hidden",
        &KeyNames::default(),
    );
    assert_eq!(scalar(&result.items, "final"), b"@hidden");
    assert!(result.unsettled.is_none(), "{result:?}");
}

#[test]
fn effect_key_building_province_rejects_unknown_and_bad_comparisons() {
    crate::test_game::enter();
    let result = read(b"body = { add_building_construction = { province = { unknown = { @hidden = 9 } level = { @bad = 4 } limit_to_victory_point = { @bad_vp = 5 } @seen = 3 id = @seen } } } final = @seen hidden = @hidden bad = @bad vp = @bad_vp", &KeyNames::default());
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    assert_eq!(scalar(&result.items, "bad"), b"@bad");
    assert_eq!(scalar(&result.items, "vp"), b"@bad_vp");
    assert!(result.misreads.iter().all(|note| note.count.is_none()));
}

#[test]
fn effect_key_conditionals_reject_else_and_repeated_nonempty_limits() {
    crate::test_game::enter();
    let result = read(b"body = { if = { limit = { invalid = { @hidden = 9 } } limit = { @seen = 3 always = yes } limit = { @duplicate = 4 } else = { limit = { @else_limit = 5 } invalid = { @else_bad = 6 } @nested = 7 add_stability = 0 } } } final = @seen hidden = @hidden duplicate = @duplicate else_limit = @else_limit else_bad = @else_bad nested = @nested", &KeyNames::default());
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "nested"), b"7");
    for key in ["hidden", "duplicate", "else_limit", "else_bad"] {
        assert_eq!(scalar(&result.items, key), format!("@{key}").as_bytes());
    }
    assert!(
        result.misreads.iter().all(|note| note.count.is_none()),
        "{result:?}"
    );
}

#[test]
fn set_relation_rule_uses_its_shared_rule_grammar() {
    crate::test_game::enter();
    let result = read(
        b"body={set_relation_rule={target=AAA usage_desc=SYN_HINT trigger=authored_trigger @seen=3 authored_rule=yes}} final=@seen",
        &KeyNames::default(),
    );
    assert!(result.unsettled.is_none(), "{result:?}");
    assert!(result.misreads.is_empty(), "{result:?}");
    assert_eq!(scalar(&result.items, "final"), b"3");
    let rule = block(block(&result.items, "body"), "set_relation_rule");
    assert_eq!(scalar(rule, "authored_rule"), b"yes");
}

#[test]
fn remove_all_power_balance_modifiers_uses_its_shared_balance_grammar() {
    crate::test_game::enter();
    let result = read(
        b"body={remove_all_power_balance_modifiers={id=authored_balance unknown={@hidden=9} @seen=3 id=@seen}} final=@seen hidden=@hidden",
        &KeyNames::default(),
    );
    assert!(result.unsettled.is_none(), "{result:?}");
    assert!(result.misreads.is_empty(), "{result:?}");
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
}

#[test]
fn set_collaboration_uses_its_shared_target_value_grammar() {
    crate::test_game::enter();
    let result = read(
        b"body={set_collaboration={target=AAA value=2 unknown={@hidden=9} @seen=3 value=@seen}} final=@seen hidden=@hidden",
        &KeyNames::default(),
    );
    assert!(result.unsettled.is_none(), "{result:?}");
    assert!(result.misreads.is_empty(), "{result:?}");
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
}

#[test]
fn state_setup_effects_read_their_own_keys_without_deferred_notes() {
    crate::test_game::enter();
    let result = read(b"body = { @province = 2 @level = 3 remove_building = { type = authored_building state = var:state province = @province level = @level tags = authored_tag } add_victory_points = { province = @province value = @level } set_victory_points = { province = 2 value = var:points } strategic_province_location = { authored_location = @province } strategic_state_location = { authored_location = @province } }", &KeyNames::default());
    assert!(result.unsettled.is_none(), "{result:?}");
    assert!(result.misreads.is_empty(), "{result:?}");
    let body = block(&result.items, "body");
    assert_eq!(scalar(block(body, "remove_building"), "level"), b"3");
    assert_eq!(scalar(block(body, "add_victory_points"), "province"), b"2");
    assert_eq!(
        scalar(block(body, "strategic_state_location"), "authored_location"),
        b"2"
    );
}

#[test]
fn building_and_victory_point_effects_skip_unknown_blocks() {
    crate::test_game::enter();
    for effect in [
        "remove_building",
        "add_victory_points",
        "set_victory_points",
    ] {
        let text = format!(
            "body = {{ {effect} = {{ unknown = {{ @hidden = 9 }} @seen = 3 province = @seen }} }} final = @seen hidden = @hidden"
        );
        let result = read(text.as_bytes(), &KeyNames::default());
        assert!(result.unsettled.is_none(), "{result:?}");
        assert_eq!(scalar(&result.items, "final"), b"3");
        assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
        assert!(result.misreads.iter().all(|note| note.count.is_none()));
    }
}

#[test]
fn building_tags_are_raw_values_and_register_no_definitions() {
    crate::test_game::enter();
    let result = read(b"body = { @tag = authored_tag remove_building = { tags = { @tag @hidden = 9 } level = 1 } } hidden = @hidden", &KeyNames::default());
    assert!(result.unsettled.is_none(), "{result:?}");
    assert!(result.misreads.is_empty(), "{result:?}");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    let tags = block(
        block(block(&result.items, "body"), "remove_building"),
        "tags",
    );
    assert_eq!(tags.len(), 4);
    assert!(tags.iter().all(|item| item.key.is_none()));
    assert_eq!(tags[0].value.scalar(), Some(b"authored_tag".as_slice()));
}

#[test]
fn building_scalar_tags_read_the_current_lexer_value() {
    crate::test_game::enter();
    for effect in ["damage_building", "remove_building"] {
        for operator in ["=", ""] {
            let text = format!("body={{ {effect}={{ level=1 tags {operator} authored_tag }} }}");
            let result = read(text.as_bytes(), &KeyNames::default());
            assert!(result.unsettled.is_none(), "{result:?}");
            assert_eq!(
                scalar(block(block(&result.items, "body"), effect), "tags"),
                b"authored_tag",
                "{text}"
            );
        }
    }
}

#[test]
fn setup_effect_scalar_readers_leave_opening_values_to_the_key_loop() {
    crate::test_game::enter();
    for (effect, key) in [
        ("remove_building", "level"),
        ("add_victory_points", "value"),
        ("strategic_province_location", "authored_location"),
        ("strategic_state_location", "authored_location"),
    ] {
        let text = format!(
            "body = {{ {effect} = {{ {key} = {{ @seen = 3 province = 2 }} }} final = @seen"
        );
        let result = read(text.as_bytes(), &KeyNames::default());
        assert!(result.unsettled.is_none(), "{effect}: {result:?}");
        assert_eq!(scalar(&result.items, "final"), b"3", "{effect}");
        assert_eq!(
            scalar(block(block(&result.items, "body"), effect), key),
            b"{"
        );
        assert!(result.misreads.iter().all(|note| note.count.is_none()));
        assert!(!result.misreads.is_empty(), "{effect}: {result:?}");
    }
}

#[test]
fn strategic_state_location_reads_its_inline_loop_after_a_scalar_value() {
    crate::test_game::enter();
    let result = read(b"body = { strategic_state_location = no @seen = 3 authored_location = @seen } } final = @seen", &KeyNames::default());
    assert!(result.unsettled.is_none(), "{result:?}");
    assert!(result.misreads.is_empty(), "{result:?}");
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(
        scalar(
            block(block(&result.items, "body"), "strategic_state_location"),
            "authored_location"
        ),
        b"3"
    );
}

#[test]
fn setup_effect_missing_operators_follow_the_native_stale_value() {
    crate::test_game::enter();
    for effect in [
        "remove_building",
        "add_victory_points",
        "strategic_province_location",
        "strategic_state_location",
    ] {
        let text = format!("body = {{ {effect} {{ @seen = 3 province = @seen }} }} final = @seen");
        let result = read(text.as_bytes(), &KeyNames::default());
        assert!(result.unsettled.is_none(), "{effect}: {result:?}");
        assert_eq!(scalar(&result.items, "final"), b"3", "{effect}");
        assert!(!result.misreads.is_empty(), "{effect}: {result:?}");
        assert!(result.misreads.iter().all(|note| note.count.is_none()));
    }
}

mod character_and_province;
mod registries;
mod registry_and_strategy;
mod root_forms;
mod scalar_and_iterator;
mod scope_and_building;
mod unit_bonus_and_helpers;
