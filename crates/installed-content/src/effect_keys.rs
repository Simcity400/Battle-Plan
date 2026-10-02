//! CEffect::ReadEffect (1405345D0): conditional, registry, scope,
//! scripted-effect token hash and +0x58 flag, then Invalid effect and
//! RecordError (1424ACF90). One dispatch in the token reader replay.

use crate::{
    key_names::KeyNames,
    parser::{
        c_string,
        native::{Field, Kind, Shape},
    },
    trigger_keys::ScopeStep,
};

#[cfg(test)]
mod tests;

// Native virtual readers; value slot 3, key slot 4.
pub(crate) const READ_SCRIPT: u64 = 0x140534280;
pub(crate) const READ_EFFECT: u64 = 0x1405345D0;
pub(crate) const READ_VALUE: u64 = 0x141385570;
pub(crate) const READ_IF_KEY: u64 = 0x141385BA0;
const IF_TOKEN: u32 = 0x27C3;

/// Static keyword IDs registered by RegisterEffectEntry@140534C90, with
/// names from register_authored_keys@140082CB0 -> 1424AFC10.
pub const REGISTERED_EFFECTS: [(u32, &str); 553] = [
    (0x2713, "add_legitimacy"),
    (0x2715, "add_stability"),
    (0x271A, "add_manpower"),
    (0x271F, "custom_effect_tooltip"),
    (0x2720, "set_capital"),
    (0x2723, "add_state_core"),
    (0x2724, "remove_state_core"),
    (0x2727, "add_breakthrough_points"),
    (0x2728, "raid_reduce_project_progress_ratio"),
    (0x272E, "change_tag_from"),
    (0x2731, "add_province_modifier"),
    (0x2735, "set_country_flag"),
    (0x2736, "clr_country_flag"),
    (0x2739, "set_global_flag"),
    (0x273A, "clr_global_flag"),
    (0x2743, "random_scientist"),
    (0x2744, "every_scientist"),
    (0x2747, "create_colonial_division_template"),
    (0x2749, "add_scientist_level"),
    (0x274A, "add_scientist_xp"),
    (0x274B, "add_scientist_trait"),
    (0x274E, "remove_building"),
    (0x2750, "set_project_flag"),
    (0x2751, "modify_project_flag"),
    (0x2752, "clr_project_flag"),
    (0x2753, "every_active_scientist"),
    (0x2754, "random_active_scientist"),
    (0x2759, "release"),
    (0x275C, "every_country"),
    (0x2762, "random_country"),
    (0x2769, "every_neighbor_country"),
    (0x276C, "random_neighbor_country"),
    (0x2770, "add_relation_rule_override"),
    (0x2771, "remove_relation_rule_override"),
    (0x277C, "every_enemy_country"),
    (0x277D, "random_enemy_country"),
    (0x278B, "injure_scientist_for_days"),
    (0x278D, "add_breakthrough_progress"),
    (0x278F, "country_event"),
    (0x2790, "state_event"),
    (0x2791, "remove_province_modifier"),
    (0x2793, "mark_technology_tree_layout_dirty"),
    (0x2796, "add_design_template_bonus"),
    (0x27A5, "construct_building_in_random_province"),
    (0x27AE, "add_raid_history_entry"),
    (0x27B3, "set_variable"),
    (0x27B7, "add_ai_strategy"),
    (0x27BB, "random"),
    (0x27BC, "random_list"),
    (0x27C0, "white_peace"),
    (0x27C2, "hidden_effect"),
    (0x27C3, "if"),
    (0x27C5, "add_state_claim"),
    (0x27C6, "remove_state_claim"),
    (0x27C8, "add_opinion_modifier"),
    (0x27C9, "teleport_railway_guns_to_deploy_province"),
    (0x27CA, "remove_opinion_modifier"),
    (0x27DD, "add_equipment_bonus"),
    (0x27F2, "reduce_focus_completion_cost"),
    (0x27F6, "strategic_province_location"),
    (0x2803, "strategic_state_location"),
    (0x2829, "create_railway_gun"),
    (0x2841, "clear_rule"),
    (0x2858, "select_tech_tree_icon_origin"),
    (0x28AD, "add_faction_power_projection"),
    (0x28BE, "create_country_leader"),
    (0x28CB, "add_state_resistance_compliance_modifier"),
    (0x28CC, "remove_state_resistance_compliance_modifier"),
    (0x28D3, "random_subject_country"),
    (0x28DB, "every_subject_country"),
    (0x28DD, "set_faction_member_upgrade_min"),
    (0x291F, "set_faction_upgrade"),
    (0x292A, "every_faction_member"),
    (0x2937, "set_faction_research_unlocked"),
    (0x2939, "set_faction_military_unlocked"),
    (0x29B4, "log"),
    (0x29D6, "create_unit"),
    (0x2B2E, "activate_advisor"),
    (0x2B2F, "deactivate_advisor"),
    (0x2C9B, "sound_effect"),
    (0x2E38, "add_equipment_subsidy"),
    (0x2E4D, "remove_civil_war_target"),
    (0x2E6B, "add_trait"),
    (0x2E6C, "remove_trait"),
    (0x2E90, "promote_leader"),
    (0x2F25, "randomize_weather"),
    (0x2F3B, "break_embargo"),
    (0x2F50, "division_template"),
    (0x2F60, "destroy_unit"),
    (0x2F62, "random_state_division"),
    (0x2F63, "every_state_division"),
    (0x2F64, "add_history_entry"),
    (0x2F65, "set_air_oob"),
    (0x2F6B, "send_embargo"),
    (0x2F80, "set_occupation_law"),
    (0x2F9A, "change_division_template"),
    (0x2F9E, "set_occupation_law_where_available"),
    (0x3004, "set_country_leader_ideology"),
    (0x3005, "add_country_leader_trait"),
    (0x3006, "remove_country_leader_trait"),
    (0x3009, "set_political_party"),
    (0x300A, "set_politics"),
    (0x300D, "add_ideas"),
    (0x300E, "remove_ideas"),
    (0x301B, "add_political_power"),
    (0x301C, "set_political_power"),
    (0x3023, "set_division_force_allow_recruiting"),
    (0x3028, "set_technology"),
    (0x3043, "gain_xp"),
    (0x304A, "create_navy_leader"),
    (0x304B, "remove_unit_leader_trait"),
    (0x304C, "add_unit_leader_trait"),
    (0x308A, "set_state_owner"),
    (0x30A0, "delete_unit"),
    (0x30D1, "puppet"),
    (0x30DC, "set_state_controller"),
    (0x30E0, "add_random_valid_trait_from_unit"),
    (0x30E7, "add_unit_medal_to_latest_entry"),
    (0x3105, "add_divisional_commander_xp"),
    (0x3112, "transfer_state"),
    (0x3148, "set_state_flag"),
    (0x3149, "clr_state_flag"),
    (0x3169, "set_party_rule"),
    (0x316C, "add_faction_influence_score"),
    (0x3173, "create_faction"),
    (0x3185, "add_nuclear_bombs"),
    (0x3186, "annex_country"),
    (0x319F, "add_building_construction"),
    (0x31AC, "dismantle_faction"),
    (0x31AF, "add_to_faction"),
    (0x31B0, "remove_from_faction"),
    (0x31B2, "give_guarantee"),
    (0x31B3, "give_military_access"),
    (0x31C9, "random_state"),
    (0x31CA, "random_owned_state"),
    (0x31CB, "random_neighbor_state"),
    (0x31CD, "every_state"),
    (0x31CE, "every_owned_state"),
    (0x31CF, "every_neighbor_state"),
    (0x31DC, "add_contested_owner"),
    (0x31E8, "news_event"),
    (0x31EE, "declare_war_on"),
    (0x31F2, "add_faction_goal_slot"),
    (0x3210, "add_faction_influence_ratio"),
    (0x32A1, "set_relation_rule"),
    (0x32FE, "add_scaled_political_power"),
    (0x32FF, "hold_election"),
    (0x3303, "add_popularity"),
    (0x3305, "remove_ideas_with_trait"),
    (0x330C, "start_civil_war"),
    (0x330E, "set_unit_organization"),
    (0x3317, "retire_country_leader"),
    (0x3318, "kill_country_leader"),
    (0x3319, "release_puppet"),
    (0x331C, "create_wargoal"),
    (0x3383, "add_claim_by"),
    (0x3384, "remove_claim_by"),
    (0x3385, "add_core_of"),
    (0x3386, "remove_core_of"),
    (0x3392, "army_experience"),
    (0x3393, "navy_experience"),
    (0x3394, "air_experience"),
    (0x33A2, "save_event_target_as"),
    (0x33A3, "save_global_event_target_as"),
    (0x33A4, "clear_global_event_target"),
    (0x33A5, "clear_global_event_targets"),
    (0x33E0, "unlock_national_focus"),
    (0x33E2, "add_tech_bonus"),
    (0x33EB, "set_demilitarized_zone"),
    (0x33ED, "add_research_slot"),
    (0x33EE, "set_research_slots"),
    (0x33F0, "set_border_war"),
    (0x33FF, "set_equipment_fraction"),
    (0x3404, "add_threat"),
    (0x340D, "send_equipment"),
    (0x341C, "set_rule"),
    (0x3424, "diplomatic_relation"),
    (0x3450, "effect_tooltip"),
    (0x3453, "add_resource"),
    (0x345C, "remove_unit_leader"),
    (0x3462, "add_timed_idea"),
    (0x3467, "end_puppet"),
    (0x3471, "load_oob"),
    (0x3492, "create_import"),
    (0x34B8, "add_named_threat"),
    (0x34B9, "set_province_controller"),
    (0x34BD, "set_major"),
    (0x34C9, "create_field_marshal"),
    (0x34CA, "create_corps_commander"),
    (0x34CB, "goto_province"),
    (0x34CC, "goto_state"),
    (0x34D2, "leave_faction"),
    (0x34E0, "complete_national_focus"),
    (0x34EA, "set_party_name"),
    (0x352C, "add_extra_state_shared_building_slots"),
    (0x357F, "reverse_add_opinion_modifier"),
    (0x35B5, "add_to_war"),
    (0x35BC, "show_ideas_tooltip"),
    (0x35DA, "create_equipment_variant"),
    (0x35E4, "add_equipment_production"),
    (0x35FE, "add_ace"),
    (0x360C, "random_owned_controlled_state"),
    (0x3625, "ai_message"),
    (0x3678, "every_other_country"),
    (0x3679, "random_other_country"),
    (0x36C2, "add_equipment_to_stockpile"),
    (0x36E1, "swap_ideas"),
    (0x36E3, "damage_building"),
    (0x36F3, "release_autonomy"),
    (0x36F4, "set_autonomy"),
    (0x370C, "add_to_tech_sharing_group"),
    (0x370D, "remove_from_tech_sharing_group"),
    (0x370F, "add_autonomy_score"),
    (0x371E, "modify_tech_sharing_bonus"),
    (0x3728, "add_autonomy_ratio"),
    (0x372B, "set_cosmetic_tag"),
    (0x372C, "drop_cosmetic_tag"),
    (0x3733, "transfer_ship"),
    (0x375F, "set_state_category"),
    (0x3792, "modify_global_flag"),
    (0x3793, "modify_country_flag"),
    (0x3794, "modify_state_flag"),
    (0x3795, "add_state_modifier"),
    (0x37A3, "set_province_name"),
    (0x37A4, "set_state_name"),
    (0x37A5, "reset_province_name"),
    (0x37A8, "reset_state_name"),
    (0x37B5, "create_production_license"),
    (0x37C2, "add_relation_modifier"),
    (0x37D1, "remove_relation_modifier"),
    (0x37DB, "set_building_level"),
    (0x3839, "set_stability"),
    (0x383A, "add_war_support"),
    (0x383B, "set_war_support"),
    (0x3845, "random_unit_leader"),
    (0x3846, "every_unit_leader"),
    (0x3847, "random_army_leader"),
    (0x3848, "every_army_leader"),
    (0x3849, "random_navy_leader"),
    (0x384A, "every_navy_leader"),
    (0x386B, "transfer_navy"),
    (0x386C, "destroy_ships"),
    (0x3871, "set_unit_leader_flag"),
    (0x3872, "modify_unit_leader_flag"),
    (0x3873, "clr_unit_leader_flag"),
    (0x3875, "set_nationality"),
    (0x3876, "demote_leader"),
    (0x3877, "retire"),
    (0x3881, "add_command_power"),
    (0x3889, "unlock_decision_tooltip"),
    (0x389A, "set_can_be_fired_in_advisor_role"),
    (0x38A6, "random_military_industrial_organization"),
    (0x38AE, "boost_planning"),
    (0x38B2, "add_region_efficiency"),
    (0x38D3, "modify_building_resources"),
    (0x38D6, "global_every_army_leader"),
    (0x38F8, "add_to_variable"),
    (0x38F9, "subtract_from_variable"),
    (0x38FA, "multiply_variable"),
    (0x38FB, "divide_variable"),
    (0x38FD, "clear_variable"),
    (0x38FE, "add_attack"),
    (0x38FF, "add_defense"),
    (0x3900, "add_planning"),
    (0x3911, "unlock_decision_category_tooltip"),
    (0x3912, "activate_mission_tooltip"),
    (0x3913, "load_focus_tree"),
    (0x3915, "add_logistics"),
    (0x391A, "add_offsite_building"),
    (0x3929, "replace_unit_leader_trait"),
    (0x394E, "free_operative"),
    (0x394F, "free_random_operative"),
    (0x395C, "add_max_trait"),
    (0x3965, "add_skill_level"),
    (0x3977, "add_to_temp_variable"),
    (0x3978, "subtract_from_temp_variable"),
    (0x3979, "multiply_temp_variable"),
    (0x397A, "divide_temp_variable"),
    (0x397B, "set_temp_variable"),
    (0x397D, "add_timed_unit_leader_trait"),
    (0x397E, "clamp_variable"),
    (0x397F, "clamp_temp_variable"),
    (0x3992, "start_border_war"),
    (0x3995, "set_border_war_data"),
    (0x3998, "cancel_border_war"),
    (0x39AF, "set_division_template_lock"),
    (0x39B4, "delete_unit_template_and_units"),
    (0x39C9, "print_variables"),
    (0x39D2, "activate_mission"),
    (0x39D3, "remove_mission"),
    (0x39D4, "recall_attache"),
    (0x39D5, "activate_targeted_decision"),
    (0x39D6, "remove_targeted_decision"),
    (0x39D7, "modify_timed_idea"),
    (0x39D8, "activate_decision"),
    (0x39D9, "set_truce"),
    (0x39DA, "round_variable"),
    (0x39E1, "unit_leader_event"),
    (0x39F4, "set_popularities"),
    (0x39F5, "create_operative_leader"),
    (0x39F6, "capture_operative"),
    (0x39F7, "random_operative"),
    (0x39F8, "every_operative"),
    (0x39F9, "send_equipment_fraction"),
    (0x39FA, "transfer_units_fraction"),
    (0x3A27, "finalize_border_war"),
    (0x3A53, "remove_decision"),
    (0x3A54, "swap_ruler_traits"),
    (0x3A55, "add_random_trait"),
    (0x3A56, "remove_exile_tag"),
    (0x3A57, "set_legitimacy"),
    (0x3A58, "become_exiled_in"),
    (0x3A59, "set_faction_name"),
    (0x3A5A, "add_maneuver"),
    (0x3A5B, "add_coordination"),
    (0x3A5C, "give_resource_rights"),
    (0x3A5D, "remove_resource_rights"),
    (0x3A69, "inherit_technology"),
    (0x3A97, "round_temp_variable"),
    (0x3A9B, "supply_units"),
    (0x3B0E, "set_faction_leader"),
    (0x3B8E, "set_keyed_oob"),
    (0x3B8F, "set_oob"),
    (0x3B90, "set_naval_oob"),
    (0x3BA1, "add_to_array"),
    (0x3BA2, "remove_from_array"),
    (0x3BA4, "clear_array"),
    (0x3BA5, "resize_array"),
    (0x3BA7, "add_to_temp_array"),
    (0x3BA8, "remove_from_temp_array"),
    (0x3BA9, "clear_temp_array"),
    (0x3BAA, "resize_temp_array"),
    (0x3BAC, "for_each_loop"),
    (0x3BB6, "add_fuel"),
    (0x3BB8, "set_fuel"),
    (0x3BBE, "for_each_scope_loop"),
    (0x3BC1, "random_scope_in_array"),
    (0x3C00, "force_update_dynamic_modifier"),
    (0x3C02, "remove_dynamic_modifier"),
    (0x3C03, "add_dynamic_modifier"),
    (0x3C04, "for_loop_effect"),
    (0x3C05, "while_loop_effect"),
    (0x3C09, "meta_effect"),
    (0x3C3C, "add_victory_points"),
    (0x3C3D, "set_victory_points"),
    (0x3C62, "launch_nuke"),
    (0x3C64, "scoped_sound_effect"),
    (0x3C6A, "find_lowest_in_array"),
    (0x3C6B, "find_highest_in_array"),
    (0x3C6D, "add_mines"),
    (0x3C79, "set_fuel_ratio"),
    (0x3C7D, "end_exile"),
    (0x3CB9, "remove_advisor_role"),
    (0x3CC3, "add_power_balance_modifier"),
    (0x3CC4, "remove_power_balance_modifier"),
    (0x3CCC, "set_power_balance_gfx"),
    (0x3D40, "kill_ideology_leader"),
    (0x3D41, "retire_ideology_leader"),
    (0x3D47, "add_days_remove"),
    (0x3D48, "add_days_mission_timeout"),
    (0x3DAD, "add_resistance"),
    (0x3DAE, "add_compliance"),
    (0x3DAF, "start_resistance"),
    (0x3DB0, "cancel_resistance"),
    (0x3DE4, "set_resistance"),
    (0x3DE5, "set_compliance"),
    (0x3EBF, "career_profile_step_missiolini"),
    (0x3ED7, "get_supply_vehicles"),
    (0x3ED8, "get_supply_vehicles_temp"),
    (0x3F49, "every_military_industrial_organization"),
    (0x3F4A, "add_mio_funds"),
    (0x3F4B, "set_mio_funds"),
    (0x3F4C, "add_mio_size"),
    (0x3F4D, "complete_mio_trait"),
    (0x3F52, "set_mio_flag"),
    (0x3F54, "clr_mio_flag"),
    (0x3F55, "modify_mio_flag"),
    (0x3F6B, "set_mio_task_capacity"),
    (0x3F6C, "add_mio_task_capacity"),
    (0x3F6D, "add_mio_research_bonus"),
    (0x3F6E, "set_mio_research_bonus"),
    (0x3F6F, "set_mio_name_key"),
    (0x3F85, "add_mio_design_team_assign_cost"),
    (0x3F86, "set_mio_design_team_assign_cost"),
    (0x3F87, "add_mio_industrial_manufacturer_assign_cost"),
    (0x3F88, "set_mio_industrial_manufacturer_assign_cost"),
    (0x3F89, "add_mio_design_team_change_cost"),
    (0x3F8A, "set_mio_design_team_change_cost"),
    (0x3F8B, "unlock_mio_trait_tooltip"),
    (0x3F8C, "add_mio_funds_gain_factor"),
    (0x3F8D, "set_mio_funds_gain_factor"),
    (0x3F8E, "add_mio_size_up_requirement_factor"),
    (0x3F8F, "set_mio_size_up_requirement_factor"),
    (0x3F91, "unlock_military_industrial_organization_tooltip"),
    (0x3F97, "unlock_mio_policy_tooltip"),
    (0x3F99, "add_mio_policy_cost"),
    (0x3F9A, "set_mio_policy_cost"),
    (0x3F9B, "add_mio_policy_cooldown"),
    (0x3F9C, "set_mio_policy_cooldown"),
    (0x3FA1, "add_cic"),
    (0x3FC3, "activate_shine_on_focus"),
    (0x3FC4, "deactivate_shine_on_focus"),
    (0x3FF4, "event_option_tooltip"),
    (0x3FF6, "show_mio_tooltip"),
    (0x4003, "complete_special_project"),
    (0x4009, "set_mio_icon"),
    (0x400B, "create_purchase_contract"),
    (0x4014, "generate_scientist_character"),
    (0x4015, "add_scientist_role"),
    (0x4017, "remove_scientist_role"),
    (0x4067, "random_purchase_contract"),
    (0x4068, "every_purchase_contract"),
    (0x4093, "random_allied_country"),
    (0x4094, "every_allied_country"),
    (0x40D1, "raid_damage_units"),
    (0x40D3, "raid_add_unit_experience"),
    (0x4128, "release_from_captivity"),
    (0x4193, "set_grand_doctrine"),
    (0x4194, "set_sub_doctrine"),
    (0x4195, "add_mastery"),
    (0x4199, "add_daily_mastery"),
    (0x41A8, "unlock_subunit"),
    (0x41BD, "unlock_tactic"),
    (0x41BE, "add_mastery_bonus"),
    (0x41C1, "release_captured_generals_from"),
    (0x41C2, "capture_by"),
    (0x41E2, "promote_officer_to_general"),
    (0x4242, "cancel_purchase_contract"),
    (0x454D, "remove_contested_owner"),
    (0x458A, "add_project_progress_ratio"),
    (0x468C, "custom_override_tooltip"),
    (0x46B4, "complete_prototype_reward_option"),
    (0x4715, "add_unit_bonus"),
    (0x4A45, "add_operation_token"),
    (0x4A46, "remove_operation_token"),
    (0x4A4F, "steal_random_tech_bonus"),
    (0x4A60, "teleport_armies"),
    (0x4A61, "every_controlled_state"),
    (0x4A6F, "set_state_province_controller"),
    (0x4A79, "reserve_dynamic_country"),
    (0x4A7E, "get_highest_scored_country_temp"),
    (0x4A7F, "create_dynamic_country"),
    (0x4A81, "get_sorted_scored_countries_temp"),
    (0x4A8D, "random_occupied_country"),
    (0x4A8E, "every_occupied_country"),
    (0x4A93, "add_resistance_target"),
    (0x4A94, "remove_resistance_target"),
    (0x4A98, "get_highest_scored_country"),
    (0x4A99, "get_sorted_scored_countries"),
    (0x4A9D, "random_country_with_original_tag"),
    (0x4A9E, "every_country_with_original_tag"),
    (0x4A9F, "add_decryption"),
    (0x4AA1, "delete_units"),
    (0x4AA2, "force_enable_resistance"),
    (0x4AA4, "force_disable_resistance"),
    (0x4AAD, "create_intelligence_agency"),
    (0x4AAE, "upgrade_intelligence_agency"),
    (0x4AC3, "create_ship"),
    (0x4AC7, "set_faction_spymaster"),
    (0x4ACF, "country_lock_all_division_template"),
    (0x4AD5, "remove_all_power_balance_modifiers"),
    (0x4B15, "randomize_temp_variable"),
    (0x4B16, "randomize_variable"),
    (0x4B17, "harm_operative_leader"),
    (0x4B18, "force_operative_leader_into_hiding"),
    (0x4B39, "add_intel"),
    (0x4B41, "operative_leader_event"),
    (0x4B5A, "kill_operative"),
    (0x4B5F, "random_controlled_state"),
    (0x4B6F, "mark_focus_tree_layout_dirty"),
    (0x4B70, "add_civil_war_target"),
    (0x4B87, "modulo_temp_variable"),
    (0x4B88, "modulo_variable"),
    (0x4BA0, "set_country_leader_name"),
    (0x4BAF, "set_country_leader_portrait"),
    (0x4BB1, "add_collaboration"),
    (0x4BB2, "set_collaboration"),
    (0x4BB4, "set_temp_variable_to_random"),
    (0x4BB7, "set_variable_to_random"),
    (0x4BBD, "set_leader_name"),
    (0x4BBF, "set_leader_portrait"),
    (0x4BC5, "set_country_leader_description"),
    (0x4BC8, "set_leader_description"),
    (0x4BDA, "execute_operation_coordinated_strike"),
    (0x4C05, "add_nationality"),
    (0x4C09, "recall_volunteers_from"),
    (0x4C23, "turn_operative"),
    (0x4C30, "every_character"),
    (0x4C31, "random_character"),
    (0x4C4E, "set_equipment_version_number"),
    (0x4C50, "add_doctrine_cost_reduction"),
    (0x4C54, "create_faction_from_template"),
    (0x4C56, "set_faction_rule"),
    (0x4C63, "set_garrison_strength"),
    (0x4C71, "add_corps_commander_role"),
    (0x4C72, "add_field_marshal_role"),
    (0x4C73, "add_naval_commander_role"),
    (0x4C74, "add_country_leader_role"),
    (0x4C75, "add_advisor_role"),
    (0x4C77, "retire_character"),
    (0x4C7A, "recruit_character"),
    (0x4C81, "promote_character"),
    (0x4C90, "set_faction_manifest"),
    (0x4C94, "add_faction_goal"),
    (0x4C95, "remove_faction_goal"),
    (0x4C99, "add_faction_initiative"),
    (0x4C9C, "every_collection_element"),
    (0x4CB0, "play_song"),
    (0x4CB1, "scoped_play_song"),
    (0x4CC3, "release_on_controlled"),
    (0x4CC4, "release_puppet_on_controlled"),
    (0x4CC8, "create_entity"),
    (0x4CCB, "destroy_entity"),
    (0x4CCC, "set_entity_movement"),
    (0x4CD0, "set_entity_position"),
    (0x4CD1, "set_entity_rotation"),
    (0x4CD2, "set_entity_scale"),
    (0x4CD7, "remove_decision_on_cooldown"),
    (0x4D06, "force_update_map_mode"),
    (0x4D07, "damage_units"),
    (0x4D19, "uncomplete_national_focus"),
    (0x4D42, "build_railway"),
    (0x4D47, "remove_wargoal"),
    (0x4D4A, "set_entity_animation"),
    (0x4D76, "start_peace_conference"),
    (0x4D82, "add_units_to_division_template"),
    (0x4D91, "swap_country_leader_traits"),
    (0x4DB0, "random_country_division"),
    (0x4DB1, "every_country_division"),
    (0x4DB2, "reseed_division_commander"),
    (0x4DB4, "give_market_access"),
    (0x4DDC, "set_power_balance"),
    (0x4DDD, "remove_power_balance"),
    (0x4DDE, "add_power_balance_value"),
    (0x4DE9, "set_division_template_cap"),
    (0x4DEA, "clear_division_template_cap"),
    (0x4DF1, "set_character_flag"),
    (0x4DF2, "modify_character_flag"),
    (0x4DF3, "clr_character_flag"),
    (0x4DF8, "every_core_state"),
    (0x4DF9, "random_core_state"),
    (0x4DFC, "transfer_state_to"),
    (0x4DFD, "set_state_owner_to"),
    (0x4DFE, "set_state_controller_to"),
    (0x4E06, "remove_country_leader_role"),
    (0x4E07, "party_leader"),
    (0x4E0B, "generate_character"),
    (0x4E0C, "every_possible_country"),
    (0x4E0D, "show_unit_leaders_tooltip"),
    (0x4E0E, "character_list_tooltip"),
    (0x4E18, "remove_unit_leader_role"),
    (0x4E19, "set_portraits"),
    (0x4E1A, "set_character_name"),
];

/// (token, value reader, key reader), sorted by token. Extracted from the
/// supported hoi4.exe with captures/astra/key-validity/classes.py (outside
/// Git), run as `python classes.py effects` from that capture directory.
/// The script reads unified-triggers/effects.tsv, follows registrar factory
/// calls using .pdata boundaries, excludes base/cleanup vtables with RTTI,
/// and writes effects-verified.tsv. Most-derived CEffect slots 3 and 4:
/// registrar examples 14001826A, 1400188CA, 1400182CA, 1400792BA.
/// custom_override_tooltip: 14137E281 installs the outer vtable on RDI;
/// 14137E29C's CHiddenEffect on RDI+0x58 is its embedded child.
pub const EFFECT_CLASSES: [(u32, u64, u64); 553] = [
    (0x2713, 0x141385570, 0x1405345D0), // add_legitimacy
    (0x2715, 0x141385570, 0x1405345D0), // add_stability
    (0x271A, 0x141385570, 0x1405345D0), // add_manpower
    (0x271F, 0x141384F50, 0x1405345D0), // custom_effect_tooltip
    (0x2720, 0x140534280, 0x1403B4F40), // set_capital
    (0x2723, 0x141385570, 0x1405345D0), // add_state_core
    (0x2724, 0x141385570, 0x1405345D0), // remove_state_core
    (0x2727, 0x140534280, 0x140496160), // add_breakthrough_points
    (0x2728, 0x141385570, 0x1405345D0), // raid_reduce_project_progress_ratio
    (0x272E, 0x141385570, 0x1405345D0), // change_tag_from
    (0x2731, 0x140534280, 0x14031B510), // add_province_modifier
    (0x2735, 0x141384F80, 0x141385A10), // set_country_flag
    (0x2736, 0x141384F80, 0x141385A10), // clr_country_flag
    (0x2739, 0x141384F80, 0x141385A10), // set_global_flag
    (0x273A, 0x141384F80, 0x141385A10), // clr_global_flag
    (0x2743, 0x140534280, 0x1402F6ED0), // random_scientist
    (0x2744, 0x140534280, 0x1402F6D70), // every_scientist
    (0x2747, 0x1403A9B40, 0x1403B1090), // create_colonial_division_template
    (0x2749, 0x140534280, 0x140496450), // add_scientist_level
    (0x274A, 0x140534280, 0x140496490), // add_scientist_xp
    (0x274B, 0x140495DC0, 0x1405345D0), // add_scientist_trait
    (0x274E, 0x140534280, 0x14031BFE0), // remove_building
    (0x2750, 0x141384F80, 0x141385A10), // set_project_flag
    (0x2751, 0x141385260, 0x141385E80), // modify_project_flag
    (0x2752, 0x141384F80, 0x141385A10), // clr_project_flag
    (0x2753, 0x140534280, 0x1402F6D70), // every_active_scientist
    (0x2754, 0x140534280, 0x1402F6ED0), // random_active_scientist
    (0x2759, 0x141385570, 0x1405345D0), // release
    (0x275C, 0x140534280, 0x1403ADCF0), // every_country
    (0x2762, 0x140534280, 0x1403AE630), // random_country
    (0x2769, 0x140534280, 0x1403ADCF0), // every_neighbor_country
    (0x276C, 0x140534280, 0x1403AE630), // random_neighbor_country
    (0x2770, 0x140534280, 0x1403AF060), // add_relation_rule_override
    (0x2771, 0x140534280, 0x1403B4150), // remove_relation_rule_override
    (0x277C, 0x140534280, 0x1403ADCF0), // every_enemy_country
    (0x277D, 0x140534280, 0x1403AE630), // random_enemy_country
    (0x278B, 0x140496070, 0x1405345D0), // injure_scientist_for_days
    (0x278D, 0x140534280, 0x140496280), // add_breakthrough_progress
    (0x278F, 0x141384F60, 0x1413859E0), // country_event
    (0x2790, 0x141384F60, 0x1413859E0), // state_event
    (0x2791, 0x140534280, 0x14031C040), // remove_province_modifier
    (0x2793, 0x141385570, 0x1405345D0), // mark_technology_tree_layout_dirty
    (0x2796, 0x140534280, 0x14049DE30), // add_design_template_bonus
    (0x27A5, 0x140534280, 0x14031B760), // construct_building_in_random_province
    (0x27AE, 0x141385570, 0x1405345D0), // add_raid_history_entry
    (0x27B3, 0x140534280, 0x1403B6640), // set_variable
    (0x27B7, 0x140534280, 0x1403AE990), // add_ai_strategy
    (0x27BB, 0x140534280, 0x141386030), // random
    (0x27BC, 0x140534280, 0x141386050), // random_list
    (0x27C0, 0x1403ACF40, 0x1405345D0), // white_peace
    (0x27C2, 0x140534280, 0x1405345D0), // hidden_effect
    (0x27C3, 0x140534280, 0x141385BA0), // if
    (0x27C5, 0x141385570, 0x1405345D0), // add_state_claim
    (0x27C6, 0x141385570, 0x1405345D0), // remove_state_claim
    (0x27C8, 0x140534280, 0x1403AEEE0), // add_opinion_modifier
    (0x27C9, 0x141385570, 0x1405345D0), // teleport_railway_guns_to_deploy_province
    (0x27CA, 0x140534280, 0x1403B4890), // remove_opinion_modifier
    (0x27DD, 0x140534280, 0x14049E0E0), // add_equipment_bonus
    (0x27F2, 0x140534280, 0x1403B3E00), // reduce_focus_completion_cost
    (0x27F6, 0x140534280, 0x14051BEC0), // strategic_province_location
    (0x2803, 0x14051BBB0, 0x1405345D0), // strategic_state_location
    (0x2829, 0x140534280, 0x1403B19E0), // create_railway_gun
    (0x2841, 0x140534280, 0x1403B1010), // clear_rule
    (0x2858, 0x141385570, 0x1405345D0), // select_tech_tree_icon_origin
    (0x28AD, 0x141385570, 0x1405345D0), // add_faction_power_projection
    (0x28BE, 0x140534280, 0x1403B33C0), // create_country_leader
    (0x28CB, 0x14031B0D0, 0x14031B530), // add_state_resistance_compliance_modifier
    (0x28CC, 0x14031B0D0, 0x14031B530), // remove_state_resistance_compliance_modifier
    (0x28D3, 0x140534280, 0x1403AE630), // random_subject_country
    (0x28DB, 0x140534280, 0x1403ADCF0), // every_subject_country
    (0x28DD, 0x140534280, 0x1404B13F0), // set_faction_member_upgrade_min
    (0x291F, 0x141385570, 0x1405345D0), // set_faction_upgrade
    (0x292A, 0x140534280, 0x1403ADCF0), // every_faction_member
    (0x2937, 0x141385570, 0x1405345D0), // set_faction_research_unlocked
    (0x2939, 0x141385570, 0x1405345D0), // set_faction_military_unlocked
    (0x29B4, 0x141385570, 0x1405345D0), // log
    (0x29D6, 0x140534280, 0x1403B1AA0), // create_unit
    (0x2B2E, 0x1403A91F0, 0x1405345D0), // activate_advisor
    (0x2B2F, 0x1403A91F0, 0x1405345D0), // deactivate_advisor
    (0x2C9B, 0x141385570, 0x1405345D0), // sound_effect
    (0x2E38, 0x140534280, 0x140307BA0), // add_equipment_subsidy
    (0x2E4D, 0x141385570, 0x1405345D0), // remove_civil_war_target
    (0x2E6B, 0x140534280, 0x1402F1060), // add_trait
    (0x2E6C, 0x140534280, 0x1402F1060), // remove_trait
    (0x2E90, 0x141385570, 0x1405345D0), // promote_leader
    (0x2F25, 0x141385570, 0x1405345D0), // randomize_weather
    (0x2F3B, 0x141385570, 0x1405345D0), // break_embargo
    (0x2F50, 0x1403A97C0, 0x1405345D0), // division_template
    (0x2F60, 0x141385570, 0x1405345D0), // destroy_unit
    (0x2F62, 0x140534280, 0x14045EAD0), // random_state_division
    (0x2F63, 0x140534280, 0x14045E970), // every_state_division
    (0x2F64, 0x140534280, 0x14045EBF0), // add_history_entry
    (0x2F65, 0x1403AB040, 0x1403B5240), // set_air_oob
    (0x2F6B, 0x141385570, 0x1405345D0), // send_embargo
    (0x2F80, 0x141385570, 0x1405345D0), // set_occupation_law
    (0x2F9A, 0x140534280, 0x1403B1020), // change_division_template
    (0x2F9E, 0x141385570, 0x1405345D0), // set_occupation_law_where_available
    (0x3004, 0x141385570, 0x1405345D0), // set_country_leader_ideology
    (0x3005, 0x1403A9510, 0x1405345D0), // add_country_leader_trait
    (0x3006, 0x1403A9510, 0x1405345D0), // remove_country_leader_trait
    (0x3009, 0x140534280, 0x1403B5310), // set_political_party
    (0x300A, 0x140534280, 0x1403B5350), // set_politics
    (0x300D, 0x1403A9810, 0x1405345D0), // add_ideas
    (0x300E, 0x1403A9810, 0x1405345D0), // remove_ideas
    (0x301B, 0x141385570, 0x1405345D0), // add_political_power
    (0x301C, 0x141385570, 0x1405345D0), // set_political_power
    (0x3023, 0x140534280, 0x1403B5A50), // set_division_force_allow_recruiting
    (0x3028, 0x140534280, 0x140323D80), // set_technology
    (0x3043, 0x141385570, 0x1405345D0), // gain_xp
    (0x304A, 0x140534280, 0x1403B1010), // create_navy_leader
    (0x304B, 0x141385570, 0x1405345D0), // remove_unit_leader_trait
    (0x304C, 0x141385570, 0x1405345D0), // add_unit_leader_trait
    (0x308A, 0x141385570, 0x1405345D0), // set_state_owner
    (0x30A0, 0x140534280, 0x1403B2260), // delete_unit
    (0x30D1, 0x1403A9C70, 0x1405345D0), // puppet
    (0x30DC, 0x141385570, 0x1405345D0), // set_state_controller
    (0x30E0, 0x140534280, 0x14045EC40), // add_random_valid_trait_from_unit
    (0x30E7, 0x140534280, 0x14045ECD0), // add_unit_medal_to_latest_entry
    (0x3105, 0x141385570, 0x1405345D0), // add_divisional_commander_xp
    (0x3112, 0x141385570, 0x1405345D0), // transfer_state
    (0x3148, 0x141384F80, 0x141385A10), // set_state_flag
    (0x3149, 0x141384F80, 0x141385A10), // clr_state_flag
    (0x3169, 0x140534280, 0x1403B52E0), // set_party_rule
    (0x316C, 0x141385570, 0x1405345D0), // add_faction_influence_score
    (0x3173, 0x141385570, 0x1405345D0), // create_faction
    (0x3185, 0x141385570, 0x1405345D0), // add_nuclear_bombs
    (0x3186, 0x140534280, 0x1403B03B0), // annex_country
    (0x319F, 0x140534280, 0x14031D4D0), // add_building_construction
    (0x31AC, 0x141385570, 0x1405345D0), // dismantle_faction
    (0x31AF, 0x141385570, 0x1405345D0), // add_to_faction
    (0x31B0, 0x141385570, 0x1405345D0), // remove_from_faction
    (0x31B2, 0x141385570, 0x1405345D0), // give_guarantee
    (0x31B3, 0x141385570, 0x1405345D0), // give_military_access
    (0x31C9, 0x140534280, 0x1403ADE50), // random_state
    (0x31CA, 0x140534280, 0x1403ADE50), // random_owned_state
    (0x31CB, 0x140534280, 0x1403ADE50), // random_neighbor_state
    (0x31CD, 0x140534280, 0x14031B220), // every_state
    (0x31CE, 0x140534280, 0x14031B220), // every_owned_state
    (0x31CF, 0x140534280, 0x14031B220), // every_neighbor_state
    (0x31DC, 0x141385570, 0x1405345D0), // add_contested_owner
    (0x31E8, 0x141384F60, 0x1413859E0), // news_event
    (0x31EE, 0x1403A9B40, 0x1403B1FE0), // declare_war_on
    (0x31F2, 0x140534280, 0x1404B12F0), // add_faction_goal_slot
    (0x3210, 0x141385570, 0x1405345D0), // add_faction_influence_ratio
    (0x32A1, 0x140534280, 0x1403AF060), // set_relation_rule
    (0x32FE, 0x141385570, 0x1403AF450), // add_scaled_political_power
    (0x32FF, 0x141385570, 0x1405345D0), // hold_election
    (0x3303, 0x1403A9B40, 0x1403AF020), // add_popularity
    (0x3305, 0x141385570, 0x1405345D0), // remove_ideas_with_trait
    (0x330C, 0x140534280, 0x1403B5C20), // start_civil_war
    (0x330E, 0x141385570, 0x1405345D0), // set_unit_organization
    (0x3317, 0x141385570, 0x1405345D0), // retire_country_leader
    (0x3318, 0x141385570, 0x1405345D0), // kill_country_leader
    (0x3319, 0x141385570, 0x1405345D0), // release_puppet
    (0x331C, 0x1403A9B40, 0x1403B1C60), // create_wargoal
    (0x3383, 0x141385570, 0x1405345D0), // add_claim_by
    (0x3384, 0x141385570, 0x1405345D0), // remove_claim_by
    (0x3385, 0x141385570, 0x1405345D0), // add_core_of
    (0x3386, 0x141385570, 0x1405345D0), // remove_core_of
    (0x3392, 0x141385570, 0x1405345D0), // army_experience
    (0x3393, 0x141385570, 0x1405345D0), // navy_experience
    (0x3394, 0x141385570, 0x1405345D0), // air_experience
    (0x33A2, 0x141385570, 0x1405345D0), // save_event_target_as
    (0x33A3, 0x141385570, 0x1405345D0), // save_global_event_target_as
    (0x33A4, 0x141385570, 0x1405345D0), // clear_global_event_target
    (0x33A5, 0x141385570, 0x1405345D0), // clear_global_event_targets
    (0x33E0, 0x141385570, 0x1405345D0), // unlock_national_focus
    (0x33E2, 0x140534280, 0x140323AC0), // add_tech_bonus
    (0x33EB, 0x141385570, 0x1405345D0), // set_demilitarized_zone
    (0x33ED, 0x141385570, 0x1405345D0), // add_research_slot
    (0x33EE, 0x141385570, 0x1405345D0), // set_research_slots
    (0x33F0, 0x141385570, 0x1405345D0), // set_border_war
    (0x33FF, 0x141385570, 0x1405345D0), // set_equipment_fraction
    (0x3404, 0x141385570, 0x1405345D0), // add_threat
    (0x340D, 0x1403A9B40, 0x14049E600), // send_equipment
    (0x341C, 0x140534280, 0x1403B1010), // set_rule
    (0x3424, 0x140534280, 0x1403B2390), // diplomatic_relation
    (0x3450, 0x140534280, 0x1405345D0), // effect_tooltip
    (0x3453, 0x140534280, 0x1403AF090), // add_resource
    (0x345C, 0x1402F0540, 0x1405345D0), // remove_unit_leader
    (0x3462, 0x140534280, 0x1403B5EB0), // add_timed_idea
    (0x3467, 0x141385570, 0x1405345D0), // end_puppet
    (0x3471, 0x141385570, 0x1405345D0), // load_oob
    (0x3492, 0x140534280, 0x1403B1260), // create_import
    (0x34B8, 0x140534280, 0x1403AEE60), // add_named_threat
    (0x34B9, 0x141385570, 0x1405345D0), // set_province_controller
    (0x34BD, 0x141385570, 0x1405345D0), // set_major
    (0x34C9, 0x140534280, 0x1403B1010), // create_field_marshal
    (0x34CA, 0x140534280, 0x1403B1010), // create_corps_commander
    (0x34CB, 0x141385570, 0x1405345D0), // goto_province
    (0x34CC, 0x141385570, 0x1405345D0), // goto_state
    (0x34D2, 0x141385570, 0x1405345D0), // leave_faction
    (0x34E0, 0x1403A9B50, 0x1403B1040), // complete_national_focus
    (0x34EA, 0x140534280, 0x1403B5280), // set_party_name
    (0x352C, 0x141385570, 0x1405345D0), // add_extra_state_shared_building_slots
    (0x357F, 0x140534280, 0x1403AEEE0), // reverse_add_opinion_modifier
    (0x35B5, 0x140534280, 0x1403AF5F0), // add_to_war
    (0x35BC, 0x1403AB900, 0x1405345D0), // show_ideas_tooltip
    (0x35DA, 0x140534280, 0x14049E440), // create_equipment_variant
    (0x35E4, 0x140534280, 0x14048EFA0), // add_equipment_production
    (0x35FE, 0x140534280, 0x1403AE9F0), // add_ace
    (0x360C, 0x140534280, 0x1403ADE50), // random_owned_controlled_state
    (0x3625, 0x140534280, 0x1403AE870), // ai_message
    (0x3678, 0x140534280, 0x1403ADCF0), // every_other_country
    (0x3679, 0x140534280, 0x1403AE630), // random_other_country
    (0x36C2, 0x140534280, 0x14049E3D0), // add_equipment_to_stockpile
    (0x36E1, 0x1403AC0D0, 0x1405345D0), // swap_ideas
    (0x36E3, 0x140534280, 0x14031BA50), // damage_building
    (0x36F3, 0x1403A9B40, 0x1403B4480), // release_autonomy
    (0x36F4, 0x1403A9B40, 0x1403B4CC0), // set_autonomy
    (0x370C, 0x141385570, 0x1405345D0), // add_to_tech_sharing_group
    (0x370D, 0x141385570, 0x1405345D0), // remove_from_tech_sharing_group
    (0x370F, 0x140534280, 0x1403AEB90), // add_autonomy_score
    (0x371E, 0x140534280, 0x140323D40), // modify_tech_sharing_bonus
    (0x3728, 0x140534280, 0x1403AEB50), // add_autonomy_ratio
    (0x372B, 0x141385570, 0x1405345D0), // set_cosmetic_tag
    (0x372C, 0x141385570, 0x1405345D0), // drop_cosmetic_tag
    (0x3733, 0x140534280, 0x1403B6120), // transfer_ship
    (0x375F, 0x141385570, 0x1405345D0), // set_state_category
    (0x3792, 0x141385260, 0x141385E80), // modify_global_flag
    (0x3793, 0x141385260, 0x141385E80), // modify_country_flag
    (0x3794, 0x141385260, 0x141385E80), // modify_state_flag
    (0x3795, 0x140534280, 0x14031B440), // add_state_modifier
    (0x37A3, 0x140534280, 0x1403B4C80), // set_province_name
    (0x37A4, 0x141385570, 0x1405345D0), // set_state_name
    (0x37A5, 0x141385570, 0x1405345D0), // reset_province_name
    (0x37A8, 0x141385570, 0x1405345D0), // reset_state_name
    (0x37B5, 0x140534280, 0x14049E4A0), // create_production_license
    (0x37C2, 0x140534280, 0x1403B3F40), // add_relation_modifier
    (0x37D1, 0x140534280, 0x1403B3F40), // remove_relation_modifier
    (0x37DB, 0x140534280, 0x14031D4D0), // set_building_level
    (0x3839, 0x141385570, 0x1405345D0), // set_stability
    (0x383A, 0x141385570, 0x1405345D0), // add_war_support
    (0x383B, 0x141385570, 0x1405345D0), // set_war_support
    (0x3845, 0x140534280, 0x14138BFE0), // random_unit_leader
    (0x3846, 0x140534280, 0x14138BE60), // every_unit_leader
    (0x3847, 0x140534280, 0x14138BFE0), // random_army_leader
    (0x3848, 0x140534280, 0x14138BE60), // every_army_leader
    (0x3849, 0x140534280, 0x14138BFE0), // random_navy_leader
    (0x384A, 0x140534280, 0x14138BE60), // every_navy_leader
    (0x386B, 0x140534280, 0x1403B6000), // transfer_navy
    (0x386C, 0x140534280, 0x1403B2310), // destroy_ships
    (0x3871, 0x141384F80, 0x141385A10), // set_unit_leader_flag
    (0x3872, 0x141385260, 0x141385E80), // modify_unit_leader_flag
    (0x3873, 0x141384F80, 0x141385A10), // clr_unit_leader_flag
    (0x3875, 0x1403AB590, 0x1405345D0), // set_nationality
    (0x3876, 0x141385570, 0x1405345D0), // demote_leader
    (0x3877, 0x141385570, 0x1405345D0), // retire
    (0x3881, 0x141385570, 0x1405345D0), // add_command_power
    (0x3889, 0x1403ACCD0, 0x1405345D0), // unlock_decision_tooltip
    (0x389A, 0x140534280, 0x1403B4E60), // set_can_be_fired_in_advisor_role
    (0x38A6, 0x140534280, 0x14139B0D0), // random_military_industrial_organization
    (0x38AE, 0x141385570, 0x1405345D0), // boost_planning
    (0x38B2, 0x141385570, 0x1405345D0), // add_region_efficiency
    (0x38D3, 0x140534280, 0x1403B3640), // modify_building_resources
    (0x38D6, 0x140534280, 0x14138BE60), // global_every_army_leader
    (0x38F8, 0x140534280, 0x1403B6640), // add_to_variable
    (0x38F9, 0x140534280, 0x1403B6640), // subtract_from_variable
    (0x38FA, 0x140534280, 0x1403B6640), // multiply_variable
    (0x38FB, 0x140534280, 0x1403B6640), // divide_variable
    (0x38FD, 0x141385570, 0x1405345D0), // clear_variable
    (0x38FE, 0x141385570, 0x1405345D0), // add_attack
    (0x38FF, 0x141385570, 0x1405345D0), // add_defense
    (0x3900, 0x141385570, 0x1405345D0), // add_planning
    (0x3911, 0x1403ACC80, 0x1405345D0), // unlock_decision_category_tooltip
    (0x3912, 0x1403A92B0, 0x1405345D0), // activate_mission_tooltip
    (0x3913, 0x1403AB080, 0x1403B4F80), // load_focus_tree
    (0x3915, 0x141385570, 0x1405345D0), // add_logistics
    (0x391A, 0x140534280, 0x14031D4D0), // add_offsite_building
    (0x3929, 0x140534280, 0x1402F1160), // replace_unit_leader_trait
    (0x394E, 0x1403AA100, 0x1403B3880), // free_operative
    (0x394F, 0x140534280, 0x1403B2F30), // free_random_operative
    (0x395C, 0x141385570, 0x1405345D0), // add_max_trait
    (0x3965, 0x141385570, 0x1405345D0), // add_skill_level
    (0x3977, 0x140534280, 0x1403B6640), // add_to_temp_variable
    (0x3978, 0x140534280, 0x1403B6640), // subtract_from_temp_variable
    (0x3979, 0x140534280, 0x1403B6640), // multiply_temp_variable
    (0x397A, 0x140534280, 0x1403B2580), // divide_temp_variable
    (0x397B, 0x140534280, 0x1403B6640), // set_temp_variable
    (0x397D, 0x140534280, 0x1402F0FF0), // add_timed_unit_leader_trait
    (0x397E, 0x140534280, 0x1403B0DD0), // clamp_variable
    (0x397F, 0x140534280, 0x1403B0B70), // clamp_temp_variable
    (0x3992, 0x140534280, 0x1403B5BB0), // start_border_war
    (0x3995, 0x140534280, 0x1403B4D70), // set_border_war_data
    (0x3998, 0x140534280, 0x1403B0AD0), // cancel_border_war
    (0x39AF, 0x140534280, 0x1403B50A0), // set_division_template_lock
    (0x39B4, 0x140534280, 0x1403B2220), // delete_unit_template_and_units
    (0x39C9, 0x140534280, 0x1403B38F0), // print_variables
    (0x39D2, 0x1403A9EC0, 0x1405345D0), // activate_mission
    (0x39D3, 0x1403A9EC0, 0x1405345D0), // remove_mission
    (0x39D4, 0x141385570, 0x1405345D0), // recall_attache
    (0x39D5, 0x140534280, 0x1403B5DE0), // activate_targeted_decision
    (0x39D6, 0x140534280, 0x1403B5DE0), // remove_targeted_decision
    (0x39D7, 0x140534280, 0x1403B5EB0), // modify_timed_idea
    (0x39D8, 0x1403A9C20, 0x1405345D0), // activate_decision
    (0x39D9, 0x140534280, 0x1403B5A90), // set_truce
    (0x39DA, 0x141385570, 0x1405345D0), // round_variable
    (0x39E1, 0x141384F60, 0x1413859E0), // unit_leader_event
    (0x39F4, 0x140534280, 0x1403B57D0), // set_popularities
    (0x39F5, 0x1403A9BB0, 0x1403B14C0), // create_operative_leader
    (0x39F6, 0x1403AA100, 0x1403B0B20), // capture_operative
    (0x39F7, 0x140534280, 0x14138BFE0), // random_operative
    (0x39F8, 0x140534280, 0x14138BE60), // every_operative
    (0x39F9, 0x1403A9B40, 0x14049E7D0), // send_equipment_fraction
    (0x39FA, 0x1403A9B40, 0x1403B6280), // transfer_units_fraction
    (0x3A27, 0x140534280, 0x1403B27C0), // finalize_border_war
    (0x3A53, 0x1403A9C20, 0x1405345D0), // remove_decision
    (0x3A54, 0x1403AC650, 0x1405345D0), // swap_ruler_traits
    (0x3A55, 0x1402F0520, 0x1405345D0), // add_random_trait
    (0x3A56, 0x141385570, 0x1405345D0), // remove_exile_tag
    (0x3A57, 0x141385570, 0x1405345D0), // set_legitimacy
    (0x3A58, 0x140534280, 0x1403B0840), // become_exiled_in
    (0x3A59, 0x141385570, 0x1405345D0), // set_faction_name
    (0x3A5A, 0x141385570, 0x1405345D0), // add_maneuver
    (0x3A5B, 0x141385570, 0x1405345D0), // add_coordination
    (0x3A5C, 0x140534280, 0x1403B32D0), // give_resource_rights
    (0x3A5D, 0x141385570, 0x1405345D0), // remove_resource_rights
    (0x3A69, 0x141385570, 0x1405345D0), // inherit_technology
    (0x3A97, 0x141385570, 0x1405345D0), // round_temp_variable
    (0x3A9B, 0x141385570, 0x1405345D0), // supply_units
    (0x3B0E, 0x141385570, 0x1405345D0), // set_faction_leader
    (0x3B8E, 0x140534280, 0x1403B5240), // set_keyed_oob
    (0x3B8F, 0x1403AB040, 0x1403B5240), // set_oob
    (0x3B90, 0x1403AB040, 0x1403B5240), // set_naval_oob
    (0x3BA1, 0x140534280, 0x1403B04D0), // add_to_array
    (0x3BA2, 0x140534280, 0x1403B04D0), // remove_from_array
    (0x3BA4, 0x141385570, 0x1405345D0), // clear_array
    (0x3BA5, 0x140534280, 0x1403AE3D0), // resize_array
    (0x3BA7, 0x140534280, 0x1403B04D0), // add_to_temp_array
    (0x3BA8, 0x140534280, 0x1403B04D0), // remove_from_temp_array
    (0x3BA9, 0x141385570, 0x1405345D0), // clear_temp_array
    (0x3BAA, 0x140534280, 0x1403AE3D0), // resize_temp_array
    (0x3BAC, 0x140534280, 0x1403B2890), // for_each_loop
    (0x3BB6, 0x141385570, 0x1405345D0), // add_fuel
    (0x3BB8, 0x141385570, 0x1405345D0), // set_fuel
    (0x3BBE, 0x140534280, 0x1403B2B50), // for_each_scope_loop
    (0x3BC1, 0x140534280, 0x1403B3AB0), // random_scope_in_array
    (0x3C00, 0x141385570, 0x1405345D0), // force_update_dynamic_modifier
    (0x3C02, 0x140534280, 0x1403AD9F0), // remove_dynamic_modifier
    (0x3C03, 0x140534280, 0x1403AD9F0), // add_dynamic_modifier
    (0x3C04, 0x140534280, 0x1403B2E10), // for_loop_effect
    (0x3C05, 0x140534280, 0x1403B6960), // while_loop_effect
    (0x3C09, 0x140534280, 0x1403B34F0), // meta_effect
    (0x3C3C, 0x140534280, 0x1403AE5F0), // add_victory_points
    (0x3C3D, 0x140534280, 0x1403AE5F0), // set_victory_points
    (0x3C62, 0x140534280, 0x1403B3320), // launch_nuke
    (0x3C64, 0x141385570, 0x1405345D0), // scoped_sound_effect
    (0x3C6A, 0x140534280, 0x1403ADA40), // find_lowest_in_array
    (0x3C6B, 0x140534280, 0x1403ADA40), // find_highest_in_array
    (0x3C6D, 0x140534280, 0x1403AEE20), // add_mines
    (0x3C79, 0x141385570, 0x1405345D0), // set_fuel_ratio
    (0x3C7D, 0x141385570, 0x1405345D0), // end_exile
    (0x3CB9, 0x140534280, 0x1403B4720), // remove_advisor_role
    (0x3CC3, 0x140534280, 0x140486480), // add_power_balance_modifier
    (0x3CC4, 0x140534280, 0x140486480), // remove_power_balance_modifier
    (0x3CCC, 0x140534280, 0x140486590), // set_power_balance_gfx
    (0x3D40, 0x141385570, 0x1405345D0), // kill_ideology_leader
    (0x3D41, 0x141385570, 0x1405345D0), // retire_ideology_leader
    (0x3D47, 0x140534280, 0x1403AED20), // add_days_remove
    (0x3D48, 0x140534280, 0x1403AECE0), // add_days_mission_timeout
    (0x3DAD, 0x141385570, 0x1405345D0), // add_resistance
    (0x3DAE, 0x141385570, 0x1405345D0), // add_compliance
    (0x3DAF, 0x141385570, 0x1405345D0), // start_resistance
    (0x3DB0, 0x141385570, 0x1405345D0), // cancel_resistance
    (0x3DE4, 0x141385570, 0x1405345D0), // set_resistance
    (0x3DE5, 0x141385570, 0x1405345D0), // set_compliance
    (0x3EBF, 0x141385570, 0x1405345D0), // career_profile_step_missiolini
    (0x3ED7, 0x14047E270, 0x140480FD0), // get_supply_vehicles
    (0x3ED8, 0x14047DE20, 0x140480FD0), // get_supply_vehicles_temp
    (0x3F49, 0x140534280, 0x14139AF50), // every_military_industrial_organization
    (0x3F4A, 0x141385570, 0x1405345D0), // add_mio_funds
    (0x3F4B, 0x141385570, 0x1405345D0), // set_mio_funds
    (0x3F4C, 0x141385570, 0x1405345D0), // add_mio_size
    (0x3F4D, 0x141385770, 0x140301200), // complete_mio_trait
    (0x3F52, 0x141384F80, 0x141385A10), // set_mio_flag
    (0x3F54, 0x141384F80, 0x141385A10), // clr_mio_flag
    (0x3F55, 0x141385260, 0x141385E80), // modify_mio_flag
    (0x3F6B, 0x141385570, 0x1405345D0), // set_mio_task_capacity
    (0x3F6C, 0x141385570, 0x1405345D0), // add_mio_task_capacity
    (0x3F6D, 0x141385570, 0x1405345D0), // add_mio_research_bonus
    (0x3F6E, 0x141385570, 0x1405345D0), // set_mio_research_bonus
    (0x3F6F, 0x141385570, 0x1405345D0), // set_mio_name_key
    (0x3F85, 0x141385570, 0x1405345D0), // add_mio_design_team_assign_cost
    (0x3F86, 0x141385570, 0x1405345D0), // set_mio_design_team_assign_cost
    (0x3F87, 0x141385570, 0x1405345D0), // add_mio_industrial_manufacturer_assign_cost
    (0x3F88, 0x141385570, 0x1405345D0), // set_mio_industrial_manufacturer_assign_cost
    (0x3F89, 0x141385570, 0x1405345D0), // add_mio_design_team_change_cost
    (0x3F8A, 0x141385570, 0x1405345D0), // set_mio_design_team_change_cost
    (0x3F8B, 0x141385770, 0x140301200), // unlock_mio_trait_tooltip
    (0x3F8C, 0x141385570, 0x1405345D0), // add_mio_funds_gain_factor
    (0x3F8D, 0x141385570, 0x1405345D0), // set_mio_funds_gain_factor
    (0x3F8E, 0x141385570, 0x1405345D0), // add_mio_size_up_requirement_factor
    (0x3F8F, 0x141385570, 0x1405345D0), // set_mio_size_up_requirement_factor
    (0x3F91, 0x141385570, 0x1405345D0), // unlock_military_industrial_organization_tooltip
    (0x3F97, 0x141385770, 0x140301210), // unlock_mio_policy_tooltip
    (0x3F99, 0x140534280, 0x1403011E0), // add_mio_policy_cost
    (0x3F9A, 0x140534280, 0x1403011E0), // set_mio_policy_cost
    (0x3F9B, 0x140534280, 0x1403011E0), // add_mio_policy_cooldown
    (0x3F9C, 0x140534280, 0x1403011E0), // set_mio_policy_cooldown
    (0x3FA1, 0x141385570, 0x1405345D0), // add_cic
    (0x3FC3, 0x141385570, 0x1405345D0), // activate_shine_on_focus
    (0x3FC4, 0x141385570, 0x1405345D0), // deactivate_shine_on_focus
    (0x3FF4, 0x141385570, 0x1405345D0), // event_option_tooltip
    (0x3FF6, 0x141385570, 0x1405345D0), // show_mio_tooltip
    (0x4003, 0x141385770, 0x1404964D0), // complete_special_project
    (0x4009, 0x141385570, 0x1405345D0), // set_mio_icon
    (0x400B, 0x140534280, 0x140307C20), // create_purchase_contract
    (0x4014, 0x140534280, 0x1403B3160), // generate_scientist_character
    (0x4015, 0x140534280, 0x1403AF490), // add_scientist_role
    (0x4017, 0x141385770, 0x1403B49C0), // remove_scientist_role
    (0x4067, 0x140534280, 0x140307A80), // random_purchase_contract
    (0x4068, 0x140534280, 0x140307920), // every_purchase_contract
    (0x4093, 0x140534280, 0x1403AE630), // random_allied_country
    (0x4094, 0x140534280, 0x1403ADCF0), // every_allied_country
    (0x40D1, 0x140534280, 0x1404A46D0), // raid_damage_units
    (0x40D3, 0x141385570, 0x1405345D0), // raid_add_unit_experience
    (0x4128, 0x141385570, 0x1405345D0), // release_from_captivity
    (0x4193, 0x141385570, 0x1405345D0), // set_grand_doctrine
    (0x4194, 0x141385770, 0x14051ECD0), // set_sub_doctrine
    (0x4195, 0x140534280, 0x14051EC00), // add_mastery
    (0x4199, 0x140534280, 0x14051EB60), // add_daily_mastery
    (0x41A8, 0x141385570, 0x1405345D0), // unlock_subunit
    (0x41BD, 0x141385570, 0x1405345D0), // unlock_tactic
    (0x41BE, 0x140534280, 0x14051EBB0), // add_mastery_bonus
    (0x41C1, 0x141385770, 0x1402F8680), // release_captured_generals_from
    (0x41C2, 0x141385770, 0x1402F8640), // capture_by
    (0x41E2, 0x141385570, 0x1405345D0), // promote_officer_to_general
    (0x4242, 0x141385570, 0x1405345D0), // cancel_purchase_contract
    (0x454D, 0x141385570, 0x1405345D0), // remove_contested_owner
    (0x458A, 0x141385570, 0x1405345D0), // add_project_progress_ratio
    (0x468C, 0x140534280, 0x1413864C0), // custom_override_tooltip
    (0x46B4, 0x140534280, 0x140496590), // complete_prototype_reward_option
    (0x4715, 0x140534280, 0x1403AF680), // add_unit_bonus
    (0x4A45, 0x140534280, 0x1403AEEA0), // add_operation_token
    (0x4A46, 0x140534280, 0x1403AEEA0), // remove_operation_token
    (0x4A4F, 0x140534280, 0x140323F70), // steal_random_tech_bonus
    (0x4A60, 0x140534280, 0x14031C3A0), // teleport_armies
    (0x4A61, 0x140534280, 0x14031B220), // every_controlled_state
    (0x4A6F, 0x140534280, 0x14031C000), // set_state_province_controller
    (0x4A79, 0x141385570, 0x1405345D0), // reserve_dynamic_country
    (0x4A7E, 0x14047CCE0, 0x140480F40), // get_highest_scored_country_temp
    (0x4A7F, 0x140534280, 0x1403B1120), // create_dynamic_country
    (0x4A81, 0x14047D580, 0x140480F80), // get_sorted_scored_countries_temp
    (0x4A8D, 0x140534280, 0x1403AE630), // random_occupied_country
    (0x4A8E, 0x140534280, 0x1403ADCF0), // every_occupied_country
    (0x4A93, 0x14031B0A0, 0x14031B3B0), // add_resistance_target
    (0x4A94, 0x141385570, 0x1405345D0), // remove_resistance_target
    (0x4A98, 0x14047D130, 0x140480F40), // get_highest_scored_country
    (0x4A99, 0x14047D9D0, 0x140480F80), // get_sorted_scored_countries
    (0x4A9D, 0x140534280, 0x1403B3970), // random_country_with_original_tag
    (0x4A9E, 0x140534280, 0x1403B2620), // every_country_with_original_tag
    (0x4A9F, 0x140534280, 0x1403AED60), // add_decryption
    (0x4AA1, 0x140534280, 0x1403B22D0), // delete_units
    (0x4AA2, 0x14031B0A0, 0x14031BA90), // force_enable_resistance
    (0x4AA4, 0x14031B0A0, 0x14031BA90), // force_disable_resistance
    (0x4AAD, 0x1403A9BA0, 0x1403B1480), // create_intelligence_agency
    (0x4AAE, 0x141385570, 0x1405345D0), // upgrade_intelligence_agency
    (0x4AC3, 0x140534280, 0x1403B1A30), // create_ship
    (0x4AC7, 0x141385570, 0x1405345D0), // set_faction_spymaster
    (0x4ACF, 0x141385770, 0x1403B34B0), // country_lock_all_division_template
    (0x4AD5, 0x140534280, 0x140486450), // remove_all_power_balance_modifiers
    (0x4B15, 0x1403A89E0, 0x1403ADE70), // randomize_temp_variable
    (0x4B16, 0x1403A89E0, 0x1403ADE70), // randomize_variable
    (0x4B17, 0x141385570, 0x1405345D0), // harm_operative_leader
    (0x4B18, 0x141385570, 0x1405345D0), // force_operative_leader_into_hiding
    (0x4B39, 0x140534280, 0x1403AEDB0), // add_intel
    (0x4B41, 0x141384F60, 0x141385790), // operative_leader_event
    (0x4B5A, 0x1403AA100, 0x1403B3880), // kill_operative
    (0x4B5F, 0x140534280, 0x1403ADE50), // random_controlled_state
    (0x4B6F, 0x141385570, 0x1405345D0), // mark_focus_tree_layout_dirty
    (0x4B70, 0x141385570, 0x1405345D0), // add_civil_war_target
    (0x4B87, 0x14047E6C0, 0x140481020), // modulo_temp_variable
    (0x4B88, 0x14047EB10, 0x140481020), // modulo_variable
    (0x4BA0, 0x140534280, 0x1402F0FA0), // set_country_leader_name
    (0x4BAF, 0x140534280, 0x1402F0FA0), // set_country_leader_portrait
    (0x4BB1, 0x140534280, 0x1403AEBD0), // add_collaboration
    (0x4BB2, 0x140534280, 0x1403AEBD0), // set_collaboration
    (0x4BB4, 0x14047EF60, 0x140481030), // set_temp_variable_to_random
    (0x4BB7, 0x14047F3E0, 0x140481030), // set_variable_to_random
    (0x4BBD, 0x141385570, 0x1405345D0), // set_leader_name
    (0x4BBF, 0x141385570, 0x1405345D0), // set_leader_portrait
    (0x4BC5, 0x140534280, 0x1402F0FA0), // set_country_leader_description
    (0x4BC8, 0x141385570, 0x1405345D0), // set_leader_description
    (0x4BDA, 0x140534280, 0x1403B27A0), // execute_operation_coordinated_strike
    (0x4C05, 0x141385570, 0x1405345D0), // add_nationality
    (0x4C09, 0x141385570, 0x1405345D0), // recall_volunteers_from
    (0x4C23, 0x1403AA100, 0x1403B3880), // turn_operative
    (0x4C30, 0x140534280, 0x1402F6D70), // every_character
    (0x4C31, 0x140534280, 0x1402F6ED0), // random_character
    (0x4C4E, 0x140534280, 0x14049E920), // set_equipment_version_number
    (0x4C50, 0x140534280, 0x1403234E0), // add_doctrine_cost_reduction
    (0x4C54, 0x141385770, 0x1404B1340), // create_faction_from_template
    (0x4C56, 0x141385570, 0x1405345D0), // set_faction_rule
    (0x4C63, 0x141385570, 0x1405345D0), // set_garrison_strength
    (0x4C71, 0x140534280, 0x1403AD950), // add_corps_commander_role
    (0x4C72, 0x140534280, 0x1403AD950), // add_field_marshal_role
    (0x4C73, 0x140534280, 0x1403AD950), // add_naval_commander_role
    (0x4C74, 0x140534280, 0x1403AEC00), // add_country_leader_role
    (0x4C75, 0x140534280, 0x1403AEA80), // add_advisor_role
    (0x4C77, 0x1403AAFA0, 0x1405345D0), // retire_character
    (0x4C7A, 0x1403AAA50, 0x1405345D0), // recruit_character
    (0x4C81, 0x1403AA3C0, 0x1405345D0), // promote_character
    (0x4C90, 0x141385570, 0x1405345D0), // set_faction_manifest
    (0x4C94, 0x141385570, 0x1405345D0), // add_faction_goal
    (0x4C95, 0x141385570, 0x1405345D0), // remove_faction_goal
    (0x4C99, 0x141385570, 0x1405345D0), // add_faction_initiative
    (0x4C9C, 0x140534280, 0x1404C3B00), // every_collection_element
    (0x4CB0, 0x141385570, 0x1405345D0), // play_song
    (0x4CB1, 0x141385570, 0x1405345D0), // scoped_play_song
    (0x4CC3, 0x141385570, 0x1405345D0), // release_on_controlled
    (0x4CC4, 0x141385570, 0x1405345D0), // release_puppet_on_controlled
    (0x4CC8, 0x140534280, 0x1403B1160), // create_entity
    (0x4CCB, 0x141385570, 0x1405345D0), // destroy_entity
    (0x4CCC, 0x140534280, 0x1403B5120), // set_entity_movement
    (0x4CD0, 0x140534280, 0x1403B5190), // set_entity_position
    (0x4CD1, 0x140534280, 0x1403B5210), // set_entity_rotation
    (0x4CD2, 0x140534280, 0x1403B5210), // set_entity_scale
    (0x4CD7, 0x141385570, 0x1405345D0), // remove_decision_on_cooldown
    (0x4D06, 0x140534280, 0x1403B2EE0), // force_update_map_mode
    (0x4D07, 0x140534280, 0x1403B1EC0), // damage_units
    (0x4D19, 0x140534280, 0x1403B64F0), // uncomplete_national_focus
    (0x4D42, 0x140534280, 0x1403B0960), // build_railway
    (0x4D47, 0x1403A9B40, 0x1403B4B00), // remove_wargoal
    (0x4D4A, 0x140534280, 0x1403B50E0), // set_entity_animation
    (0x4D76, 0x1403ABDF0, 0x1405345D0), // start_peace_conference
    (0x4D82, 0x1403A9B40, 0x1403AF6B0), // add_units_to_division_template
    (0x4D91, 0x1402F0810, 0x1405345D0), // swap_country_leader_traits
    (0x4DB0, 0x140534280, 0x14045EAD0), // random_country_division
    (0x4DB1, 0x140534280, 0x14045E970), // every_country_division
    (0x4DB2, 0x140534280, 0x14045ECF0), // reseed_division_commander
    (0x4DB4, 0x141385570, 0x1405345D0), // give_market_access
    (0x4DDC, 0x140534280, 0x1404864D0), // set_power_balance
    (0x4DDD, 0x140534280, 0x140486450), // remove_power_balance
    (0x4DDE, 0x140534280, 0x1404863E0), // add_power_balance_value
    (0x4DE9, 0x140534280, 0x1403B5A10), // set_division_template_cap
    (0x4DEA, 0x140534280, 0x1403B1020), // clear_division_template_cap
    (0x4DF1, 0x141384F80, 0x141385A10), // set_character_flag
    (0x4DF2, 0x141385260, 0x141385E80), // modify_character_flag
    (0x4DF3, 0x141384F80, 0x141385A10), // clr_character_flag
    (0x4DF8, 0x140534280, 0x14031B220), // every_core_state
    (0x4DF9, 0x140534280, 0x1403ADE50), // random_core_state
    (0x4DFC, 0x141385570, 0x1405345D0), // transfer_state_to
    (0x4DFD, 0x141385570, 0x1405345D0), // set_state_owner_to
    (0x4DFE, 0x141385570, 0x1405345D0), // set_state_controller_to
    (0x4E06, 0x140534280, 0x1403B47D0), // remove_country_leader_role
    (0x4E07, 0x140534280, 0x1402F6ED0), // party_leader
    (0x4E0B, 0x140534280, 0x1403B2F70), // generate_character
    (0x4E0C, 0x140534280, 0x1403ADCF0), // every_possible_country
    (0x4E0D, 0x1403ABD40, 0x1405345D0), // show_unit_leaders_tooltip
    (0x4E0E, 0x140534280, 0x1402F6D70), // character_list_tooltip
    (0x4E18, 0x1403AAC00, 0x1405345D0), // remove_unit_leader_role
    (0x4E19, 0x140534280, 0x1402F12C0), // set_portraits
    (0x4E1A, 0x1403AB2C0, 0x1405345D0), // set_character_name
];

/// A bare keyword's native token, as the trigger registry maps its names.
pub fn registered(keyword: &[u8]) -> Option<u32> {
    REGISTERED_EFFECTS
        .iter()
        .find(|(_, name)| name.as_bytes().eq_ignore_ascii_case(keyword))
        .map(|(token, _)| *token)
}

/// The registered class's value/key readers, found by native token ID.
pub fn class(token: u32) -> Option<(u64, u64)> {
    EFFECT_CLASSES
        .binary_search_by_key(&token, |(id, _, _)| *id)
        .ok()
        .map(|index| (EFFECT_CLASSES[index].1, EFFECT_CLASSES[index].2))
}

pub(crate) fn body_error(key: &str, value: &str) -> String {
    format!(
        "field {key} is {value} where CEffect::ReadScript reads a block: it is reported ({READ_SCRIPT:X} -> 1424ACF90) and nothing more is read"
    )
}

fn class_shape(readers: (u64, u64), field: &Field<'_>) -> Shape<()> {
    let (value_reader, key_reader) = readers;
    match readers {
        (READ_SCRIPT, READ_EFFECT) => Shape::Effect,
        (READ_VALUE, _) => Shape::One,
        _ => {
            let read = if value_reader == READ_SCRIPT {
                Shape::EffectByKey {
                    trigger_limits: key_reader == READ_IF_KEY,
                }
            } else {
                Shape::ByKey
            };
            let name = field.keyword().unwrap_or_default();
            Shape::Reported {
                report: format!(
                    "Deferred effect `{}`: value reader {value_reader:#X}, key reader {key_reader:#X}; by-key reading retained",
                    String::from_utf8_lossy(&name)
                ),
                read: Box::new(read),
            }
        }
    }
}

/// The one ReadEffect dispatch, including the special conditional path.
pub(crate) fn shape(names: Option<&KeyNames>, field: &Field<'_>) -> Shape<()> {
    let keyword = field.keyword();
    if matches!(keyword.as_deref(), Some(b"else" | b"else_if")) {
        // The same CIfEffect readers as the registered `if` class.
        return class_shape(class(IF_TOKEN).expect("registered CIfEffect"), field);
    }
    if let Some(readers) = keyword.as_deref().and_then(registered).and_then(class) {
        return class_shape(readers, field);
    }
    let Some(names) = names else {
        return Shape::Unsettled("ReadEffect (1405345D0) was given no content names for its scope and scripted-effect lookups".into());
    };
    if field.opens() {
        match names.scope(c_string(&field.key.text), field.key.kind == Kind::Quoted) {
            ScopeStep::Scope(_) => return Shape::Effect,
            ScopeStep::NotScope => {}
            ScopeStep::Unknown(why) => return Shape::Unsettled(why),
        }
    }
    if names
        .scripted_effects
        .contains(&c_string(&field.key.text).to_ascii_lowercase())
    {
        // CScriptedEffect's vtable 14299F7C8, slot 3 = 141385570:
        // the one value is read, never a block skipped.
        return Shape::One;
    }
    // Invalid effect + RecordError: skip only a current `{`, define nothing.
    Shape::Skip
}
