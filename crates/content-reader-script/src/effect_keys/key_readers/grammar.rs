
use super::{Fallback, KeyReader, ValueReader};
#[derive(Clone, Copy)]
pub(crate) enum Helper {
    Advisor,
    BorderWarSide,
    CharacterTemplate,
    Cid,
    CountryLeader,
    CreateEquipmentVariantSpec,
    EquipmentBonus,
    EquipmentBonusEntries,
    EquipmentVariant,
    Modifier,
    Mtth,
    OptionalEquipmentAsset,
    PoliticalParties,
    Portraits,
    PortraitSizes,
    ProvinceTarget,
    PurchaseEquipment,
    RandomListMember,
    Rule,
    RuleOverride,
    ScalarFields,
    ScientistSkills,
    ScientistTemplate,
    UnitLeaderData,
    UnitLeaderTemplate,
    UnitOfficerData,
    VariantModules,
    VariantUpgrades,
}
const SET_CAPITAL_KEYS: u64 = 0x1403CDBB0;
const ADD_BREAKTHROUGH_POINTS_KEYS: u64 = 0x1404B5C50;
const ADD_PROVINCE_MODIFIER_KEYS: u64 = 0x140334080;
const RANDOM_SCIENTIST_KEYS: u64 = 0x14030F9D0;
const EVERY_SCIENTIST_KEYS: u64 = 0x14030F870;
const ADD_SCIENTIST_LEVEL_KEYS: u64 = 0x1404b5f40;
const ADD_SCIENTIST_XP_KEYS: u64 = 0x1404B5F80;
const REMOVE_BUILDING_KEYS: u64 = 0x140334B50;
const EVERY_COUNTRY_KEYS: u64 = 0x1403C6920;
const RANDOM_COUNTRY_KEYS: u64 = 0x1403C7260;
const ADD_RELATION_RULE_OVERRIDE_KEYS: u64 = 0x1403C7C90;
const REMOVE_RELATION_RULE_OVERRIDE_KEYS: u64 = 0x1403CCDA0;
const ADD_BREAKTHROUGH_PROGRESS_KEYS: u64 = 0x1404B5D70;
const REMOVE_PROVINCE_MODIFIER_KEYS: u64 = 0x140334BB0;
const ADD_DESIGN_TEMPLATE_BONUS_KEYS: u64 = 0x1404BD920;
const CONSTRUCT_BUILDING_IN_RANDOM_PROVINCE_KEYS: u64 = 0x1403342d0;
const SET_VARIABLE_KEYS: u64 = 0x1403CF2B0;
const ADD_AI_STRATEGY_KEYS: u64 = 0x1403C75C0;
const RANDOM_KEYS: u64 = 0x141437120;
const RANDOM_LIST_KEYS: u64 = 0x141437140;
const IF_KEYS: u64 = 0x141436c90;
const ADD_OPINION_MODIFIER_KEYS: u64 = 0x1403C7B10;
const REMOVE_OPINION_MODIFIER_KEYS: u64 = 0x1403CD4E0;
const ADD_EQUIPMENT_BONUS_KEYS: u64 = 0x1404BDBD0;
const REDUCE_FOCUS_COMPLETION_COST_KEYS: u64 = 0x1403CCA60;
const STRATEGIC_PROVINCE_LOCATION_KEYS: u64 = 0x14056F590;
const STRATEGIC_STATE_LOCATION_KEYS: u64 = 0x14056F280;
const CREATE_RAILWAY_GUN_KEYS: u64 = 0x1403CA620;
const SHARED_DEFINITION_KEYS: u64 = 0x1403C9C50;
const CREATE_COUNTRY_LEADER_KEYS: u64 = 0x1403CC020;
const SET_FACTION_MEMBER_UPGRADE_MIN_KEYS: u64 = 0x1404D0E10;
const CREATE_UNIT_KEYS: u64 = 0x1403CA6E0;
const ADD_EQUIPMENT_SUBSIDY_KEYS: u64 = 0x140320820;
const ADD_TRAIT_KEYS: u64 = 0x140309B60;
const RANDOM_STATE_DIVISION_KEYS: u64 = 0x14047e600;
const EVERY_STATE_DIVISION_KEYS: u64 = 0x14047e4a0;
const ADD_HISTORY_ENTRY_KEYS: u64 = 0x14047E720;
const CHANGE_DIVISION_TEMPLATE_KEYS: u64 = 0x1403C9C60;
const SET_POLITICAL_PARTY_KEYS: u64 = 0x1403CDF80;
const SET_POLITICS_KEYS: u64 = 0x1403CDFC0;
const SET_DIVISION_FORCE_ALLOW_RECRUITING_KEYS: u64 = 0x1403CE6C0;
const SET_TECHNOLOGY_KEYS: u64 = 0x14033C8E0;
const DELETE_UNIT_KEYS: u64 = 0x1403CAEA0;
const ADD_RANDOM_VALID_TRAIT_FROM_UNIT_KEYS: u64 = 0x14047e770;
const ADD_UNIT_MEDAL_TO_LATEST_ENTRY_KEYS: u64 = 0x14047e800;
const SET_PARTY_RULE_KEYS: u64 = 0x1403CDF50;
const ANNEX_COUNTRY_KEYS: u64 = 0x1403C8FF0;
const ADD_BUILDING_CONSTRUCTION_KEYS: u64 = 0x140336040;
const RANDOM_STATE_KEYS: u64 = 0x1403C6A80;
const EVERY_STATE_KEYS: u64 = 0x140333D90;
const ADD_FACTION_GOAL_SLOT_KEYS: u64 = 0x1404D0D10;
const START_CIVIL_WAR_KEYS: u64 = 0x1403CE890;
const ADD_TECH_BONUS_KEYS: u64 = 0x14033c620;
const DIPLOMATIC_RELATION_KEYS: u64 = 0x1403CAFD0;
const ADD_RESOURCE_KEYS: u64 = 0x1403C7CC0;
const ADD_TIMED_IDEA_KEYS: u64 = 0x1403CEB20;
const CREATE_IMPORT_KEYS: u64 = 0x1403C9EA0;
const ADD_NAMED_THREAT_KEYS: u64 = 0x1403C7A90;
const SET_PARTY_NAME_KEYS: u64 = 0x1403CDEF0;
const ADD_TO_WAR_KEYS: u64 = 0x1403C8220;
const CREATE_EQUIPMENT_VARIANT_KEYS: u64 = 0x1404BDF30;
const ADD_EQUIPMENT_PRODUCTION_KEYS: u64 = 0x1404AE9B0;
const ADD_ACE_KEYS: u64 = 0x1403C7620;
const ADD_EQUIPMENT_TO_STOCKPILE_KEYS: u64 = 0x1404BDEC0;
const DAMAGE_BUILDING_KEYS: u64 = 0x1403345c0;
const ADD_AUTONOMY_SCORE_KEYS: u64 = 0x1403C77C0;
const MODIFY_TECH_SHARING_BONUS_KEYS: u64 = 0x14033C8A0;
const ADD_AUTONOMY_RATIO_KEYS: u64 = 0x1403C7780;
const TRANSFER_SHIP_KEYS: u64 = 0x1403CED90;
const ADD_STATE_MODIFIER_KEYS: u64 = 0x140333FB0;
const SET_PROVINCE_NAME_KEYS: u64 = 0x1403CD8D0;
const CREATE_PRODUCTION_LICENSE_KEYS: u64 = 0x1404BDF90;
const ADD_RELATION_MODIFIER_KEYS: u64 = 0x1403CCBA0;
const RANDOM_UNIT_LEADER_KEYS: u64 = 0x14143D0D0;
const EVERY_UNIT_LEADER_KEYS: u64 = 0x14143CF50;
const TRANSFER_NAVY_KEYS: u64 = 0x1403CEC70;
const DESTROY_SHIPS_KEYS: u64 = 0x1403CAF50;
const SET_CAN_BE_FIRED_IN_ADVISOR_ROLE_KEYS: u64 = 0x1403CDAD0;
const RANDOM_MILITARY_INDUSTRIAL_ORGANIZATION_KEYS: u64 = 0x14144C220;
const MODIFY_BUILDING_RESOURCES_KEYS: u64 = 0x1403CC2A0;
const FREE_RANDOM_OPERATIVE_KEYS: u64 = 0x1403CBB90;
const DIVIDE_TEMP_VARIABLE_KEYS: u64 = 0x1403CB1E0;
const ADD_TIMED_UNIT_LEADER_TRAIT_KEYS: u64 = 0x140309AF0;
const CLAMP_VARIABLE_KEYS: u64 = 0x1403C9A10;
const CLAMP_TEMP_VARIABLE_KEYS: u64 = 0x1403C97B0;
const START_BORDER_WAR_KEYS: u64 = 0x1403CE820;
const SET_BORDER_WAR_DATA_KEYS: u64 = 0x1403CD9E0;
const CANCEL_BORDER_WAR_KEYS: u64 = 0x1403C9710;
const SET_DIVISION_TEMPLATE_LOCK_KEYS: u64 = 0x1403CDD10;
const DELETE_UNIT_TEMPLATE_AND_UNITS_KEYS: u64 = 0x1403CAE60;
const PRINT_VARIABLES_KEYS: u64 = 0x1403CC550;
const ACTIVATE_TARGETED_DECISION_KEYS: u64 = 0x1403CEA50;
const SET_TRUCE_KEYS: u64 = 0x1403CE700;
const SET_POPULARITIES_KEYS: u64 = 0x1403CE440;
const FINALIZE_BORDER_WAR_KEYS: u64 = 0x1403CB420;
const BECOME_EXILED_IN_KEYS: u64 = 0x1403C9480;
const GIVE_RESOURCE_RIGHTS_KEYS: u64 = 0x1403CBF30;
const ADD_TO_ARRAY_KEYS: u64 = 0x1403C9110;
const RESIZE_ARRAY_KEYS: u64 = 0x1403C7000;
const FOR_EACH_LOOP_KEYS: u64 = 0x1403CB4F0;
const FOR_EACH_SCOPE_LOOP_KEYS: u64 = 0x1403CB7B0;
const RANDOM_SCOPE_IN_ARRAY_KEYS: u64 = 0x1403CC710;
const REMOVE_DYNAMIC_MODIFIER_KEYS: u64 = 0x1403C6620;
const FOR_LOOP_EFFECT_KEYS: u64 = 0x1403CBA70;
const WHILE_LOOP_EFFECT_KEYS: u64 = 0x1403CF5D0;
const META_EFFECT_KEYS: u64 = 0x1403CC150;
const ADD_VICTORY_POINTS_KEYS: u64 = 0x1403C7220;
const LAUNCH_NUKE_KEYS: u64 = 0x1403CBF80;
const FIND_LOWEST_IN_ARRAY_KEYS: u64 = 0x1403C6670;
const ADD_MINES_KEYS: u64 = 0x1403C7A50;
const REMOVE_ADVISOR_ROLE_KEYS: u64 = 0x1403CD370;
const ADD_POWER_BALANCE_MODIFIER_KEYS: u64 = 0x1404A5E90;
const SET_POWER_BALANCE_GFX_KEYS: u64 = 0x1404A5FA0;
const ADD_DAYS_REMOVE_KEYS: u64 = 0x1403C7950;
const ADD_DAYS_MISSION_TIMEOUT_KEYS: u64 = 0x1403C7910;
const EVERY_MILITARY_INDUSTRIAL_ORGANIZATION_KEYS: u64 = 0x14144C0A0;
const CREATE_PURCHASE_CONTRACT_KEYS: u64 = 0x1403208a0;
const GENERATE_SCIENTIST_CHARACTER_KEYS: u64 = 0x1403CBDC0;
const ADD_SCIENTIST_ROLE_KEYS: u64 = 0x1403C80C0;
const RAID_DAMAGE_UNITS_KEYS: u64 = 0x1404C40F0;
const ADD_MASTERY_KEYS: u64 = 0x1405722d0;
const ADD_DAILY_MASTERY_KEYS: u64 = 0x140572230;
const ADD_MASTERY_BONUS_KEYS: u64 = 0x140572280;
const CUSTOM_OVERRIDE_TOOLTIP_KEYS: u64 = 0x1414375b0;
const ADD_UNIT_BONUS_KEYS: u64 = 0x1403C82B0;
const ADD_OPERATION_TOKEN_KEYS: u64 = 0x1403C7AD0;
const STEAL_RANDOM_TECH_BONUS_KEYS: u64 = 0x14033CAD0;
const TELEPORT_ARMIES_KEYS: u64 = 0x140334F10;
const SET_STATE_PROVINCE_CONTROLLER_KEYS: u64 = 0x140334B70;
const CREATE_DYNAMIC_COUNTRY_KEYS: u64 = 0x1403C9D60;
const ADD_RESISTANCE_TARGET_KEYS: u64 = 0x140333F20;
const RANDOM_COUNTRY_WITH_ORIGINAL_TAG_KEYS: u64 = 0x1403CC5D0;
const EVERY_COUNTRY_WITH_ORIGINAL_TAG_KEYS: u64 = 0x1403CB280;
const ADD_DECRYPTION_KEYS: u64 = 0x1403C7990;
const DELETE_UNITS_KEYS: u64 = 0x1403CAF10;
const FORCE_ENABLE_RESISTANCE_KEYS: u64 = 0x140334600;
const CREATE_SHIP_KEYS: u64 = 0x1403CA670;
const ADD_INTEL_KEYS: u64 = 0x1403C79E0;
const SET_COUNTRY_LEADER_NAME_KEYS: u64 = 0x140309AA0;
const ADD_COLLABORATION_KEYS: u64 = 0x1403C7800;
const EXECUTE_OPERATION_COORDINATED_STRIKE_KEYS: u64 = 0x1403CB400;
const ADD_DOCTRINE_COST_REDUCTION_KEYS: u64 = 0x14033C050;
const ADD_CORPS_COMMANDER_ROLE_KEYS: u64 = 0x1403C6580;
const ADD_COUNTRY_LEADER_ROLE_KEYS: u64 = 0x1403C7830;
const ADD_ADVISOR_ROLE_KEYS: u64 = 0x1403C76B0;
const EVERY_COLLECTION_ELEMENT_KEYS: u64 = 0x1404E58F0;
const CREATE_ENTITY_KEYS: u64 = 0x1403C9DA0;
const SET_ENTITY_POSITION_KEYS: u64 = 0x1403CDE00;
const FORCE_UPDATE_MAP_MODE_KEYS: u64 = 0x1403CBB40;
const DAMAGE_UNITS_KEYS: u64 = 0x1403CAB00;
const UNCOMPLETE_NATIONAL_FOCUS_KEYS: u64 = 0x1403CF160;
const BUILD_RAILWAY_KEYS: u64 = 0x1403C95A0;
const SET_ENTITY_ANIMATION_KEYS: u64 = 0x1403CDD50;
const SET_POWER_BALANCE_KEYS: u64 = 0x1404a5ee0;
const REMOVE_POWER_BALANCE_KEYS: u64 = 0x1404a5e60;
const ADD_POWER_BALANCE_VALUE_KEYS: u64 = 0x1404a5df0;
const SET_DIVISION_TEMPLATE_CAP_KEYS: u64 = 0x1403CE680;
const REMOVE_COUNTRY_LEADER_ROLE_KEYS: u64 = 0x1403CD420;
const GENERATE_CHARACTER_KEYS: u64 = 0x1403CBBD0;
const SET_PORTRAITS_KEYS: u64 = 0x140309DC0;
const CLEAR_RULE_CLASS: u64 = 0x1427EE810;
const SET_RULE_CLASS: u64 = 0x1427EE730;
const NAVY_LEADER_CLASS: u64 = 0x1427E82C8;
const FIELD_MARSHAL_CLASS: u64 = 0x1427E81F8;
const CORPS_COMMANDER_CLASS: u64 = 0x1427E8128;

pub(super) fn reader(address: u64, vtable: u64) -> Option<KeyReader> {
    Some(match address {
        SET_CAPITAL_KEYS => KeyReader::Keys(set_capital, Fallback::Only),
        ADD_BREAKTHROUGH_POINTS_KEYS => KeyReader::Keys(add_breakthrough_points, Fallback::Only),
        ADD_PROVINCE_MODIFIER_KEYS => KeyReader::Keys(add_province_modifier, Fallback::Only),
        RANDOM_SCIENTIST_KEYS => KeyReader::Keys(random_scientist, Fallback::Effect),
        EVERY_SCIENTIST_KEYS => KeyReader::Keys(every_state, Fallback::Effect),
        ADD_SCIENTIST_LEVEL_KEYS => KeyReader::Keys(add_scientist_level, Fallback::Only),
        ADD_SCIENTIST_XP_KEYS => KeyReader::Keys(add_scientist_xp, Fallback::Only),
        REMOVE_BUILDING_KEYS => KeyReader::Keys(remove_building, Fallback::Only),
        EVERY_COUNTRY_KEYS => KeyReader::Keys(every_state, Fallback::Effect),
        RANDOM_COUNTRY_KEYS => KeyReader::Keys(random_scientist, Fallback::Effect),
        ADD_RELATION_RULE_OVERRIDE_KEYS => {
            KeyReader::Keys(add_relation_rule_override, Fallback::One)
        }
        REMOVE_RELATION_RULE_OVERRIDE_KEYS => {
            KeyReader::Keys(remove_relation_rule_override, Fallback::One)
        }
        ADD_BREAKTHROUGH_PROGRESS_KEYS => KeyReader::Keys(add_breakthrough_points, Fallback::Only),
        REMOVE_PROVINCE_MODIFIER_KEYS => KeyReader::Keys(remove_province_modifier, Fallback::Only),
        ADD_DESIGN_TEMPLATE_BONUS_KEYS => {
            KeyReader::Keys(add_design_template_bonus, Fallback::Only)
        }
        CONSTRUCT_BUILDING_IN_RANDOM_PROVINCE_KEYS => KeyReader::Keys(no_own_fields, Fallback::One),
        SET_VARIABLE_KEYS => KeyReader::Variable,
        ADD_AI_STRATEGY_KEYS => KeyReader::Keys(no_own_fields, Fallback::AIStrategy),
        RANDOM_KEYS => KeyReader::Keys(random, Fallback::Effect),
        RANDOM_LIST_KEYS => KeyReader::Keys(random_list, Fallback::RandomListMember),
        IF_KEYS => KeyReader::Conditional,
        ADD_OPINION_MODIFIER_KEYS => KeyReader::Keys(add_opinion_modifier, Fallback::Only),
        REMOVE_OPINION_MODIFIER_KEYS => KeyReader::Keys(add_relation_modifier, Fallback::Only),
        ADD_EQUIPMENT_BONUS_KEYS => KeyReader::Keys(add_equipment_bonus, Fallback::Only),
        REDUCE_FOCUS_COMPLETION_COST_KEYS => {
            KeyReader::Keys(reduce_focus_completion_cost, Fallback::Only)
        }
        STRATEGIC_PROVINCE_LOCATION_KEYS => KeyReader::Keys(no_own_fields, Fallback::One),
        STRATEGIC_STATE_LOCATION_KEYS => KeyReader::Keys(no_own_fields, Fallback::One),
        CREATE_RAILWAY_GUN_KEYS => KeyReader::Keys(create_railway_gun, Fallback::Only),
        SHARED_DEFINITION_KEYS => match vtable {
            CLEAR_RULE_CLASS | SET_RULE_CLASS => {
                KeyReader::Keys(no_own_fields, Fallback::Reader(Helper::Rule))
            }
            NAVY_LEADER_CLASS | FIELD_MARSHAL_CLASS | CORPS_COMMANDER_CLASS => {
                KeyReader::Keys(no_own_fields, Fallback::Reader(Helper::UnitLeaderData))
            }
            _ => return None,
        },
        CREATE_COUNTRY_LEADER_KEYS => KeyReader::Keys(create_country_leader, Fallback::Effect),
        SET_FACTION_MEMBER_UPGRADE_MIN_KEYS => {
            KeyReader::Keys(set_faction_member_upgrade_min, Fallback::Only)
        }
        CREATE_UNIT_KEYS => KeyReader::Keys(create_unit, Fallback::Only),
        ADD_EQUIPMENT_SUBSIDY_KEYS => KeyReader::Keys(add_equipment_subsidy, Fallback::Effect),
        ADD_TRAIT_KEYS => KeyReader::Keys(add_trait, Fallback::Effect),
        RANDOM_STATE_DIVISION_KEYS => KeyReader::Keys(random_scientist, Fallback::Effect),
        EVERY_STATE_DIVISION_KEYS => KeyReader::Keys(every_state, Fallback::Effect),
        ADD_HISTORY_ENTRY_KEYS => KeyReader::Keys(add_history_entry, Fallback::One),
        CHANGE_DIVISION_TEMPLATE_KEYS => KeyReader::Keys(change_division_template, Fallback::One),
        SET_POLITICAL_PARTY_KEYS => KeyReader::Keys(set_political_party, Fallback::Only),
        SET_POLITICS_KEYS => KeyReader::Keys(set_politics, Fallback::Only),
        SET_DIVISION_FORCE_ALLOW_RECRUITING_KEYS => {
            KeyReader::Keys(set_division_force_allow_recruiting, Fallback::One)
        }
        SET_TECHNOLOGY_KEYS => KeyReader::Keys(set_technology, Fallback::One),
        DELETE_UNIT_KEYS => KeyReader::Keys(delete_unit, Fallback::Only),
        ADD_RANDOM_VALID_TRAIT_FROM_UNIT_KEYS => KeyReader::Keys(set_portraits, Fallback::Effect),
        ADD_UNIT_MEDAL_TO_LATEST_ENTRY_KEYS => {
            KeyReader::Keys(add_unit_medal_to_latest_entry, Fallback::One)
        }
        SET_PARTY_RULE_KEYS => KeyReader::Keys(set_party_rule, Fallback::Reader(Helper::Rule)),
        ANNEX_COUNTRY_KEYS => KeyReader::Keys(annex_country, Fallback::Only),
        ADD_BUILDING_CONSTRUCTION_KEYS => {
            KeyReader::Keys(add_building_construction, Fallback::Only)
        }
        RANDOM_STATE_KEYS => KeyReader::Keys(random_state, Fallback::Effect),
        EVERY_STATE_KEYS => KeyReader::Keys(every_state, Fallback::Effect),
        ADD_FACTION_GOAL_SLOT_KEYS => KeyReader::Keys(add_faction_goal_slot, Fallback::Effect),
        START_CIVIL_WAR_KEYS => KeyReader::Keys(start_civil_war, Fallback::Effect),
        ADD_TECH_BONUS_KEYS => KeyReader::Keys(add_tech_bonus, Fallback::Effect),
        DIPLOMATIC_RELATION_KEYS => KeyReader::Keys(diplomatic_relation, Fallback::Effect),
        ADD_RESOURCE_KEYS => KeyReader::Keys(add_resource, Fallback::Only),
        ADD_TIMED_IDEA_KEYS => KeyReader::Keys(add_timed_idea, Fallback::Effect),
        CREATE_IMPORT_KEYS => KeyReader::Keys(create_import, Fallback::Only),
        ADD_NAMED_THREAT_KEYS => KeyReader::Keys(add_named_threat, Fallback::Effect),
        SET_PARTY_NAME_KEYS => KeyReader::Keys(set_party_name, Fallback::Only),
        ADD_TO_WAR_KEYS => KeyReader::Keys(add_to_war, Fallback::Only),
        CREATE_EQUIPMENT_VARIANT_KEYS => KeyReader::Keys(
            create_equipment_variant,
            Fallback::Reader(Helper::CreateEquipmentVariantSpec),
        ),
        ADD_EQUIPMENT_PRODUCTION_KEYS => KeyReader::Keys(add_equipment_production, Fallback::Only),
        ADD_ACE_KEYS => KeyReader::Keys(add_ace, Fallback::Only),
        ADD_EQUIPMENT_TO_STOCKPILE_KEYS => {
            KeyReader::Keys(add_equipment_to_stockpile, Fallback::Only)
        }
        DAMAGE_BUILDING_KEYS => KeyReader::Keys(damage_building, Fallback::Only),
        ADD_AUTONOMY_SCORE_KEYS => KeyReader::Keys(add_autonomy_ratio, Fallback::Only),
        MODIFY_TECH_SHARING_BONUS_KEYS => {
            KeyReader::Keys(modify_tech_sharing_bonus, Fallback::Only)
        }
        ADD_AUTONOMY_RATIO_KEYS => KeyReader::Keys(add_autonomy_ratio, Fallback::Only),
        TRANSFER_SHIP_KEYS => KeyReader::Keys(transfer_ship, Fallback::Only),
        ADD_STATE_MODIFIER_KEYS => KeyReader::Keys(add_state_modifier, Fallback::Only),
        SET_PROVINCE_NAME_KEYS => KeyReader::Keys(set_province_name, Fallback::Only),
        CREATE_PRODUCTION_LICENSE_KEYS => {
            KeyReader::Keys(create_production_license, Fallback::Only)
        }
        ADD_RELATION_MODIFIER_KEYS => KeyReader::Keys(add_relation_modifier, Fallback::One),
        RANDOM_UNIT_LEADER_KEYS => KeyReader::Keys(random_unit_leader, Fallback::Effect),
        EVERY_UNIT_LEADER_KEYS => KeyReader::Keys(every_unit_leader, Fallback::Effect),
        TRANSFER_NAVY_KEYS => KeyReader::Keys(transfer_navy, Fallback::Only),
        DESTROY_SHIPS_KEYS => KeyReader::Keys(destroy_ships, Fallback::Effect),
        SET_CAN_BE_FIRED_IN_ADVISOR_ROLE_KEYS => {
            KeyReader::Keys(set_can_be_fired_in_advisor_role, Fallback::Effect)
        }
        RANDOM_MILITARY_INDUSTRIAL_ORGANIZATION_KEYS => {
            KeyReader::Keys(random_unit_leader, Fallback::Effect)
        }
        MODIFY_BUILDING_RESOURCES_KEYS => {
            KeyReader::Keys(modify_building_resources, Fallback::Only)
        }
        FREE_RANDOM_OPERATIVE_KEYS => KeyReader::Keys(free_random_operative, Fallback::Effect),
        DIVIDE_TEMP_VARIABLE_KEYS => KeyReader::DivideVariable,
        ADD_TIMED_UNIT_LEADER_TRAIT_KEYS => {
            KeyReader::Keys(add_timed_unit_leader_trait, Fallback::One)
        }
        CLAMP_VARIABLE_KEYS => KeyReader::Keys(clamp_temp_variable, Fallback::One),
        CLAMP_TEMP_VARIABLE_KEYS => KeyReader::Keys(clamp_temp_variable, Fallback::One),
        START_BORDER_WAR_KEYS => KeyReader::Keys(start_border_war, Fallback::One),
        SET_BORDER_WAR_DATA_KEYS => KeyReader::Keys(set_border_war_data, Fallback::One),
        CANCEL_BORDER_WAR_KEYS => KeyReader::Keys(cancel_border_war, Fallback::One),
        SET_DIVISION_TEMPLATE_LOCK_KEYS => {
            KeyReader::Keys(set_division_template_lock, Fallback::One)
        }
        DELETE_UNIT_TEMPLATE_AND_UNITS_KEYS => {
            KeyReader::Keys(delete_unit_template_and_units, Fallback::Only)
        }
        PRINT_VARIABLES_KEYS => KeyReader::Keys(print_variables, Fallback::One),
        ACTIVATE_TARGETED_DECISION_KEYS => {
            KeyReader::Keys(activate_targeted_decision, Fallback::One)
        }
        SET_TRUCE_KEYS => KeyReader::Keys(set_truce, Fallback::Only),
        SET_POPULARITIES_KEYS => KeyReader::Keys(no_own_fields, Fallback::Popularities),
        FINALIZE_BORDER_WAR_KEYS => KeyReader::Keys(finalize_border_war, Fallback::One),
        BECOME_EXILED_IN_KEYS => KeyReader::Keys(become_exiled_in, Fallback::Only),
        GIVE_RESOURCE_RIGHTS_KEYS => KeyReader::Keys(give_resource_rights, Fallback::Only),
        ADD_TO_ARRAY_KEYS => KeyReader::Keys(add_to_array, Fallback::One),
        RESIZE_ARRAY_KEYS => KeyReader::Keys(add_to_array, Fallback::One),
        FOR_EACH_LOOP_KEYS => KeyReader::Keys(for_each_loop, Fallback::Effect),
        FOR_EACH_SCOPE_LOOP_KEYS => KeyReader::Keys(for_each_scope_loop, Fallback::Effect),
        RANDOM_SCOPE_IN_ARRAY_KEYS => KeyReader::Keys(random_scope_in_array, Fallback::Effect),
        REMOVE_DYNAMIC_MODIFIER_KEYS => KeyReader::Keys(remove_dynamic_modifier, Fallback::Only),
        FOR_LOOP_EFFECT_KEYS => KeyReader::Keys(for_loop_effect, Fallback::Effect),
        WHILE_LOOP_EFFECT_KEYS => KeyReader::Keys(while_loop_effect, Fallback::Effect),
        META_EFFECT_KEYS => KeyReader::Keys(meta_effect, Fallback::One),
        ADD_VICTORY_POINTS_KEYS => KeyReader::Keys(add_victory_points, Fallback::Only),
        LAUNCH_NUKE_KEYS => KeyReader::Keys(launch_nuke, Fallback::One),
        FIND_LOWEST_IN_ARRAY_KEYS => KeyReader::Keys(find_lowest_in_array, Fallback::Effect),
        ADD_MINES_KEYS => KeyReader::Keys(add_mines, Fallback::Only),
        REMOVE_ADVISOR_ROLE_KEYS => KeyReader::Keys(remove_advisor_role, Fallback::Effect),
        ADD_POWER_BALANCE_MODIFIER_KEYS => {
            KeyReader::Keys(add_power_balance_modifier, Fallback::Only)
        }
        SET_POWER_BALANCE_GFX_KEYS => KeyReader::Keys(set_power_balance_gfx, Fallback::Only),
        ADD_DAYS_REMOVE_KEYS => KeyReader::Keys(add_days_remove, Fallback::Only),
        ADD_DAYS_MISSION_TIMEOUT_KEYS => KeyReader::Keys(add_days_mission_timeout, Fallback::Only),
        EVERY_MILITARY_INDUSTRIAL_ORGANIZATION_KEYS => {
            KeyReader::Keys(every_military_industrial_organization, Fallback::Effect)
        }
        CREATE_PURCHASE_CONTRACT_KEYS => {
            KeyReader::Keys(create_purchase_contract, Fallback::Effect)
        }
        GENERATE_SCIENTIST_CHARACTER_KEYS => {
            KeyReader::Keys(generate_scientist_character, Fallback::Only)
        }
        ADD_SCIENTIST_ROLE_KEYS => KeyReader::Keys(add_scientist_role, Fallback::Only),
        RAID_DAMAGE_UNITS_KEYS => KeyReader::Keys(raid_damage_units, Fallback::One),
        ADD_MASTERY_KEYS => KeyReader::Keys(add_mastery, Fallback::Effect),
        ADD_DAILY_MASTERY_KEYS => KeyReader::Keys(add_daily_mastery, Fallback::Effect),
        ADD_MASTERY_BONUS_KEYS => KeyReader::Keys(add_mastery_bonus, Fallback::Effect),
        CUSTOM_OVERRIDE_TOOLTIP_KEYS => KeyReader::Keys(custom_override_tooltip, Fallback::Effect),
        ADD_UNIT_BONUS_KEYS => KeyReader::Keys(add_unit_bonus, Fallback::UnitBonus),
        ADD_OPERATION_TOKEN_KEYS => KeyReader::Keys(add_operation_token, Fallback::Only),
        STEAL_RANDOM_TECH_BONUS_KEYS => KeyReader::Keys(steal_random_tech_bonus, Fallback::Effect),
        TELEPORT_ARMIES_KEYS => KeyReader::Keys(teleport_armies, Fallback::Only),
        SET_STATE_PROVINCE_CONTROLLER_KEYS => {
            KeyReader::Keys(set_state_province_controller, Fallback::Only)
        }
        CREATE_DYNAMIC_COUNTRY_KEYS => KeyReader::Keys(create_dynamic_country, Fallback::Effect),
        ADD_RESISTANCE_TARGET_KEYS => KeyReader::Keys(add_resistance_target, Fallback::Effect),
        RANDOM_COUNTRY_WITH_ORIGINAL_TAG_KEYS => {
            KeyReader::Keys(random_country_with_original_tag, Fallback::Effect)
        }
        EVERY_COUNTRY_WITH_ORIGINAL_TAG_KEYS => {
            KeyReader::Keys(every_country_with_original_tag, Fallback::Effect)
        }
        ADD_DECRYPTION_KEYS => KeyReader::Keys(add_decryption, Fallback::Effect),
        DELETE_UNITS_KEYS => KeyReader::Keys(delete_unit_template_and_units, Fallback::Effect),
        FORCE_ENABLE_RESISTANCE_KEYS => KeyReader::Keys(force_enable_resistance, Fallback::Effect),
        CREATE_SHIP_KEYS => KeyReader::Keys(create_ship, Fallback::One),
        ADD_INTEL_KEYS => KeyReader::Keys(add_intel, Fallback::Effect),
        SET_COUNTRY_LEADER_NAME_KEYS => KeyReader::Keys(set_country_leader_name, Fallback::Only),
        ADD_COLLABORATION_KEYS => KeyReader::Keys(add_collaboration, Fallback::Effect),
        EXECUTE_OPERATION_COORDINATED_STRIKE_KEYS => {
            KeyReader::Keys(execute_operation_coordinated_strike, Fallback::One)
        }
        ADD_DOCTRINE_COST_REDUCTION_KEYS => {
            KeyReader::Keys(add_doctrine_cost_reduction, Fallback::Effect)
        }
        ADD_CORPS_COMMANDER_ROLE_KEYS => {
            KeyReader::Keys(set_portraits, Fallback::Reader(Helper::UnitLeaderData))
        }
        ADD_COUNTRY_LEADER_ROLE_KEYS => KeyReader::Keys(add_country_leader_role, Fallback::Effect),
        ADD_ADVISOR_ROLE_KEYS => KeyReader::Keys(add_advisor_role, Fallback::Effect),
        EVERY_COLLECTION_ELEMENT_KEYS => {
            KeyReader::Keys(every_collection_element, Fallback::Effect)
        }
        CREATE_ENTITY_KEYS => KeyReader::Keys(create_entity, Fallback::Only),
        SET_ENTITY_POSITION_KEYS => KeyReader::Keys(set_entity_position, Fallback::Only),
        FORCE_UPDATE_MAP_MODE_KEYS => KeyReader::Keys(force_update_map_mode, Fallback::Only),
        DAMAGE_UNITS_KEYS => KeyReader::Keys(damage_units, Fallback::Only),
        UNCOMPLETE_NATIONAL_FOCUS_KEYS => {
            KeyReader::Keys(uncomplete_national_focus, Fallback::Effect)
        }
        BUILD_RAILWAY_KEYS => KeyReader::Keys(build_railway, Fallback::Effect),
        SET_ENTITY_ANIMATION_KEYS => KeyReader::Keys(set_entity_animation, Fallback::Only),
        SET_POWER_BALANCE_KEYS => KeyReader::Keys(set_power_balance, Fallback::Only),
        REMOVE_POWER_BALANCE_KEYS => KeyReader::Keys(remove_power_balance, Fallback::Only),
        ADD_POWER_BALANCE_VALUE_KEYS => KeyReader::Keys(add_power_balance_value, Fallback::Only),
        SET_DIVISION_TEMPLATE_CAP_KEYS => KeyReader::Keys(set_division_template_cap, Fallback::One),
        REMOVE_COUNTRY_LEADER_ROLE_KEYS => {
            KeyReader::Keys(remove_country_leader_role, Fallback::Effect)
        }
        GENERATE_CHARACTER_KEYS => KeyReader::GenerateCharacter,
        SET_PORTRAITS_KEYS => KeyReader::Keys(set_portraits, Fallback::Reader(Helper::Portraits)),
        _ => return None,
    })
}

pub(super) fn helper(reader: Helper) -> KeyReader {
    match reader {
        Helper::Advisor => KeyReader::Keys(advisor, Fallback::Only),
        Helper::BorderWarSide => KeyReader::Keys(border_war_side, Fallback::One),
        Helper::CharacterTemplate => KeyReader::Keys(character_template, Fallback::Only),
        Helper::Cid => KeyReader::Keys(cid, Fallback::One),
        Helper::CountryLeader => KeyReader::Keys(country_leader, Fallback::Only),
        Helper::CreateEquipmentVariantSpec => {
            KeyReader::Keys(create_equipment_variant_spec, Fallback::Only)
        }
        Helper::EquipmentBonus => KeyReader::Keys(equipment_bonus, Fallback::EquipmentStatRegistry),
        Helper::EquipmentBonusEntries => {
            KeyReader::Keys(no_own_fields, Fallback::Object(Helper::EquipmentBonus))
        }
        Helper::EquipmentVariant => KeyReader::Keys(equipment_variant, Fallback::Only),
        Helper::Modifier => KeyReader::Keys(no_own_fields, Fallback::ModifierRegistry),
        Helper::Mtth => KeyReader::Keys(no_own_fields, Fallback::MeanTimeToHappen),
        Helper::OptionalEquipmentAsset => KeyReader::Keys(optional_equipment_asset, Fallback::Only),
        Helper::PoliticalParties => KeyReader::Keys(no_own_fields, Fallback::PoliticalPartyScript),
        Helper::Portraits => KeyReader::Keys(portraits, Fallback::Only),
        Helper::PortraitSizes => KeyReader::Keys(portrait_sizes, Fallback::Only),
        Helper::ProvinceTarget => KeyReader::Keys(province_target, Fallback::Only),
        Helper::PurchaseEquipment => KeyReader::Keys(add_equipment_to_stockpile, Fallback::Only),
        Helper::RandomListMember => KeyReader::Keys(random_list_member, Fallback::Effect),
        Helper::Rule => KeyReader::Keys(rule, Fallback::One),
        Helper::RuleOverride => KeyReader::Keys(no_own_fields, Fallback::One),
        Helper::ScalarFields => KeyReader::Keys(no_own_fields, Fallback::One),
        Helper::ScientistSkills => KeyReader::Keys(no_own_fields, Fallback::One),
        Helper::ScientistTemplate => KeyReader::Keys(scientist_template, Fallback::Only),
        Helper::UnitLeaderData => KeyReader::Keys(
            unit_leader_data,
            Fallback::Reader(Helper::UnitLeaderTemplate),
        ),
        Helper::UnitLeaderTemplate => KeyReader::Keys(unit_leader_template, Fallback::Only),
        Helper::UnitOfficerData => KeyReader::Keys(unit_officer_data, Fallback::Only),
        Helper::VariantModules => KeyReader::Keys(no_own_fields, Fallback::One),
        Helper::VariantUpgrades => KeyReader::Keys(no_own_fields, Fallback::EquipmentUpgrade),
    }
}

fn every_state(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"tooltip") => ValueReader::Localization,
        Some(b"limit") => ValueReader::Limit,
        Some(b"visible_when_empty" | b"display_individual_scopes" | b"random_select_amount") => {
            ValueReader::One
        }
        _ => return None,
    })
}

fn add_opinion_modifier(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"target" | b"modifier" | b"days") => ValueReader::One,
        _ => return None,
    })
}

fn set_technology(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"popup") => ValueReader::One,
        _ => return None,
    })
}

fn add_building_construction(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"type" | b"level" | b"instant_build") => ValueReader::One,
        Some(b"province") => ValueReader::Province,
        _ => return None,
    })
}

fn random_state(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"tooltip") => ValueReader::Localization,
        Some(b"limit") => ValueReader::Limit,
        Some(b"prioritize") => ValueReader::ValueList,
        Some(b"visible_when_empty") => ValueReader::One,
        _ => return None,
    })
}

fn add_tech_bonus(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"name" | b"category" | b"technology" | b"bonus" | b"ahead_reduction" | b"uses") => {
            ValueReader::One
        }
        _ => return None,
    })
}

fn add_timed_idea(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"idea" | b"months" | b"days" | b"years") => ValueReader::One,
        _ => return None,
    })
}

fn set_party_name(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"name" | b"ideology" | b"long_name") => ValueReader::One,
        _ => return None,
    })
}

fn add_equipment_to_stockpile(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"type" | b"amount" | b"producer" | b"variant_name") => ValueReader::One,
        _ => return None,
    })
}

fn clamp_temp_variable(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"min" | b"max" | b"var") => ValueReader::One,
        _ => return None,
    })
}

fn add_to_array(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"size" | b"index" | b"value" | b"array") => ValueReader::One,
        _ => return None,
    })
}

fn remove_dynamic_modifier(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"modifier" | b"days" | b"scope") => ValueReader::One,
        _ => return None,
    })
}

fn set_country_leader_name(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"name" | b"desc" | b"ideology" | b"portrait") => ValueReader::One,
        _ => return None,
    })
}

fn add_timed_unit_leader_trait(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"days" | b"trait") => ValueReader::One,
        _ => return None,
    })
}

fn add_trait(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"ideology" | b"trait" | b"character") => ValueReader::One,
        Some(b"slot") => ValueReader::Current,
        _ => return None,
    })
}

fn set_portraits(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"character") => ValueReader::One,
        _ => return None,
    })
}

fn random_scientist(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"tooltip") => ValueReader::Localization,
        Some(b"limit") => ValueReader::Limit,
        Some(b"visible_when_empty") => ValueReader::One,
        _ => return None,
    })
}

fn add_equipment_subsidy(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"seller_trigger" | b"cic" | b"equipment_type") => ValueReader::One,
        Some(b"seller_tags") => ValueReader::ValueList,
        _ => return None,
    })
}

fn create_purchase_contract(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"equipment") => ValueReader::Object(Helper::PurchaseEquipment),
        Some(b"civilian_factories" | b"seller" | b"buyer") => ValueReader::One,
        _ => return None,
    })
}

fn add_state_modifier(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"modifier") => ValueReader::Object(Helper::Modifier),
        _ => return None,
    })
}

fn add_resistance_target(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"id" | b"tooltip" | b"amount" | b"days" | b"occupied" | b"occupier") => {
            ValueReader::One
        }
        _ => return None,
    })
}

fn add_province_modifier(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"days") => ValueReader::One,
        Some(b"static_modifiers") => ValueReader::ValueList,
        Some(b"province") => ValueReader::ObjectIfOpen(Helper::ProvinceTarget),
        _ => return None,
    })
}

fn no_own_fields(_: Option<&[u8]>) -> Option<ValueReader> {
    None
}

fn force_enable_resistance(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"clear" | b"occupied" | b"occupier") => ValueReader::One,
        _ => return None,
    })
}

fn damage_building(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"damage" | b"repair_speed_modifier" | b"type" | b"state" | b"province") => {
            ValueReader::One
        }
        Some(b"tags") => ValueReader::ValueListIfOpenElseCurrent,
        _ => return None,
    })
}

fn remove_building(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"type" | b"province" | b"state" | b"level") => ValueReader::One,
        Some(b"tags") => ValueReader::ValueListIfOpenElseCurrent,
        _ => return None,
    })
}

fn set_state_province_controller(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"controller") => ValueReader::One,
        Some(b"limit") => ValueReader::Trigger,
        _ => return None,
    })
}

fn remove_province_modifier(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"static_modifiers") => ValueReader::ValueList,
        Some(b"province") => ValueReader::ObjectIfOpen(Helper::ProvinceTarget),
        _ => return None,
    })
}

fn teleport_armies(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"limit") => ValueReader::Trigger,
        Some(b"to_province" | b"to_state" | b"to_state_array") => ValueReader::One,
        _ => return None,
    })
}

fn add_doctrine_cost_reduction(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"name" | b"category" | b"technology" | b"uses" | b"cost_reduction") => {
            ValueReader::One
        }
        _ => return None,
    })
}

fn modify_tech_sharing_bonus(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"id" | b"bonus") => ValueReader::One,
        _ => return None,
    })
}

fn steal_random_tech_bonus(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(
            b"dynamic" | b"name" | b"target" | b"category" | b"bonus" | b"folder"
            | b"ahead_reduction" | b"uses" | b"instant" | b"base_bonus",
        ) => ValueReader::One,
        _ => return None,
    })
}

fn find_lowest_in_array(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"array" | b"index" | b"value") => ValueReader::One,
        _ => return None,
    })
}

fn add_victory_points(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"value" | b"province") => ValueReader::One,
        _ => return None,
    })
}

fn add_ace(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"name" | b"type" | b"is_female" | b"surname" | b"callsign") => ValueReader::One,
        _ => return None,
    })
}

fn add_advisor_role(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"advisor") => ValueReader::Object(Helper::Advisor),
        Some(b"character" | b"activate") => ValueReader::One,
        _ => return None,
    })
}

fn add_autonomy_ratio(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"localization" | b"value") => ValueReader::One,
        _ => return None,
    })
}

fn add_collaboration(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"target" | b"value") => ValueReader::One,
        _ => return None,
    })
}

fn add_country_leader_role(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"promote_leader" | b"character") => ValueReader::One,
        Some(b"country_leader") => ValueReader::Object(Helper::CountryLeader),
        _ => return None,
    })
}

fn add_days_mission_timeout(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"days" | b"mission") => ValueReader::One,
        _ => return None,
    })
}

fn add_days_remove(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"days" | b"decision") => ValueReader::One,
        _ => return None,
    })
}

fn add_decryption(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"target" | b"amount" | b"ratio") => ValueReader::One,
        _ => return None,
    })
}

fn add_intel(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"target" | b"civilian_intel" | b"army_intel" | b"navy_intel" | b"airforce_intel") => {
            ValueReader::One
        }
        _ => return None,
    })
}

fn add_mines(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"amount" | b"region") => ValueReader::One,
        _ => return None,
    })
}

fn add_named_threat(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"name" | b"threat") => ValueReader::One,
        _ => return None,
    })
}

fn add_operation_token(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"tag" | b"token") => ValueReader::One,
        _ => return None,
    })
}

fn add_relation_rule_override(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"usage_desc" | b"target" | b"trigger") => ValueReader::One,
        _ => return None,
    })
}

fn add_resource(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"type") => ValueReader::Resource,
        Some(b"amount" | b"state" | b"days" | b"show_state_in_tooltip") => ValueReader::One,
        _ => return None,
    })
}

fn add_scientist_role(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"scientist") => ValueReader::Object(Helper::ScientistTemplate),
        Some(b"character") => ValueReader::One,
        _ => return None,
    })
}

fn add_to_war(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"single_target_only" | b"hostility_reason") => ValueReader::One,
        Some(b"enemy" | b"targeted_alliance") => ValueReader::ScopeSelectorChain,
        _ => return None,
    })
}

fn add_unit_bonus(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"name") => ValueReader::One,
        _ => return None,
    })
}

fn annex_country(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"target" | b"transfer_troops") => ValueReader::One,
        _ => return None,
    })
}

fn become_exiled_in(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"target" | b"legitimacy") => ValueReader::One,
        _ => return None,
    })
}

fn build_railway(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"controller_priority") => ValueReader::Object(Helper::Mtth),
        Some(b"path") => ValueReader::ValueList,
        Some(
            b"level"
            | b"fallback"
            | b"target_state"
            | b"start_state"
            | b"start_province"
            | b"target_province"
            | b"build_only_on_allied",
        ) => ValueReader::One,
        _ => return None,
    })
}

fn cancel_border_war(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"attacker" | b"defender" | b"dont_fire_events") => ValueReader::One,
        _ => return None,
    })
}

fn change_division_template(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"division_template") => ValueReader::One,
        _ => return None,
    })
}

fn create_dynamic_country(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"original_tag" | b"copy_tag") => ValueReader::One,
        _ => return None,
    })
}

fn create_entity(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(
            b"id" | b"entity" | b"visible" | b"var" | b"min_zoom" | b"scale" | b"rotation" | b"x"
            | b"y" | b"z" | b"province" | b"state",
        ) => ValueReader::One,
        _ => return None,
    })
}

fn create_import(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"amount" | b"start_date" | b"factories" | b"resource" | b"exporter") => {
            ValueReader::One
        }
        _ => return None,
    })
}

fn create_railway_gun(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"name" | b"equipment" | b"location") => ValueReader::One,
        _ => return None,
    })
}

fn create_ship(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"name" | b"type" | b"creator" | b"equipment_variant") => ValueReader::One,
        _ => return None,
    })
}

fn create_unit(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(
            b"id"
            | b"division"
            | b"divisional_commander_xp"
            | b"owner"
            | b"count"
            | b"prioritize_location"
            | b"allow_spawning_on_enemy_provs",
        ) => ValueReader::One,
        Some(b"country_score") => ValueReader::Object(Helper::Mtth),
        Some(b"officer") => ValueReader::Object(Helper::UnitOfficerData),
        _ => return None,
    })
}

fn damage_units(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"limit") => ValueReader::Trigger,
        Some(
            b"state" | b"ratio" | b"province" | b"army" | b"navy" | b"region"
            | b"division_template" | b"damage" | b"str_damage" | b"org_damage",
        ) => ValueReader::One,
        _ => return None,
    })
}

fn delete_unit_template_and_units(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"division_template" | b"disband") => ValueReader::One,
        _ => return None,
    })
}

fn delete_unit(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"id" | b"state" | b"division_template" | b"disband") => ValueReader::One,
        _ => return None,
    })
}

fn destroy_ships(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"type" | b"count") => ValueReader::One,
        _ => return None,
    })
}

fn diplomatic_relation(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"country" | b"relation" | b"active") => ValueReader::One,
        _ => return None,
    })
}

fn every_country_with_original_tag(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"tooltip") => ValueReader::Localization,
        Some(b"limit") => ValueReader::Limit,
        Some(
            b"original_tag_to_check"
            | b"visible_when_empty"
            | b"display_individual_scopes"
            | b"random_select_amount",
        ) => ValueReader::One,
        _ => return None,
    })
}

fn execute_operation_coordinated_strike(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"amount") => ValueReader::One,
        _ => return None,
    })
}

fn finalize_border_war(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(
            b"attacker" | b"defender" | b"attacker_win" | b"defender_win" | b"dont_fire_events",
        ) => ValueReader::One,
        _ => return None,
    })
}

fn for_each_loop(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"index" | b"value" | b"break" | b"array") => ValueReader::One,
        _ => return None,
    })
}

fn for_each_scope_loop(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"tooltip" | b"break" | b"array") => ValueReader::One,
        _ => return None,
    })
}

fn for_loop_effect(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"break" | b"start" | b"end" | b"value" | b"add") => ValueReader::One,
        Some(b"compare") => ValueReader::Comparison,
        _ => return None,
    })
}

fn force_update_map_mode(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"limit") => ValueReader::Trigger,
        Some(b"mapmode") => ValueReader::One,
        _ => return None,
    })
}

fn free_random_operative(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"all" | b"captured_by") => ValueReader::One,
        _ => return None,
    })
}

fn generate_scientist_character(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"traits") => ValueReader::RawList,
        Some(b"portrait" | b"portrait_tag_override") => ValueReader::One,
        Some(b"skills") => ValueReader::Object(Helper::ScientistSkills),
        Some(b"gender") => ValueReader::Gender,
        _ => return None,
    })
}

fn give_resource_rights(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"state" | b"receiver") => ValueReader::One,
        Some(b"resources") => ValueReader::RawList,
        _ => return None,
    })
}

fn launch_nuke(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"state" | b"controller" | b"province" | b"use_nuke" | b"nuke_type") => {
            ValueReader::One
        }
        _ => return None,
    })
}

fn create_country_leader(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(
            b"female" | b"id" | b"name" | b"picture" | b"desc" | b"ideology" | b"expire" | b"token",
        ) => ValueReader::One,
        Some(b"traits") => ValueReader::ValueListOrSkip,
        _ => return None,
    })
}

fn meta_effect(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"text") => ValueReader::RawText,
        Some(b"debug") => ValueReader::One,
        _ => return None,
    })
}

fn modify_building_resources(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"amount" | b"resource") => ValueReader::One,
        Some(b"building") => ValueReader::Building,
        _ => return None,
    })
}

fn print_variables(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"file" | b"text" | b"append" | b"print_global") => ValueReader::One,
        Some(b"var_list") => ValueReader::ValueList,
        _ => return None,
    })
}

fn random_country_with_original_tag(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"tooltip") => ValueReader::Localization,
        Some(b"limit") => ValueReader::Limit,
        Some(b"original_tag_to_check" | b"visible_when_empty") => ValueReader::One,
        _ => return None,
    })
}

fn random_scope_in_array(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"limit") => ValueReader::Limit,
        Some(b"array") => ValueReader::One,
        _ => return None,
    })
}

fn reduce_focus_completion_cost(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"cost") => ValueReader::One,
        Some(b"focus") => ValueReader::ValueListIfOpen,
        _ => return None,
    })
}

fn add_relation_modifier(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"target" | b"modifier") => ValueReader::One,
        _ => return None,
    })
}

fn remove_relation_rule_override(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"target" | b"trigger") => ValueReader::One,
        _ => return None,
    })
}

fn remove_advisor_role(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"slot") => ValueReader::Current,
        Some(b"character") => ValueReader::One,
        _ => return None,
    })
}

fn remove_country_leader_role(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"ideology" | b"character") => ValueReader::One,
        _ => return None,
    })
}

fn set_province_name(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"id" | b"name") => ValueReader::One,
        _ => return None,
    })
}

fn set_border_war_data(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(
            b"attacker"
            | b"defender"
            | b"modifier"
            | b"combat_width"
            | b"max_units"
            | b"max_attackers"
            | b"max_defenders"
            | b"attacker_modifier"
            | b"defender_modifier"
            | b"change_state_after_war",
        ) => ValueReader::One,
        _ => return None,
    })
}

fn set_can_be_fired_in_advisor_role(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"value" | b"slot" | b"character") => ValueReader::One,
        _ => return None,
    })
}

fn set_capital(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"state" | b"remember_old_capital") => ValueReader::One,
        _ => return None,
    })
}

fn set_division_template_lock(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"division_template" | b"is_locked") => ValueReader::One,
        _ => return None,
    })
}

fn set_entity_animation(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"id" | b"animation") => ValueReader::One,
        _ => return None,
    })
}

fn set_entity_position(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"id" | b"x" | b"y" | b"z" | b"state" | b"province") => ValueReader::One,
        _ => return None,
    })
}

fn set_party_rule(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"ideology") => ValueReader::One,
        _ => return None,
    })
}

fn set_political_party(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"ideology" | b"popularity") => ValueReader::One,
        _ => return None,
    })
}

fn set_politics(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(
            b"name"
            | b"last_election"
            | b"election_frequency"
            | b"ruling_party"
            | b"elections_allowed"
            | b"long_name",
        ) => ValueReader::One,
        Some(b"parties") => ValueReader::Object(Helper::PoliticalParties),
        _ => return None,
    })
}

fn set_division_template_cap(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"division_template" | b"division_cap") => ValueReader::One,
        _ => return None,
    })
}

fn set_division_force_allow_recruiting(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"division_template" | b"force_allow_recruiting") => ValueReader::One,
        _ => return None,
    })
}

fn set_truce(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"target" | b"days") => ValueReader::One,
        _ => return None,
    })
}

fn start_border_war(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"attacker" | b"defender") => ValueReader::ObjectOrSkip(Helper::BorderWarSide),
        Some(b"combat_width" | b"change_state_after_war" | b"minimum_duration_in_days") => {
            ValueReader::One
        }
        _ => return None,
    })
}

fn start_civil_war(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(
            b"size"
            | b"capital"
            | b"ideology"
            | b"ruling_party"
            | b"keep_all_characters"
            | b"army_ratio"
            | b"navy_ratio"
            | b"air_ratio"
            | b"keep_political_party_members"
            | b"keep_political_leader",
        ) => ValueReader::One,
        Some(b"states") => ValueReader::ValueListExceptAll,
        Some(b"keep_unit_leaders") => ValueReader::ValueListOrSkip,
        Some(b"states_filter" | b"keep_unit_leaders_trigger" | b"keep_scientists_trigger") => {
            ValueReader::Trigger
        }
        _ => return None,
    })
}

fn activate_targeted_decision(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"target" | b"decision") => ValueReader::One,
        _ => return None,
    })
}

fn transfer_navy(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"target" | b"is_government_in_exile") => ValueReader::One,
        _ => return None,
    })
}

fn transfer_ship(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"type" | b"target" | b"prefer_name" | b"exclude_refitting") => ValueReader::One,
        _ => return None,
    })
}

fn uncomplete_national_focus(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"focus" | b"uncomplete_children" | b"refund_political_power") => ValueReader::One,
        _ => return None,
    })
}

fn while_loop_effect(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"limit") => ValueReader::Limit,
        Some(b"break") => ValueReader::One,
        _ => return None,
    })
}

fn add_history_entry(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"key" | b"subject" | b"allow") => ValueReader::One,
        _ => return None,
    })
}

fn add_unit_medal_to_latest_entry(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"unit_medals") => ValueReader::One,
        _ => return None,
    })
}

fn add_power_balance_value(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"value") => ValueReader::One,
        Some(b"tooltip_side" | b"id") => ValueReader::Current,
        _ => return None,
    })
}

fn remove_power_balance(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"id") => ValueReader::Current,
        _ => return None,
    })
}

fn add_power_balance_modifier(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"modifier") => ValueReader::One,
        Some(b"id") => ValueReader::Current,
        _ => return None,
    })
}

fn set_power_balance(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"left_side" | b"right_side" | b"id") => ValueReader::Current,
        Some(b"set_value" | b"set_default") => ValueReader::One,
        _ => return None,
    })
}

fn set_power_balance_gfx(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"side" | b"id") => ValueReader::Current,
        Some(b"gfx") => ValueReader::One,
        _ => return None,
    })
}

fn add_equipment_production(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(
            b"efficency"
            | b"efficiency"
            | b"name"
            | b"amount"
            | b"progress"
            | b"requested_factories"
            | b"industrial_manufacturer",
        ) => ValueReader::One,
        Some(b"equipment") => ValueReader::Object(Helper::EquipmentVariant),
        _ => return None,
    })
}

fn add_breakthrough_points(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"value") => ValueReader::One,
        Some(b"specialization") => ValueReader::Specialization,
        _ => return None,
    })
}

fn add_scientist_level(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"specialization" | b"level") => ValueReader::One,
        _ => return None,
    })
}

fn add_scientist_xp(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"specialization" | b"experience") => ValueReader::One,
        _ => return None,
    })
}

fn add_design_template_bonus(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"name" | b"equipment" | b"uses" | b"cost_factor") => ValueReader::One,
        _ => return None,
    })
}

fn add_equipment_bonus(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"name" | b"project" | b"prefix") => ValueReader::One,
        Some(b"bonus") => ValueReader::Object(Helper::EquipmentBonusEntries),
        _ => return None,
    })
}

fn create_equipment_variant(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"type" | b"allow_without_tech" | b"design_team") => ValueReader::One,
        _ => return None,
    })
}

fn create_production_license(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"target" | b"new_prioritised" | b"cost_factor") => ValueReader::One,
        Some(b"equipment") => ValueReader::Object(Helper::EquipmentVariant),
        _ => return None,
    })
}

fn raid_damage_units(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"ratio" | b"damage" | b"str_damage" | b"org_damage" | b"plane_loss") => {
            ValueReader::One
        }
        _ => return None,
    })
}

fn add_faction_goal_slot(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"category" | b"value") => ValueReader::One,
        _ => return None,
    })
}

fn set_faction_member_upgrade_min(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"upgrade") => ValueReader::One,
        _ => return None,
    })
}

fn every_collection_element(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"input") => ValueReader::Collection,
        _ => return None,
    })
}

fn add_daily_mastery(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(
            b"name" | b"amount" | b"days" | b"track" | b"folder" | b"grand_doctrine"
            | b"sub_doctrine" | b"track_index",
        ) => ValueReader::One,
        _ => return None,
    })
}

fn add_mastery_bonus(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(
            b"name" | b"days" | b"bonus" | b"track" | b"folder" | b"grand_doctrine"
            | b"sub_doctrine" | b"track_index",
        ) => ValueReader::One,
        _ => return None,
    })
}

fn add_mastery(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(
            b"amount" | b"track" | b"folder" | b"grand_doctrine" | b"sub_doctrine" | b"track_index",
        ) => ValueReader::One,
        _ => return None,
    })
}

fn random(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"chance") => ValueReader::One,
        _ => return None,
    })
}

fn random_list(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"log" | b"seed") => ValueReader::One,
        _ => return None,
    })
}

fn custom_override_tooltip(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"tooltip") => ValueReader::Localization,
        _ => return None,
    })
}

fn every_unit_leader(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(
            b"include_invisible"
            | b"visible_when_empty"
            | b"display_individual_scopes"
            | b"random_select_amount",
        ) => ValueReader::One,
        Some(b"tooltip") => ValueReader::Localization,
        Some(b"limit") => ValueReader::Limit,
        _ => return None,
    })
}

fn random_unit_leader(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"include_invisible" | b"visible_when_empty") => ValueReader::One,
        Some(b"tooltip") => ValueReader::Localization,
        Some(b"limit") => ValueReader::Limit,
        _ => return None,
    })
}

fn every_military_industrial_organization(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"tooltip") => ValueReader::Localization,
        Some(b"limit") => ValueReader::Limit,
        Some(
            b"include_invisible"
            | b"visible_when_empty"
            | b"display_individual_scopes"
            | b"random_select_amount",
        ) => ValueReader::One,
        _ => return None,
    })
}

fn advisor(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"name" | b"slot" | b"idea_token") => ValueReader::Current,
        Some(
            b"desc"
            | b"picture"
            | b"cost"
            | b"removal_cost"
            | b"command_power"
            | b"always_show_on_actions_tooltip"
            | b"can_be_fired",
        ) => ValueReader::One,
        Some(b"allowed" | b"visible" | b"available" | b"do_effect") => ValueReader::Trigger,
        Some(b"on_add" | b"on_remove") => ValueReader::Effect,
        Some(b"ledger") => ValueReader::Ledger,
        Some(b"modifier") => ValueReader::Object(Helper::Modifier),
        Some(b"ai_will_do") => ValueReader::Object(Helper::Mtth),
        Some(b"traits") => ValueReader::ValueListOrSkip,
        Some(b"research_bonus") => ValueReader::ObjectOrSkip(Helper::ScalarFields),
        _ => return None,
    })
}

fn border_war_side(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(
            b"state" | b"province" | b"num_provinces" | b"modifier" | b"dig_in_factor"
            | b"num_units" | b"on_win" | b"on_lose" | b"on_cancel" | b"terrain_factor",
        ) => ValueReader::One,
        Some(b"leader_score") => ValueReader::Object(Helper::Mtth),
        _ => return None,
    })
}

fn character_template(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"name" | b"nationality" | b"can_be_captured" | b"generic") => ValueReader::One,
        Some(b"gender") => ValueReader::Gender,
        Some(b"portraits") => ValueReader::Object(Helper::Portraits),
        Some(b"available" | b"allowed_civil_war" | b"remove") => ValueReader::Trigger,
        Some(b"advisor") => ValueReader::Object(Helper::Advisor),
        Some(b"country_leader") => ValueReader::Object(Helper::CountryLeader),
        Some(b"navy_leader" | b"field_marshal" | b"corps_commander") => {
            ValueReader::Object(Helper::UnitLeaderTemplate)
        }
        Some(b"scientist") => ValueReader::Object(Helper::ScientistTemplate),
        _ => return None,
    })
}

fn cid(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"id" | b"type") => ValueReader::One,
        _ => return None,
    })
}

fn country_leader(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"id" | b"desc" | b"expire") => ValueReader::One,
        Some(b"ideology") => ValueReader::Current,
        Some(b"traits") => ValueReader::ValueListOrSkip,
        _ => return None,
    })
}

fn create_equipment_variant_spec(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(
            b"name"
            | b"icon"
            | b"show_position"
            | b"mark_older_equipment_obsolete"
            | b"obsolete"
            | b"parent_version"
            | b"name_group"
            | b"role_icon_index"
            | b"model",
        ) => ValueReader::One,
        Some(b"upgrades") => ValueReader::ObjectOrSkip(Helper::VariantUpgrades),
        Some(b"industrial_organisation") => ValueReader::ObjectIfOpen(Helper::Cid),
        Some(b"modules") => ValueReader::ObjectOrSkip(Helper::VariantModules),
        Some(b"optional_assets") => ValueReader::OptionalAssetsList,
        _ => return None,
    })
}

fn equipment_bonus(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"instant") => ValueReader::One,
        _ => return None,
    })
}

fn equipment_variant(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(
            b"type" | b"version" | b"creator" | b"create_if_missing" | b"version_name"
            | b"license_creator",
        ) => ValueReader::One,
        _ => return None,
    })
}

fn optional_equipment_asset(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"icon" | b"model") => ValueReader::One,
        _ => return None,
    })
}

fn portraits(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"civilian" | b"army" | b"navy" | b"air" | b"operative" | b"scientist") => {
            ValueReader::ObjectIfOpen(Helper::PortraitSizes)
        }
        _ => return None,
    })
}

fn portrait_sizes(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"small" | b"large") => ValueReader::One,
        _ => return None,
    })
}

fn province_target(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"id") => ValueReader::ProvinceId,
        Some(
            b"all_provinces" | b"limit_to_coastal" | b"limit_to_border" | b"limit_to_naval_base",
        ) => ValueReader::One,
        Some(b"limit_to_victory_point") => ValueReader::ProvinceVictoryPoint,
        _ => return None,
    })
}

fn random_list_member(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"modifier") => ValueReader::MtthModifier,
        _ => return None,
    })
}

fn rule(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"icon") => ValueReader::SkipIfOpen,
        Some(b"desc") => ValueReader::One,
        Some(b"override") => ValueReader::Object(Helper::RuleOverride),
        _ => return None,
    })
}

fn scientist_template(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"desc") => ValueReader::One,
        Some(b"visible") => ValueReader::Trigger,
        Some(b"traits") => ValueReader::RawList,
        Some(b"skills") => ValueReader::Object(Helper::ScientistSkills),
        _ => return None,
    })
}

fn unit_leader_data(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(
            b"id" | b"name" | b"picture" | b"desc" | b"female" | b"gfx" | b"portrait_path"
            | b"token",
        ) => ValueReader::One,
        Some(b"rank") => ValueReader::SkipIfOpen,
        _ => return None,
    })
}

fn unit_leader_template(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(
            b"skill"
            | b"attack_skill"
            | b"defense_skill"
            | b"planning_skill"
            | b"logistics_skill"
            | b"maneuvering_skill"
            | b"coordination_skill"
            | b"legacy_id"
            | b"desc",
        ) => ValueReader::One,
        Some(b"traits") => ValueReader::ValueListOrSkip,
        Some(b"visible") => ValueReader::Trigger,
        _ => return None,
    })
}

fn unit_officer_data(key: Option<&[u8]>) -> Option<ValueReader> {
    Some(match key {
        Some(b"name" | b"seed" | b"male") => ValueReader::One,
        Some(b"portraits") => ValueReader::Object(Helper::Portraits),
        _ => return None,
    })
}

#[cfg(any(test, feature = "test-support"))]
impl crate::effect_keys::EffectShape {
    pub const ACE: Self = Self(crate::effect_keys::READ_SCRIPT, ADD_ACE_KEYS);
    pub const ADVISOR_DISMISSAL: Self = Self(crate::effect_keys::READ_SCRIPT, SET_CAN_BE_FIRED_IN_ADVISOR_ROLE_KEYS);
    pub const ADVISOR_ROLE: Self = Self(crate::effect_keys::READ_SCRIPT, ADD_ADVISOR_ROLE_KEYS);
    pub const CAPTURED_OPERATIVE: Self = Self(crate::effect_keys::READ_SCRIPT, FREE_RANDOM_OPERATIVE_KEYS);
    pub const CHARACTER_TRAIT: Self = Self(crate::effect_keys::READ_SCRIPT, ADD_TRAIT_KEYS);
    pub const COMMANDER_ROLE: Self = Self(crate::effect_keys::READ_SCRIPT, ADD_CORPS_COMMANDER_ROLE_KEYS);
    pub const COUNTRY_LEADER: Self = Self(crate::effect_keys::READ_SCRIPT, CREATE_COUNTRY_LEADER_KEYS);
    pub const COUNTRY_LEADER_ROLE: Self = Self(crate::effect_keys::READ_SCRIPT, ADD_COUNTRY_LEADER_ROLE_KEYS);
    pub const DIVISION_HISTORY: Self = Self(crate::effect_keys::READ_SCRIPT, ADD_HISTORY_ENTRY_KEYS);
    pub const SCIENTIST_EXPERIENCE: Self = Self(crate::effect_keys::READ_SCRIPT, ADD_SCIENTIST_XP_KEYS);
    pub const SCIENTIST_LEVEL: Self = Self(crate::effect_keys::READ_SCRIPT, ADD_SCIENTIST_LEVEL_KEYS);
    pub const SCIENTIST_ROLE: Self = Self(crate::effect_keys::READ_SCRIPT, ADD_SCIENTIST_ROLE_KEYS);
    pub const TIMED_TRAIT: Self = Self(crate::effect_keys::READ_SCRIPT, ADD_TIMED_UNIT_LEADER_TRAIT_KEYS);
    pub const UNIT_MEDAL: Self = Self(crate::effect_keys::READ_SCRIPT, ADD_UNIT_MEDAL_TO_LATEST_ENTRY_KEYS);
    pub const UNIT_TRAIT_CHOICE: Self = Self(crate::effect_keys::READ_SCRIPT, ADD_RANDOM_VALID_TRAIT_FROM_UNIT_KEYS);
    pub const VARIABLE: Self = Self(crate::effect_keys::READ_SCRIPT, SET_VARIABLE_KEYS);
}

#[cfg(any(test, feature = "test-support"))]
pub(in crate::effect_keys) const SYNTHETIC_EFFECTS: &[(&str, u64, u64)] = &[
    ("if", IF_KEYS, 0),
    ("set_capital", SET_CAPITAL_KEYS, 0),
    ("add_breakthrough_points", ADD_BREAKTHROUGH_POINTS_KEYS, 0),
    ("add_province_modifier", ADD_PROVINCE_MODIFIER_KEYS, 0),
    ("random_scientist", RANDOM_SCIENTIST_KEYS, 0),
    ("every_scientist", EVERY_SCIENTIST_KEYS, 0),
    ("add_scientist_level", ADD_SCIENTIST_LEVEL_KEYS, 0),
    ("add_scientist_xp", ADD_SCIENTIST_XP_KEYS, 0),
    ("remove_building", REMOVE_BUILDING_KEYS, 0),
    ("every_country", EVERY_COUNTRY_KEYS, 0),
    ("random_country", RANDOM_COUNTRY_KEYS, 0),
    ("add_relation_rule_override", ADD_RELATION_RULE_OVERRIDE_KEYS, 0),
    ("remove_relation_rule_override", REMOVE_RELATION_RULE_OVERRIDE_KEYS, 0),
    ("add_breakthrough_progress", ADD_BREAKTHROUGH_PROGRESS_KEYS, 0),
    ("remove_province_modifier", REMOVE_PROVINCE_MODIFIER_KEYS, 0),
    ("add_design_template_bonus", ADD_DESIGN_TEMPLATE_BONUS_KEYS, 0),
    ("construct_building_in_random_province", CONSTRUCT_BUILDING_IN_RANDOM_PROVINCE_KEYS, 0),
    ("set_variable", SET_VARIABLE_KEYS, 0),
    ("add_ai_strategy", ADD_AI_STRATEGY_KEYS, 0),
    ("random", RANDOM_KEYS, 0),
    ("random_list", RANDOM_LIST_KEYS, 0),
    ("add_opinion_modifier", ADD_OPINION_MODIFIER_KEYS, 0),
    ("remove_opinion_modifier", REMOVE_OPINION_MODIFIER_KEYS, 0),
    ("add_equipment_bonus", ADD_EQUIPMENT_BONUS_KEYS, 0),
    ("reduce_focus_completion_cost", REDUCE_FOCUS_COMPLETION_COST_KEYS, 0),
    ("strategic_province_location", STRATEGIC_PROVINCE_LOCATION_KEYS, 0),
    ("strategic_state_location", STRATEGIC_STATE_LOCATION_KEYS, 0),
    ("create_railway_gun", CREATE_RAILWAY_GUN_KEYS, 0),
    ("create_country_leader", CREATE_COUNTRY_LEADER_KEYS, 0),
    ("set_faction_member_upgrade_min", SET_FACTION_MEMBER_UPGRADE_MIN_KEYS, 0),
    ("create_unit", CREATE_UNIT_KEYS, 0),
    ("add_equipment_subsidy", ADD_EQUIPMENT_SUBSIDY_KEYS, 0),
    ("add_trait", ADD_TRAIT_KEYS, 0),
    ("random_state_division", RANDOM_STATE_DIVISION_KEYS, 0),
    ("every_state_division", EVERY_STATE_DIVISION_KEYS, 0),
    ("add_history_entry", ADD_HISTORY_ENTRY_KEYS, 0),
    ("change_division_template", CHANGE_DIVISION_TEMPLATE_KEYS, 0),
    ("set_political_party", SET_POLITICAL_PARTY_KEYS, 0),
    ("set_politics", SET_POLITICS_KEYS, 0),
    ("set_division_force_allow_recruiting", SET_DIVISION_FORCE_ALLOW_RECRUITING_KEYS, 0),
    ("set_technology", SET_TECHNOLOGY_KEYS, 0),
    ("delete_unit", DELETE_UNIT_KEYS, 0),
    ("add_random_valid_trait_from_unit", ADD_RANDOM_VALID_TRAIT_FROM_UNIT_KEYS, 0),
    ("add_unit_medal_to_latest_entry", ADD_UNIT_MEDAL_TO_LATEST_ENTRY_KEYS, 0),
    ("set_party_rule", SET_PARTY_RULE_KEYS, 0),
    ("annex_country", ANNEX_COUNTRY_KEYS, 0),
    ("add_building_construction", ADD_BUILDING_CONSTRUCTION_KEYS, 0),
    ("random_state", RANDOM_STATE_KEYS, 0),
    ("every_state", EVERY_STATE_KEYS, 0),
    ("add_faction_goal_slot", ADD_FACTION_GOAL_SLOT_KEYS, 0),
    ("start_civil_war", START_CIVIL_WAR_KEYS, 0),
    ("add_tech_bonus", ADD_TECH_BONUS_KEYS, 0),
    ("diplomatic_relation", DIPLOMATIC_RELATION_KEYS, 0),
    ("add_resource", ADD_RESOURCE_KEYS, 0),
    ("add_timed_idea", ADD_TIMED_IDEA_KEYS, 0),
    ("create_import", CREATE_IMPORT_KEYS, 0),
    ("add_named_threat", ADD_NAMED_THREAT_KEYS, 0),
    ("set_party_name", SET_PARTY_NAME_KEYS, 0),
    ("add_to_war", ADD_TO_WAR_KEYS, 0),
    ("create_equipment_variant", CREATE_EQUIPMENT_VARIANT_KEYS, 0),
    ("add_equipment_production", ADD_EQUIPMENT_PRODUCTION_KEYS, 0),
    ("add_ace", ADD_ACE_KEYS, 0),
    ("add_equipment_to_stockpile", ADD_EQUIPMENT_TO_STOCKPILE_KEYS, 0),
    ("damage_building", DAMAGE_BUILDING_KEYS, 0),
    ("add_autonomy_score", ADD_AUTONOMY_SCORE_KEYS, 0),
    ("modify_tech_sharing_bonus", MODIFY_TECH_SHARING_BONUS_KEYS, 0),
    ("add_autonomy_ratio", ADD_AUTONOMY_RATIO_KEYS, 0),
    ("transfer_ship", TRANSFER_SHIP_KEYS, 0),
    ("add_state_modifier", ADD_STATE_MODIFIER_KEYS, 0),
    ("set_province_name", SET_PROVINCE_NAME_KEYS, 0),
    ("create_production_license", CREATE_PRODUCTION_LICENSE_KEYS, 0),
    ("add_relation_modifier", ADD_RELATION_MODIFIER_KEYS, 0),
    ("random_unit_leader", RANDOM_UNIT_LEADER_KEYS, 0),
    ("every_unit_leader", EVERY_UNIT_LEADER_KEYS, 0),
    ("transfer_navy", TRANSFER_NAVY_KEYS, 0),
    ("destroy_ships", DESTROY_SHIPS_KEYS, 0),
    ("set_can_be_fired_in_advisor_role", SET_CAN_BE_FIRED_IN_ADVISOR_ROLE_KEYS, 0),
    ("random_military_industrial_organization", RANDOM_MILITARY_INDUSTRIAL_ORGANIZATION_KEYS, 0),
    ("modify_building_resources", MODIFY_BUILDING_RESOURCES_KEYS, 0),
    ("free_random_operative", FREE_RANDOM_OPERATIVE_KEYS, 0),
    ("divide_temp_variable", DIVIDE_TEMP_VARIABLE_KEYS, 0),
    ("add_timed_unit_leader_trait", ADD_TIMED_UNIT_LEADER_TRAIT_KEYS, 0),
    ("clamp_variable", CLAMP_VARIABLE_KEYS, 0),
    ("clamp_temp_variable", CLAMP_TEMP_VARIABLE_KEYS, 0),
    ("start_border_war", START_BORDER_WAR_KEYS, 0),
    ("set_border_war_data", SET_BORDER_WAR_DATA_KEYS, 0),
    ("cancel_border_war", CANCEL_BORDER_WAR_KEYS, 0),
    ("set_division_template_lock", SET_DIVISION_TEMPLATE_LOCK_KEYS, 0),
    ("delete_unit_template_and_units", DELETE_UNIT_TEMPLATE_AND_UNITS_KEYS, 0),
    ("print_variables", PRINT_VARIABLES_KEYS, 0),
    ("activate_targeted_decision", ACTIVATE_TARGETED_DECISION_KEYS, 0),
    ("set_truce", SET_TRUCE_KEYS, 0),
    ("set_popularities", SET_POPULARITIES_KEYS, 0),
    ("finalize_border_war", FINALIZE_BORDER_WAR_KEYS, 0),
    ("become_exiled_in", BECOME_EXILED_IN_KEYS, 0),
    ("give_resource_rights", GIVE_RESOURCE_RIGHTS_KEYS, 0),
    ("add_to_array", ADD_TO_ARRAY_KEYS, 0),
    ("resize_array", RESIZE_ARRAY_KEYS, 0),
    ("for_each_loop", FOR_EACH_LOOP_KEYS, 0),
    ("for_each_scope_loop", FOR_EACH_SCOPE_LOOP_KEYS, 0),
    ("random_scope_in_array", RANDOM_SCOPE_IN_ARRAY_KEYS, 0),
    ("remove_dynamic_modifier", REMOVE_DYNAMIC_MODIFIER_KEYS, 0),
    ("for_loop_effect", FOR_LOOP_EFFECT_KEYS, 0),
    ("while_loop_effect", WHILE_LOOP_EFFECT_KEYS, 0),
    ("meta_effect", META_EFFECT_KEYS, 0),
    ("add_victory_points", ADD_VICTORY_POINTS_KEYS, 0),
    ("launch_nuke", LAUNCH_NUKE_KEYS, 0),
    ("find_lowest_in_array", FIND_LOWEST_IN_ARRAY_KEYS, 0),
    ("add_mines", ADD_MINES_KEYS, 0),
    ("remove_advisor_role", REMOVE_ADVISOR_ROLE_KEYS, 0),
    ("add_power_balance_modifier", ADD_POWER_BALANCE_MODIFIER_KEYS, 0),
    ("set_power_balance_gfx", SET_POWER_BALANCE_GFX_KEYS, 0),
    ("add_days_remove", ADD_DAYS_REMOVE_KEYS, 0),
    ("add_days_mission_timeout", ADD_DAYS_MISSION_TIMEOUT_KEYS, 0),
    ("every_military_industrial_organization", EVERY_MILITARY_INDUSTRIAL_ORGANIZATION_KEYS, 0),
    ("create_purchase_contract", CREATE_PURCHASE_CONTRACT_KEYS, 0),
    ("generate_scientist_character", GENERATE_SCIENTIST_CHARACTER_KEYS, 0),
    ("add_scientist_role", ADD_SCIENTIST_ROLE_KEYS, 0),
    ("raid_damage_units", RAID_DAMAGE_UNITS_KEYS, 0),
    ("add_mastery", ADD_MASTERY_KEYS, 0),
    ("add_daily_mastery", ADD_DAILY_MASTERY_KEYS, 0),
    ("add_mastery_bonus", ADD_MASTERY_BONUS_KEYS, 0),
    ("custom_override_tooltip", CUSTOM_OVERRIDE_TOOLTIP_KEYS, 0),
    ("add_unit_bonus", ADD_UNIT_BONUS_KEYS, 0),
    ("add_operation_token", ADD_OPERATION_TOKEN_KEYS, 0),
    ("steal_random_tech_bonus", STEAL_RANDOM_TECH_BONUS_KEYS, 0),
    ("teleport_armies", TELEPORT_ARMIES_KEYS, 0),
    ("set_state_province_controller", SET_STATE_PROVINCE_CONTROLLER_KEYS, 0),
    ("create_dynamic_country", CREATE_DYNAMIC_COUNTRY_KEYS, 0),
    ("add_resistance_target", ADD_RESISTANCE_TARGET_KEYS, 0),
    ("random_country_with_original_tag", RANDOM_COUNTRY_WITH_ORIGINAL_TAG_KEYS, 0),
    ("every_country_with_original_tag", EVERY_COUNTRY_WITH_ORIGINAL_TAG_KEYS, 0),
    ("add_decryption", ADD_DECRYPTION_KEYS, 0),
    ("delete_units", DELETE_UNITS_KEYS, 0),
    ("force_enable_resistance", FORCE_ENABLE_RESISTANCE_KEYS, 0),
    ("create_ship", CREATE_SHIP_KEYS, 0),
    ("add_intel", ADD_INTEL_KEYS, 0),
    ("set_country_leader_name", SET_COUNTRY_LEADER_NAME_KEYS, 0),
    ("add_collaboration", ADD_COLLABORATION_KEYS, 0),
    ("execute_operation_coordinated_strike", EXECUTE_OPERATION_COORDINATED_STRIKE_KEYS, 0),
    ("add_doctrine_cost_reduction", ADD_DOCTRINE_COST_REDUCTION_KEYS, 0),
    ("add_corps_commander_role", ADD_CORPS_COMMANDER_ROLE_KEYS, 0),
    ("add_country_leader_role", ADD_COUNTRY_LEADER_ROLE_KEYS, 0),
    ("add_advisor_role", ADD_ADVISOR_ROLE_KEYS, 0),
    ("every_collection_element", EVERY_COLLECTION_ELEMENT_KEYS, 0),
    ("create_entity", CREATE_ENTITY_KEYS, 0),
    ("set_entity_position", SET_ENTITY_POSITION_KEYS, 0),
    ("force_update_map_mode", FORCE_UPDATE_MAP_MODE_KEYS, 0),
    ("damage_units", DAMAGE_UNITS_KEYS, 0),
    ("uncomplete_national_focus", UNCOMPLETE_NATIONAL_FOCUS_KEYS, 0),
    ("build_railway", BUILD_RAILWAY_KEYS, 0),
    ("set_entity_animation", SET_ENTITY_ANIMATION_KEYS, 0),
    ("set_power_balance", SET_POWER_BALANCE_KEYS, 0),
    ("remove_power_balance", REMOVE_POWER_BALANCE_KEYS, 0),
    ("add_power_balance_value", ADD_POWER_BALANCE_VALUE_KEYS, 0),
    ("set_division_template_cap", SET_DIVISION_TEMPLATE_CAP_KEYS, 0),
    ("remove_country_leader_role", REMOVE_COUNTRY_LEADER_ROLE_KEYS, 0),
    ("generate_character", GENERATE_CHARACTER_KEYS, 0),
    ("set_portraits", SET_PORTRAITS_KEYS, 0),
    ("set_temp_variable", SET_VARIABLE_KEYS, 0),
    ("add_to_temp_variable", SET_VARIABLE_KEYS, 0),
    ("subtract_from_temp_variable", SET_VARIABLE_KEYS, 0),
    ("multiply_temp_variable", SET_VARIABLE_KEYS, 0),
    ("add_to_variable", SET_VARIABLE_KEYS, 0),
    ("subtract_from_variable", SET_VARIABLE_KEYS, 0),
    ("multiply_variable", SET_VARIABLE_KEYS, 0),
    ("divide_variable", DIVIDE_TEMP_VARIABLE_KEYS, 0),
    ("set_collaboration", ADD_COLLABORATION_KEYS, 0),
    ("remove_all_power_balance_modifiers", REMOVE_POWER_BALANCE_KEYS, 0),
    ("add_dynamic_modifier", REMOVE_DYNAMIC_MODIFIER_KEYS, 0),
    ("set_relation_rule", SHARED_DEFINITION_KEYS, SET_RULE_CLASS),
    ("clear_relation_rule", SHARED_DEFINITION_KEYS, CLEAR_RULE_CLASS),
    ("create_navy_leader", SHARED_DEFINITION_KEYS, NAVY_LEADER_CLASS),
    ("create_field_marshal", SHARED_DEFINITION_KEYS, FIELD_MARSHAL_CLASS),
    ("create_corps_commander", SHARED_DEFINITION_KEYS, CORPS_COMMANDER_CLASS),
    ("set_victory_points", ADD_VICTORY_POINTS_KEYS, 0),
    ("every_character", EVERY_SCIENTIST_KEYS, 0),
    ("random_character", RANDOM_SCIENTIST_KEYS, 0),
    ("random_active_scientist", RANDOM_SCIENTIST_KEYS, 0),
    ("every_active_scientist", EVERY_SCIENTIST_KEYS, 0),
    ("every_country_division", EVERY_STATE_DIVISION_KEYS, 0),
    ("random_country_division", RANDOM_STATE_DIVISION_KEYS, 0),
    ("remove_operation_token", ADD_OPERATION_TOKEN_KEYS, 0),
    ("set_country_leader_description", SET_COUNTRY_LEADER_NAME_KEYS, 0),
    ("set_country_leader_portrait", SET_COUNTRY_LEADER_NAME_KEYS, 0),
    ("add_field_marshal_role", ADD_CORPS_COMMANDER_ROLE_KEYS, 0),
    ("add_naval_commander_role", ADD_CORPS_COMMANDER_ROLE_KEYS, 0),
    ("remove_trait", ADD_TRAIT_KEYS, 0),
    ("find_highest_in_array", FIND_LOWEST_IN_ARRAY_KEYS, 0),
    ("clear_division_template_cap", SET_DIVISION_TEMPLATE_CAP_KEYS, 0),
    ("random_army_leader", RANDOM_UNIT_LEADER_KEYS, 0),
    ("every_army_leader", EVERY_UNIT_LEADER_KEYS, 0),
    ("resize_temp_array", RESIZE_ARRAY_KEYS, 0),
    ("remove_power_balance_modifier", ADD_POWER_BALANCE_MODIFIER_KEYS, 0),
    ("remove_targeted_decision", ACTIVATE_TARGETED_DECISION_KEYS, 0),
    ("remove_relation_modifier", ADD_RELATION_MODIFIER_KEYS, 0),
];
