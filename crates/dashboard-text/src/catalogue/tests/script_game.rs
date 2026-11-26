use content_reader_script::{scope::ScopeKind, test_game::{Executable, TriggerShape}, trigger_keys::Reader};
use dashboard_core::test_fixtures::game;
use engine_executable::executable_tables::{self, Tables};
use std::sync::OnceLock;

pub fn enter() {
    executable_tables::enter(Some(tables()));
}

fn tables() -> Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    *TABLES.get_or_init(|| executable().build())
}

pub fn executable() -> Executable {
    use Reader::{Assign, Body, Comparison};
    use ScopeKind::{Character, Combat, Country, IndustrialOrganisation, State};

    let country = Country.bit();
    let state = State.bit();
    let character = Character.bit();
    let combat = Combat.bit();
    let organisation = IndustrialOrganisation.bit();
    let mut game = game::executable();
    for (mask, reader, words) in [
        (0, Body, &[
            "all_of_scopes", "any_of_scopes", "check_variable", "clamp_temp_variable",
            "is_in_array", "modulo_temp_variable", "set_temp_variable_to_random",
        ][..]),
        (0, Assign, &["has_event_target", "has_variable", "round_temp_variable"]),
        (country, Assign, &[
            "has_defensive_war", "has_elections", "has_focus_tree", "has_idea_with_trait", "has_template",
            "has_volunteers", "is_faction_leader", "is_in_tech_sharing_group", "is_neighbor_of",
            "is_subject",
        ]),
        (country, Comparison, &["has_stability", "has_war_support", "surrender_progress"]),
        (country, Body, &[
            "all_owned_state", "any_controlled_state", "any_neighbor_country", "any_owned_state",
            "any_scientist",
        ]),
        (state, Body, &["any_country_with_core"]),
        (state, Assign, &[
            "impassable", "is_claimed_by", "is_fully_controlled_by",
            "is_in_home_area", "is_on_continent", "is_owned_and_controlled_by", "state",
        ]),
        (country | character | combat, Assign, &["tag"]),
        (character, Assign, &["has_ideology", "is_active_scientist", "is_deployed"]),
        (character, Body, &["has_scientist_level"]),
        (combat, Assign, &["is_fighting_in_strategic_region", "night"]),
        (organisation, Assign, &[
            "has_mio_equipment_type", "has_mio_policy", "has_mio_policy_active",
            "has_mio_research_category", "is_military_industrial_organization", "is_mio_assigned_to_task",
            "is_mio_available", "is_mio_visible",
        ]),
        (organisation, Comparison, &["has_mio_size"]),
    ] {
        for word in words {
            game = game.trigger(word, mask, reader, 0);
        }
    }
    for (shape, mask, names) in [
        (TriggerShape::Flag, state, &["has_state_flag"][..]),
        (TriggerShape::Flag, organisation, &["has_mio_flag"]),
        (TriggerShape::CompletedOrganisationTraitCount, organisation, &["has_mio_number_of_completed_traits"]),
        (TriggerShape::OrganisationTrait, organisation, &["has_mio_trait", "is_mio_trait_completed"]),
        (TriggerShape::OrganisationTraitAvailability, organisation, &["is_mio_trait_available"]),
        (TriggerShape::VariableExpression, 0, &[
            "add_to_temp_variable", "divide_temp_variable", "multiply_temp_variable",
            "set_temp_variable", "subtract_from_temp_variable",
        ]),
    ] {
        for name in names {
            game = shape.register(game, name, mask);
        }
    }
    game.numeric_scope("GetName", executable_tables::numeric::NumericScope::Country, 1)
        .numeric_callback("GetDateString", 7)
        .variables(|vars| {
            vars.global = vec!["countries", "states"];
            vars.industrial_organisation = vec!["size"];
            vars.country = vec![
                "controlled_states", "core_states", "faction_leader", "modifier",
                "num_of_controlled_factories", "num_owned_controlled_states", "num_owned_states",
                "opinion", "owned_controlled_states", "owned_states", "stability", "war_support",
            ];
            vars.state = vec![
                "building_level", "compliance", "compliance_speed", "controller", "core_countries",
                "owner", "resistance", "resistance_speed", "state_population", "state_population_k",
            ];
        })
}
