use super::super::{tests::Fixture, *};
use crate::scripted_text::{EventObject, EventScope};

#[test]
fn native_empty_dispatch_and_core_refusals() {
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
    for (body, class) in [
        (
            "create_dynamic_country={original_tag=AAA add_political_power=2}",
            "CCreateDynamicCountry",
        ),
        (
            "force_disable_resistance={clear=yes add_political_power=2}",
            "CForceDisableResistanceEffect",
        ),
    ] {
        let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
        let out = fixture.render(body, &mut scopes, Content::default());
        assert!(out.text.is_empty());
        assert_eq!(
            out.refusals.values().sum::<usize>(),
            1,
            "{body}: {:?}",
            out.refusals
        );
        assert!(
            out.refusals.keys().any(|why| why.contains(class)
                && why.contains("140532780")
                && why.contains("140533D80")),
            "{:?}",
            out.refusals
        );
    }
    let mut names = installed_content::key_names::KeyNames::default();
    names.scripted_effects.insert(b"synthetic_child".to_vec());
    for (body, reason) in [
        (
            "create_dynamic_country={synthetic_child=yes}",
            Some("140532780"),
        ),
        ("create_dynamic_country={ignored_field=yes else={}}", None),
    ] {
        let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
        let out = fixture.render(
            body,
            &mut scopes,
            Content {
                key_names: Some(&names),
                ..Default::default()
            },
        );
        assert!(out.text.is_empty());
        assert_eq!(
            out.refusals.values().sum::<usize>(),
            usize::from(reason.is_some()),
            "{body}: {:?}",
            out.refusals
        );
        if let Some(reason) = reason {
            assert!(out.refusals.keys().any(|why| why.contains(reason)));
        }
    }
    let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
    let out = fixture.render(
        "create_dynamic_country={unclassified=yes}",
        &mut scopes,
        Content::default(),
    );
    assert_eq!(out.refusals.values().sum::<usize>(), 1);
    assert!(
        out.refusals
            .keys()
            .any(|why| why.contains("content names unavailable"))
    );
}
