use crate::{
    effect_keys::tests::{block, read, scalar},
    key_names::KeyNames,
};

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
fn hash_keys_read_the_current_lexer_token() {
    crate::test_game::enter();
    for (effect, key) in [
        ("add_trait", "slot"),
        ("remove_trait", "slot"),
        ("add_power_balance_value", "id"),
        ("add_power_balance_value", "tooltip_side"),
    ] {
        let text = format!("@identifier = 7 body = {{ {effect} = {{ {key} = @identifier }} }}");
        let result = read(text.as_bytes(), &KeyNames::default());
        assert_eq!(
            scalar(block(block(&result.items, "body"), effect), key),
            b"@identifier"
        );
        assert!(result.unsettled.is_none(), "{result:?}");
        no_deferred(&result);
    }
}

#[test]
fn legacy_leader_traits_are_values_and_expire_is_scalar() {
    crate::test_game::enter();
    let result = read(
        b"body = { create_country_leader = { traits = { @lost = 8 native_trait } expire = 1939.1.1 hidden_effect = { @kept = 3 add_stability = 0 } } } lost = @lost kept = @kept",
        &KeyNames::default(),
    );
    assert_eq!(scalar(&result.items, "lost"), b"@lost");
    assert_eq!(scalar(&result.items, "kept"), b"3");
    let leader = block(block(&result.items, "body"), "create_country_leader");
    let traits = block(leader, "traits");
    assert!(traits.iter().all(|item| item.key.is_none()));
    assert_eq!(scalar(leader, "expire"), b"1939.1.1");
    assert!(result.unsettled.is_none(), "{result:?}");
    no_deferred(&result);
}

#[test]
fn country_leader_role_uses_its_own_nested_reader() {
    crate::test_game::enter();
    let result = read(
        b"@ideology = 7 body = { add_country_leader_role = { country_leader = { ideology = @ideology traits = { @lost = 8 native_trait } invalid = { @hidden = 9 } @kept = 3 desc = @kept } } } kept = @kept lost = @lost hidden = @hidden",
        &KeyNames::default(),
    );
    assert_eq!(scalar(&result.items, "kept"), b"3");
    assert_eq!(scalar(&result.items, "lost"), b"@lost");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    let role = block(
        block(block(&result.items, "body"), "add_country_leader_role"),
        "country_leader",
    );
    assert_eq!(scalar(role, "ideology"), b"@ideology");
    assert!(result.unsettled.is_none(), "{result:?}");
    no_deferred(&result);
}

#[test]
fn equipment_production_embeds_the_variant_reference() {
    crate::test_game::enter();
    let result = read(
        b"body = { add_equipment_production = { equipment = { type = native_type invalid = { @lost = 8 } @kept = 3 version_name = @kept } } } kept = @kept lost = @lost",
        &KeyNames::default(),
    );
    assert_eq!(scalar(&result.items, "kept"), b"3");
    assert_eq!(scalar(&result.items, "lost"), b"@lost");
    assert!(result.unsettled.is_none(), "{result:?}");
    no_deferred(&result);
}

#[test]
fn resource_types_lookup_the_recorded_roster_before_skipping() {
    crate::test_game::enter();
    let result = read(
        b"body = { add_resource = { type = { @lost = 8 } @kept = 3 amount = @kept } } lost = @lost kept = @kept",
        &KeyNames::default(),
    );
    assert_eq!(scalar(&result.items, "kept"), b"3");
    assert_eq!(scalar(&result.items, "lost"), b"@lost");
    assert!(result.unsettled.is_none(), "{result:?}");
    no_deferred(&result);

    let mut names = KeyNames::default();
    names.counts.resources.insert(b"native_resource".to_vec());
    let result = read(
        b"body = { add_resource = { type = \"NATIVE_RESOURCE\" type { @kept = 4 amount = @kept } final = @kept",
        &names,
    );
    assert_eq!(scalar(block(&result.items, "body"), "final"), b"4");
    assert_eq!(
        scalar(block(block(&result.items, "body"), "add_resource"), "type"),
        b"NATIVE_RESOURCE"
    );
    no_deferred(&result);
}

#[test]
fn random_list_members_read_weight_modifiers_then_effects() {
    crate::test_game::enter();
    let result = read(
        b"body = { random_list = { seed = 1 log = yes 1 = { modifier = { factor = 1 invalid = { @lost = 8 } @kept = 3 always = yes } hidden_effect = { @nested = 5 add_stability = 0 } invalid_effect = { @hidden = 9 } } } } kept = @kept nested = @nested lost = @lost hidden = @hidden",
        &KeyNames::default(),
    );
    assert_eq!(scalar(&result.items, "kept"), b"3");
    assert_eq!(scalar(&result.items, "nested"), b"5");
    assert_eq!(scalar(&result.items, "lost"), b"@lost");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    assert!(result.unsettled.is_none(), "{result:?}");
    no_deferred(&result);
}

#[test]
fn building_tags_are_lists_and_railway_priorities_are_mtth() {
    crate::test_game::enter();
    let result = read(
        b"body = { damage_building = { tags = { @lost = 8 native_tag } damage = 1 } build_railway = { path = { @path = 9 1 } controller_priority = { modifier = { invalid = { @hidden = 10 } @kept = 3 factor = 1 always = yes } } } } kept = @kept lost = @lost path = @path hidden = @hidden",
        &KeyNames::default(),
    );
    assert_eq!(scalar(&result.items, "kept"), b"3");
    for key in ["lost", "path", "hidden"] {
        assert_eq!(scalar(&result.items, key), format!("@{key}").as_bytes());
    }
    assert!(result.unsettled.is_none(), "{result:?}");
    no_deferred(&result);
}

#[test]
fn meta_text_reads_raw_characters_without_defining_tokens() {
    crate::test_game::enter();
    let result = read(
        b"body = { meta_effect = { text = { @lost = 8 } @kept = 3 debug = yes } } lost = @lost kept = @kept",
        &KeyNames::default(),
    );
    assert_eq!(scalar(&result.items, "kept"), b"3");
    assert_eq!(scalar(&result.items, "lost"), b"@lost");
    assert!(result.unsettled.is_none(), "{result:?}");
    no_deferred(&result);
}

#[test]
fn ai_strategy_helpers_follow_type_and_read_lists_as_values() {
    crate::test_game::enter();
    let result = read(
        b"body = { add_ai_strategy = { type = put_unit_buffers states = { @lost = 8 1 } ratio = 1 order_id = 1 subtract_fronts_from_need = yes subtract_invasions_from_need = no invalid = { @hidden = 9 } @kept = 3 value = @kept } } kept = @kept lost = @lost hidden = @hidden",
        &KeyNames::default(),
    );
    let strategy = block(block(&result.items, "body"), "add_ai_strategy");
    assert!(
        block(strategy, "states")
            .iter()
            .all(|item| item.key.is_none())
    );
    assert_eq!(scalar(&result.items, "kept"), b"3");
    assert_eq!(scalar(&result.items, "lost"), b"@lost");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    assert!(result.unsettled.is_none(), "{result:?}");
    no_deferred(&result);

    let result = read(
        b"body = { add_ai_strategy = { type = scorched_earth_prio states = { @lost = 8 1 } @kept = 3 value = @kept } } kept = @kept lost = @lost",
        &KeyNames::default(),
    );
    assert_eq!(scalar(&result.items, "kept"), b"3");
    assert_eq!(scalar(&result.items, "lost"), b"@lost");
    assert!(result.unsettled.is_none(), "{result:?}");
    no_deferred(&result);
}

#[test]
fn ai_strategy_fuel_stockpile_helper_reads_its_thresholds() {
    crate::test_game::enter();
    let result = read(
        b"body = { add_ai_strategy = { type = stockpile_fuel stop_above = 2 start_below = 1 invalid = { @hidden = 9 } @kept = 3 value = @kept } } kept = @kept hidden = @hidden",
        &KeyNames::default(),
    );
    let strategy = block(block(&result.items, "body"), "add_ai_strategy");
    assert_eq!(scalar(strategy, "stop_above"), b"2");
    assert_eq!(scalar(strategy, "start_below"), b"1");
    assert_eq!(scalar(&result.items, "kept"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    assert!(result.unsettled.is_none(), "{result:?}");
    no_deferred(&result);
}

#[test]
fn ai_strategy_trigger_helpers_accept_repeated_children() {
    crate::test_game::enter();

    let result = read(
        b"body = { add_ai_strategy = { type = front_control execution_type = balanced ordertype = front country_trigger = { always = yes always = no invalid = { @lost = 8 } @kept = 3 always = yes } state_trigger = { @nested = 5 always = yes } } } kept = @kept nested = @nested lost = @lost",
        &KeyNames::default(),
    );
    assert_eq!(scalar(&result.items, "kept"), b"3");
    assert_eq!(scalar(&result.items, "nested"), b"5");
    assert_eq!(scalar(&result.items, "lost"), b"@lost");
    assert!(result.unsettled.is_none(), "{result:?}");
    no_deferred(&result);
}

#[test]
fn ai_strategy_invalid_operative_mission_id_skips_the_block() {
    crate::test_game::enter();
    let result = read(
        b"body = { add_ai_strategy = { type = operative_mission id = { @lost = 8 } @kept = 3 priority = @kept mission = build_intel_network state = 1 num_operatives = 1 } } kept = @kept lost = @lost",
        &KeyNames::default(),
    );
    assert_eq!(scalar(&result.items, "kept"), b"3");
    assert_eq!(scalar(&result.items, "lost"), b"@lost");
    assert!(result.unsettled.is_none(), "{result:?}");
    no_deferred(&result);
}

#[test]
fn ai_strategy_errors_skip_current_openings_with_stale_values() {
    crate::test_game::enter();
    for (strategy_type, previous, rejected) in [
        ("front_control", "execution_type", "execution_type"),
        ("operative_mission", "priority", "id"),
    ] {
        let text = format!(
            "body = {{ add_ai_strategy = {{ type = {strategy_type} {previous} = invalid {rejected} {{ @lost = 8 }} @kept = 3 value = @kept }} }} kept = @kept lost = @lost"
        );
        let result = read(text.as_bytes(), &KeyNames::default());
        assert_eq!(scalar(&result.items, "kept"), b"3");
        assert_eq!(scalar(&result.items, "lost"), b"@lost");
        assert!(result.unsettled.is_none(), "{result:?}");
        no_deferred(&result);
    }
}

#[test]
fn ai_strategy_numeric_type_preserves_the_allocated_helper() {
    crate::test_game::enter();
    for changes in ["type = 75", "type = conquer type = 75", "type = not_a_type"] {
        let text = format!(
            "body = {{ add_ai_strategy = {{ type = put_unit_buffers {changes} states = {{ @lost = 8 1 }} @kept = 3 value = @kept }} }} kept = @kept lost = @lost"
        );
        let result = read(text.as_bytes(), &KeyNames::default());
        let strategy = block(block(&result.items, "body"), "add_ai_strategy");
        assert!(
            block(strategy, "states")
                .iter()
                .all(|item| item.key.is_none())
        );
        assert_eq!(scalar(&result.items, "kept"), b"3");
        assert_eq!(scalar(&result.items, "lost"), b"@lost");
        assert!(result.unsettled.is_none(), "{result:?}");
        no_deferred(&result);
    }
}

#[test]
fn ai_strategy_names_unknown_global_helper_context() {
    crate::test_game::enter();
    let result = read(
        b"body = { add_ai_strategy = { type = 70 states = { 1 } } }",
        &KeyNames::default(),
    );
    assert!(
        result
            .unsettled
            .as_ref()
            .is_some_and(|note| note.what.contains("AI strategy ") && note.what.contains("global")),
        "{result:?}"
    );
    no_deferred(&result);
}

#[test]
fn ai_strategy_scalar_lookups_consume_without_catalogues() {
    crate::test_game::enter();
    let mut names = KeyNames::default();
    names.countries.insert(b"ZZZ".to_vec());
    for (key, valid) in [
        ("area", "\"NATIVE_AREA\""),
        ("tag", "\"ZZZ\""),
        ("state", "7suffix"),
        ("strategic_region", "\"9\""),
    ] {
        let text = format!(
            "body = {{ add_ai_strategy = {{ type = front_unit_request {key} = {valid} @kept = 4 final = @kept"
        );
        let result = read(text.as_bytes(), &names);
        assert_eq!(
            scalar(
                block(block(&result.items, "body"), "add_ai_strategy"),
                "final"
            ),
            b"4"
        );
        assert!(result.unsettled.is_none(), "{result:?}");
        no_deferred(&result);
    }
}

#[test]
fn ai_strategy_block_catalogue_lookup_is_a_counted_refusal() {
    crate::test_game::enter();
    let result = read(
        b"body = { add_ai_strategy = { type = front_unit_request area = { name = missing } } }",
        &KeyNames::default(),
    );
    assert!(
        result
            .unsettled
            .as_ref()
            .is_some_and(|note| note.what.contains("AI area block")),
        "{result:?}"
    );
    assert!(
        result.misreads.iter().any(|note| note.count == Some(1)
            && note.what.contains("Deferred effect AI strategy AI area")),
        "{result:?}"
    );
}
