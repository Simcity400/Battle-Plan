//! Complete runtime patch names and test verification inventory.
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
    country_input_mutations, faction_companion_mutations, faction_goal_mutations,
    local_subunit_mutations, naval_tree_mutations, opinion_mutations, party_leader_mutations,
    political_party_mutations, political_status_mutations, province_combat_mutations,
    province_unit_mutations, raid_mutations,
    railway_mutations, regional_mission_mutations, script_flag_mutations,
    script_variable_mutations, ship_model_mutations, template_mutations, territory_mutations,
    world_input_mutations, world_lifetime_sites,
};
use engine_native::cgame_state;
use engine_naval_history::naval_result_sources;
use engine_peace::peace_finalizer_site;
use engine_production::{construction_ic_sites, line_output_sites, market_mutations};

/// One patch: its family, site name, RVA and the bytes it replaces.
#[derive(Clone, Debug)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct Patch {
    pub(crate) family: &'static str,
    pub(crate) name: String,
    pub(crate) rva: usize,
    pub(crate) len: usize,
    pub(crate) sentinel: Vec<u8>,
}

/// A detour or replaced CALL patches five bytes.
pub(crate) const PATCH: usize = 5;
/// A replaced `CALL [RAX+disp32]` patches six.
const INDIRECT: usize = 6;
// These indirect CALL hooks authenticate the shared opcode prefix, then
// retain the complete live fingerprint for publication. Register its full
// pinned span so neighbour checks include the captured suffix too.
const LEADER_CALL_SENTINELS: [[u8; 16]; 2] = [
    [
        0xff, 0x90, 0xe8, 0x00, 0x00, 0x00, 0x48, 0x85, 0xc0, 0x74, 0x0d, 0x4c, 0x8b, 0xc7, 0x33,
        0xd2,
    ],
    [
        0xff, 0x90, 0xe8, 0x00, 0x00, 0x00, 0x48, 0x85, 0xc0, 0x74, 0x0d, 0x4c, 0x8b, 0xc3, 0xb2,
        0x01,
    ],
];

/// A tap owns the instructions it relocates; the epilogue form (span 0)
/// writes its CALL, the copied epilogue and the moved JMP: 15 bytes.
fn tap(sentinel: &[u8; 16]) -> usize {
    match crate::hook::tap_span(sentinel).expect("tap sentinel decodes") {
        0 => 15,
        span => span,
    }
}

// Hash-backed native descriptors authenticate their complete entry prefix.
#[cfg(test)]
fn hashed_sentinel(rva: usize, expected: u64) -> [u8; 16] {
    static IMAGE: std::sync::OnceLock<engine_executable::executable_tables::pe::PeImage> =
        std::sync::OnceLock::new();
    let image = IMAGE.get_or_init(|| {
        let directory =
            std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).expect("hook checks require BATTLE_PLAN_GAME_DIR");
        let bytes = std::fs::read(std::path::Path::new(&directory).join("hoi4.exe")).unwrap();
        let identity = engine_executable::build::hash_executable(bytes.as_slice()).unwrap();
        assert!(engine_executable::build::PINNED_BUILD.matches(&identity));
        engine_executable::executable_tables::pe::PeImage::parse(bytes).unwrap()
    });
    let bytes = image.bytes(u32::try_from(rva).unwrap(), 16).unwrap();
    let actual = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3)
    });
    assert_eq!(actual, expected, "hook entry fingerprint mismatch");
    bytes.try_into().unwrap()
}

#[cfg(not(test))]
fn hashed_sentinel(_rva: usize, _expected: u64) -> [u8; 16] {
    [0; 16]
}
pub(crate) fn patches() -> Vec<Patch> {
    let mut out = Vec::new();
    let mut add = |family: &'static str, name: &str, rva: usize, len: usize, sentinel: &[u8]| {
        out.push(Patch {
            family,
            name: name.to_owned(),
            rva,
            len,
            sentinel: sentinel.to_vec(),
        })
    };
    for (name, rva, hash) in engine_native::naval_mine_inputs::WRITERS
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
            .zip(engine_naval_history::cnaval_accident_report::WRITERS)
            .map(|(name, (rva, hash))| (name, rva, hash)),
        )
    {
        add(
            "naval_map_changes",
            name,
            rva,
            PATCH,
            &hashed_sentinel(rva, hash),
        );
    }
    for site in engine_mutations::navy_ledger_mutations::MUTATIONS {
        add(
            "naval_map_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for site in collector_observers::fleet_region_changes::SITES {
        add(
            "fleet_region_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    add(
        "hourly",
        "HourlyUpdate",
        cgame_state::HOURLY_UPDATE_RVA,
        PATCH,
        &cgame_state::HOURLY_UPDATE_SENTINEL,
    );
    for site in engine_mutations::unit_leader_mutations::MUTATIONS {
        add(
            "unit_leader_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for site in engine_mutations::census_mutations::MUTATIONS {
        add("census_changes", site.name, site.rva, PATCH, &site.sentinel);
    }
    for (site, _) in engine_mutations::orders_group_mutations::MUTATIONS {
        add(
            "orders_group_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    use engine_events::map_event_sites as map;
    for (name, rva, sentinel) in [
        (
            "FakeDivisionRevealed",
            map::FAKE_REVEAL_VISIBILITY_CALL_RVA,
            map::FAKE_REVEAL_VISIBILITY_CALL_SENTINEL,
        ),
        (
            "GeneralCaptured",
            map::GENERAL_CAPTURE_CALL_RVA,
            map::GENERAL_CAPTURE_CALL_SENTINEL,
        ),
        (
            "ParadropLanding",
            map::PARADROP_LANDING_CALL_RVA,
            map::PARADROP_LANDING_CALL_SENTINEL,
        ),
        (
            "RailwayGunOverrun",
            map::RAILWAY_OVERRUN_RVA,
            map::RAILWAY_OVERRUN_SENTINEL,
        ),
        (
            "ResistanceActivity",
            map::RESISTANCE_ACTIVITY_RVA,
            map::RESISTANCE_ACTIVITY_SENTINEL,
        ),
        (
            "BuildingDamage",
            map::BUILDING_NOTIFICATION_RVA,
            map::BUILDING_NOTIFICATION_SENTINEL,
        ),
    ] {
        add("map_events", name, rva, PATCH, &sentinel);
    }
    add(
        "map_events",
        "CriticalShoreBombardment",
        map::CRITICAL_BOMBARDMENT_TAP_RVA,
        tap(&map::CRITICAL_BOMBARDMENT_TAP_SENTINEL),
        &map::CRITICAL_BOMBARDMENT_TAP_SENTINEL[..tap(&map::CRITICAL_BOMBARDMENT_TAP_SENTINEL)],
    );
    for site in ccharacter_portraits::MUTATIONS {
        add(
            "portrait_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for (index, site) in engine_native::cequipment_variant::mutations::MUTATIONS
        .iter()
        .enumerate()
    {
        if engine_native::cequipment_variant::mutations::FORWARDED_SITES.contains(&index) {
            continue;
        }
        add("design_changes", site.name, site.rva, PATCH, &site.sentinel);
    }
    for site in engine_native::cequipment_variant::source_mutations::MUTATIONS {
        add(
            "design_source_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for site in engine_native::cequipment_variant::source_mutations::ORGANISATIONS {
        add(
            "design_organisation_liveness",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for site in engine_native::cproduction_status::design_templates::MUTATIONS {
        add(
            "design_template_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for site in engine_native::cair_wing::replacement::MUTATIONS {
        add(
            "wing_preferences",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for site in advisor_mutations::MUTATIONS {
        add(
            "advisor_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for site in character_role_mutations::MUTATIONS {
        add(
            "advisor_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for site in diplomacy_mutations::MUTATIONS {
        add(
            "relation_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for site in air_history_mutations::MUTATIONS {
        add(
            "history_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    add(
        "propaganda_changes",
        "daily propaganda append",
        engine_intelligence::propaganda_history::DAILY_UPDATE_RVA,
        PATCH,
        &engine_intelligence::propaganda_history::DAILY_UPDATE_SENTINEL,
    );
    add(
        "propaganda_changes",
        "strategic operative reset",
        engine_intelligence::propaganda_history::RESET_RVA,
        PATCH,
        &engine_intelligence::propaganda_history::RESET_SENTINEL,
    );
    for site in &engine_intelligence::network_auxiliary::SITES {
        add(
            "network_auxiliary",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for site in &engine_intelligence::intel_source_sites::SITES {
        add("intel_sources", site.name, site.rva, PATCH, &site.sentinel);
    }
    for (site, _) in script_flag_mutations::MUTATIONS {
        add("flag_changes", site.name, site.rva, PATCH, &site.sentinel);
    }
    for (site, _) in building_mutations::MUTATIONS {
        add(
            "building_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for site in engine_mutations::doctrine_mutations::SITES {
        add(
            "world_input_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for (index, (rva, sentinel)) in engine_mutations::doctrine_mutations::EFFECTIVE_GAIN_STORES
        .iter()
        .enumerate()
    {
        add(
            "world_input_changes",
            &format!("doctrine_gain_{index}"),
            *rva,
            tap(sentinel),
            &sentinel[..tap(sentinel)],
        );
    }
    // The advisor pool family's last site, the unappointment helper, is
    // country_input_changes' patch, which forwards it.
    let (unappoint, pool_sites) = advisor_pool_mutations::MUTATIONS.split_last().unwrap();
    assert_eq!(
        unappoint.0.rva,
        country_input_mutations::MUTATIONS[13].0.rva
    );
    for (site, _) in pool_sites {
        add(
            "advisor_pool_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for (site, _) in party_leader_mutations::MUTATIONS {
        add(
            "party_leader_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for (site, _, _) in script_variable_mutations::MUTATIONS {
        add(
            "variable_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for site in political_party_mutations::MUTATIONS {
        add("party_changes", site.name, site.rva, PATCH, &site.sentinel);
    }
    for (site, _) in air_wing_equipment_mutations::MUTATIONS {
        add("wing_changes", site.name, site.rva, PATCH, &site.sentinel);
    }
    for site in air_wing_equipment_mutations::MAP_MUTATIONS {
        // Includes direct incoming-wing append/erase. Their disjoint base
        // receiver certifies transfer order without changing wing equipment.
        // Ordinary entry detours publish five bytes; retour owns relocation.
        add("wing_changes", site.name, site.rva, PATCH, &site.sentinel);
    }
    for (site, _) in character_roster_mutations::MUTATIONS {
        add("roster_changes", site.name, site.rva, PATCH, &site.sentinel);
    }
    // Entry detours and replaced five-byte CALLs alike patch five bytes.
    for (site, _, _) in political_status_mutations::MUTATIONS {
        add(
            "political_status_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for site in regional_mission_mutations::MUTATIONS {
        add(
            "mission_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    add(
        "air_detection_overrides",
        "console vector writer",
        engine_map_air::map_air_sortie_inputs::DETECTION_OVERRIDE_WRITER_RVA,
        PATCH,
        &hashed_sentinel(
            engine_map_air::map_air_sortie_inputs::DETECTION_OVERRIDE_WRITER_RVA,
            engine_map_air::map_air_sortie_inputs::DETECTION_OVERRIDE_WRITER_PREFIX_HASH,
        ),
    );
    add(
        "radar_symbols",
        "group update",
        engine_map_air::radar_group::UPDATE_RVA,
        PATCH,
        &hashed_sentinel(
            engine_map_air::radar_group::UPDATE_RVA,
            engine_map_air::radar_group::UPDATE_PREFIX_HASH,
        ),
    );
    for (name, rva, hash) in [
        (
            "enable",
            engine_map_air::map_air_country_region_modifier::GROUND_CREWS_ENABLE_RVA,
            engine_map_air::map_air_country_region_modifier::GROUND_CREWS_COMMAND_PREFIX_HASH,
        ),
        (
            "disable",
            engine_map_air::map_air_country_region_modifier::GROUND_CREWS_DISABLE_RVA,
            engine_map_air::map_air_country_region_modifier::GROUND_CREWS_COMMAND_PREFIX_HASH,
        ),
        (
            "construction",
            engine_map_air::map_air_country_region_modifier::GROUND_CREWS_CONSTRUCTOR_RVA,
            engine_map_air::map_air_country_region_modifier::GROUND_CREWS_CONSTRUCTOR_PREFIX_HASH,
        ),
    ] {
        add(
            "ground_crews",
            name,
            rva,
            PATCH,
            &hashed_sentinel(rva, hash),
        );
    }
    for (site, _) in railway_mutations::MUTATIONS {
        add("rail_changes", site.name, site.rva, PATCH, &site.sentinel);
    }
    for (site, _) in territory_mutations::MUTATIONS {
        add(
            "territory_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for site in engine_mutations::division_panel_mutations::MUTATIONS {
        add(
            "division_panel_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    add(
        "map_events",
        "scripted_refresh",
        map_event_sites::SCRIPTED_REFRESH_CALL_RVA,
        PATCH,
        &map_event_sites::SCRIPTED_REFRESH_CALL_SENTINEL,
    );
    for site in &engine_mutations::war_participation_sites::SITES {
        add(
            "war_participation",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for site in army_stat_mutations::MUTATIONS {
        add(
            "army_stat_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for site in ship_model_mutations::MUTATIONS {
        add(
            "ship_model_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    // The whole descriptor list includes recruitment, exile resources, crew
    // writers and sent-expeditionary references. Other families observe these
    // sites through the country-input forwarder, without a second patch.
    for (name, rva, len, sentinel) in collector_naval_management::changes::patch_sentinels() {
        add("naval_management_changes", name, rva, len, &sentinel);
    }
    for (site, _, _) in country_input_mutations::MUTATIONS {
        add(
            "country_input_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for (site, _) in raid_mutations::MUTATIONS {
        add("raid_changes", site.name, site.rva, PATCH, &site.sentinel);
    }
    let daily = raid_mutations::DAILY_CAPACITIES;
    add(
        "raid_changes",
        daily.name,
        daily.rva,
        PATCH,
        &daily.sentinel,
    );
    // The naval tree family patches only its commander sites; its country
    // sites are the patches of the families that forward them.
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
        add(
            "naval_tree_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for site in engine_native::cwar_goal::mutations::SITES {
        add(
            "wargoal_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    // Daily diplomacy also expires goals; occupation owns that one patch and
    // forwards a notification spanning the native call to wargoal_changes.
    for site in engine_occupation::cresistance::changes::SITES {
        add(
            "occupation_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for (site, _) in world_input_mutations::MUTATIONS {
        // World inputs also owns every MIO roster/policy/design entry below;
        // dependent design observers receive the shared entries by forwarding.
        add(
            "world_input_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for site in air_record_mutations::MUTATIONS {
        add(
            "air_record_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for (site, _) in local_subunit_mutations::MUTATIONS {
        add(
            "local_subunit_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    // The complete manifest includes template data, country configuration,
    // occupation lifecycle and HQ deployment detours; none are taps.
    for (site, _) in template_mutations::MUTATIONS {
        add(
            "template_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for (site, _, patch) in faction_companion_mutations::MUTATIONS {
        let len = if matches!(patch, faction_companion_mutations::Patch::Tap) {
            tap(&site.sentinel)
        } else {
            PATCH
        };
        add(
            "faction_companions",
            site.name,
            site.rva,
            len,
            &site.sentinel[..if matches!(patch, faction_companion_mutations::Patch::Tap) {
                len
            } else {
                site.sentinel.len()
            }],
        );
    }
    for (site, _, patch) in faction_goal_mutations::MUTATIONS {
        let len = if matches!(patch, faction_goal_mutations::Patch::Tap) {
            tap(&site.sentinel)
        } else {
            PATCH
        };
        add(
            "faction_goals",
            site.name,
            site.rva,
            len,
            &site.sentinel[..if matches!(patch, faction_goal_mutations::Patch::Tap) {
                len
            } else {
                site.sentinel.len()
            }],
        );
    }
    for site in province_unit_mutations::MUTATIONS {
        add(
            "province_unit_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for site in province_combat_mutations::MUTATIONS {
        add(
            "province_combat_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for (site, _) in air_base_mutations::MUTATIONS {
        add(
            "air_base_carry_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for site in opinion_mutations::MUTATIONS
        .iter()
        .chain(&opinion_mutations::RELATIONS)
    {
        add(
            "opinion_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for site in equipment_destruction::SITES {
        add(
            "equipment_destruction",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for index in crate::market_changes::OBSERVED_SITES {
        let site = &market_mutations::SITES[index];
        let len = if index == market_mutations::FUEL_SENT_SITE
            || index == market_mutations::FUEL_LOST_SITE
        {
            tap(&site.sentinel)
        } else {
            PATCH
        };
        add(
            "market_changes",
            site.name,
            site.rva,
            len,
            &site.sentinel[..if len != PATCH {
                len
            } else {
                site.sentinel.len()
            }],
        );
    }
    for (index, site) in naval_result_sources::SITES.into_iter().enumerate() {
        add(
            "naval_result_sources",
            &format!("site {index}"),
            site.rva,
            PATCH,
            &site.sentinel[..site.sentinel_len],
        );
    }
    for (index, site) in experience_gain_sites::SITES.iter().enumerate() {
        add(
            "experience_gains",
            &format!("tap {index}"),
            site.rva,
            tap(&site.sentinel),
            &site.sentinel[..tap(&site.sentinel)],
        );
    }
    for (index, site) in air_loss_sites::SITES.iter().enumerate() {
        add(
            "air_losses",
            &format!("call {index}"),
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    add(
        "building_bombing",
        "ApplyDamage",
        building_damage_sites::APPLY_DAMAGE_RVA,
        PATCH,
        &building_damage_sites::APPLY_DAMAGE_SENTINEL,
    );
    for site in nuclear_strike_sites::SITES {
        add(
            "nuclear_strikes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for (name, rva, sentinel) in [
        (
            "credit",
            line_output_sites::CREDIT.rva,
            line_output_sites::CREDIT.sentinel,
        ),
        (
            "delivery",
            line_output_sites::DELIVERY.rva,
            line_output_sites::DELIVERY.sentinel,
        ),
        (
            "rocket",
            line_output_sites::ROCKET_ADVANCE.rva,
            line_output_sites::ROCKET_ADVANCE.sentinel,
        ),
    ] {
        add("line_output", name, rva, PATCH, &sentinel);
    }
    for site in crate::construction_ic::SITES {
        let site: &construction_ic_sites::Site = site;
        add(
            "construction_ic",
            "construction",
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for (name, site) in [
        ("reset", &world_lifetime_sites::RESET),
        ("construct", &world_lifetime_sites::CONSTRUCT),
    ] {
        add(
            "world_lifetime",
            name,
            site.rva,
            tap(&site.sentinel),
            &site.sentinel[..tap(&site.sentinel)],
        );
    }
    for (name, site) in [
        ("delete", &world_lifetime_sites::DELETE),
        ("state_reload", &world_lifetime_sites::STATE_RELOAD),
    ] {
        add("world_lifetime", name, site.rva, PATCH, &site.sentinel);
    }
    for (name, rva, sentinel) in [
        (
            "finalize",
            land_events::FINALIZE_RVA,
            land_events::FINALIZE_SENTINEL,
        ),
        (
            "destroy",
            land_events::DESTROY_RVA,
            land_events::DESTROY_SENTINEL,
        ),
        (
            "border_finalize",
            land_events::BORDER_FINALIZE_RVA,
            land_events::BORDER_FINALIZE_SENTINEL,
        ),
        (
            "border_terminate",
            land_events::BORDER_TERMINATE_RVA,
            land_events::BORDER_TERMINATE_SENTINEL,
        ),
        (
            "dispatch",
            land_events::DISPATCH_RVA,
            land_events::DISPATCH_SENTINEL,
        ),
        (
            "destruction",
            land_events::DESTRUCTION_RVA,
            land_events::DESTRUCTION_SENTINEL,
        ),
        (
            "lifecycle",
            land_wipe::LIFECYCLE_RVA,
            land_wipe::LIFECYCLE_SENTINEL,
        ),
    ] {
        let len = if rva == land_wipe::LIFECYCLE_RVA {
            8
        } else {
            PATCH
        };
        add("land_events", name, rva, len, &sentinel);
    }
    for ((name, rva), sentinel) in land_events::DISPATCH_LEADER_CALL_NAMES
        .into_iter()
        .zip(land_events::DISPATCH_LEADER_CALLS)
        .zip(LEADER_CALL_SENTINELS)
    {
        assert_eq!(
            sentinel[..INDIRECT],
            land_events::DISPATCH_LEADER_CALL_SENTINEL
        );
        add("land_events", name, rva, INDIRECT, &sentinel);
    }
    add(
        "combat_log_prune",
        "prune",
        ncombat_log_cmanager::PRUNE_RVA,
        PATCH,
        &ncombat_log_cmanager::PRUNE_SENTINEL,
    );
    for (name, rva, sentinel) in [
        (
            "group_create_call",
            combat_log_sites::GROUP_CREATE_CALL_RVA,
            combat_log_sites::GROUP_CREATE_CALL_SENTINEL,
        ),
        (
            "own_equipment",
            combat_log_sites::OWN_EQUIPMENT_RVA,
            combat_log_sites::OWN_EQUIPMENT_SENTINEL,
        ),
        (
            "own_variant",
            combat_log_sites::OWN_VARIANT_RVA,
            combat_log_sites::OWN_VARIANT_SENTINEL,
        ),
        (
            "enemy",
            combat_log_sites::ENEMY_RVA,
            combat_log_sites::ENEMY_SENTINEL,
        ),
        (
            "recovered",
            combat_log_sites::RECOVERED_RVA,
            combat_log_sites::RECOVERED_SENTINEL,
        ),
        (
            "manpower",
            combat_log_sites::MANPOWER_RVA,
            combat_log_sites::MANPOWER_SENTINEL,
        ),
        (
            "result",
            combat_log_sites::RESULT_RVA,
            combat_log_sites::RESULT_SENTINEL,
        ),
        (
            "group_prune",
            combat_log_sites::GROUP_PRUNE_RVA,
            combat_log_sites::GROUP_PRUNE_SENTINEL,
        ),
        (
            "group_destroy",
            combat_log_sites::GROUP_DESTROY_RVA,
            combat_log_sites::GROUP_DESTROY_SENTINEL,
        ),
        (
            "battle_allocate",
            combat_log_sites::BATTLE_ALLOCATE_RVA,
            combat_log_sites::BATTLE_ALLOCATE_SENTINEL,
        ),
        (
            "battle_release",
            combat_log_sites::BATTLE_RELEASE_RVA,
            combat_log_sites::BATTLE_RELEASE_SENTINEL,
        ),
        (
            "battle_reset",
            combat_log_sites::BATTLE_RESET_RVA,
            combat_log_sites::BATTLE_RESET_SENTINEL,
        ),
        (
            "battle_load",
            combat_log_sites::BATTLE_LOAD_RVA,
            combat_log_sites::BATTLE_LOAD_SENTINEL,
        ),
    ] {
        add("combat_log_history", name, rva, PATCH, &sentinel);
    }
    add(
        "combat_log_history",
        "template_counter",
        combat_log_sites::TEMPLATE_COUNTER_RVA,
        tap(&combat_log_sites::TEMPLATE_COUNTER_SENTINEL),
        &combat_log_sites::TEMPLATE_COUNTER_SENTINEL
            [..tap(&combat_log_sites::TEMPLATE_COUNTER_SENTINEL)],
    );
    for (rva, sentinel, _) in clogistics_status::BALANCE_CALLS {
        add("logistics_balance", "balance call", rva, PATCH, &sentinel);
    }
    for (rva, sentinel, _) in clogistics_status::HISTORY_CALLS {
        add("logistics_balance", "history call", rva, PATCH, &sentinel);
    }
    let mut fuel = vec![fuel_awards::BEGIN, fuel_awards::END];
    fuel.extend(fuel_awards::LAND);
    fuel.extend([fuel_awards::NAVAL, fuel_awards::AIR, fuel_awards::RESET]);
    for (index, (rva, sentinel)) in fuel.into_iter().enumerate() {
        add(
            "fuel_awards",
            &format!("tap {index}"),
            rva,
            tap(&sentinel),
            &sentinel[..tap(&sentinel)],
        );
    }
    add(
        "operation_outcomes",
        "completion",
        operation_outcome_sites::COMPLETION_RVA,
        PATCH,
        &operation_outcome_sites::COMPLETION_SENTINEL,
    );
    for site in crate::politics_changes::entry_sites() {
        add(
            "politics_changes",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for site in crate::politics_changes::TAPS {
        add(
            "politics_changes",
            site.name,
            site.rva,
            tap(&site.sentinel),
            &site.sentinel[..tap(&site.sentinel)],
        );
    }
    for (name, site) in [
        ("war", event_feed_sites::WAR),
        ("annex", event_feed_sites::ANNEX),
    ] {
        add("event_feed_observer", name, site.rva, PATCH, &site.sentinel);
    }
    for entry in event_feed_sites::provenance::ENTRIES {
        add(
            "event_feed_observer",
            entry.name,
            entry.rva,
            PATCH,
            &entry.sentinel,
        );
    }
    for site in engine_events::script_event_sites::TAPS {
        add(
            "event_feed_observer",
            site.name,
            site.rva,
            tap(&site.sentinel),
            &site.sentinel[..tap(&site.sentinel)],
        );
    }
    for site in engine_events::script_event_sites::REGISTRY_ENTRIES
        .into_iter()
        .chain([engine_events::script_event_sites::ACTIVATE_TARGETED])
    {
        add(
            "event_feed_observer",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for (name, rva, sentinel) in engine_decisions::cdecision::EVENT_FEED_ENTRIES {
        add("event_feed_observer", name, rva, PATCH, &sentinel);
    }
    // The duration taps and the candidate clear tap are descriptors only:
    // their enclosing CALL and entry detours replace them.
    for site in engine_events::script_event_sites::LIFECYCLE_TAPS
        .into_iter()
        .chain(engine_events::script_event_sites::decisions::TAPS)
        .filter(|site| {
            site.rva != engine_events::script_event_sites::decisions::CLEAR_CANDIDATES.rva
        })
    {
        add(
            if site.rva
                == engine_events::script_event_sites::decisions::TARGETED_SELECTION_RESULT.rva
            {
                "politics_changes"
            } else {
                "event_feed_observer"
            },
            site.name,
            site.rva,
            tap(&site.sentinel),
            &site.sentinel[..tap(&site.sentinel)],
        );
    }
    for site in engine_events::script_event_sites::DELAYED_ENTRIES
        .into_iter()
        .take(2)
        .chain(engine_events::script_event_sites::LIFECYCLE_ENTRIES)
        .chain(engine_events::script_event_sites::decisions::ENTRIES)
    {
        add(
            "event_feed_observer",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel,
        );
    }
    for site in engine_events::script_event_sites::decisions::CALLS {
        let verified = engine_events::script_event_sites::decisions::call_sentinel_len(site);
        add(
            "event_feed_observer",
            site.name,
            site.rva,
            PATCH,
            &site.sentinel[..verified],
        );
    }
    add(
        "variable_changes",
        engine_events::script_event_sites::DELAYED_PASS.name,
        engine_events::script_event_sites::DELAYED_PASS.rva,
        PATCH,
        &engine_events::script_event_sites::DELAYED_PASS.sentinel,
    );
    for site in engine_events::script_event_sites::DELAYED_TAPS {
        add(
            "variable_changes",
            site.name,
            site.rva,
            tap(&site.sentinel),
            &site.sentinel[..tap(&site.sentinel)],
        );
    }
    add(
        "peace_settlements",
        "finalizer",
        peace_finalizer_site::RVA,
        PATCH,
        &peace_finalizer_site::SENTINEL,
    );
    out
}
