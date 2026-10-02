use crate::{
    effect_keys::tests::{block, read, scalar},
    key_names::KeyNames,
};

#[test]
fn effect_key_only_skips_rejected_blocks() {
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
            .contains("bad_variant_access")
    );
}

#[test]
fn effect_key_divide_temp_if_zero_is_one_scoped_value() {
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
    let result = read(b"body = { add_building_construction = { province = { unknown = { @hidden = 9 } level = { @bad = 4 } limit_to_victory_point = { @bad_vp = 5 } @seen = 3 id = @seen } } } final = @seen hidden = @hidden bad = @bad vp = @bad_vp", &KeyNames::default());
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    assert_eq!(scalar(&result.items, "bad"), b"@bad");
    assert_eq!(scalar(&result.items, "vp"), b"@bad_vp");
    assert!(result.misreads.iter().all(|note| note.count.is_none()));
}

#[test]
fn effect_key_conditionals_reject_else_and_repeated_nonempty_limits() {
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
fn effect_key_table_is_sorted_and_only_selects_readscript_classes() {
    use super::*;
    assert!(KEY_READERS.windows(2).all(|pair| pair[0].0 < pair[1].0));
    for &(token, _) in KEY_READERS {
        if matches!(token, 0x36D3 | 0x38ED) {
            continue;
        }
        assert_eq!(crate::effect_keys::class(token).unwrap().0, READ_SCRIPT);
    }
}
