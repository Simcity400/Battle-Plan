use crate::{
    effect_keys::tests::{block, read, scalar},
    key_names::KeyNames,
};

#[test]
fn scalar_forms_read_one_and_keep_native_fallbacks() {
    crate::test_game::enter();
    let rows = [
        ("add_faction_goal_slot", "value"),
        ("add_relation_rule_override", "target"),
        ("delete_units", "division_template"),
        ("add_daily_mastery", "amount"),
        ("destroy_ships", "count"),
        ("add_design_template_bonus", "cost_factor"),
        ("set_can_be_fired_in_advisor_role", "character"),
        ("create_railway_gun", "location"),
        ("find_highest_in_array", "array"),
        ("find_lowest_in_array", "array"),
        ("cancel_border_war", "attacker"),
        ("modify_tech_sharing_bonus", "bonus"),
        ("add_decryption", "amount"),
        ("add_breakthrough_points", "value"),
        ("add_collaboration", "target"),
        ("change_division_template", "division_template"),
        ("clear_division_template_cap", "division_template"),
        ("add_scientist_level", "level"),
        ("become_exiled_in", "legitimacy"),
        ("add_history_entry", "key"),
        ("set_border_war_data", "attacker"),
        ("create_entity", "province"),
        ("finalize_border_war", "defender"),
    ];
    for (effect, own_key) in rows {
        let text =
            format!("body = {{ {effect} = {{ {own_key} = {{ @seen = 3 marker = 1 }} final = @seen");
        let result = read(text.as_bytes(), &KeyNames::default());
        assert_eq!(
            scalar(block(&result.items, "body"), "final"),
            b"3",
            "{effect}: {result:?}"
        );
        assert_eq!(
            scalar(block(block(&result.items, "body"), effect), own_key),
            b"{",
            "{effect}: {result:?}"
        );
        assert!(
            result.misreads.iter().all(|note| note.count.is_none()),
            "{effect}: {result:?}"
        );
    }
}

#[test]
fn no_skip_unknown_forms_consume_only_the_opening() {
    crate::test_game::enter();
    for effect in [
        "add_relation_rule_override",
        "strategic_province_location",
        "change_division_template",
        "add_history_entry",
        "cancel_border_war",
        "set_border_war_data",
        "finalize_border_war",
    ] {
        let text = format!(
            "body = {{ {effect} = {{ arbitrary_name = {{ @seen = 3 marker = 1 }} final = @seen"
        );
        let result = read(text.as_bytes(), &KeyNames::default());
        assert_eq!(
            scalar(block(&result.items, "body"), "final"),
            b"3",
            "{effect}: {result:?}"
        );
        assert_eq!(
            scalar(
                block(block(&result.items, "body"), effect),
                "arbitrary_name"
            ),
            b"{",
            "{effect}: {result:?}"
        );
        assert!(
            result.misreads.iter().all(|note| note.count.is_none()),
            "{effect}: {result:?}"
        );
    }
}

#[test]
fn effect_key_force_mapmode_limits_are_unconditional_trigger_reads() {
    crate::test_game::enter();
    let result = read(b"body = { force_update_map_mode = { limit = { always = yes } limit = { @seen = 3 always = yes } mapmode = custom } } final = @seen", &KeyNames::default());
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert!(
        result.misreads.iter().all(|note| note.count.is_none()),
        "{result:?}"
    );
}

#[test]
fn effect_key_breakthrough_specialization_rejects_an_opening_value() {
    crate::test_game::enter();
    let result = read(b"body = { add_breakthrough_points = { specialization = { @hidden = 9 } @seen = 3 value = @seen } } final = @seen hidden = @hidden", &KeyNames::default());
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    assert!(
        result.misreads.iter().all(|note| note.count.is_none()),
        "{result:?}"
    );
}

#[test]
fn specialization_stale_block_needs_a_registry_and_counts_a_refusal() {
    crate::test_game::enter();
    let result = read(
        b"body = { add_breakthrough_points = { value = custom specialization { target = unknown } } }",
        &KeyNames::default(),
    );
    assert!(
        result
            .unsettled
            .as_ref()
            .is_some_and(|note| note.what.contains("specialization block")),
        "{result:?}"
    );
    assert!(
        result
            .misreads
            .iter()
            .any(|note| note.count == Some(1)
                && note.what.contains("Deferred effect specialization")),
        "{result:?}"
    );
}

#[test]
fn effect_key_static_province_uses_its_own_province_keys() {
    crate::test_game::enter();
    let result = read(b"body = { remove_province_modifier = { province = { level = { @hidden = 9 } first = { @hidden_first = 8 } limit_to_supply_node = { @bad_supply = 6 } limit_to_victory_point = { @bad_vp = 7 } @seen = 3 id = @seen } static_modifiers = { arbitrary_modifier } } } final = @seen hidden = @hidden first = @hidden_first supply = @bad_supply vp = @bad_vp", &KeyNames::default());
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    assert_eq!(scalar(&result.items, "first"), b"@hidden_first");
    assert_eq!(scalar(&result.items, "supply"), b"@bad_supply");
    assert_eq!(scalar(&result.items, "vp"), b"@bad_vp");
    assert!(
        result.misreads.iter().all(|note| note.count.is_none()),
        "{result:?}"
    );
}

#[test]
fn effect_key_static_province_accepts_all_provinces_as_one_value() {
    crate::test_game::enter();
    let result = read(b"body = { remove_province_modifier = { province = { all_provinces = { @seen = 3 marker = 1 } } } final = @seen", &KeyNames::default());
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert!(
        result.misreads.iter().all(|note| note.count.is_none()),
        "{result:?}"
    );
}

#[test]
fn effect_key_static_modifiers_read_the_list_after_a_scalar_value() {
    crate::test_game::enter();
    let result = read(b"body = { remove_province_modifier = { static_modifiers = arbitrary_modifier @hidden = 9 } @seen = 3 marker = 1 } final = @seen hidden = @hidden", &KeyNames::default());

    let body = block(&result.items, "body");
    assert_eq!(scalar(body, "final"), b"3");
    assert_eq!(scalar(body, "hidden"), b"@hidden");
    assert!(
        result.misreads.iter().all(|note| note.count.is_none()),
        "{result:?}"
    );
}

