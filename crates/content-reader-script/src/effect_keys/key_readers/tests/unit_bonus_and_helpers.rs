use crate::{
    effect_keys::tests::{block, read, scalar},
    key_names::KeyNames,
};

fn sub_units() -> crate::sub_units::SubUnits {
    let mut units = crate::sub_units::SubUnits::default();
    units.read(
        b"sub_units = { synthetic_unit = { } } sub_unit_categories = { synthetic_category }",
        "common/units/synthetic.txt",
    );
    units
}

fn no_deferred(result: &crate::native::Read) {
    assert!(
        result
            .misreads
            .iter()
            .all(|note| !note.what.starts_with("Deferred effect")),
        "{result:?}"
    );
}

#[test]
fn own_keys_only_rejects_unknown_blocks() {
    crate::test_game::enter();
    for effect in [
        "add_days_remove",
        "add_mines",
        "set_faction_member_upgrade_min",
        "create_import",
        "set_entity_animation",
        "set_entity_position",
    ] {
        let text = format!(
            "body = {{ {effect} = {{ unknown = {{ @hidden = 9 marker = 1 }} @seen = 3 marker = 1 }} }} final = @seen hidden = @hidden"
        );
        let result = read(text.as_bytes(), &KeyNames::default());
        assert_eq!(scalar(&result.items, "final"), b"3");
        assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
        no_deferred(&result);
    }
}

#[test]
fn no_skip_fallback_keeps_opening_value_scalar() {
    crate::test_game::enter();
    for effect in [
        "set_division_force_allow_recruiting",
        "execute_operation_coordinated_strike",
        "remove_relation_rule_override",
        "add_unit_medal_to_latest_entry",
        "set_party_rule",
    ] {
        let text =
            format!("body = {{ {effect} = {{ arbitrary = {{ @seen = 3 marker = 1 }} final = @seen");
        let result = read(text.as_bytes(), &KeyNames::default());
        assert_eq!(scalar(block(&result.items, "body"), "final"), b"3");
        assert_eq!(
            scalar(block(block(&result.items, "body"), effect), "arbitrary"),
            b"{"
        );
        no_deferred(&result);
    }
}

#[test]
fn own_keys_then_effect_dispatches_nested_effects() {
    crate::test_game::enter();
    for effect in ["free_random_operative", "add_random_valid_trait_from_unit"] {
        let text = format!(
            "body = {{ {effect} = {{ invalid = {{ @hidden = 9 marker = 1 }} hidden_effect = {{ @seen = 3 add_stability = 0 }} }} }} final = @seen hidden = @hidden"
        );
        let result = read(text.as_bytes(), &KeyNames::default());
        assert_eq!(scalar(&result.items, "final"), b"3");
        assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
        no_deferred(&result);
    }
}

#[test]
fn state_province_limits_are_unconditional_trigger_bodies() {
    crate::test_game::enter();
    let result = read(
        b"body = { set_state_province_controller = { limit = { always = yes } limit = { @seen = 3 always = yes invalid = { @hidden = 9 marker = 1 } } } } final = @seen hidden = @hidden",
        &KeyNames::default(),
    );
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    no_deferred(&result);
}

#[test]
fn random_mio_limits_reject_repeat_after_a_child() {
    crate::test_game::enter();
    let result = read(
        b"body = { random_military_industrial_organization = { tooltip = { arg = { @seen = 3 marker = 1 } } limit = { always = yes } limit = { @hidden = 9 marker = 1 } } } final = @seen hidden = @hidden",
        &KeyNames::default(),
    );
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    no_deferred(&result);
}

#[test]
fn print_variables_string_list_does_not_define_entries() {
    crate::test_game::enter();
    let result = read(
        b"body = { print_variables = { var_list = { @hidden = 9 } @seen = 3 marker = 1 } } final = @seen hidden = @hidden",
        &KeyNames::default(),
    );
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    no_deferred(&result);
}

#[test]
fn scientist_template_uses_trigger_raw_list_and_scalar_skill_readers() {
    crate::test_game::enter();
    let result = read(
        b"body = { add_scientist_role = { scientist = { traits = { @list = 9 } visible = { invalid = { @hidden = 7 marker = 1 } always = yes } skills = { @seen = 3 arbitrary_specialization = @seen } invalid = { @bad = 8 marker = 1 } } } } final = @seen list = @list hidden = @hidden bad = @bad",
        &KeyNames::default(),
    );
    assert_eq!(scalar(&result.items, "final"), b"3");
    for key in ["list", "hidden", "bad"] {
        assert_eq!(scalar(&result.items, key), format!("@{key}").as_bytes());
    }
    no_deferred(&result);
}

#[test]
fn generated_scientist_gender_errors_skip_blocks() {
    crate::test_game::enter();
    let result = read(
        b"body = { generate_scientist_character = { gender = { @hidden = 9 marker = 1 } traits = { @list = 7 } skills = { @seen = 3 arbitrary_specialization = @seen } } } final = @seen hidden = @hidden list = @list",
        &KeyNames::default(),
    );
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    assert_eq!(scalar(&result.items, "list"), b"@list");
    no_deferred(&result);
}

#[test]
fn purchase_equipment_own_keys_rejects_unknown_blocks() {
    crate::test_game::enter();
    let result = read(
        b"body = { create_purchase_contract = { equipment = { type = var:equipment amount = 1 producer = var:producer variant_name = variant invalid = { @hidden = 9 marker = 1 } @seen = 3 amount = @seen } } } final = @seen hidden = @hidden",
        &KeyNames::default(),
    );
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    no_deferred(&result);
}

#[test]
fn party_rule_override_keeps_definitions_and_reads_scalar_entry_as_an_object() {
    crate::test_game::enter();
    let result = read(
        b"body = { set_party_rule = { override = { @inside = 4 0 = @inside } @seen = 3 desc = @seen } } final = @seen inside = @inside",
        &KeyNames::default(),
    );
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "inside"), b"4");
    no_deferred(&result);
    let result = read(
        b"body = { set_party_rule = { override = scalar @inside = 4 0 = @inside } @seen = 3 desc = @seen } } final = @seen inside = @inside",
        &KeyNames::default(),
    );
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "inside"), b"4");
    let overrides = block(
        block(block(&result.items, "body"), "set_party_rule"),
        "override",
    );
    assert_eq!(
        overrides
            .iter()
            .find(|item| item.key.as_deref() == Some(b"0".as_slice()))
            .unwrap()
            .value
            .scalar(),
        Some(b"4".as_slice())
    );
    no_deferred(&result);
}

#[test]
fn unit_bonus_uses_recorded_unit_category_terrain_and_equipment_names() {
    crate::test_game::enter();
    let mut names = KeyNames {
        sub_units: Some(sub_units()),
        ..Default::default()
    };
    names.effects.terrains.insert(b"synthetic_terrain".to_vec());
    names
        .effects
        .equipment
        .insert(b"synthetic_equipment".to_vec());
    for unit in ["synthetic_unit", "synthetic_category"] {
        let text = format!(
            "body = {{ add_unit_bonus = {{ name = test_bonus {unit} = {{ invalid_stat = {{ @bad_stat = 9 marker = 1 }} soft_attack = 1 synthetic_terrain = {{ invalid = {{ @bad_terrain = 8 marker = 1 }} @terrain_seen = 4 attack = @terrain_seen }} need = {{ missing_equipment = {{ @bad_equipment = 7 marker = 1 }} @equipment_seen = 5 synthetic_equipment = @equipment_seen }} battalion_mult = {{ category = synthetic_category @support_seen = 6 soft_attack = @support_seen }} @seen = 3 soft_attack = @seen }} missing_unit = {{ @bad_unit = 2 marker = 1 }} }} }} final = @seen terrain = @terrain_seen equipment = @equipment_seen support = @support_seen bad_stat = @bad_stat bad_terrain = @bad_terrain bad_equipment = @bad_equipment bad_unit = @bad_unit"
        );
        let result = read(text.as_bytes(), &names);
        assert!(result.unsettled.is_none(), "{result:?}");
        for (key, value) in [
            ("final", b"3"),
            ("terrain", b"4"),
            ("equipment", b"5"),
            ("support", b"6"),
        ] {
            assert_eq!(scalar(&result.items, key), value);
        }
        for key in ["bad_stat", "bad_terrain", "bad_equipment", "bad_unit"] {
            assert_eq!(scalar(&result.items, key), format!("@{key}").as_bytes());
        }
        no_deferred(&result);
    }
}

#[test]
fn unit_bonus_does_not_match_quoted_content_keys() {
    crate::test_game::enter();
    let names = KeyNames {
        sub_units: Some(sub_units()),
        ..Default::default()
    };
    let result = read(
        b"body = { add_unit_bonus = { \"synthetic_unit\" = { @hidden = 9 soft_attack = 1 } @seen = 3 name = bonus } } final = @seen hidden = @hidden",
        &names,
    );
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    no_deferred(&result);
}

#[test]
fn unit_bonus_transport_reads_the_current_token_like_technology_units() {
    crate::test_game::enter();
    let names = KeyNames {
        sub_units: Some(sub_units()),
        ..Default::default()
    };
    let result = read(
        b"body = { add_unit_bonus = { synthetic_unit = { soft_attack = 2 transport motorized } } }",
        &names,
    );
    let stats = block(
        block(block(&result.items, "body"), "add_unit_bonus"),
        "synthetic_unit",
    );
    assert_eq!(scalar(stats, "transport"), b"motorized");
    no_deferred(&result);
}

#[test]
fn unit_bonus_transport_block_is_a_counted_refusal() {
    crate::test_game::enter();
    let names = KeyNames {
        sub_units: Some(sub_units()),
        ..Default::default()
    };
    let result = read(
        b"body = { add_unit_bonus = { synthetic_unit = { transport = { type = motorized } } } }",
        &names,
    );
    assert!(
        result
            .unsettled
            .as_ref()
            .is_some_and(|note| note.what.contains("transport block")),
        "{result:?}"
    );
    assert!(
        result.misreads.iter().any(|note| note.count == Some(1)
            && note.what.contains("Deferred effect unit stats transport")),
        "{result:?}"
    );
}
