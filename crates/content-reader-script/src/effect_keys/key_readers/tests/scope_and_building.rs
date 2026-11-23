use crate::{effect_keys::tests::{block, read, scalar}, key_names::KeyNames, native::Read};

fn no_deferred(result: &Read) {
    assert!(
        result
            .misreads
            .iter()
            .all(|note| !note.what.starts_with("Deferred effect ")),
        "{result:?}"
    );
}

#[test]
fn targeted_resistance_reads_own_fields_and_effect_children() {
    crate::test_game::enter();
    for effect in ["force_disable_resistance", "force_enable_resistance"] {
        let result = read(
            format!(
                "body={{ {effect}={{clear=yes occupied=AAA occupier=BBB add_political_power=2}} }}"
            )
            .as_bytes(),
            &KeyNames::default(),
        );
        no_deferred(&result);
        let fields = block(block(&result.items, "body"), effect);
        assert_eq!(scalar(fields, "clear"), b"yes");
        assert_eq!(scalar(fields, "occupied"), b"AAA");
        assert_eq!(scalar(fields, "occupier"), b"BBB");
        assert_eq!(scalar(fields, "add_political_power"), b"2");
    }
}

#[test]
fn resistance_target_reads_amount_and_country_operands_before_children() {
    crate::test_game::enter();
    let result = read(
        b"body={add_resistance_target={id=3 tooltip=SYN_TIP amount=var:syn days=7 occupied=AAA occupier=BBB add_political_power=2}}",
        &KeyNames::default(),
    );
    no_deferred(&result);
    let fields = block(block(&result.items, "body"), "add_resistance_target");
    for (name, expected) in [
        ("id", &b"3"[..]),
        ("tooltip", &b"SYN_TIP"[..]),
        ("amount", &b"var:syn"[..]),
        ("days", &b"7"[..]),
        ("occupied", &b"AAA"[..]),
        ("occupier", &b"BBB"[..]),
        ("add_political_power", &b"2"[..]),
    ] {
        assert_eq!(scalar(fields, name), expected, "{name}");
    }
}

#[test]
fn reached_effect_readers_remove_the_named_deferrals() {
    crate::test_game::enter();
    for effect in [
        "for_loop_effect",
        "add_scientist_xp",
        "add_autonomy_ratio",
        "while_loop_effect",
        "resize_array",
        "resize_temp_array",
        "random",
        "add_advisor_role",
        "remove_power_balance_modifier",
        "add_power_balance_modifier",
        "create_dynamic_country",
        "set_political_party",
        "set_power_balance",
        "add_ace",
        "launch_nuke",
        "teleport_armies",
        "set_division_template_lock",
        "set_truce",
        "remove_advisor_role",
        "start_border_war",
        "transfer_navy",
        "set_power_balance_gfx",
        "remove_power_balance",
        "steal_random_tech_bonus",
        "add_intel",
        "modify_building_resources",
        "delete_unit",
        "every_collection_element",
        "add_timed_unit_leader_trait",
        "set_division_template_cap",
        "construct_building_in_random_province",
        "damage_units",
    ] {
        assert!(crate::effect_keys::registered(effect.as_bytes()).is_some(), "{effect}");
        let text = format!("body = {{ {effect} = {{ }} }}");
        no_deferred(&read(text.as_bytes(), &KeyNames::default()));
    }
}

#[test]
fn for_compare_rejects_an_opening_value() {
    crate::test_game::enter();
    let result = read(
        b"body = { for_loop_effect = { compare = { @hidden = 9 marker = 1 } @seen = 3 add_stability = 0 } } final = @seen hidden = @hidden",
        &KeyNames::default(),
    );
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    no_deferred(&result);
}

#[test]
fn while_limit_skips_only_after_a_trigger_child() {
    crate::test_game::enter();
    let result = read(
        b"body = { while_loop_effect = { limit = { invalid = { @hidden = 9 } } limit = { @seen = 3 always = yes } limit = { @duplicate = 4 always = yes } add_stability = 0 } } final = @seen hidden = @hidden duplicate = @duplicate",
        &KeyNames::default(),
    );
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    assert_eq!(scalar(&result.items, "duplicate"), b"@duplicate");
    no_deferred(&result);
}

#[test]
fn hash_readers_keep_the_current_lexer_text() {
    crate::test_game::enter();
    let result = read(
        b"@given = 3 body = { set_power_balance_gfx = { id = @given side = @given gfx = @given } }",
        &KeyNames::default(),
    );
    let fields = block(block(&result.items, "body"), "set_power_balance_gfx");
    assert_eq!(scalar(fields, "id"), b"@given");
    assert_eq!(scalar(fields, "side"), b"@given");
    assert_eq!(scalar(fields, "gfx"), b"3");
    no_deferred(&result);
}

#[test]
fn repeated_teleport_and_damage_limits_are_both_read() {
    crate::test_game::enter();
    for effect in ["teleport_armies", "damage_units"] {
        let text = format!(
            "body = {{ {effect} = {{ limit = {{ always = yes }} limit = {{ @seen = 3 always = yes }} invalid = {{ @hidden = 9 marker = 1 }} }} }} final = @seen hidden = @hidden"
        );
        let result = read(text.as_bytes(), &KeyNames::default());
        assert_eq!(scalar(&result.items, "final"), b"3");
        assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
        no_deferred(&result);
    }
}

#[test]
fn advisor_embeds_the_existing_role_dispatch() {
    crate::test_game::enter();
    let result = read(
        b"body = { add_advisor_role = { advisor = { allowed = { invalid_trigger = { @hidden = 9 } always = yes } invalid = { @unknown = 8 marker = 1 } traits = { @trait = 7 } @seen = 3 slot = political_advisor } } } final = @seen hidden = @hidden unknown = @unknown trait = @trait",
        &KeyNames::default(),
    );
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    assert_eq!(scalar(&result.items, "unknown"), b"@unknown");
    assert_eq!(scalar(&result.items, "trait"), b"@trait");
    no_deferred(&result);
}

#[test]
fn border_side_ignores_unknown_keys_without_skipping() {
    crate::test_game::enter();
    let result = read(
        b"body = { start_border_war = { attacker = { unknown = { @seen = 3 state = 1 } marker = 1 } } final = @seen",
        &KeyNames::default(),
    );
    let fields = block(
        block(block(&result.items, "body"), "start_border_war"),
        "attacker",
    );
    assert_eq!(scalar(fields, "unknown"), b"{");
    assert_eq!(scalar(&result.items, "final"), b"3");
    no_deferred(&result);
}

#[test]
fn border_side_leader_score_reads_mtth_triggers() {
    crate::test_game::enter();
    let result = read(
        b"body = { start_border_war = { attacker = { leader_score = { modifier = { invalid = { @hidden = 9 } @seen = 3 factor = 2 } } } } } final = @seen hidden = @hidden",
        &KeyNames::default(),
    );
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    no_deferred(&result);
}

#[test]
fn modify_building_resources_rejects_unresolved_value_tokens() {
    crate::test_game::enter();
    let mut names = KeyNames::default();
    names.counts.buildings.insert(b"native_building".to_vec());
    let result = read(
        b"body = { modify_building_resources = { building = native_building building = { @hidden = 9 marker = 1 } @seen = 3 amount = 1 } } final = @seen hidden = @hidden",
        &names,
    );
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    no_deferred(&result);
}

#[test]
fn collection_operators_define_only_inside_limit() {
    crate::test_game::enter();
    let result = read(
        b"body = { every_collection_element = { input = { input = scope:root operators = { country_and_all_subjects @lost = 2 limit = { invalid = { @hidden = 3 } @kept = 4 always = yes } } } } } lost = @lost hidden = @hidden kept = @kept",
        &KeyNames::default(),
    );
    assert_eq!(scalar(&result.items, "lost"), b"@lost");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    assert_eq!(scalar(&result.items, "kept"), b"4");
    no_deferred(&result);
}

#[test]
fn dynamic_building_key_consumes_one_value() {
    crate::test_game::enter();
    let result = read(
        b"body = { construct_building_in_random_province = { dynamic_building = { @seen = 3 value = 1 } final = @seen",
        &KeyNames::default(),
    );
    let fields = block(
        block(&result.items, "body"),
        "construct_building_in_random_province",
    );
    assert_eq!(scalar(fields, "dynamic_building"), b"{");
    assert_eq!(scalar(block(&result.items, "body"), "final"), b"3");
    no_deferred(&result);
}
