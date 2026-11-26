use engine_air_history::{air_history_mutations, air_loss_sites, air_record_mutations};
use engine_characters::ccharacter_portraits;
use engine_events::{
    building_damage_sites, diplomacy_mutations, equipment_destruction, event_feed_sites,
    experience_gain_sites, map_event_sites, nuclear_strike_sites, operation_outcome_sites,
};
use engine_fuel::fuel_awards;
use engine_land_inputs::{
    clogistics_status, combat_log_sites, land_events, land_wipe, ncombat_log_cmanager,
};
use engine_mutations::{
    advisor_mutations, advisor_pool_mutations, air_base_mutations, air_wing_equipment_mutations,
    army_stat_mutations, building_mutations, character_role_mutations, character_roster_mutations,
    combat_engagement_mutations, country_input_mutations, faction_companion_mutations, faction_goal_mutations,
    local_subunit_mutations, naval_tree_mutations, opinion_mutations, party_leader_mutations,
    political_party_mutations, political_status_mutations, province_combat_mutations,
    province_unit_mutations, raid_mutations, railway_mutations, regional_mission_mutations,
    script_flag_mutations, script_variable_mutations, ship_model_mutations, template_mutations,
    territory_mutations, world_input_mutations, world_lifetime_sites,
};
use engine_native::cgame_state;
use engine_naval_history::naval_result_sources;
use engine_peace::peace_finalizer_site;
use engine_production::{construction_ic_sites, line_output_sites, market_mutations};
use engine_sites::Rva;

use collector_hook_failures::Patch;

pub(crate) const PATCH: usize = 5;
const INDIRECT: usize = 6;
const FIVE: Option<usize> = Some(PATCH);
const TAP: Option<usize> = None;

pub(crate) fn patches() -> Vec<Patch> {
    let mut out = Vec::new();
    let mut add = |family: &'static str, name: &str, rva: Rva, len: Option<usize>| {
        out.push(Patch {
            family,
            name: name.to_owned(),
            rva,
            len,
        })
    };
    for (name, rva) in engine_native_naval::naval_mine_inputs::WRITERS
        .into_iter()
        .enumerate()
        .filter(|(index, _)| *index != 5)
        .map(|(_, site)| site)
        .chain(
            [
                "standing report creation/update",
                "standing report clear",
                "standing report expiry",
                "mine incident",
            ]
            .into_iter()
            .zip(engine_naval_history::cnaval_accident_report::WRITERS),
        )
    {
        add("naval_map_changes", name, rva, FIVE);
    }
    for site in engine_mutations::navy_ledger_mutations::MUTATIONS {
        add("naval_map_changes", site.name, site.rva, FIVE);
    }
    for site in collector_observers::fleet_region_changes::SITES {
        add("fleet_region_changes", site.name, site.rva, FIVE);
    }
    add(
        "hourly",
        "HourlyUpdate",
        cgame_state::HOURLY_UPDATE_RVA,
        FIVE,
    );
    add(
        "command_batch",
        "CommandBatch",
        cgame_state::COMMAND_BATCH_RVA,
        FIVE,
    );
    for site in engine_mutations::unit_leader_mutations::MUTATIONS {
        add("unit_leader_changes", site.name, site.rva, FIVE);
    }
    for site in engine_mutations::census_mutations::MUTATIONS {
        add("census_changes", site.name, site.rva, FIVE);
    }
    for (site, _) in engine_mutations::orders_group_mutations::MUTATIONS {
        add("orders_group_changes", site.name, site.rva, FIVE);
    }
    for (site, _) in engine_mutations::order_membership_mutations::MUTATIONS {
        add("order_membership_changes", site.name, site.rva, FIVE);
    }
    use engine_events::map_event_sites as map;
    for (name, rva) in [
        ("FakeDivisionRevealed", map::FAKE_REVEAL_VISIBILITY_CALL_RVA),
        ("GeneralCaptured", map::GENERAL_CAPTURE_CALL_RVA),
        ("ParadropLanding", map::PARADROP_LANDING_CALL_RVA),
        ("RailwayGunOverrun", map::RAILWAY_OVERRUN_RVA),
        ("ResistanceActivity", map::RESISTANCE_ACTIVITY_RVA),
        ("BuildingDamage", map::BUILDING_NOTIFICATION_RVA),
    ] {
        add("map_events", name, rva, FIVE);
    }
    add(
        "map_events",
        "CriticalShoreBombardment",
        map::CRITICAL_BOMBARDMENT_TAP_RVA,
        TAP,
    );
    for site in ccharacter_portraits::MUTATIONS {
        add("portrait_changes", site.name, site.rva, FIVE);
    }
    for (index, site) in engine_native::cequipment_variant::mutations::MUTATIONS
        .iter()
        .enumerate()
    {
        if engine_native::cequipment_variant::mutations::FORWARDED_SITES.contains(&index) {
            continue;
        }
        add("design_changes", site.name, site.rva, FIVE);
    }
    for site in engine_native::cequipment_variant::source_mutations::MUTATIONS {
        add("design_source_changes", site.name, site.rva, FIVE);
    }
    for site in engine_native::cequipment_variant::source_mutations::ORGANISATIONS {
        add("design_organisation_liveness", site.name, site.rva, FIVE);
    }
    for site in engine_native::cproduction_status::design_templates::MUTATIONS {
        add("design_template_changes", site.name, site.rva, FIVE);
    }
    for site in engine_native_air::cair_wing::replacement::MUTATIONS {
        add("wing_preferences", site.name, site.rva, FIVE);
    }
    for site in advisor_mutations::MUTATIONS {
        add("advisor_changes", site.name, site.rva, FIVE);
    }
    for site in character_role_mutations::MUTATIONS {
        add("advisor_changes", site.name, site.rva, FIVE);
    }
    for site in diplomacy_mutations::MUTATIONS {
        add("relation_changes", site.name, site.rva, FIVE);
    }
    for site in air_history_mutations::MUTATIONS {
        add("history_changes", site.name, site.rva, FIVE);
    }
    add(
        "propaganda_changes",
        "daily propaganda append",
        engine_intelligence::propaganda_history::DAILY_UPDATE_RVA,
        FIVE,
    );
    add(
        "propaganda_changes",
        "strategic operative reset",
        engine_intelligence::propaganda_history::RESET_RVA,
        FIVE,
    );
    for site in &engine_intelligence::network_auxiliary::SITES {
        add("network_auxiliary", site.name, site.rva, FIVE);
    }
    for site in &engine_intelligence::intel_source_sites::SITES {
        add("intel_sources", site.name, site.rva, FIVE);
    }
    for (site, _) in script_flag_mutations::MUTATIONS {
        add("flag_changes", site.name, site.rva, FIVE);
    }
    for (site, _) in building_mutations::MUTATIONS {
        add("building_changes", site.name, site.rva, FIVE);
    }
    for site in engine_mutations::doctrine_mutations::SITES {
        add("world_input_changes", site.name, site.rva, FIVE);
    }
    for (index, rva) in engine_mutations::doctrine_mutations::EFFECTIVE_GAIN_STORES
        .into_iter()
        .enumerate()
    {
        add(
            "world_input_changes",
            &format!("doctrine_gain_{index}"),
            rva,
            TAP,
        );
    }
    let (unappoint, pool_sites) = advisor_pool_mutations::MUTATIONS.split_last().unwrap();
    assert_eq!(
        unappoint.0.rva,
        country_input_mutations::MUTATIONS[13].0.rva
    );
    for (site, _) in pool_sites {
        add("advisor_pool_changes", site.name, site.rva, FIVE);
    }
    for (site, _) in party_leader_mutations::MUTATIONS {
        add("party_leader_changes", site.name, site.rva, FIVE);
    }
    for (site, _, _) in script_variable_mutations::MUTATIONS {
        add("variable_changes", site.name, site.rva, FIVE);
    }
    for site in political_party_mutations::MUTATIONS {
        add("party_changes", site.name, site.rva, FIVE);
    }
    for (site, _) in air_wing_equipment_mutations::MUTATIONS {
        add("wing_changes", site.name, site.rva, FIVE);
    }
    for site in air_wing_equipment_mutations::MAP_MUTATIONS {
        add("wing_changes", site.name, site.rva, FIVE);
    }
    for (site, _) in character_roster_mutations::MUTATIONS {
        add("roster_changes", site.name, site.rva, FIVE);
    }
    for (site, _, _) in political_status_mutations::MUTATIONS {
        add("political_status_changes", site.name, site.rva, FIVE);
    }
    for site in regional_mission_mutations::MUTATIONS {
        add("mission_changes", site.name, site.rva, FIVE);
    }
    add(
        "air_detection_overrides",
        "console vector writer",
        engine_map_air::map_air_sortie_inputs::DETECTION_OVERRIDE_WRITER_RVA,
        FIVE,
    );
    add(
        "radar_symbols",
        "group update",
        engine_map_air::radar_group::UPDATE_RVA,
        FIVE,
    );
    use engine_map_air::map_air_country_region_modifier as ground_crews;
    for (name, rva) in [
        ("enable", ground_crews::GROUND_CREWS_ENABLE_RVA),
        ("disable", ground_crews::GROUND_CREWS_DISABLE_RVA),
        ("construction", ground_crews::GROUND_CREWS_CONSTRUCTOR_RVA),
    ] {
        add("ground_crews", name, rva, FIVE);
    }
    for (site, _) in railway_mutations::MUTATIONS {
        add("rail_changes", site.name, site.rva, FIVE);
    }
    for (site, _) in territory_mutations::MUTATIONS {
        add("territory_changes", site.name, site.rva, FIVE);
    }
    for site in engine_mutations::division_panel_mutations::MUTATIONS {
        add("division_panel_changes", site.name, site.rva, FIVE);
    }
    add(
        "map_events",
        "scripted_refresh",
        map_event_sites::SCRIPTED_REFRESH_CALL_RVA,
        FIVE,
    );
    for site in &engine_mutations::war_participation_sites::SITES {
        add("war_participation", site.name, site.rva, FIVE);
    }
    for site in army_stat_mutations::MUTATIONS {
        add("army_stat_changes", site.name, site.rva, FIVE);
    }
    for site in ship_model_mutations::MUTATIONS {
        add("ship_model_changes", site.name, site.rva, FIVE);
    }
    for (name, rva) in collector_naval_management::changes::patch_sites() {
        add("naval_management_changes", name, rva, TAP);
    }
    for (site, _, _) in country_input_mutations::MUTATIONS {
        add("country_input_changes", site.name, site.rva, FIVE);
    }
    for (site, _) in raid_mutations::MUTATIONS {
        add("raid_changes", site.name, site.rva, FIVE);
    }
    let daily = raid_mutations::DAILY_CAPACITIES;
    add("raid_changes", daily.name, daily.rva, FIVE);
    let country = naval_tree_mutations::COUNTRY_SITES;
    assert_eq!(
        naval_tree_mutations::MUTATIONS[0].rva,
        country_input_mutations::MUTATIONS[0].0.rva
    );
    assert_eq!(
        naval_tree_mutations::MUTATIONS[1].rva,
        engine_events::politics_sites::RESET_COUNTRY.rva
    );
    for site in &naval_tree_mutations::MUTATIONS[country..] {
        add("naval_tree_changes", site.name, site.rva, FIVE);
    }
    for site in engine_native::cwar_relation::goals::mutations::SITES {
        add("wargoal_changes", site.name, site.rva, FIVE);
    }
    for site in engine_occupation::cresistance::changes::SITES {
        add("occupation_changes", site.name, site.rva, FIVE);
    }
    for (site, _) in world_input_mutations::MUTATIONS {
        add("world_input_changes", site.name, site.rva, FIVE);
    }
    for site in air_record_mutations::MUTATIONS {
        add("air_record_changes", site.name, site.rva, FIVE);
    }
    for (site, _) in local_subunit_mutations::MUTATIONS {
        add("local_subunit_changes", site.name, site.rva, FIVE);
    }
    for (site, _) in template_mutations::MUTATIONS {
        add("template_changes", site.name, site.rva, FIVE);
    }
    for (site, _, patch) in faction_companion_mutations::MUTATIONS {
        let len = if matches!(patch, faction_companion_mutations::Patch::Tap) {
            TAP
        } else {
            FIVE
        };
        add("faction_companions", site.name, site.rva, len);
    }
    for (site, _, patch) in faction_goal_mutations::MUTATIONS {
        let len = if matches!(patch, faction_goal_mutations::Patch::Tap) {
            TAP
        } else {
            FIVE
        };
        add("faction_goals", site.name, site.rva, len);
    }
    for site in province_unit_mutations::MUTATIONS {
        add("province_unit_changes", site.name, site.rva, FIVE);
    }
    for site in province_combat_mutations::MUTATIONS {
        add("province_combat_changes", site.name, site.rva, FIVE);
    }
    for site in combat_engagement_mutations::MUTATIONS {
        add("combat_engagement_refreshes", site.name, site.rva, FIVE);
    }
    for (site, _) in air_base_mutations::MUTATIONS {
        add("air_base_carry_changes", site.name, site.rva, FIVE);
    }
    for site in opinion_mutations::MUTATIONS
        .iter()
        .chain(&opinion_mutations::RELATIONS)
    {
        add("opinion_changes", site.name, site.rva, FIVE);
    }
    for site in equipment_destruction::SITES {
        add("equipment_destruction", site.name, site.rva, FIVE);
    }
    for index in collector_observers::market_changes::OBSERVED_SITES {
        let site = &market_mutations::SITES[index];
        let len = if index == market_mutations::FUEL_SENT_SITE
            || index == market_mutations::FUEL_LOST_SITE
        {
            TAP
        } else {
            FIVE
        };
        add("market_changes", site.name, site.rva, len);
    }
    for (index, site) in naval_result_sources::SITES.into_iter().enumerate() {
        add(
            "naval_result_sources",
            &format!("site {index}"),
            site.rva,
            Some(site.span),
        );
    }
    for (index, site) in experience_gain_sites::SITES.iter().enumerate() {
        add("experience_gains", &format!("tap {index}"), site.rva, TAP);
    }
    for (index, site) in engine_native_air::command_inputs::SITES.iter().enumerate() {
        add(
            "player_commands",
            &format!("site {index}"),
            site.rva,
            Some(site.span),
        );
    }
    for (index, site) in air_loss_sites::SITES.iter().enumerate() {
        add("air_losses", &format!("call {index}"), site.rva, FIVE);
    }
    add(
        "building_bombing",
        "ApplyDamage",
        building_damage_sites::APPLY_DAMAGE_RVA,
        FIVE,
    );
    for site in nuclear_strike_sites::SITES {
        add("nuclear_strikes", site.name, site.rva, FIVE);
    }
    for (name, rva) in [
        ("credit", line_output_sites::CREDIT.rva),
        ("delivery", line_output_sites::DELIVERY.rva),
        ("rocket", line_output_sites::ROCKET_ADVANCE.rva),
    ] {
        add("line_output", name, rva, FIVE);
    }
    for site in collector_output::construction_ic::SITES {
        let site: &construction_ic_sites::Site = site;
        add("construction_ic", "construction", site.rva, FIVE);
    }
    for (name, site) in [
        ("reset", &world_lifetime_sites::RESET),
        ("construct", &world_lifetime_sites::CONSTRUCT),
    ] {
        add("world_lifetime", name, site.rva, TAP);
    }
    for (name, site) in [
        ("delete", &world_lifetime_sites::DELETE),
        ("state_reload", &world_lifetime_sites::STATE_RELOAD),
        ("straits_reload", &world_lifetime_sites::STRAITS_RELOAD),
    ] {
        add("world_lifetime", name, site.rva, FIVE);
    }
    for (name, rva) in [
        ("finalize", land_events::FINALIZE_RVA),
        ("destroy", land_events::DESTROY_RVA),
        ("border_finalize", land_events::BORDER_FINALIZE_RVA),
        ("border_terminate", land_events::BORDER_TERMINATE_RVA),
        ("dispatch", land_events::DISPATCH_RVA),
        ("destruction", land_events::DESTRUCTION_RVA),
        ("start_combat", engine_land_inputs::land_edges::START_COMBAT_RVA),
        ("army_combat_ended", engine_land_inputs::land_edges::ARMY_COMBAT_ENDED_RVA),
        ("attack_activity", engine_land_inputs::land_edges::ATTACK_ACTIVITY_CALL_RVA),
    ] {
        add("land_events", name, rva, FIVE);
    }
    add(
        "land_events",
        "lifecycle",
        land_wipe::LIFECYCLE_RVA,
        Some(8),
    );
    for (name, rva) in land_events::DISPATCH_LEADER_CALL_NAMES
        .into_iter()
        .zip(land_events::DISPATCH_LEADER_CALLS)
    {
        add("land_events", name, rva, Some(INDIRECT));
    }
    add(
        "combat_log_prune",
        "prune",
        ncombat_log_cmanager::PRUNE_RVA,
        FIVE,
    );
    for (name, rva) in [
        ("group_create_call", combat_log_sites::GROUP_CREATE_CALL_RVA),
        ("own_equipment", combat_log_sites::OWN_EQUIPMENT_RVA),
        ("own_variant", combat_log_sites::OWN_VARIANT_RVA),
        ("enemy", combat_log_sites::ENEMY_RVA),
        ("recovered", combat_log_sites::RECOVERED_RVA),
        ("manpower", combat_log_sites::MANPOWER_RVA),
        ("result", combat_log_sites::RESULT_RVA),
        ("group_prune", combat_log_sites::GROUP_PRUNE_RVA),
        ("group_destroy", combat_log_sites::GROUP_DESTROY_RVA),
        ("battle_allocate", combat_log_sites::BATTLE_ALLOCATE_RVA),
        ("battle_release", combat_log_sites::BATTLE_RELEASE_RVA),
        ("battle_reset", combat_log_sites::BATTLE_RESET_RVA),
        ("battle_load", combat_log_sites::BATTLE_LOAD_RVA),
    ] {
        add("combat_log_history", name, rva, FIVE);
    }
    add(
        "combat_log_history",
        "template_counter",
        combat_log_sites::TEMPLATE_COUNTER_RVA,
        TAP,
    );
    for (rva, _) in clogistics_status::BALANCE_CALLS {
        add("logistics_balance", "balance call", rva, FIVE);
    }
    for (rva, _) in clogistics_status::HISTORY_CALLS {
        add("logistics_balance", "history call", rva, FIVE);
    }
    let mut fuel = vec![fuel_awards::BEGIN, fuel_awards::END];
    fuel.extend(fuel_awards::LAND);
    fuel.extend([fuel_awards::NAVAL, fuel_awards::AIR, fuel_awards::RESET]);
    for (index, rva) in fuel.into_iter().enumerate() {
        add("fuel_awards", &format!("tap {index}"), rva, TAP);
    }
    add(
        "operation_outcomes",
        "completion",
        operation_outcome_sites::COMPLETION_RVA,
        FIVE,
    );
    for site in collector_politics::politics_changes::entry_sites() {
        add("politics_changes", site.name, site.rva, FIVE);
    }
    for site in collector_politics::politics_changes::TAPS {
        add("politics_changes", site.name, site.rva, TAP);
    }
    for (name, site) in [
        ("war", event_feed_sites::WAR),
        ("annex", event_feed_sites::ANNEX),
    ] {
        add("event_feed_observer", name, site.rva, FIVE);
    }
    for entry in event_feed_sites::provenance::ENTRIES {
        add("event_feed_observer", entry.name, entry.rva, FIVE);
    }
    use engine_events::script_event_sites as script;
    for site in script::TAPS
        .into_iter()
        .chain(engine_events::event_window_sites::VERDICT_TAPS)
        .chain(engine_events::event_window_sites::PRESENTATION_TAPS)
    {
        add("event_feed_observer", site.name, site.rva, TAP);
    }
    for site in script::REGISTRY_ENTRIES
        .into_iter()
        .chain([script::ACTIVATE_TARGETED])
    {
        add("event_feed_observer", site.name, site.rva, FIVE);
    }
    for (name, rva) in engine_decisions::cdecision::EVENT_FEED_ENTRIES {
        add("event_feed_observer", name, rva, FIVE);
    }
    for site in script::LIFECYCLE_TAPS
        .into_iter()
        .chain(script::decisions::TAPS)
        .filter(|site| site.rva != script::decisions::CLEAR_CANDIDATES.rva)
    {
        add(
            if site.rva == script::decisions::TARGETED_SELECTION_RESULT.rva {
                "politics_changes"
            } else {
                "event_feed_observer"
            },
            site.name,
            site.rva,
            TAP,
        );
    }
    for site in script::DELAYED_ENTRIES
        .into_iter()
        .take(2)
        .chain(script::LIFECYCLE_ENTRIES)
        .chain(script::decisions::ENTRIES)
        .chain(script::decisions::CALLS)
    {
        add("event_feed_observer", site.name, site.rva, FIVE);
    }
    add(
        "variable_changes",
        script::DELAYED_PASS.name,
        script::DELAYED_PASS.rva,
        FIVE,
    );
    for site in script::DELAYED_TAPS {
        add("variable_changes", site.name, site.rva, TAP);
    }
    add(
        "peace_settlements",
        "finalizer",
        peace_finalizer_site::RVA,
        FIVE,
    );
    out
}
