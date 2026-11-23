use super::super::{tests::Fixture, *};
use crate::scripted_text::{EventObject, EventScope};

#[test]
fn empty_dispatch() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = Fixture::new();
    for body in [
        "add_ai_strategy={}",
        "division_template={}",
        "clr_unit_leader_flag={}",
        "clr_character_flag={}",
        "clear_global_event_target=synthetic",
        "clr_global_flag={}",
        "clear_rule={}",
        "create_dynamic_country={ original_tag=AAA copy_tag=BBB }",
        "create_entity={}",
        "damage_units={}",
        "delete_unit={}",
        "destroy_entity=synthetic",
        "drop_cosmetic_tag=synthetic",
        "execute_operation_coordinated_strike={}",
        "force_disable_resistance={ clear=yes occupied=AAA occupier=BBB }",
        "force_update_dynamic_modifier=synthetic",
        "force_update_map_mode={}",
        "for_each_loop={ add_political_power=2 }",
        "for_loop_effect={ add_political_power=2 }",
        "generate_character={}",
        "mark_focus_tree_layout_dirty=synthetic",
        "mark_technology_tree_layout_dirty=synthetic",
        "modify_unit_leader_flag={}",
        "modify_character_flag={}",
        "modify_country_flag={}",
        "modify_global_flag={}",
        "modify_state_flag={}",
        "print_variables={}",
        "reserve_dynamic_country=synthetic",
        "set_unit_leader_flag={}",
        "set_character_flag={}",
        "set_country_leader_ideology=synthetic",
        "set_country_leader_name={}",
        "set_country_leader_portrait={}",
        "set_country_leader_description={}",
        "set_entity_animation={}",
        "set_entity_position={}",
        "set_global_flag={}",
        "set_leader_name=synthetic",
        "set_leader_portrait=synthetic",
        "set_leader_description=synthetic",
        "set_character_name={}",
        "set_occupation_law=synthetic",
        "set_occupation_law_where_available=synthetic",
        "set_political_party={}",
        "set_portraits={}",
        "set_power_balance_gfx={}",
        "set_project_flag={}",
        "set_state_flag={}",
        "set_state_province_controller={}",
        "teleport_armies={}",
        "teleport_railway_guns_to_deploy_province=synthetic",
        "transfer_ship={}",
        "while_loop_effect={ add_political_power=2 }",
        "career_profile_step_missiolini=synthetic",
        "clr_mio_flag={}",
        "set_mio_flag={}",
    ] {
        let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
        let out = fixture.render(body, &mut scopes, Content::default());
        assert!(out.text.is_empty(), "{body}: {}", out.text);
        assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals);
    }
}

#[test]
fn empty_containers_describe_and_probe_children_once() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = Fixture::new();
    let names = content_reader_script::key_names::KeyNames::default();
    let content = Content {
        key_names: Some(&names),
        ..Default::default()
    };
    for (effect_key, own) in [
        ("create_dynamic_country", "original_tag=AAA copy_tag=BBB"),
        (
            "force_disable_resistance",
            "clear=yes occupied=AAA occupier=BBB",
        ),
    ] {
        let body = format!("{effect_key}={{{own} add_political_power=2 add_political_power=3}}");
        let items = content_reader_script::effect_keys::read_body(body.as_bytes(), &names).unwrap();
        let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
        let out = fixture.render_native(&body, &mut scopes, content);
        assert_eq!(out.text, "PP2\nPP3\n", "{effect_key}: {:?}", out.refusals);
        assert!(out.refusals.is_empty(), "{effect_key}: {:?}", out.refusals);
        fixture.with_renderer(&mut scopes, content, |renderer| {
            let selected =
                crate::triggers::event_scopes::project(&renderer.graph.borrow(), 0).unwrap();
            assert!(renderer.multiple(&items, selected), "{effect_key}");
            assert!(renderer.out.refusals.is_empty());
        });
    }
}

#[test]
fn empty_containers_keep_child_refusals_once() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = Fixture::new();
    for effect_key in ["create_dynamic_country", "force_disable_resistance"] {
        let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
        let out = fixture.render(
            &format!("{effect_key}={{add_political_power=2 every_purchase_contract={{}}}}"),
            &mut scopes,
            Content::default(),
        );
        assert_eq!(out.text, "PP2\n");
        assert_eq!(out.refusals.values().sum::<usize>(), 1);
        assert!(
            out.refusals
                .keys()
                .all(|why| { why.contains("every_purchase_contract") })
        );
    }
}
