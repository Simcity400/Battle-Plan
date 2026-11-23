use crate::{
    effect_keys::tests::{block, read, scalar},
    key_names::KeyNames,
};

fn native(read: &crate::native::Read) {
    assert!(read.unsettled.is_none(), "{read:?}");
    assert!(
        read.misreads
            .iter()
            .all(|note| !note.what.starts_with("Deferred effect")),
        "{read:?}"
    );
}

#[test]
fn own_keys_then_skip_or_dispatch_are_native() {
    crate::test_game::enter();
    for effect in [
        "set_capital",
        "add_days_mission_timeout",
        "remove_opinion_modifier",
        "transfer_ship",
        "set_country_leader_portrait",
        "set_country_leader_description",
        "set_country_leader_name",
        "add_victory_points",
        "set_victory_points",
        "add_autonomy_score",
        "add_operation_token",
        "remove_operation_token",
    ] {
        let text = format!(
            "body = {{ {effect} = {{ invalid = {{ @hidden = 9 }} @seen = 3 marker = 1 }} }} final = @seen hidden = @hidden"
        );
        let result = read(text.as_bytes(), &KeyNames::default());
        assert_eq!(scalar(&result.items, "final"), b"3");
        assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
        native(&result);
    }

    for effect in [
        "for_each_loop",
        "add_named_threat",
        "uncomplete_national_focus",
        "remove_country_leader_role",
        "add_mastery",
        "add_mastery_bonus",
    ] {
        let text = format!(
            "body = {{ {effect} = {{ invalid = {{ @hidden = 9 }} ABC = {{ @seen = 3 add_stability = 0 }} }} }} final = @seen hidden = @hidden"
        );
        let mut names = KeyNames::default();
        names.countries.insert(b"ABC".to_vec());
        let result = read(text.as_bytes(), &names);
        assert_eq!(scalar(&result.items, "final"), b"3");
        assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
        native(&result);
    }
}

#[test]
fn raw_and_value_lists_define_nothing() {
    crate::test_game::enter();
    for (effect, field) in [
        ("give_resource_rights", "resources"),
        ("add_equipment_subsidy", "seller_tags"),
        ("reduce_focus_completion_cost", "focus"),
        ("remove_building", "tags"),
        ("add_province_modifier", "static_modifiers"),
    ] {
        let text =
            format!("body = {{ {effect} = {{ {field} = {{ @hidden = 9 }} }} }} final = @hidden");
        let result = read(text.as_bytes(), &KeyNames::default());
        assert_eq!(scalar(&result.items, "final"), b"@hidden");
        native(&result);
    }
}

#[test]
fn civil_war_filters_read_every_trigger_body() {
    crate::test_game::enter();
    for filter in [
        "states_filter",
        "keep_unit_leaders_trigger",
        "keep_scientists_trigger",
    ] {
        let text = format!(
            "body = {{ start_civil_war = {{ {filter} = {{ invalid = {{ @hidden = 9 }} always = yes }} {filter} = {{ @seen = 3 always = yes }} }} }} final = @seen hidden = @hidden"
        );
        let result = read(text.as_bytes(), &KeyNames::default());
        assert_eq!(scalar(&result.items, "final"), b"3");
        assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
        native(&result);
    }
    let result = read(b"body = { start_civil_war = { states = all keep_unit_leaders = { @hidden = 9 } size = 0.5 } } final = @hidden", &KeyNames::default());
    assert_eq!(scalar(&result.items, "final"), b"@hidden");
    native(&result);
}

#[test]
fn unit_leader_data_delegates_and_rank_skips() {
    crate::test_game::enter();
    for effect in [
        "add_corps_commander_role",
        "add_field_marshal_role",
        "add_naval_commander_role",
    ] {
        let text = format!(
            "body = {{ {effect} = {{ rank = {{ @rank_hidden = 9 }} visible = {{ invalid = {{ @trigger_hidden = 8 }} always = yes }} traits = {{ @list_hidden = 7 }} @seen = 3 skill = 1 }} }} final = @seen rank = @rank_hidden trigger = @trigger_hidden list = @list_hidden"
        );
        let result = read(text.as_bytes(), &KeyNames::default());
        assert_eq!(scalar(&result.items, "final"), b"3");
        for key in ["rank", "trigger", "list"] {
            assert!(scalar(&result.items, key).starts_with(b"@"));
        }
        native(&result);
    }
}

#[test]
fn portrait_roles_read_only_native_sizes() {
    crate::test_game::enter();
    let result = read(b"body = { set_portraits = { army = { invalid = { @hidden = 9 } small = portrait @seen = 3 large = portrait } unknown_role = { @role_hidden = 8 } } } final = @seen hidden = @hidden role = @role_hidden", &KeyNames::default());
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    assert_eq!(scalar(&result.items, "role"), b"@role_hidden");
    native(&result);
}

#[test]
fn production_equipment_reads_an_unconditional_object() {
    crate::test_game::enter();
    let result = read(b"body = { create_production_license = { equipment = scalar invalid = { @hidden = 9 } @seen = 3 type = model } } final = @seen hidden = @hidden", &KeyNames::default());

    let body = block(&result.items, "body");
    assert_eq!(scalar(body, "final"), b"3");
    assert_eq!(scalar(body, "hidden"), b"@hidden");
    let equipment = block(block(body, "create_production_license"), "equipment");
    assert_eq!(scalar(equipment, "type"), b"model");
    native(&result);
}

#[test]
fn province_targets_skip_invalid_comparisons() {
    crate::test_game::enter();
    let result = read(b"body = { add_province_modifier = { province = { limit_to_victory_point = { @hidden = 9 } @seen = 3 limit_to_coastal = yes } } } final = @seen hidden = @hidden", &KeyNames::default());
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    native(&result);

    let result = read(b"body = { add_province_modifier = { province = { limit_to_coastal = yes limit_to_victory_point { @seen = 3 limit_to_coastal = yes } days = 2 } } final = @seen", &KeyNames::default());
    assert_eq!(scalar(&result.items, "final"), b"3");
    native(&result);
}

#[test]
fn province_id_accepts_opening_as_scoped_variable_text() {
    crate::test_game::enter();
    let result = read(b"body = { add_province_modifier = { province = { id = { @seen = 3 limit_to_coastal = yes } days = 2 } } final = @seen", &KeyNames::default());
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(
        scalar(
            block(
                block(block(&result.items, "body"), "add_province_modifier"),
                "province"
            ),
            "id"
        ),
        b"{"
    );
    native(&result);
}

#[test]
fn war_selector_chains_report_against_current_token() {
    crate::test_game::enter();
    let result = read(b"body = { add_to_war = { enemy = invalid.root enemy { @hidden = 9 } @seen = 3 single_target_only = yes } } final = @seen hidden = @hidden", &KeyNames::default());
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    native(&result);

    let result = read(
        b"body = { add_to_war = { enemy = { @seen = 3 single_target_only = yes } final = @seen",
        &KeyNames::default(),
    );
    assert_eq!(scalar(block(&result.items, "body"), "final"), b"3");
    assert_eq!(
        scalar(block(block(&result.items, "body"), "add_to_war"), "enemy"),
        b"{"
    );
    native(&result);
}

#[test]
fn raid_damage_unknown_keys_read_one_without_skip() {
    crate::test_game::enter();
    let result = read(
        b"body = { raid_damage_units = { arbitrary = { @seen = 3 ratio = yes } final = @seen",
        &KeyNames::default(),
    );
    assert_eq!(scalar(block(&result.items, "body"), "final"), b"3");
    assert_eq!(
        scalar(
            block(block(&result.items, "body"), "raid_damage_units"),
            "arbitrary"
        ),
        b"{"
    );
    native(&result);
}

#[test]
fn mastery_track_filter_reads_scalar_braces() {
    crate::test_game::enter();
    for effect in ["add_mastery", "add_mastery_bonus"] {
        let text =
            format!("body = {{ {effect} = {{ track = {{ @seen = 3 amount = 1 }} final = @seen");
        let result = read(text.as_bytes(), &KeyNames::default());
        assert_eq!(scalar(block(&result.items, "body"), "final"), b"3");
        assert_eq!(
            scalar(block(block(&result.items, "body"), effect), "track"),
            b"{"
        );
        native(&result);
    }
}

#[test]
fn state_modifier_embeds_the_modifier_reader() {
    crate::test_game::enter();
    let result = read(b"body = { add_state_modifier = { modifier = { custom_modifier_tooltip = label hidden_modifier = scalar added_modifier = { @seen = 3 name = label } } invalid = { @hidden = 9 } } } final = @seen hidden = @hidden", &KeyNames::default());
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    native(&result);
}

#[test]
fn equipment_bonus_entries_embed_the_specific_reader() {
    crate::test_game::enter();
    let result = read(b"body = { add_equipment_bonus = { bonus = { equipment = { instant = yes @seen = 3 instant = no } } invalid = { @hidden = 9 } } } final = @seen hidden = @hidden", &KeyNames::default());
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    native(&result);
}

#[test]
fn character_iterators_reject_duplicate_nonempty_limits() {
    crate::test_game::enter();
    for effect in [
        "every_character",
        "every_scientist",
        "random_character",
        "random_scientist",
        "random_active_scientist",
    ] {
        let text = format!(
            "body = {{ {effect} = {{ limit = {{ always = yes }} limit = {{ @hidden = 9 }} @seen = 3 visible_when_empty = yes }} }} final = @seen hidden = @hidden"
        );
        let result = read(text.as_bytes(), &KeyNames::default());
        assert_eq!(scalar(&result.items, "final"), b"3");
        assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
        native(&result);
    }
}

#[test]
fn generate_character_registers_before_reading_fields() {
    crate::test_game::enter();
    let result = read(b"body = { generate_character = { token_base = sample @seen = 3 name = label token_base = sample name = { @hidden = 9 } } } final = @seen hidden = @hidden", &KeyNames::default());
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    native(&result);
}

#[test]
fn rejected_generated_token_base_counts_the_external_claim_refusal() {
    crate::test_game::enter();
    let result = read(
        b"body = { generate_character = { token_base = 1invalid name = { inherited = yes } } }",
        &KeyNames::default(),
    );
    assert!(
        result
            .unsettled
            .as_ref()
            .is_some_and(|note| note.what.contains("token_base 0 needs claims")),
        "{result:?}"
    );
    assert!(
        result.misreads.iter().any(|note| note.count == Some(1)
            && note
                .what
                .contains("Deferred effect generate_character token_base")),
        "{result:?}"
    );
}

#[test]
fn generate_character_has_a_separate_generated_database() {
    crate::test_game::enter();
    let mut names = KeyNames::default();
    names.characters.insert(b"sample".to_vec());
    let result = read(b"body = { generate_character = { token_base = sample @seen = 3 name = label } } final = @seen", &names);
    assert_eq!(scalar(&result.items, "final"), b"3");
    native(&result);
}

#[test]
fn generated_gender_uses_current_token() {
    crate::test_game::enter();
    let result = read(b"body = { generate_character = { token_base = sample gender = male gender { @hidden = 9 } @seen = 3 name = label } } final = @seen hidden = @hidden", &KeyNames::default());
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    native(&result);

    for gender in ["female", "male", "undefined"] {
        let text = format!(
            "body = {{ generate_character = {{ token_base = sample scientist = {{ traits = {{ trait }} }} gender {gender} name label @seen = 3 nationality = ABC }} }} final = @seen"
        );
        let result = read(text.as_bytes(), &KeyNames::default());
        assert_eq!(scalar(&result.items, "final"), b"3");
        let generated = block(block(&result.items, "body"), "generate_character");
        assert_eq!(scalar(generated, "gender"), gender.as_bytes());
        assert_eq!(scalar(generated, "name"), b"{");
        native(&result);
    }

    let result = read(b"body = { generate_character = { token_base = sample scientist = { traits = { trait } } gender random @seen = 3 name = label } } final = @seen", &KeyNames::default());
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(
        scalar(
            block(block(&result.items, "body"), "generate_character"),
            "gender"
        ),
        b"{"
    );
    native(&result);
}

#[test]
fn generated_gender_rejects_quoted_keyword_values() {
    crate::test_game::enter();
    for gender in ["male", "female", "undefined"] {
        let text = format!(
            "body = {{ generate_character = {{ token_base = sample scientist = {{ traits = {{ trait }} }} gender \"{gender}\" @seen = 3 name = label }} }} final = @seen"
        );
        let result = read(text.as_bytes(), &KeyNames::default());
        assert_eq!(scalar(&result.items, "final"), b"3");
        assert_eq!(
            scalar(
                block(block(&result.items, "body"), "generate_character"),
                "gender"
            ),
            b"{"
        );
        native(&result);
    }
}
