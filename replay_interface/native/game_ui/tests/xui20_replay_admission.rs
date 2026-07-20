use game_ui::combat_detail_losses::{
    admit_xui20_replay, LandCombatPanel, Xui20CombatDetailLosses, Xui20SurfaceState,
};
use replay_core::{
    AirCombatLossRow, AirRegionCombatRow, AirRegionDamageRow, EndedCombatRow, LandCombatSideDetail,
    LandCombatTacticIdentity, Leaf, NavalCombatMemberRow, NavalCombatRow, WarObjectRow,
    WarParticipantOverviewRow, WarParticipationBreakdown,
};

fn side() -> LandCombatSideDetail {
    LandCombatSideDetail {
        units: Leaf::Measured(3),
        committed: Leaf::Measured(2),
        reserve: Leaf::Measured(1),
        air_kills: Leaf::Measured(4),
        org_loss_summary_index: Leaf::Measured(0),
        num_org_losses: Leaf::Measured(1),
        anti_air_attack: Leaf::Measured(0.25),
        air_damage_str: Leaf::Measured(0.1),
        air_damage_org: Leaf::Measured(0.2),
        ground_damage_str: Leaf::Measured(0.3),
        ground_damage_org: Leaf::Measured(0.4),
        prevented_damage_str: Leaf::Measured(0.05),
        prevented_damage_org: Leaf::Measured(0.06),
        progress: Leaf::Measured(0.53),
        progress_raw: Leaf::Measured(53_000),
        tactic: Leaf::Measured(Some(LandCombatTacticIdentity {
            id: Leaf::Measured(7),
            token: Leaf::Measured("tactic_basic_attack".into()),
            name: Leaf::Measured("TACTIC_BASIC_ATTACK".into()),
        })),
        tactic_status: Leaf::Measured("exact".into()),
        losses_status: Leaf::Measured("unavailable_no_authenticated_live_carrier".into()),
        width_status: Leaf::Measured("unavailable_pending_x2_control".into()),
        width_observations: Leaf::Absent,
        current_loss_observations: Leaf::Absent,
        counts_incomplete: Leaf::Measured(false),
        reserve_division_ids: Leaf::Measured(vec![30]),
        reserve_members_status: Leaf::Measured("exact".into()),
        damage_attribution: Leaf::Measured(Vec::new()),
    }
}

fn land() -> LandCombatPanel {
    LandCombatPanel {
        combat_id: Leaf::Measured(9001),
        location_province: Leaf::Measured(42),
        attackers: Leaf::Measured(vec![10, 11]),
        defenders: Leaf::Measured(vec![20]),
        combat_width: Leaf::Measured(80.0),
        terrain: Leaf::Measured("plains".into()),
        progress: Leaf::Measured(0.7),
        phase_event: Leaf::Measured("updated".into()),
        detail_generation: Leaf::Measured(2),
        detail_location_province: Leaf::Measured(42),
        age_days: Leaf::Measured(1),
        duration_days: Leaf::Measured(1),
        side_detail_completeness: Leaf::Measured("both".into()),
        participant_events: Leaf::Measured(Vec::new()),
        attacker: Leaf::Measured(side()),
        defender: Leaf::Measured(side()),
        attacker_leader: Leaf::Absent,
        defender_leader: Leaf::Absent,
        displayed_modifiers: Leaf::Absent,
    }
}

fn breakdown() -> WarParticipationBreakdown {
    WarParticipationBreakdown {
        land_damage: Leaf::Measured(10),
        occupation: Leaf::Measured(20),
        air_damage: Leaf::Measured(30),
        strategic_bombing: Leaf::Measured(40),
        sunk_ships: Leaf::Measured(50),
        sunk_convoys: Leaf::Measured(60),
        casualties: Leaf::Measured(70),
        lend_lease_given: Leaf::Measured(80),
        lend_lease_received: Leaf::Measured(90),
    }
}

fn participant(tag: &str, id: i64) -> WarParticipantOverviewRow {
    WarParticipantOverviewRow {
        tag: Leaf::Measured(tag.into()),
        tag_db_id: Leaf::Measured(id),
        participation_raw: Leaf::Measured(50_000),
        capitulation_progress_raw: Leaf::Measured(10_000),
        breakdown: Leaf::Measured(breakdown()),
    }
}

fn war() -> WarObjectRow {
    WarObjectRow {
        key: "axis_allies_1936".into(),
        attackers: Leaf::Measured(vec!["GER".into(), "ITA".into()]),
        defenders: Leaf::Measured(vec!["ENG".into()]),
        war_name: Leaf::Measured("GERMAN_BRITISH_WAR".into()),
        front: Leaf::Measured("western".into()),
        start_hour: Leaf::Measured(100),
        attacker_participants: Leaf::Measured(vec![participant("GER", 1), participant("ITA", 2)]),
        defender_participants: Leaf::Measured(vec![participant("ENG", 3)]),
        overall_progress_raw: Leaf::Measured(-12_500),
        first: Leaf::Absent,
        second: Leaf::Absent,
        first_tag_db_id: Leaf::Absent,
        second_tag_db_id: Leaf::Absent,
        start_date: Leaf::Absent,
        first_casualties: Leaf::Absent,
        second_casualties: Leaf::Absent,
        casualties_semantics: Leaf::Absent,
        casualties_is_save_snapshot: Leaf::Absent,
    }
}

fn naval_member(side: &str, owner: i64, ship: i64) -> NavalCombatMemberRow {
    NavalCombatMemberRow {
        side: Leaf::Measured(side.into()),
        group_index: Leaf::Measured(0),
        member_index: Leaf::Measured(0),
        is_convoy: Leaf::Measured(false),
        ship_instance_id: Leaf::Measured(ship),
        cship_join_validated: Leaf::Measured(true),
        owner_tag_db_id: Leaf::Measured(owner),
        task_force_id: Leaf::Measured(owner + 100),
        fleet_id: Leaf::Measured(owner + 200),
        battle_slot_index: Leaf::Measured(0),
        state_bitmask: Leaf::Measured(1),
        state_raw: Leaf::Measured(1),
        state_name: Leaf::Measured("FEX_STATE_WAITING".into()),
        strength: Leaf::Measured(0.8),
        organisation: Leaf::Measured(0.7),
        sunk_convoys: Leaf::Measured(0),
    }
}

fn naval() -> NavalCombatRow {
    NavalCombatRow {
        combat_id: Leaf::Measured(77),
        generation: Leaf::Measured(3),
        location_province: Leaf::Measured(900),
        region_id: Leaf::Measured(12),
        attacker: Leaf::Absent,
        defender: Leaf::Absent,
        result_type: Leaf::Absent,
        progress: Leaf::Absent,
        attacker_member_count: Leaf::Measured(1),
        defender_member_count: Leaf::Measured(1),
        attacker_positioning: Leaf::Measured(0.75),
        defender_positioning: Leaf::Measured(0.65),
        attacker_anti_air_raw: Leaf::Measured(123_000),
        attacker_anti_air: Leaf::Measured(1.23),
        defender_anti_air_raw: Leaf::Measured(98_000),
        defender_anti_air: Leaf::Measured(0.98),
        attacker_task_force_ids: Leaf::Measured(vec![101]),
        defender_task_force_ids: Leaf::Measured(vec![102]),
        attacker_country_tag_db_ids: Leaf::Measured(vec![1]),
        defender_country_tag_db_ids: Leaf::Measured(vec![2]),
        members: Leaf::Measured(vec![
            naval_member("attacker", 1, 1001),
            naval_member("defender", 2, 1002),
        ]),
    }
}

fn air_loss() -> AirCombatLossRow {
    AirCombatLossRow {
        owner: Leaf::Measured("GER".into()),
        owner_tag_db_id: Leaf::Measured(1),
        wing_id: Leaf::Measured(11),
        generation: Leaf::Measured(2),
        mission_mask: Leaf::Measured(4),
        mission_region: Leaf::Measured(12),
        old_count: Leaf::Measured(10),
        new_count: Leaf::Measured(7),
        count_delta: Leaf::Measured(-3),
        old_equipment_amount_sum: Leaf::Measured(10.0),
        new_equipment_amount_sum: Leaf::Measured(7.5),
        equipment_amount_delta: Leaf::Measured(-2.5),
        flags: Leaf::Measured(0),
        air_accidents: Leaf::Measured(1),
        experience: Leaf::Measured(0.4),
    }
}

fn air_region() -> AirRegionCombatRow {
    AirRegionCombatRow {
        owner: Leaf::Measured("GER".into()),
        owner_tag_db_id: Leaf::Measured(1),
        region_id: Leaf::Measured(12),
        side_vector: Leaf::Measured(0),
        selection_flag: Leaf::Measured(1),
        classification_flag: Leaf::Measured(2),
        copied_flag: Leaf::Measured(0),
        wing_id: Leaf::Measured(11),
        date_raw: Leaf::Measured(1234),
        plane_count: Leaf::Measured(7),
        mission_mask: Leaf::Measured(4),
        attributed_country: Leaf::Measured("ENG".into()),
        attributed_country_tag_db_id: Leaf::Measured(2),
        damage: Leaf::Measured(vec![AirRegionDamageRow {
            channel: Leaf::Measured("air_damage".into()),
            cause_type: Leaf::Measured(3),
            cause: Leaf::Measured("fighter".into()),
            amount: Leaf::Measured(2.5),
        }]),
    }
}

fn exact_binding() -> Xui20CombatDetailLosses {
    Xui20CombatDetailLosses {
        land: vec![land()],
        ended_land: std::collections::BTreeMap::new(),
        ended_land_status: replay_core::ViewStatus::Present,
        naval: vec![naval()],
        air_losses: Leaf::Measured(vec![air_loss()]),
        air_region_combat: Leaf::Measured(vec![air_region()]),
        sunk_convoys: Leaf::Measured(Vec::new()),
        wars: Leaf::Measured(vec![war()]),
        war_relations: Leaf::Measured(Vec::new()),
    }
}

#[test]
fn unavailable_ended_collection_is_not_collapsed_to_measured_empty() {
    let mut binding = exact_binding();
    binding.ended_land_status = replay_core::ViewStatus::DomainUnavailable;
    let admission = admit_xui20_replay(&binding);
    assert!(matches!(
        admission.land_ended_lifecycle,
        Xui20SurfaceState::ExternalInputRequired { .. }
    ));
}

#[test]
fn exact_replay_owned_surfaces_admit_and_external_joins_stay_explicit() {
    let admission = admit_xui20_replay(&exact_binding());
    assert_eq!(
        admission.land_active,
        Xui20SurfaceState::Admitted { row_count: 1 }
    );
    assert_eq!(
        admission.land_side_detail,
        Xui20SurfaceState::Admitted { row_count: 1 }
    );
    assert_eq!(
        admission.war_overview_exact,
        Xui20SurfaceState::Admitted { row_count: 1 }
    );
    assert_eq!(
        admission.naval_active,
        Xui20SurfaceState::Admitted { row_count: 1 }
    );
    assert_eq!(
        admission.air_history,
        Xui20SurfaceState::Admitted { row_count: 1 }
    );
    assert_eq!(
        admission.air_region_history,
        Xui20SurfaceState::Admitted { row_count: 1 }
    );
    assert_eq!(
        admission.combat_log_families,
        Xui20SurfaceState::Admitted { row_count: 5 }
    );
    assert!(matches!(
        admission.land_terminal,
        Xui20SurfaceState::ExternalInputRequired { .. }
    ));
    assert!(matches!(
        admission.naval_terminal,
        Xui20SurfaceState::ExternalInputRequired { .. }
    ));
    assert!(matches!(
        admission.global_cross_family_order,
        Xui20SurfaceState::NotApplicable { .. }
    ));
}

#[test]
fn ended_lifecycle_is_admitted_separately_from_active_land() {
    let mut binding = exact_binding();
    binding.ended_land.insert(
        (22, 4),
        EndedCombatRow {
            combat_id: 22,
            generation: 4,
            kind: Leaf::Measured("land".into()),
            lifecycle: Leaf::Measured("ended".into()),
            location_province: Leaf::Measured(13),
            attackers: Leaf::Unobtained,
            defenders: Leaf::Unobtained,
            combat_width: Leaf::Unobtained,
            terrain: Leaf::Unobtained,
            progress: Leaf::Unobtained,
            side_detail_availability: Leaf::Measured("unavailable".into()),
            detail: Leaf::Unobtained,
        },
    );
    let admission = admit_xui20_replay(&binding);
    assert_eq!(
        admission.land_ended_lifecycle,
        Xui20SurfaceState::Admitted { row_count: 1 }
    );
    assert_eq!(
        admission.land_active,
        Xui20SurfaceState::Admitted { row_count: 1 }
    );
    assert!(matches!(
        admission.land_terminal,
        Xui20SurfaceState::ExternalInputRequired { .. }
    ));
}

#[test]
fn malformed_exact_values_reject_instead_of_rendering() {
    let mut binding = exact_binding();
    let Leaf::Measured(attacker) = &mut binding.land[0].attacker else {
        panic!("attacker")
    };
    attacker.progress = Leaf::Measured(0.1);
    assert!(matches!(
        admit_xui20_replay(&binding).land_side_detail,
        Xui20SurfaceState::Rejected { .. }
    ));
}

#[test]
fn missing_scheduled_w1_payload_is_external_not_zero_filled() {
    let mut binding = exact_binding();
    let Leaf::Measured(wars) = &mut binding.wars else {
        panic!("wars")
    };
    wars[0].war_name = Leaf::Unobtained;
    assert!(matches!(
        admit_xui20_replay(&binding).war_overview_exact,
        Xui20SurfaceState::ExternalInputRequired { .. }
    ));
}

#[test]
fn missing_land_display_or_lifecycle_inputs_fail_closed() {
    let mut binding = exact_binding();
    binding.land[0].terrain = Leaf::Unobtained;
    assert!(matches!(
        admit_xui20_replay(&binding).land_active,
        Xui20SurfaceState::ExternalInputRequired { .. }
    ));

    let mut binding = exact_binding();
    binding.land[0].detail_generation = Leaf::Absent;
    assert!(matches!(
        admit_xui20_replay(&binding).land_side_detail,
        Xui20SurfaceState::ExternalInputRequired { .. }
    ));
}

#[test]
fn malformed_land_display_and_lifecycle_values_reject() {
    let mut binding = exact_binding();
    binding.land[0].combat_width = Leaf::Measured(0.0);
    assert!(matches!(
        admit_xui20_replay(&binding).land_active,
        Xui20SurfaceState::Rejected { .. }
    ));

    let mut binding = exact_binding();
    binding.land[0].detail_location_province = Leaf::Measured(43);
    assert!(matches!(
        admit_xui20_replay(&binding).land_side_detail,
        Xui20SurfaceState::Rejected { .. }
    ));
}

#[test]
fn naval_member_order_counts_and_country_sets_are_exact() {
    let mut binding = exact_binding();
    let Leaf::Measured(members) = &mut binding.naval[0].members else {
        panic!("members")
    };
    members.swap(0, 1);
    assert!(matches!(
        admit_xui20_replay(&binding).naval_active,
        Xui20SurfaceState::Rejected { .. }
    ));

    let mut binding = exact_binding();
    binding.naval[0].attacker_country_tag_db_ids = Leaf::Measured(vec![99]);
    assert!(matches!(
        admit_xui20_replay(&binding).naval_active,
        Xui20SurfaceState::Rejected { .. }
    ));

    let mut binding = exact_binding();
    binding.naval[0].attacker_member_count = Leaf::Unobtained;
    assert!(matches!(
        admit_xui20_replay(&binding).naval_active,
        Xui20SurfaceState::ExternalInputRequired { .. }
    ));
}

#[test]
fn air_loss_arithmetic_and_region_damage_are_fail_closed() {
    let mut binding = exact_binding();
    let Leaf::Measured(losses) = &mut binding.air_losses else {
        panic!("air losses")
    };
    losses[0].count_delta = Leaf::Measured(-2);
    assert!(matches!(
        admit_xui20_replay(&binding).air_history,
        Xui20SurfaceState::Rejected { .. }
    ));
    assert!(matches!(
        admit_xui20_replay(&binding).combat_log_families,
        Xui20SurfaceState::Rejected { .. }
    ));

    let mut binding = exact_binding();
    let Leaf::Measured(regions) = &mut binding.air_region_combat else {
        panic!("air regions")
    };
    let Leaf::Measured(damage) = &mut regions[0].damage else {
        panic!("damage")
    };
    damage[0].amount = Leaf::Measured(f64::NAN);
    assert!(matches!(
        admit_xui20_replay(&binding).air_region_history,
        Xui20SurfaceState::Rejected { .. }
    ));

    let mut binding = exact_binding();
    let Leaf::Measured(regions) = &mut binding.air_region_combat else {
        panic!("air regions")
    };
    regions[0].attributed_country_tag_db_id = Leaf::Unobtained;
    assert!(matches!(
        admit_xui20_replay(&binding).air_region_history,
        Xui20SurfaceState::ExternalInputRequired { .. }
    ));
}

#[test]
fn w1_requires_complete_unique_ordered_sides_and_metrics() {
    let mut binding = exact_binding();
    let Leaf::Measured(wars) = &mut binding.wars else {
        panic!("wars")
    };
    wars[0].front = Leaf::Absent;
    assert!(matches!(
        admit_xui20_replay(&binding).war_overview_exact,
        Xui20SurfaceState::ExternalInputRequired { .. }
    ));

    let mut binding = exact_binding();
    let Leaf::Measured(wars) = &mut binding.wars else {
        panic!("wars")
    };
    let Leaf::Measured(defenders) = &mut wars[0].defenders else {
        panic!("defenders")
    };
    defenders[0] = "GER".into();
    let Leaf::Measured(participants) = &mut wars[0].defender_participants else {
        panic!("defender participants")
    };
    participants[0].tag = Leaf::Measured("GER".into());
    assert!(matches!(
        admit_xui20_replay(&binding).war_overview_exact,
        Xui20SurfaceState::Rejected { .. }
    ));

    let mut binding = exact_binding();
    let Leaf::Measured(wars) = &mut binding.wars else {
        panic!("wars")
    };
    let Leaf::Measured(participants) = &mut wars[0].attacker_participants else {
        panic!("attacker participants")
    };
    participants[0].tag_db_id = Leaf::Unobtained;
    assert!(matches!(
        admit_xui20_replay(&binding).war_overview_exact,
        Xui20SurfaceState::ExternalInputRequired { .. }
    ));
}
