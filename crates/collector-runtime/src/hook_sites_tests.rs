use engine_mutations::{
    army_stat_mutations, template_mutations, unit_leader_mutations, world_input_mutations,
};
use engine_production::market_mutations;

use crate::hook_inventory::{PATCH, patches};
use collector_hook_failures::Patch;
use engine_sites::Rva;

fn runtime_hook_families() -> std::collections::BTreeSet<&'static str> {
    crate::runtime::hook_patch_families().collect()
}

fn missing_runtime_families(patches: &[Patch]) -> Vec<&'static str> {
    let registered = patches.iter().map(|patch| patch.family).collect();
    runtime_hook_families()
        .difference(&registered)
        .copied()
        .collect()
}

fn assert_family_completeness(patches: &[Patch]) {
    let missing = missing_runtime_families(patches);
    assert!(
        missing.is_empty(),
        "runtime hook families missing from patch registry: {}",
        missing.join(", ")
    );
}

#[test]
fn naval_management_integration_registers_every_installed_patch() {
    let patches = patches();
    let naval: Vec<_> = patches
        .iter()
        .filter(|patch| patch.family == "naval_management_changes")
        .collect();
    assert_eq!(naval.len(), 68);
    assert!(
        naval
            .iter()
            .any(|patch| patch.name == "repair split command")
    );
    for (name, rva) in collector_naval_management::changes::patch_sites() {
        assert_eq!(
            naval
                .iter()
                .filter(|patch| patch.name == name && patch.rva == rva)
                .count(),
            1
        );
    }
}

#[test]
fn market_inventory_covers_exactly_the_installed_detours_and_fuel_taps() {
    let registered: Vec<_> = patches()
        .into_iter()
        .filter(|patch| patch.family == "market_changes")
        .collect();
    assert_eq!(
        registered.len(),
        collector_observers::market_changes::OBSERVED_SITES.len()
    );
    for (patch, index) in registered
        .iter()
        .zip(collector_observers::market_changes::OBSERVED_SITES)
    {
        let site = &market_mutations::SITES[index];
        assert_eq!(patch.name, site.name);
        assert_eq!(patch.rva, site.rva);
        assert_eq!(
            patch.len,
            if [
                market_mutations::FUEL_SENT_SITE,
                market_mutations::FUEL_LOST_SITE
            ]
            .contains(&index)
            {
                None
            } else {
                Some(PATCH)
            }
        );
    }
    let removal = &market_mutations::SITES[market_mutations::CONTRACT_REMOVAL_SITE];
    assert!(registered.iter().any(|patch| patch.name == removal.name
        && patch.rva == removal.rva
        && patch.len == Some(PATCH)));
    for nested in [1, 2, 22, 26, 27, 28] {
        assert!(
            registered
                .iter()
                .all(|patch| patch.rva != market_mutations::SITES[nested].rva)
        );
    }
}

#[test]
fn no_two_hooks_share_a_site_or_overlap() {
    let mut patches = patches();
    for name in [
        "mio_queue_append",
        "mio_state_copy",
        "mio_allowed_policies",
        "mio_queue_move",
        "mio_read_state",
        "mio_queue_remove",
        "mio_queue_replace",
        "mio_unlock_trait",
        "design_mio_bonuses",
        "design_mio_link",
        "design_mio_move",
        "design_mio_clone",
        "design_mio_read",
        "mio_policy_score_refresh",
    ] {
        let (site, _) = world_input_mutations::MUTATIONS
            .iter()
            .find(|(site, _)| site.name == name)
            .expect("MIO site belongs to the world-input catalogue");
        let registered: Vec<_> = patches.iter().filter(|patch| patch.name == name).collect();
        assert_eq!(registered.len(), 1, "{name} needs one patch owner");
        assert_eq!(registered[0].family, "world_input_changes", "{name}");
        assert_eq!(
            (registered[0].rva, registered[0].len),
            (site.rva, Some(PATCH))
        );
    }
    assert_family_completeness(&patches);
    assert!(
        patches
            .iter()
            .all(|patch| patch.len.is_none_or(|len| len >= PATCH))
    );
    patches.sort_by_key(|patch| patch.rva);
    let collisions: Vec<String> = patches
        .windows(2)
        .filter(|pair| pair[0].rva.0 + pair[0].len.unwrap_or(PATCH) > pair[1].rva.0)
        .map(|pair| {
            format!(
                "{} {} at {:#x}+{:?} overlaps {} {} at {:#x}",
                pair[0].family,
                pair[0].name,
                pair[0].rva.0,
                pair[0].len,
                pair[1].family,
                pair[1].name,
                pair[1].rva.0
            )
        })
        .collect();
    assert!(collisions.is_empty(), "{collisions:#?}");
}

#[test]
fn experience_and_progression_families_register_every_descriptor() {
    let patches = patches();
    for (family, sites) in [
        (
            "division_panel_changes",
            engine_mutations::division_panel_mutations::MUTATIONS.as_slice(),
        ),
        (
            "army_stat_changes",
            army_stat_mutations::MUTATIONS.as_slice(),
        ),
        (
            "unit_leader_changes",
            unit_leader_mutations::MUTATIONS.as_slice(),
        ),
    ] {
        assert_eq!(
            patches
                .iter()
                .filter(|patch| patch.family == family)
                .count(),
            sites.len()
        );
        for site in sites {
            assert!(patches.iter().any(|patch| patch.family == family
                && patch.name == site.name
                && patch.rva == site.rva
                && patch.len == Some(PATCH)));
        }
    }
}

#[test]
fn faction_phase_and_advisor_sites_are_registered_and_shared_resource_helpers_are_untapped() {
    let registered = patches();
    for (site, _, _) in engine_mutations::faction_companion_mutations::MUTATIONS {
        assert_eq!(
            registered
                .iter()
                .filter(|patch| patch.rva == site.rva)
                .count(),
            1,
            "{}",
            site.name
        );
    }
    let names = engine_mutations::faction_companion_mutations::MUTATIONS
        .iter()
        .map(|(site, _, _)| site.name)
        .collect::<Vec<_>>();
    for required in [
        "complete_system_extraction",
        "influence_refresh_boundary",
        "rank_refresh_boundary",
        "rank_input_receiver",
        "rank_input_sum",
        "resize_advisor_slots",
        "select_advisor_slot",
        "projection_resource_operand",
        "projection_other_air_operand",
    ] {
        assert!(names.contains(&required), "{required}");
    }
    assert!(
        !names
            .iter()
            .any(|name| name.contains("extraction_call") || *name == "rebuild_advisor_slots")
    );
    const SHARED_POOL_CLEAR: Rva = Rva(0xc22500);
    const SHARED_POOL_ADD_ASSIGN: Rva = Rva(0xc20ee0);
    for rva in [SHARED_POOL_CLEAR, SHARED_POOL_ADD_ASSIGN] {
        assert!(
            !engine_mutations::faction_companion_mutations::MUTATIONS
                .iter()
                .any(|(site, _, _)| site.rva == rva)
        );
    }
}

#[test]
fn every_template_detour_is_registered_and_occupation_reset_has_one_owner() {
    let patches = patches();
    let registered = patches
        .iter()
        .filter(|patch| patch.family == "template_changes")
        .collect::<Vec<_>>();
    assert_eq!(registered.len(), template_mutations::MUTATIONS.len());
    for (site, _) in template_mutations::MUTATIONS {
        assert_eq!(
            registered
                .iter()
                .filter(|patch| patch.name == site.name
                    && patch.rva == site.rva
                    && patch.len == Some(PATCH))
                .count(),
            1,
            "{}",
            site.name
        );
    }
    let reset = patches
        .iter()
        .filter(|patch| {
            patch
                .rva
                .0
                .abs_diff(engine_occupation::ccountry_occupation_status::RESET_RVA.0)
                <= 16
        })
        .collect::<Vec<_>>();
    assert_eq!(reset.len(), 1, "{reset:?}");
    assert_eq!(reset[0].family, "template_changes");
    assert_eq!(reset[0].name, "reset_occupation_status");
}

#[test]
fn every_orders_group_detour_is_registered_and_group_destruction_has_one_owner() {
    let patches = patches();
    let registered = patches
        .iter()
        .filter(|patch| patch.family == "orders_group_changes")
        .collect::<Vec<_>>();
    assert_eq!(
        registered.len(),
        engine_mutations::orders_group_mutations::MUTATIONS.len()
    );
    for (site, _) in engine_mutations::orders_group_mutations::MUTATIONS {
        assert_eq!(
            registered
                .iter()
                .filter(|patch| patch.name == site.name
                    && patch.rva == site.rva
                    && patch.len == Some(PATCH))
                .count(),
            1,
            "{}",
            site.name
        );
    }
    let destroy = &template_mutations::MUTATIONS
        .iter()
        .find(|(site, _)| site.name == "destroy_hq_group")
        .unwrap()
        .0;
    let owners = patches
        .iter()
        .filter(|patch| patch.rva == destroy.rva)
        .collect::<Vec<_>>();
    assert_eq!(owners.len(), 1, "{owners:?}");
    assert_eq!(owners[0].family, "template_changes");
}
