use super::*;
use std::collections::BTreeSet;

const UNRELATED_DOMAIN: u16 = 0xBEEF;

/// One journalled tracker: how to put `n` rows into its baseline and read `n` back.
struct JournalProbe {
    field: &'static str,
    advance: fn(&mut CompactPerfGateTrackers, usize),
    size: fn(&CompactPerfGateTrackers) -> usize,
    has_entry: fn(&CompactPerfGateTrackers) -> bool,
}

fn division_sample(div_id: u32, raw_organisation: u32) -> DivisionMovementSample {
    DivisionMovementSample {
        training_scalars: Default::default(),
        first_combat_countries: Default::default(),
        retained_visual_variant: Default::default(),
        owner_authority: Default::default(),
        native_type: DIVISION_TYPE,
        ordinary_route: None, alternate_route: None,
        current_template: None,
        route_head: None,
        template_preference: None, acclimatization: None,
        div_id,
        deleted: false,
        moving: false,
        movement_scalars: compact_contract::division_routes::MovementScalars { numerator_raw: None, ..Default::default() },

        location: 10,
        move_target_province: None,
        movement_source_province: None,
        raw_organisation,
        raw_max_organisation: Some(10_000_000),
        organisation_bucket: division_delta_bucket(raw_organisation),
        raw_strength: 100_000,
        strength_bucket: division_delta_bucket(100_000),
        combat_ref_count: 0,
        in_combat: false,
        owner_tag_db_id: Some(1),
        army_id: None,
        template_id: None,
        raw_experience: None,
        raw_max_supply: None,
        raw_counter_supply_status: None,
        raw_current_supply: None,
        raw_supply_gain: None,
        raw_fuel: None,
        raw_disrupted_supply: None,
        raw_dig_in: None,
        raw_dig_in_cap: None,
        counter_training_visible: None,
        counter_dig_in_frame: None,
        counter_invalid_orders_visible: None,
        strategic_redeployment: None,
        current_manpower: None,
        cumulative_losses: None,
        order_link: DivisionOrderLink::None,
    }
}

fn division_map(n: usize) -> BTreeMap<u32, DivisionMovementSample> {
    (1..=n as u32)
        .map(|id| (id, division_sample(id, 1_000)))
        .collect()
}

fn scripted_row(id: u32) -> (
    Domain51RowKey,
    domain51::scripted_state_domain51_collector::compact::ScriptedStateRow,
) {
    (
        Domain51RowKey::Token {
            scope_kind: 0,
            owner_id: id,
            value_kind: 0,
            token_id: id,
        },
        domain51::scripted_state_domain51_collector::compact::ScriptedStateRow {
            scope_kind: 0,
            owner_id: id,
            value_kind: 0,
            key: None,
            token_id: Some(id),
            value_raw: Some(1),
            date_hour_raw: None,
            deleted: false,
            delayed_due_hours: None,
        },
    )
}

fn attrition_row(id: u32) -> CompactDivisionAttritionRow {
    CompactDivisionAttritionRow {
        raw_division_id: id,
        province_id: 1,
        owner_tag_db_id: 1,
        flags: 0,
        total_raw: 1,
        threshold_raw: 1,
        base_raw: 1,
        modifier_raw: 0,
        deleted: false,
    }
}

const PROBES: &[JournalProbe] = &[
    JournalProbe {
        field: "division_movement",
        advance: |t, n| t.division_movement.advance(division_map(n)),
        size: |t| t.division_movement.len(),
        has_entry: |t| t.division_movement.has_frame_entry(),
    },
    JournalProbe {
        field: "division_equipment",
        advance: |t, n| {
            t.division_equipment.baseline.advance(DivisionEquipmentBaseline {
                previous: BTreeMap::new(),
                fingerprints: (1..=n as u32)
                    .map(|id| {
                        (
                            (1, id),
                            DivisionEquipmentFingerprint {
                                template_id: None,
                                raw_len: id,
                                raw_hash: u64::from(id),
                            },
                        )
                    })
                    .collect(),
            })
        },
        size: |t| t.division_equipment.baseline.fingerprints.len(),
        has_entry: |t| t.division_equipment.baseline.has_frame_entry(),
    },
    JournalProbe {
        field: "map_control",
        advance: |t, n| {
            t.map_control.previous_states.advance(
                (1..=n as u32)
                    .map(|id| {
                        (
                            id,
                            CompactMapStateRow {
                                state_id: id,
                                owner_tag_db_id: 1,
                                controller_tag_db_id: 1,
                            },
                        )
                    })
                    .collect(),
            )
        },
        size: |t| t.map_control.previous_states.len(),
        has_entry: |t| {
            t.map_control.previous_states.has_frame_entry()
                || t.map_control.previous_provinces.has_frame_entry()
        },
    },
    JournalProbe {
        field: "map_detail",
        advance: |t, n| {
            t.map_detail.previous_states.advance(
                (1..=n as u32)
                    .map(|id| CompactMapDetailStateRow {
                        state_id: id,
                        resources: None,
                        buildings: None,
                    })
                    .collect(),
            )
        },
        size: |t| t.map_detail.previous_states.len(),
        has_entry: |t| {
            t.map_detail.previous_states.has_frame_entry()
                || t.map_detail.previous_provinces.has_frame_entry()
        },
    },
    JournalProbe {
        field: "weather",
        advance: |t, n| {
            t.weather.previous_regions.advance(
                (1..=n as u32)
                    .map(|id| {
                        (
                            id,
                            CompactWeatherRegionRow {
                                region_id: id,
                                flags: 0,
                                temperature_raw: 0,
                                phenomenon: 0,
                                next_weather_change_raw: 0,
                                active_modifier_count: 0,
                                intensity: 0,
                            },
                        )
                    })
                    .collect(),
            )
        },
        size: |t| t.weather.previous_regions.len(),
        has_entry: |t| {
            t.weather.previous_regions.has_frame_entry()
                || t.weather.previous_provinces.has_frame_entry()
        },
    },
    JournalProbe {
        field: "supply_logistics_attrition_live",
        advance: |t, n| {
            t.supply_logistics_attrition_live
                .baseline
                .advance(SupplyLogisticsBaseline {
                    last_division_attrition: (1..=n as u32)
                        .map(|id| (id, attrition_row(id)))
                        .collect(),
                    ..SupplyLogisticsBaseline::default()
                })
        },
        size: |t| {
            t.supply_logistics_attrition_live
                .baseline
                .last_division_attrition
                .len()
        },
        has_entry: |t| t.supply_logistics_attrition_live.baseline.has_frame_entry(),
    },
    JournalProbe {
        field: "construction_buildings_state_damage_live",
        advance: |t, n| {
            t.construction_buildings_state_damage_live
                .baseline
                .advance(Domain48Baseline {
                    last_scanned_states: (1..=n as u32).collect(),
                    ..Domain48Baseline::default()
                })
        },
        size: |t| {
            t.construction_buildings_state_damage_live
                .baseline
                .last_scanned_states
                .len()
        },
        has_entry: |t| {
            t.construction_buildings_state_damage_live
                .baseline
                .has_frame_entry()
        },
    },
    JournalProbe {
        field: "scripted_state_flags_variables_live",
        advance: |t, n| {
            t.scripted_state_flags_variables_live
                .last_emitted
                .advance((1..=n as u32).map(scripted_row).collect())
        },
        size: |t| t.scripted_state_flags_variables_live.last_emitted.len(),
        has_entry: |t| {
            t.scripted_state_flags_variables_live
                .last_emitted
                .has_frame_entry()
        },
    },
];

fn probe_tag(field: &str) -> u16 {
    JOURNALLED_TRACKER_RETENTION
        .iter()
        .find(|(name, _)| *name == field)
        .map(|(_, tag)| *tag)
        .unwrap_or_else(|| panic!("`{field}` is a journalled tracker field"))
}

/// The production order: the build's seal, then the sink verdict with nothing
/// withheld.
fn seal(trackers: &mut CompactPerfGateTrackers) {
    commit_delta_pin_transactions(trackers, &mut Vec::new());
    commit_country_staged_advances(trackers, &BTreeSet::new());
}

fn rejected(domains: &[u16]) -> BTreeSet<u16> {
    domains.iter().copied().collect()
}

#[test]
fn every_journalled_tracker_has_a_probe_and_no_probe_names_anything_else() {
    let listed: BTreeSet<&str> = JOURNALLED_TRACKER_RETENTION
        .iter()
        .map(|(field, _)| *field)
        .collect();
    let probed: BTreeSet<&str> = PROBES.iter().map(|probe| probe.field).collect();
    assert_eq!(listed, probed);
    for (field, _) in JOURNALLED_TRACKER_RETENTION {
        assert!(
            !DOMAIN_SCOPED_TRACKER_RETENTION
                .iter()
                .any(|(name, _)| name == field),
            "`{field}` takes the journal, not the blanket carry-over"
        );
    }
}

#[test]
fn own_rejection_preserves_undo_and_stages_required_gap_recovery() {
    for probe in PROBES {
        let mut trackers = CompactPerfGateTrackers::default();
        (probe.advance)(&mut trackers, 1);
        seal(&mut trackers);
        assert!(!(probe.has_entry)(&trackers), "{}", probe.field);
        (probe.advance)(&mut trackers, 2);
        (probe.advance)(&mut trackers, 3);
        assert!((probe.has_entry)(&trackers), "{}", probe.field);
        reset_trackers_after_rejected_bracket(&mut trackers, &rejected(&[probe_tag(probe.field)]));
        let division_gap = probe_tag(probe.field) == COMPACT_DOMAIN_DIVISIONS;
        assert_eq!(
            (probe.size)(&trackers),
            if division_gap { 0 } else { 1 },
            "`{}`: D20 stages a full baseline after its gap; sparse-only domains restore the old pin",
            probe.field
        );
        assert_eq!((probe.has_entry)(&trackers), division_gap, "{}", probe.field);
        if division_gap {
            trackers.division_movement.restore_frame_entry();
            assert_eq!((probe.size)(&trackers), 1, "an unpublished D20 gap preserves the old pin");
        }
    }
}

#[test]
fn an_unrelated_rejection_keeps_the_advance_and_the_open_transaction() {
    for probe in PROBES {
        let others: Vec<u16> = JOURNALLED_TRACKER_RETENTION
            .iter()
            .map(|(_, tag)| *tag)
            .filter(|tag| *tag != probe_tag(probe.field))
            .chain([UNRELATED_DOMAIN])
            .collect();
        let mut trackers = CompactPerfGateTrackers::default();
        (probe.advance)(&mut trackers, 1);
        seal(&mut trackers);
        (probe.advance)(&mut trackers, 2);
        reset_trackers_after_rejected_bracket(&mut trackers, &rejected(&others));
        assert_eq!((probe.size)(&trackers), 2, "{}", probe.field);
        assert!(
            (probe.has_entry)(&trackers),
            "`{}`: the transaction is settled by the seal, not by a sibling's tear",
            probe.field
        );
        seal(&mut trackers);
        assert!(!(probe.has_entry)(&trackers), "{}", probe.field);
        reset_trackers_after_rejected_bracket(&mut trackers, &rejected(&[probe_tag(probe.field)]));
        assert_eq!(
            (probe.size)(&trackers),
            if probe_tag(probe.field) == COMPACT_DOMAIN_DIVISIONS { 0 } else { 2 },
            "`{}`: a new D20 gap stages recovery while other domains retain the sealed advance",
            probe.field
        );
    }
}

#[test]
fn a_refused_frame_returns_the_restored_baseline_to_the_published_slot() {
    for probe in PROBES {
        let mut refused = CompactPerfGateTrackers::default();
        (probe.advance)(&mut refused, 1);
        seal(&mut refused);
        (probe.advance)(&mut refused, 2);
        // The build's seal ran before the sink refused the frame; the entry has
        // to outlive it.
        commit_delta_pin_transactions(&mut refused, &mut Vec::new());
        let mut published = CompactPerfGateTrackers::default();
        restore_country_trackers_after_refused_frame(&mut refused, &mut published);
        assert_eq!(
            (probe.size)(&published),
            1,
            "`{}`: the sink refused the frame, so the fold holds what the frame found",
            probe.field
        );
        assert!(!(probe.has_entry)(&published), "{}", probe.field);
    }
}

#[test]
fn published_domain_failure_requires_d20_full_recovery_and_restores_sparse_pins() {
    for probe in PROBES {
        let mut trackers = CompactPerfGateTrackers::default();
        (probe.advance)(&mut trackers, 1);
        seal(&mut trackers);
        (probe.advance)(&mut trackers, 2);
        commit_delta_pin_transactions(&mut trackers, &mut Vec::new());
        assert!(
            (probe.has_entry)(&trackers),
            "`{}`: the build's seal settles nothing; the sink has not spoken",
            probe.field
        );
        commit_country_staged_advances(&mut trackers, &BTreeSet::new());
        assert_eq!((probe.size)(&trackers), 2, "{}", probe.field);
        assert!(!(probe.has_entry)(&trackers), "{}", probe.field);

        (probe.advance)(&mut trackers, 3);
        commit_delta_pin_transactions(&mut trackers, &mut Vec::new());
        commit_country_staged_advances(&mut trackers, &rejected(&[probe_tag(probe.field)]));
        assert_eq!(
            (probe.size)(&trackers),
            if probe_tag(probe.field) == COMPACT_DOMAIN_DIVISIONS { 0 } else { 2 },
            "`{}`: a published D20 gap requires a full baseline; other sparse pins restore",
            probe.field
        );
        assert!(!(probe.has_entry)(&trackers), "{}", probe.field);
    }
}

#[test]
fn rejecting_everything_keeps_undo_for_d20_full_recovery() {
    let everything: BTreeSet<u16> = (0u16..=1000).collect();
    for probe in PROBES {
        let mut trackers = CompactPerfGateTrackers::default();
        (probe.advance)(&mut trackers, 1);
        seal(&mut trackers);
        (probe.advance)(&mut trackers, 2);
        reset_trackers_after_rejected_bracket(&mut trackers, &everything);
        assert_eq!(
            (probe.size)(&trackers),
            if probe_tag(probe.field) == COMPACT_DOMAIN_DIVISIONS { 0 } else { 1 },
            "{}", probe.field,
        );
        if probe_tag(probe.field) == COMPACT_DOMAIN_DIVISIONS {
            trackers.division_movement.restore_frame_entry();
            assert_eq!((probe.size)(&trackers), 1);
        }
    }
}

/// A frame is delivered to the fold or its record became a gap; the pattern is
/// fixed so the run is reproducible and about 40% of frames are rejected.
fn frame_rejected(hour: u32) -> bool {
    matches!(hour % 5, 1 | 3) || hour % 17 == 0
}

fn apply_division_record(
    mirror: &mut BTreeMap<u32, DivisionMovementSample>,
    rows: &[DivisionMovementSample],
    delta: bool,
) {
    if !delta {
        mirror.clear();
    }
    for row in rows {
        if row.deleted {
            assert!(delta, "a whole payload carries no tombstone");
            mirror.remove(&row.div_id);
        } else {
            mirror.insert(row.div_id, row.clone());
        }
    }
}

#[test]
fn a_division_stream_recovers_full_after_gaps_and_folds_each_observed_hour() {
    let mut trackers = CompactPerfGateTrackers::default();
    let mut mirror = BTreeMap::new();
    let mut delivered = 0u32;
    let mut post_rejection_baselines = 0u32;
    let mut previous_rejected = false;
    for hour in 0..240u32 {
        let truth: BTreeMap<u32, DivisionMovementSample> = (1..=60u32)
            .filter(|id| (id + hour / 5) % 9 != 0)
            .map(|id| (id, division_sample(id, 1_000 + (hour * 37 + id * 11) % 500)))
            .collect();
        let active: Vec<DivisionMovementSample> = truth.values().cloned().collect();
        let root_rows = active.len() as u32;
        let (rows, _changed, status) =
            compact_division_emitted_rows(active, root_rows, 0, &mut trackers.division_movement, &country_roster_readset::Budget::default()).unwrap();
        let delta = status & COMPACT_STATUS_DELTA_DATA != 0;
        if frame_rejected(hour) {
            let rejected_domains = rejected(&[COMPACT_DOMAIN_DIVISIONS]);
            reset_trackers_after_rejected_bracket(&mut trackers, &rejected_domains);
            commit_country_staged_advances(&mut trackers, &rejected_domains);
            assert!(trackers.division_movement.is_empty());
            previous_rejected = true;
            continue;
        }
        assert_eq!(delta, delivered > 0 && !previous_rejected, "hour {hour}: gaps require a full baseline");
        if previous_rejected {
            post_rejection_baselines += 1;
        }
        apply_division_record(&mut mirror, &rows, delta);
        commit_country_staged_advances(&mut trackers, &BTreeSet::new());
        assert_eq!(mirror, truth, "hour {hour}");
        delivered += 1;
        previous_rejected = false;
    }
    assert!(post_rejection_baselines > 40, "{post_rejection_baselines}");
}

fn detail_state_row(state_id: u32, amount: i32) -> CompactMapDetailStateRow {
    CompactMapDetailStateRow {
        state_id,
        resources: Some(vec![CompactStateResourceEntry {
            resource_id: 1,
            amount_raw: amount,
        }]),
        buildings: None,
    }
}

#[test]
fn a_map_detail_patch_stream_across_rejections_folds_to_the_rows_each_hour_observed() {
    let mut tracker = CompactMapDetailTracker::default();
    let mut mirror: BTreeMap<u32, CompactMapDetailStateRow> = BTreeMap::new();
    let mut delivered = 0u32;
    let mut post_rejection_patches = 0u32;
    let mut previous_rejected = false;
    let provinces: Vec<CompactMapDetailProvinceRow> = (1..=5u32)
        .map(|id| CompactMapDetailProvinceRow {
            province_id: id,
            state_id: 1,
            buildings: None,
        })
        .collect();
    for hour in 0..240u32 {
        let states: Vec<CompactMapDetailStateRow> = (1..=40u32)
            .map(|id| detail_state_row(id, ((hour / 3) * 7 + id * (hour % 4)) as i32))
            .collect();
        let delta = compact_map_detail_delta(&mut tracker, &states, &provinces, true, false);
        if frame_rejected(hour) {
            tracker.restore_frame_entries();
            previous_rejected = true;
            continue;
        }
        if delivered > 0 {
            assert!(
                !delta.emit_full_snapshot,
                "hour {hour}: every record after the first is a patch"
            );
        }
        if previous_rejected {
            post_rejection_patches += 1;
        }
        if delta.emit_full_snapshot {
            mirror.clear();
        }
        for row in &delta.state_rows {
            mirror.insert(row.state_id, row.clone());
        }
        tracker.close_frame_transactions();
        let truth: BTreeMap<u32, CompactMapDetailStateRow> =
            states.iter().map(|row| (row.state_id, row.clone())).collect();
        assert_eq!(mirror, truth, "hour {hour}");
        delivered += 1;
        previous_rejected = false;
    }
    assert!(post_rejection_patches > 40, "{post_rejection_patches}");
}

#[test]
fn a_withheld_verdict_restores_the_entry_and_keeps_the_discovery_cache() {
    // The verdict restores what the fold never received; the byte-witnessed
    // topology is not that, and dropping it there would cost every degraded
    // d44 frame a rediscovery. The in-build reset still drops it, as the
    // blanket reset did.
    let topology = || CompactMapControlTopology {
        province_array: 0,
        province_count: 0,
        states: Vec::new(),
        provinces: Vec::new(),
    };
    let probe = PROBES
        .iter()
        .find(|probe| probe.field == "supply_logistics_attrition_live")
        .unwrap();
    let tag = probe_tag(probe.field);
    let mut trackers = CompactPerfGateTrackers::default();
    trackers.supply_logistics_attrition_live.topology = Some(topology());
    (probe.advance)(&mut trackers, 1);
    seal(&mut trackers);
    (probe.advance)(&mut trackers, 2);
    commit_delta_pin_transactions(&mut trackers, &mut Vec::new());
    commit_country_staged_advances(&mut trackers, &rejected(&[tag]));
    assert_eq!((probe.size)(&trackers), 1);
    assert!(
        trackers.supply_logistics_attrition_live.topology.is_some(),
        "the verdict keeps the cache the next frame revalidates"
    );

    (probe.advance)(&mut trackers, 2);
    reset_trackers_after_rejected_bracket(&mut trackers, &rejected(&[tag]));
    assert_eq!((probe.size)(&trackers), 1);
    assert!(
        trackers.supply_logistics_attrition_live.topology.is_none(),
        "the in-build reset drops it"
    );
}
