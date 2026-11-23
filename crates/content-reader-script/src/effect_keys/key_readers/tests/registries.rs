use crate::{
    effect_keys::tests::{block, read, scalar},
    key_names::KeyNames,
};

#[test]
fn effect_equipment_bonus_uses_native_fixed_stat_registry() {
    crate::test_game::enter();
    let result = read(
        b"body = { add_equipment_bonus = { bonus = { arbitrary_equipment = { not_a_stat = { @hidden = 9 } soft_attack = { @seen = 3 instant = yes } } } } final = @seen hidden = @hidden",
        &KeyNames::default(),
    );
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    assert!(result.misreads.iter().all(|note| note.count.is_none()));
}

#[test]
fn effect_modifier_uses_recorded_registry_for_quoted_names() {
    crate::test_game::enter();
    let mut names = KeyNames::default();
    names
        .effects
        .modifiers
        .insert(b"mod_authored_modifier".to_vec());
    let result = read(
        b"body = { add_state_modifier = { modifier = { absent_modifier = { @hidden = 9 } \"MOD_AUTHORED_MODIFIER\" = { @seen = 3 name = marker } } } final = @seen hidden = @hidden",
        &names,
    );
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    assert!(result.misreads.iter().all(|note| note.count.is_none()));
}

#[test]
fn effect_modifier_builtin_needs_no_recorded_stack_name() {
    crate::test_game::enter();
    let result = read(
        b"body = { add_state_modifier = { modifier = { \"ARMY_ATTACK_FACTOR\" = { @seen = 3 name = marker } } } final = @seen",
        &KeyNames::default(),
    );
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert!(result.misreads.iter().all(|note| note.count.is_none()));
}

#[test]
fn effect_modifier_unknown_registry_suffix_is_named() {
    crate::test_game::enter();
    let mut names = KeyNames::default();
    names.effects.modifiers_gap =
        Some("common/modifier_definitions/mod.txt:3 unread suffix".into());
    let result = read(
        b"body = { add_state_modifier = { modifier = { unknown_modifier = { @hidden = 9 } } } }",
        &names,
    );
    let note = result
        .unsettled
        .expect("modifier lookup needs the unread suffix");
    assert!(note.what.contains("unknown_modifier"), "{note:?}");
    assert!(note.what.contains("mod.txt:3 unread suffix"), "{note:?}");
}

#[test]
fn effect_popularities_rejects_invalid_keys_and_accepts_exact_var_prefix() {
    crate::test_game::enter();
    let result = read(
        b"body = { set_popularities = { invalid_ideology = { @hidden = 9 } VAR:x = { @bad_case = 7 } var: = { @seen = 3 root = 10 } } final = @seen hidden = @hidden bad_case = @bad_case",
        &KeyNames::default(),
    );
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    assert_eq!(scalar(&result.items, "bad_case"), b"@bad_case");
    let body = block(&result.items, "body");
    assert_eq!(scalar(block(body, "set_popularities"), "var:"), b"{");
    assert!(result.misreads.iter().all(|note| note.count.is_none()));
}

#[test]
fn effect_popularities_resolves_country_and_alias_before_rejecting() {
    crate::test_game::enter();
    let names = KeyNames {
        countries: [b"ABC".to_vec()].into(),
        aliases: [b"def".to_vec()].into(),
        ..Default::default()
    };
    for key in ["ABC", "DeF", "RoOt", "PrEv", "ThIs", "FrOm"] {
        let text = format!(
            "body = {{ set_popularities = {{ abc = {{ @hidden = 9 }} {key} = {{ @seen = 3 var:x = 10 }} final = @seen hidden = @hidden"
        );
        let result = read(text.as_bytes(), &names);
        assert!(result.unsettled.is_none(), "{key}: {result:?}");
        let body = block(&result.items, "body");
        assert_eq!(scalar(body, "final"), b"3", "{key}");
        assert_eq!(scalar(body, "hidden"), b"@hidden", "{key}");
        assert!(result.misreads.iter().all(|note| note.count.is_none()));
    }
}
