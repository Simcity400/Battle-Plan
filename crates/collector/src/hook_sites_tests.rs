//! Every native site any collector hook family patches, with the bytes each
//! patch replaces. A site has one patch owner: two families patching one
//! entry, or patches whose spans overlap, make publication fail at attach
//! for that optional family. Dependent
//! observers dispatch through the owner's detour instead.
use engine_mutations::{
    army_stat_mutations, faction_companion_mutations, faction_goal_mutations, template_mutations,
    unit_leader_mutations, world_input_mutations,
};
use engine_production::market_mutations;

use crate::hook_inventory::{PATCH, Patch, patches};

fn runtime_hook_families() -> std::collections::BTreeSet<&'static str> {
    crate::runtime::hook_patch_families().collect()
}

fn timed_hook_families() -> std::collections::BTreeSet<&'static str> {
    use crate::hook_timing::Family;
    Family::ALL
        .into_iter()
        .map(|family| match family {
            // Separate timing buckets share one runtime publication owner.
            Family::PoliticsCapture | Family::PoliticsCaptureTaps => "politics_changes",
            Family::EventFeed | Family::EventFeedProvenance => "event_feed_observer",
            Family::VariantChanges => "design_changes",
            Family::FactionResourceRebuild | Family::FactionRefreshPhases => "faction_companions",
            family => family.name(),
        })
        .collect()
}

fn assert_timing_inventory(
    runtime: &std::collections::BTreeSet<&str>,
    timed: &std::collections::BTreeSet<&str>,
) {
    let missing: Vec<_> = runtime.difference(timed).copied().collect();
    let extra: Vec<_> = timed.difference(runtime).copied().collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "hook family inventories disagree; missing from timing: {}; missing from runtime: {}",
        missing.join(", "),
        extra.join(", ")
    );
}

fn missing_runtime_families(patches: &[Patch]) -> Vec<&'static str> {
    let registered = patches.iter().map(|patch| patch.family).collect();
    runtime_hook_families()
        .difference(&registered)
        .copied()
        .collect()
}

fn assert_family_completeness(patches: &[Patch]) {
    // Timing has its own exhaustive guard; it must not mask patch safety diagnostics.
    let missing = missing_runtime_families(patches);
    assert!(
        missing.is_empty(),
        "runtime hook families missing from patch registry: {}",
        missing.join(", ")
    );
}

#[test]
fn timing_hook_families_match_runtime_restoration() {
    assert_timing_inventory(&runtime_hook_families(), &timed_hook_families());
    println!(
        "{} runtime hook families: 0 timing inventory differences",
        runtime_hook_families().len()
    );
}

#[test]
#[should_panic(expected = "missing from timing: runtime_only; missing from runtime: timing_only")]
fn timing_inventory_differences_are_reported_by_name() {
    assert_timing_inventory(
        &["shared", "runtime_only"].into_iter().collect(),
        &["shared", "timing_only"].into_iter().collect(),
    );
}

#[test]
#[should_panic(expected = "runtime hook families missing from patch registry: unit_leader_changes")]
fn runtime_family_without_registered_patches_fails_completeness() {
    let mut patches = patches();
    patches.retain(|patch| patch.family != "unit_leader_changes");
    assert_family_completeness(&patches);
}

#[test]
fn every_runtime_hook_family_has_registered_patches() {
    let patches = patches();
    assert_family_completeness(&patches);
    println!(
        "{} runtime hook families: 0 missing from patch registry",
        runtime_hook_families().len()
    );
}

#[test]
fn original_family_omissions_are_reported_by_name() {
    let mut patches = patches();
    patches.retain(|patch| {
        !matches!(
            patch.family,
            "unit_leader_changes" | "census_changes" | "map_events"
        )
    });
    assert_eq!(
        missing_runtime_families(&patches),
        ["census_changes", "map_events", "unit_leader_changes"]
    );
}

fn verification_collisions(patches: &[Patch]) -> Vec<String> {
    let mut collisions = Vec::new();
    for (index, verified) in patches.iter().enumerate() {
        for (other, written) in patches.iter().enumerate() {
            if index != other
                && verified.rva < written.rva + written.len
                && written.rva < verified.rva + verified.sentinel.len()
            {
                collisions.push(format!(
                    "{} / {} verification covers {} / {} patch",
                    verified.family, verified.name, written.family, written.name
                ));
            }
        }
    }
    collisions
}

#[test]
fn no_verified_span_overlaps_another_written_patch() {
    let patches = patches();
    assert_family_completeness(&patches);
    for patch in &patches {
        assert!((PATCH..=16).contains(&patch.sentinel.len()), "{patch:?}");
        assert!(patch.sentinel.len() >= patch.len, "{patch:?}");
    }
    let collisions = verification_collisions(&patches);
    assert!(collisions.is_empty(), "{collisions:#?}");
    println!(
        "{} registered hooks: 0 verified-span/written-span overlaps",
        patches.len()
    );
}

#[test]
fn adjacent_decision_pairs_reject_the_original_full_verification_spans() {
    let mut patches = patches();
    for site in [
        engine_events::script_event_sites::decisions::REACTIVATE_TARGETED_DAYS,
        engine_events::script_event_sites::decisions::REQUEST_TARGETED_TIMED,
    ] {
        let patch = patches
            .iter_mut()
            .find(|patch| patch.rva == site.rva)
            .unwrap();
        patch.sentinel = site.sentinel.to_vec();
    }
    let collisions = verification_collisions(&patches);
    assert_eq!(collisions.len(), 2, "{collisions:#?}");
    assert!(
        collisions
            .iter()
            .any(|reason| reason.contains("DecisionReactivateTargetedDays")
                && reason.contains("DecisionReactivateTargetedState"))
    );
    assert!(
        collisions
            .iter()
            .any(|reason| reason.contains("DecisionRequestTargetedTimed")
                && reason.contains("DecisionRequestTargeted patch"))
    );
}

#[test]
fn every_verified_span_matches_the_pinned_executable() {
    let directory = std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR"))
        .expect("hook sentinel checks require BATTLE_PLAN_GAME_DIR with the supported installation");
    let bytes = std::fs::read(std::path::Path::new(&directory).join("hoi4.exe")).unwrap();
    let identity = engine_executable::build::hash_executable(bytes.as_slice()).unwrap();
    assert!(
        engine_executable::build::PINNED_BUILD.matches(&identity),
        "hook sentinel checks require the pinned executable: {identity:?}"
    );
    let image = engine_executable::executable_tables::pe::PeImage::parse(bytes).unwrap();
    let patches = patches();
    assert_family_completeness(&patches);
    let mismatches: Vec<_> = patches
        .iter()
        .filter_map(|patch| {
            let actual = image.bytes(u32::try_from(patch.rva).unwrap(), patch.sentinel.len());
            match actual {
                Ok(actual) if actual == patch.sentinel => None,
                Ok(actual) => Some(format!(
                    "{} / {} sentinel differs: expected {:02x?}, executable {actual:02x?}",
                    patch.family, patch.name, patch.sentinel
                )),
                Err(reason) => Some(format!(
                    "{} / {} executable read failed: {reason}",
                    patch.family, patch.name
                )),
            }
        })
        .collect();
    assert!(mismatches.is_empty(), "{mismatches:#?}");
    println!(
        "{} registered hooks: 0 sentinel mismatches against pinned hoi4.exe",
        patches.len()
    );
}

#[test]
fn naval_management_integration_registers_every_installed_patch() {
    let patches = patches();
    let naval: Vec<_> = patches
        .iter()
        .filter(|patch| patch.family == "naval_management_changes")
        .collect();
    assert_eq!(naval.len(), 66);
    assert!(
        naval
            .iter()
            .any(|patch| patch.name == "repair split command")
    );
    for (name, rva, len) in collector_naval_management::changes::patch_sites() {
        assert_eq!(
            naval
                .iter()
                .filter(|patch| patch.name == name && patch.rva == rva && patch.len == len)
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
        crate::market_changes::OBSERVED_SITES.len()
    );
    for (patch, index) in registered.iter().zip(crate::market_changes::OBSERVED_SITES) {
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
                7
            } else {
                PATCH
            }
        );
    }
    // Active removal precedes deferred destruction and reference retirement.
    let removal = &market_mutations::SITES[market_mutations::CONTRACT_REMOVAL_SITE];
    assert!(
        registered.iter().any(|patch| patch.name == removal.name
            && patch.rva == removal.rva
            && patch.len == PATCH)
    );
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
    // Guard the MIO lane's complete registration through the family catalogue.
    // The three shared design setters/move remain this family's patches.
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
        assert_eq!((registered[0].rva, registered[0].len), (site.rva, PATCH));
    }
    assert_family_completeness(&patches);
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

#[test]
fn game_fog_writers_authenticate_entries_and_leave_nudge_construction_with_buildings() {
    let path = std::path::PathBuf::from(
        std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).expect("native site test requires BATTLE_PLAN_GAME_DIR"),
    )
    .join("hoi4.exe");
    let bytes = std::fs::read(path).unwrap();
    let identity = engine_executable::build::hash_executable(std::io::Cursor::new(&bytes)).unwrap();
    assert!(engine_executable::build::PINNED_BUILD.matches(&identity));
    let image = engine_executable::executable_tables::PeImage::parse(bytes).unwrap();
    let sites: Vec<_> = world_input_mutations::MUTATIONS
        .iter()
        .filter(|(_, domain)| *domain == world_input_mutations::Domain::GameFog)
        .map(|(site, _)| site)
        .collect();
    assert_eq!(sites.len(), 4);
    let nudge = &engine_mutations::building_mutations::MUTATIONS
        .iter()
        .find(|(site, _)| site.name == "enter_nudge_mode")
        .unwrap()
        .0;
    for site in sites {
        assert_ne!(site.rva, nudge.rva);
        assert_eq!(
            image.function(site.rva as u32).unwrap().0,
            site.rva as u32,
            "{}",
            site.name
        );
        assert_eq!(
            image.bytes(site.rva as u32, 16).unwrap(),
            site.sentinel,
            "{}",
            site.name
        );
    }
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
                && patch.len == PATCH));
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
    // These are the generic pool helpers, not faction-specific simulation stores.
    const SHARED_POOL_CLEAR: usize = 0xbbd630;
    const SHARED_POOL_ADD_ASSIGN: usize = 0xbbc010;
    for rva in [SHARED_POOL_CLEAR, SHARED_POOL_ADD_ASSIGN] {
        assert!(
            !engine_mutations::faction_companion_mutations::MUTATIONS
                .iter()
                .any(|(site, _, _)| site.rva == rva)
        );
    }
}

#[test]
fn every_faction_tap_decodes_with_its_family_and_site() {
    let mut failures = Vec::new();
    for (site, _, patch) in faction_companion_mutations::MUTATIONS {
        if matches!(patch, faction_companion_mutations::Patch::Call) {
            assert_eq!(
                site.sentinel[0], 0xe8,
                "{} must be a direct native call",
                site.name
            );
        }
        if matches!(patch, faction_companion_mutations::Patch::Tap)
            && let Err(why) = crate::hook::tap_span(&site.sentinel)
        {
            failures.push(format!("FactionCompanions/{}: {why}", site.name));
        }
    }
    for (site, _, patch) in faction_goal_mutations::MUTATIONS {
        if matches!(patch, faction_goal_mutations::Patch::Tap)
            && let Err(why) = crate::hook::tap_span(&site.sentinel)
        {
            failures.push(format!("FactionGoals/{}: {why}", site.name));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
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
                    && patch.len == PATCH)
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
                .abs_diff(engine_occupation::ccountry_occupation_status::RESET_RVA)
                <= 16
        })
        .collect::<Vec<_>>();
    assert_eq!(reset.len(), 1, "{reset:?}");
    assert_eq!(reset[0].family, "template_changes");
    assert_eq!(reset[0].name, "reset_occupation_status");
}
