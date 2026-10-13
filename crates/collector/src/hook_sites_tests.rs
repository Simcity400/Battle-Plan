//! Every native site any collector hook family patches, with the bytes each
//! patch replaces. A site has one patch owner: two families patching one
//! entry, or patches whose spans overlap, make publication fail at attach
//! (and with it every family enabled after the failing one). Dependent
//! observers dispatch through the owner's detour instead.
use engine::{
    advisor_mutations, advisor_pool_mutations, air_base_mutations, air_history_mutations,
    air_loss_sites, air_record_mutations, air_wing_equipment_mutations, army_stat_mutations,
    building_damage_sites, building_mutations, ccharacter_portraits, cgame_state,
    character_role_mutations, character_roster_mutations, clogistics_status, construction_ic_sites,
    country_input_mutations, diplomacy_mutations, equipment_destruction, event_feed_sites,
    experience_gain_sites, fuel_awards, land_events, land_wipe, line_output_sites,
    local_subunit_mutations, naval_result_sources, naval_tree_mutations, ncombat_log_cmanager,
    nuclear_strike_sites, operation_outcome_sites, opinion_mutations, party_leader_mutations,
    peace_finalizer_site, political_party_mutations, political_status_mutations,
    province_unit_mutations, raid_mutations, railway_mutations, regional_mission_mutations,
    script_flag_mutations, script_variable_mutations, ship_model_mutations, template_mutations,
    territory_mutations, world_input_mutations, world_lifetime_sites,
};

/// One patch: its family, site name, RVA and the bytes it replaces.
#[derive(Clone, Debug)]
struct Patch {
    family: &'static str,
    name: String,
    rva: usize,
    len: usize,
}

/// A detour or replaced CALL patches five bytes.
const PATCH: usize = 5;
/// A replaced `CALL [RAX+disp32]` patches six.
const INDIRECT: usize = 6;

/// A tap owns the instructions it relocates; the epilogue form (span 0)
/// writes its CALL, the copied epilogue and the moved JMP: 15 bytes.
fn tap(sentinel: &[u8; 16]) -> usize {
    match crate::hook::tap_span(sentinel).expect("tap sentinel decodes") {
        0 => 15,
        span => span,
    }
}

/// Every site every family installs, as runtime.rs constructs them.
fn patches() -> Vec<Patch> {
    let mut out = Vec::new();
    let mut add = |family: &'static str, name: &str, rva: usize, len: usize| {
        out.push(Patch {
            family,
            name: name.to_owned(),
            rva,
            len,
        })
    };
    add(
        "hourly",
        "HourlyUpdate",
        cgame_state::HOURLY_UPDATE_RVA,
        PATCH,
    );
    for site in ccharacter_portraits::MUTATIONS {
        add("portrait_changes", site.name, site.rva, PATCH);
    }
    for site in advisor_mutations::MUTATIONS {
        add("advisor_changes", site.name, site.rva, PATCH);
    }
    for site in character_role_mutations::MUTATIONS {
        add("advisor_changes", site.name, site.rva, PATCH);
    }
    for site in diplomacy_mutations::MUTATIONS {
        add("relation_changes", site.name, site.rva, PATCH);
    }
    for site in air_history_mutations::MUTATIONS {
        add("history_changes", site.name, site.rva, PATCH);
    }
    for (site, _) in script_flag_mutations::MUTATIONS {
        add("flag_changes", site.name, site.rva, PATCH);
    }
    for (site, _) in building_mutations::MUTATIONS {
        add("building_changes", site.name, site.rva, PATCH);
    }
    // The advisor pool family's last site, the unappointment helper, is
    // country_input_changes' patch, which forwards it.
    let (unappoint, pool_sites) = advisor_pool_mutations::MUTATIONS.split_last().unwrap();
    assert_eq!(
        unappoint.0.rva,
        country_input_mutations::MUTATIONS[13].0.rva
    );
    for (site, _) in pool_sites {
        add("advisor_pool_changes", site.name, site.rva, PATCH);
    }
    for (site, _) in party_leader_mutations::MUTATIONS {
        add("party_leader_changes", site.name, site.rva, PATCH);
    }
    for (site, _, _) in script_variable_mutations::MUTATIONS {
        add("variable_changes", site.name, site.rva, PATCH);
    }
    for site in political_party_mutations::MUTATIONS {
        add("party_changes", site.name, site.rva, PATCH);
    }
    for (site, _) in air_wing_equipment_mutations::MUTATIONS {
        add("wing_changes", site.name, site.rva, PATCH);
    }
    for (site, _) in character_roster_mutations::MUTATIONS {
        add("roster_changes", site.name, site.rva, PATCH);
    }
    // Entry detours and replaced five-byte CALLs alike patch five bytes.
    for (site, _, _) in political_status_mutations::MUTATIONS {
        add("political_status_changes", site.name, site.rva, PATCH);
    }
    for site in regional_mission_mutations::MUTATIONS {
        add("mission_changes", site.name, site.rva, PATCH);
    }
    for (site, _) in railway_mutations::MUTATIONS {
        add("rail_changes", site.name, site.rva, PATCH);
    }
    for (site, _) in territory_mutations::MUTATIONS {
        add("territory_changes", site.name, site.rva, PATCH);
    }
    for site in army_stat_mutations::MUTATIONS {
        add("army_stat_changes", site.name, site.rva, PATCH);
    }
    for site in ship_model_mutations::MUTATIONS {
        add("ship_model_changes", site.name, site.rva, PATCH);
    }
    for (site, _, _) in country_input_mutations::MUTATIONS {
        add("country_input_changes", site.name, site.rva, PATCH);
    }
    for (site, _) in raid_mutations::MUTATIONS {
        add("raid_changes", site.name, site.rva, PATCH);
    }
    let daily = raid_mutations::DAILY_CAPACITIES;
    add("raid_changes", daily.name, daily.rva, PATCH);
    // The naval tree family patches only its commander sites; its country
    // sites are the patches of the families that forward them.
    let country = naval_tree_mutations::COUNTRY_SITES;
    assert_eq!(
        naval_tree_mutations::MUTATIONS[0].rva,
        country_input_mutations::MUTATIONS[0].0.rva
    );
    assert_eq!(
        naval_tree_mutations::MUTATIONS[1].rva,
        engine::politics_sites::RESET_COUNTRY.rva
    );
    for site in &naval_tree_mutations::MUTATIONS[country..] {
        add("naval_tree_changes", site.name, site.rva, PATCH);
    }
    for (site, _) in world_input_mutations::MUTATIONS {
        add("world_input_changes", site.name, site.rva, PATCH);
    }
    for site in air_record_mutations::MUTATIONS {
        add("air_record_changes", site.name, site.rva, PATCH);
    }
    for (site, _) in local_subunit_mutations::MUTATIONS {
        add("local_subunit_changes", site.name, site.rva, PATCH);
    }
    for (site, _) in template_mutations::MUTATIONS {
        add("template_changes", site.name, site.rva, PATCH);
    }
    for site in province_unit_mutations::MUTATIONS {
        add("province_unit_changes", site.name, site.rva, PATCH);
    }
    for (site, _) in air_base_mutations::MUTATIONS {
        add("air_base_carry_changes", site.name, site.rva, PATCH);
    }
    for site in opinion_mutations::MUTATIONS
        .iter()
        .chain(&opinion_mutations::RELATIONS)
    {
        add("opinion_changes", site.name, site.rva, PATCH);
    }
    for site in equipment_destruction::SITES {
        add("equipment_destruction", site.name, site.rva, PATCH);
    }
    for (index, site) in naval_result_sources::SITES.into_iter().enumerate() {
        add(
            "naval_result_sources",
            &format!("site {index}"),
            site.rva,
            PATCH,
        );
    }
    for (index, site) in experience_gain_sites::SITES.iter().enumerate() {
        add(
            "experience_gains",
            &format!("tap {index}"),
            site.rva,
            tap(&site.sentinel),
        );
    }
    for (index, site) in air_loss_sites::SITES.iter().enumerate() {
        add("air_losses", &format!("call {index}"), site.rva, PATCH);
    }
    add(
        "building_bombing",
        "ApplyDamage",
        building_damage_sites::APPLY_DAMAGE_RVA,
        PATCH,
    );
    for site in nuclear_strike_sites::SITES {
        add("nuclear_strikes", site.name, site.rva, PATCH);
    }
    for (name, rva) in [
        ("credit", line_output_sites::CREDIT.rva),
        ("delivery", line_output_sites::DELIVERY.rva),
        ("rocket", line_output_sites::ROCKET_ADVANCE.rva),
    ] {
        add("line_output", name, rva, PATCH);
    }
    for site in crate::construction_ic::SITES {
        let site: &construction_ic_sites::Site = site;
        add("construction_ic", "construction", site.rva, PATCH);
    }
    for (name, site) in [
        ("reset", &world_lifetime_sites::RESET),
        ("construct", &world_lifetime_sites::CONSTRUCT),
    ] {
        add("world_lifetime", name, site.rva, tap(&site.sentinel));
    }
    for (name, site) in [
        ("delete", &world_lifetime_sites::DELETE),
        ("state_reload", &world_lifetime_sites::STATE_RELOAD),
    ] {
        add("world_lifetime", name, site.rva, PATCH);
    }
    for (name, rva) in [
        ("finalize", land_events::FINALIZE_RVA),
        ("destroy", land_events::DESTROY_RVA),
        ("border_finalize", land_events::BORDER_FINALIZE_RVA),
        ("border_terminate", land_events::BORDER_TERMINATE_RVA),
        ("dispatch", land_events::DISPATCH_RVA),
        ("destruction", land_events::DESTRUCTION_RVA),
        ("lifecycle", land_wipe::LIFECYCLE_RVA),
    ] {
        add("land_events", name, rva, PATCH);
    }
    for (name, rva) in land_events::DISPATCH_LEADER_CALL_NAMES
        .into_iter()
        .zip(land_events::DISPATCH_LEADER_CALLS)
    {
        add("land_events", name, rva, INDIRECT);
    }
    add(
        "combat_log_prune",
        "prune",
        ncombat_log_cmanager::PRUNE_RVA,
        PATCH,
    );
    for (rva, _, _) in clogistics_status::BALANCE_CALLS {
        add("logistics_balance", "balance call", rva, PATCH);
    }
    for (rva, _, _) in clogistics_status::HISTORY_CALLS {
        add("logistics_balance", "history call", rva, PATCH);
    }
    let mut fuel = vec![fuel_awards::BEGIN, fuel_awards::END];
    fuel.extend(fuel_awards::LAND);
    fuel.extend([fuel_awards::NAVAL, fuel_awards::AIR, fuel_awards::RESET]);
    for (index, (rva, sentinel)) in fuel.into_iter().enumerate() {
        add("fuel_awards", &format!("tap {index}"), rva, tap(&sentinel));
    }
    add(
        "operation_outcomes",
        "completion",
        operation_outcome_sites::COMPLETION_RVA,
        PATCH,
    );
    for site in crate::politics_changes::entry_sites() {
        add("politics_changes", site.name, site.rva, PATCH);
    }
    for site in crate::politics_changes::TAPS {
        add("politics_changes", site.name, site.rva, tap(&site.sentinel));
    }
    for (name, site) in [
        ("war", event_feed_sites::WAR),
        ("annex", event_feed_sites::ANNEX),
    ] {
        add("event_feed_observer", name, site.rva, PATCH);
    }
    for entry in event_feed_sites::provenance::ENTRIES {
        add("event_feed_observer", entry.name, entry.rva, PATCH);
    }
    add(
        "peace_settlements",
        "finalizer",
        peace_finalizer_site::RVA,
        PATCH,
    );
    out
}

#[test]
fn no_two_hooks_share_a_site_or_overlap() {
    let mut patches = patches();
    assert!(patches.iter().all(|patch| patch.len >= PATCH));
    patches.sort_by_key(|patch| patch.rva);
    let collisions: Vec<String> = patches
        .windows(2)
        .filter(|pair| pair[0].rva + pair[0].len > pair[1].rva)
        .map(|pair| {
            format!(
                "{} {} at {:#x}+{} overlaps {} {} at {:#x}",
                pair[0].family,
                pair[0].name,
                pair[0].rva,
                pair[0].len,
                pair[1].family,
                pair[1].name,
                pair[1].rva
            )
        })
        .collect();
    assert!(collisions.is_empty(), "{collisions:#?}");
}
