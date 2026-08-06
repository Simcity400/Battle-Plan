use game_ui::combat_detail_losses::{
    admit_xui20_replay, LandCombatPanel, Xui20CombatDetailLosses, Xui20FrameHour,
    Xui20SurfaceState,
};
use replay_core::{
    AirCombatLossRow, AirRegionCombatRow, AirRegionDamageRow, EndedCombatRow, LandCombatSideDetail,
    LandCombatTacticIdentity, Leaf, NavalCombatMemberRow, NavalCombatRow, SunkShipEventExact,
    SunkShipTextUnavailable, WarObjectRow,
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
        losses_status: Leaf::Measured("exact_no_writer_call_this_hour".into()),
        width_status: Leaf::Measured("exact_no_synchronous_call_this_hour".into()),
        width_observations: Leaf::Absent,
        current_loss_observations: Leaf::Absent,
        leader_id: Leaf::Absent,
        leader_status: Leaf::Measured("unavailable_panel_not_observed".into()),
        effective_displayed_modifiers: Leaf::Absent,
        effective_displayed_modifiers_status: Leaf::Measured(
            "unavailable_panel_not_observed".into(),
        ),
        panel_capture_sequence: Leaf::Absent,
        panel_observation_source: Leaf::Absent,
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
        attacker_displayed_modifiers: Leaf::Absent,
        defender_displayed_modifiers: Leaf::Absent,
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
        war_id: Leaf::Measured(1),
        producer_ordinal: Leaf::Measured(0),
        aggregate_summary_flag: Leaf::Measured(false),
        attackers: Leaf::Measured(vec!["GER".into(), "ITA".into()]),
        defenders: Leaf::Measured(vec!["ENG".into()]),
        war_name: Leaf::Measured("GERMAN_BRITISH_WAR".into()),
        front: Leaf::Absent,
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
        threat_fixed5_raw: Leaf::Absent,
        threat_fixed_scale: Leaf::Absent,
        first_was_instigator: Leaf::Absent,
        hostility_reason_instigator_raw: Leaf::Absent,
        hostility_reason_instigator: Leaf::Absent,
        hostility_reason_defender_raw: Leaf::Absent,
        hostility_reason_defender: Leaf::Absent,
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
        attacker_positioning_raw: Leaf::Measured(75_000),
        attacker_positioning: Leaf::Measured(0.75),
        defender_positioning_raw: Leaf::Measured(65_000),
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

/// A real engine hour counter inside the shared 1850..=2200 window; the same
/// value `replay_core`'s own D86 fixtures carry. It used to be `44`, which is
/// ~43.8 million below the window's floor -- a value the gate accepted only
/// because it never applied the calendar half.
const FIXTURE_SINK_DATE_RAW: i32 = 60_759_299;

/// The schema `hour_index` `FIXTURE_SINK_DATE_RAW` denotes.
/// `fixture_frame_hour_is_exactly_the_fixture_sinking_hour` pins the two
/// together through the shared predicate, so neither can be edited alone.
const FIXTURE_FRAME_HOUR: u64 = 16_959_300;

/// The hour every gate call in this file admits against. The gate has no
/// default to fall back on, so each call states it.
fn frame_hour() -> Xui20FrameHour {
    Xui20FrameHour::Measured(FIXTURE_FRAME_HOUR)
}

fn sunk_ship_row() -> SunkShipEventExact {
    SunkShipEventExact {
        event_sequence: 2,
        combat_join_present: true,
        combat_id: 9,
        combat_generation: 2,
        victim_country_id_raw: 2,
        attacker_country_id_raw: 101,
        ui_source_country_id_raw: 1,
        sink_date_raw: FIXTURE_SINK_DATE_RAW,
        victim_member_battle_slot_index: 1,
        killed: true,
        assisted_kill: false,
        victim_ship_reference_raw: [7, 8],
        damage_source_reference_raw: [9, 10],
        task_force_id_raw: 11,
        terminal_context_raw: [12, 13],
        cause_record_flag: false,
        ship_name: Some("HMS Exact".into()),
        ship_name_unavailable: None,
        ship_class_name: Some("Destroyer".into()),
        ship_class_name_unavailable: None,
        attribution_source_name: Some("KMS Exact".into()),
        attribution_source_name_unavailable: None,
        attribution_source_class_name: Some("Cruiser".into()),
        attribution_source_class_name_unavailable: None,
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
        convoy_terminal: Leaf::Unobtained,
        sunk_ships: Leaf::Measured(vec![sunk_ship_row()]),
        sunk_ships_completeness: replay_core::SunkShipHistoryCompleteness::Complete,
        wars: Leaf::Measured(vec![war()]),
        war_relations: Leaf::Measured(Vec::new()),
    }
}

/// The two fixture constants are one fact spelled two ways. Pinning them
/// against the shared conversion (rather than against each other) means an
/// edit to either one, or a change to the shared epoch, turns this red instead
/// of quietly making every calendar assertion below vacuous.
#[test]
fn fixture_frame_hour_is_exactly_the_fixture_sinking_hour() {
    let row = sunk_ship_row();
    assert!(
        replay_core::sunk_ship_event_sink_hour_is_admissible(&row, FIXTURE_FRAME_HOUR),
        "the fixture sinking must be admissible at the fixture frame hour"
    );
    assert!(
        !replay_core::sunk_ship_event_sink_hour_is_admissible(&row, FIXTURE_FRAME_HOUR - 1),
        "one hour earlier the same sinking is still in the future"
    );
}

#[test]
fn unavailable_ended_collection_is_not_collapsed_to_measured_empty() {
    let mut binding = exact_binding();
    binding.ended_land_status = replay_core::ViewStatus::DomainUnavailable;
    let admission = admit_xui20_replay(&binding, &frame_hour());
    assert!(matches!(
        admission.land_ended_lifecycle,
        Xui20SurfaceState::ExternalInputRequired { .. }
    ));
}

#[test]
fn exact_replay_owned_surfaces_admit_and_external_joins_stay_explicit() {
    let admission = admit_xui20_replay(&exact_binding(), &frame_hour());
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
    assert!(matches!(
        admission.combat_log_families,
        Xui20SurfaceState::ExternalInputRequired { .. }
    ));
    assert!(matches!(
        admission.land_terminal,
        Xui20SurfaceState::ExternalInputRequired { .. }
    ));
    assert_eq!(
        admission.naval_terminal,
        Xui20SurfaceState::Admitted { row_count: 1 }
    );
    assert!(matches!(
        admission.convoy_terminal,
        Xui20SurfaceState::ExternalInputRequired { .. }
    ));
    assert!(matches!(
        admission.global_cross_family_order,
        Xui20SurfaceState::NotApplicable { .. }
    ));
    assert!(matches!(
        admission.war_pair_history,
        Xui20SurfaceState::ExternalInputRequired { .. }
    ));
}

#[test]
fn attach_baseline_sunk_rows_admit_only_with_explicitly_absent_combat_join() {
    let mut binding = exact_binding();
    let Leaf::Measured(rows) = &mut binding.sunk_ships else {
        panic!("sunk ships")
    };
    rows[0].combat_join_present = false;
    rows[0].combat_id = 0;
    rows[0].combat_generation = 0;
    rows[0].victim_member_battle_slot_index = 0;
    assert_eq!(
        admit_xui20_replay(&binding, &frame_hour()).naval_terminal,
        Xui20SurfaceState::Admitted { row_count: 1 }
    );

    let Leaf::Measured(rows) = &mut binding.sunk_ships else {
        panic!("sunk ships")
    };
    rows[0].combat_id = 9;
    assert!(matches!(
        admit_xui20_replay(&binding, &frame_hour()).naval_terminal,
        Xui20SurfaceState::Rejected { .. }
    ));
}

// A gap in the sequence is what a dropped sinking leaves behind, and the
// surface must not admit a history that still claims to be complete.
#[test]
fn sunk_rows_reject_a_gap_in_the_history_sequence() {
    let mut binding = exact_binding();
    let Leaf::Measured(rows) = &mut binding.sunk_ships else {
        panic!("sunk ships")
    };
    let mut second = rows[0].clone();
    second.event_sequence = 3;
    rows.push(second);
    assert_eq!(
        admit_xui20_replay(&binding, &frame_hour()).naval_terminal,
        Xui20SurfaceState::Admitted { row_count: 2 },
        "a contiguous two-row history is the control"
    );

    let Leaf::Measured(rows) = &mut binding.sunk_ships else {
        panic!("sunk ships")
    };
    rows[1].event_sequence = 4;
    assert!(
        matches!(
            admit_xui20_replay(&binding, &frame_hour()).naval_terminal,
            Xui20SurfaceState::Rejected { .. }
        ),
        "a history that skips sequence 3 is still strictly increasing"
    );
}

// D86 admits a sinking whose exact text will not decode rather than refusing
// the record, so the replay surface must admit that row too -- and must still
// reject a row that claims a missing name with no reason for it, or a present
// name that is empty.
#[test]
fn sunk_rows_admit_an_explicitly_unavailable_name_but_not_an_unexplained_one() {
    let mut binding = exact_binding();
    let Leaf::Measured(rows) = &mut binding.sunk_ships else {
        panic!("sunk ships")
    };
    rows[0].ship_name = None;
    rows[0].ship_name_unavailable = Some(SunkShipTextUnavailable::EngineEmpty);
    assert_eq!(
        admit_xui20_replay(&binding, &frame_hour()).naval_terminal,
        Xui20SurfaceState::Admitted { row_count: 1 }
    );

    let Leaf::Measured(rows) = &mut binding.sunk_ships else {
        panic!("sunk ships")
    };
    rows[0].ship_name_unavailable = None;
    assert!(
        matches!(
            admit_xui20_replay(&binding, &frame_hour()).naval_terminal,
            Xui20SurfaceState::Rejected { .. }
        ),
        "an absent name with no reason beside it is not an admitted absence"
    );

    let Leaf::Measured(rows) = &mut binding.sunk_ships else {
        panic!("sunk ships")
    };
    rows[0].ship_name = Some(String::new());
    assert!(
        matches!(
            admit_xui20_replay(&binding, &frame_hour()).naval_terminal,
            Xui20SurfaceState::Rejected { .. }
        ),
        "emptiness is carried as a reason, never as an empty present name"
    );

    let Leaf::Measured(rows) = &mut binding.sunk_ships else {
        panic!("sunk ships")
    };
    rows[0].ship_name = Some("HMS Exact".into());
    rows[0].ship_name_unavailable = Some(SunkShipTextUnavailable::PayloadUnreadable);
    assert!(
        matches!(
            admit_xui20_replay(&binding, &frame_hour()).naval_terminal,
            Xui20SurfaceState::Rejected { .. }
        ),
        "a row cannot carry both a name and a reason it is missing"
    );
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
    let admission = admit_xui20_replay(&binding, &frame_hour());
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
        admit_xui20_replay(&binding, &frame_hour()).land_side_detail,
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
        admit_xui20_replay(&binding, &frame_hour()).war_overview_exact,
        Xui20SurfaceState::ExternalInputRequired { .. }
    ));
}

#[test]
fn missing_land_display_or_lifecycle_inputs_fail_closed() {
    let mut binding = exact_binding();
    binding.land[0].terrain = Leaf::Unobtained;
    assert!(matches!(
        admit_xui20_replay(&binding, &frame_hour()).land_active,
        Xui20SurfaceState::ExternalInputRequired { .. }
    ));

    let mut binding = exact_binding();
    binding.land[0].detail_generation = Leaf::Absent;
    assert!(matches!(
        admit_xui20_replay(&binding, &frame_hour()).land_side_detail,
        Xui20SurfaceState::ExternalInputRequired { .. }
    ));
}

#[test]
fn malformed_land_display_and_lifecycle_values_reject() {
    let mut binding = exact_binding();
    binding.land[0].combat_width = Leaf::Measured(0.0);
    assert!(matches!(
        admit_xui20_replay(&binding, &frame_hour()).land_active,
        Xui20SurfaceState::Rejected { .. }
    ));

    let mut binding = exact_binding();
    binding.land[0].detail_location_province = Leaf::Measured(43);
    assert!(matches!(
        admit_xui20_replay(&binding, &frame_hour()).land_side_detail,
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
        admit_xui20_replay(&binding, &frame_hour()).naval_active,
        Xui20SurfaceState::Rejected { .. }
    ));

    let mut binding = exact_binding();
    binding.naval[0].attacker_country_tag_db_ids = Leaf::Measured(vec![99]);
    assert!(matches!(
        admit_xui20_replay(&binding, &frame_hour()).naval_active,
        Xui20SurfaceState::Rejected { .. }
    ));

    let mut binding = exact_binding();
    binding.naval[0].attacker_member_count = Leaf::Unobtained;
    assert!(matches!(
        admit_xui20_replay(&binding, &frame_hour()).naval_active,
        Xui20SurfaceState::ExternalInputRequired { .. }
    ));
}

#[test]
fn naval_side_fixed_point_pairs_are_complete_and_exact() {
    let mut binding = exact_binding();
    binding.naval[0].attacker_anti_air_raw = Leaf::Unobtained;
    assert!(matches!(
        admit_xui20_replay(&binding, &frame_hour()).naval_active,
        Xui20SurfaceState::ExternalInputRequired { .. }
    ));

    let mut binding = exact_binding();
    binding.naval[0].attacker_positioning = Leaf::Measured(0.750_01);
    assert!(matches!(
        admit_xui20_replay(&binding, &frame_hour()).naval_active,
        Xui20SurfaceState::Rejected { .. }
    ));

    let mut binding = exact_binding();
    binding.naval[0].defender_anti_air = Leaf::Measured(0.99);
    assert!(matches!(
        admit_xui20_replay(&binding, &frame_hour()).naval_active,
        Xui20SurfaceState::Rejected { .. }
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
        admit_xui20_replay(&binding, &frame_hour()).air_history,
        Xui20SurfaceState::Rejected { .. }
    ));
    assert!(matches!(
        admit_xui20_replay(&binding, &frame_hour()).combat_log_families,
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
        admit_xui20_replay(&binding, &frame_hour()).air_region_history,
        Xui20SurfaceState::Rejected { .. }
    ));

    let mut binding = exact_binding();
    let Leaf::Measured(regions) = &mut binding.air_region_combat else {
        panic!("air regions")
    };
    regions[0].attributed_country_tag_db_id = Leaf::Unobtained;
    assert!(matches!(
        admit_xui20_replay(&binding, &frame_hour()).air_region_history,
        Xui20SurfaceState::ExternalInputRequired { .. }
    ));
}

#[test]
fn w1_requires_complete_unique_ordered_sides_and_metrics() {
    let mut binding = exact_binding();
    let Leaf::Measured(wars) = &mut binding.wars else {
        panic!("wars")
    };
    wars[0].overall_progress_raw = Leaf::Unobtained;
    assert!(matches!(
        admit_xui20_replay(&binding, &frame_hour()).war_overview_exact,
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
        admit_xui20_replay(&binding, &frame_hour()).war_overview_exact,
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
        admit_xui20_replay(&binding, &frame_hour()).war_overview_exact,
        Xui20SurfaceState::ExternalInputRequired { .. }
    ));
}

// The XUI20 gate's D86 row rule is `replay_core`'s ONE copy of it, not a
// transcription. This gate used to spell the whole rule out again -- the
// value/reason pairing, the combat-join shape, the contiguous-from-2 sequence
// -- so a fix landing in `replay_core` alone would have been undone here, which
// is exactly the shape of the two earlier silent data losses.
//
// Reintroducing a local copy that differs from the shared predicate in ANY of
// the mutations below turns this red.
#[test]
fn sunk_ship_gate_verdict_tracks_replay_cores_shared_row_predicate() {
    let mutations: Vec<(&str, fn(&mut SunkShipEventExact))> = vec![
        ("control", |_row| {}),
        ("baseline sequence", |row| row.event_sequence = 1),
        ("sequence off by one", |row| row.event_sequence = 3),
        ("joined combat with no id", |row| row.combat_id = 0),
        ("joined combat with no generation", |row| {
            row.combat_generation = 0
        }),
        ("battle slot out of range", |row| {
            row.victim_member_battle_slot_index = 4_097
        }),
        ("battle slot at the ceiling", |row| {
            row.victim_member_battle_slot_index = 4_096
        }),
        ("no victim", |row| row.victim_country_id_raw = 0),
        ("no attacker", |row| row.attacker_country_id_raw = 0),
        ("non-positive sink date", |row| row.sink_date_raw = 0),
        ("killed and assisted at once", |row| row.assisted_kill = true),
        ("cause record on an assist", |row| {
            row.assisted_kill = true;
            row.killed = false;
            row.cause_record_flag = true;
        }),
        ("no victim ship reference", |row| {
            row.victim_ship_reference_raw = [0, 0]
        }),
        ("required name present but empty", |row| {
            row.ship_name = Some(String::new())
        }),
        ("required name missing with no reason", |row| {
            row.ship_name = None
        }),
        ("required name missing WITH a reason", |row| {
            row.ship_name = None;
            row.ship_name_unavailable = Some(SunkShipTextUnavailable::EngineEmpty);
        }),
        ("optional name missing with no reason", |row| {
            row.attribution_source_name = None
        }),
        ("name carries an interior NUL", |row| {
            row.ship_name = Some("HMS\0Exact".into())
        }),
        ("name at the length ceiling", |row| {
            row.ship_name = Some("x".repeat(4_096))
        }),
        ("name past the length ceiling", |row| {
            row.ship_name = Some("x".repeat(4_097))
        }),
    ];

    for (label, mutate) in mutations {
        let mut binding = exact_binding();
        let Leaf::Measured(rows) = &mut binding.sunk_ships else {
            panic!("sunk ships")
        };
        mutate(&mut rows[0]);
        let row = rows[0].clone();
        let shared = replay_core::sunk_ship_history_rows_are_admissible(
            std::slice::from_ref(&row),
            binding.sunk_ships_completeness,
        )
        .is_ok();
        let gate_admitted = matches!(
            admit_xui20_replay(&binding, &frame_hour()).naval_terminal,
            Xui20SurfaceState::Admitted { .. }
        );
        assert_eq!(
            shared, gate_admitted,
            "{label}: the XUI20 gate and replay_core's shared D86 row predicate \
             disagree; a rule enforced on one side and not its sibling is not \
             enforced"
        );
    }
}

// ---------------------------------------------------------------------------
// D86's calendar half, applied at the binding gate
// ---------------------------------------------------------------------------
//
// `replay_core` refuses a sunk-ship row whose `sink_date_raw` does not convert
// through the shared 1850..=2200 window, or whose converted hour is later than
// the frame's. The binding gate is a second production surface with its own
// refusal contract, and it used to apply only the hour-INDEPENDENT half -- so a
// row this crate had refused could still be admitted here, and the two layers
// could return opposite verdicts for the same row on the same frame.

/// A sinking dated outside the shared calendar window is refused even though
/// every hour-independent field is intact.
///
/// Mutation: drop the threaded hour check from `validate_sunk_ships` and the
/// out-of-window rows below are admitted, turning this red.
#[test]
fn sunk_rows_outside_the_shared_calendar_window_are_refused() {
    // The endpoints come from `portable_admission` through `replay_core`, not
    // from arithmetic retyped here: a hand-written window in a test that checks
    // a window is the same drift the shared crate exists to prevent.
    for (label, raw) in [
        (
            "one hour below the window floor",
            replay_core::RAW_DATE_MIN - 1,
        ),
        (
            "one hour above the window ceiling",
            replay_core::RAW_DATE_MAX + 1,
        ),
    ] {
        let mut binding = exact_binding();
        let Leaf::Measured(rows) = &mut binding.sunk_ships else {
            panic!("sunk ships")
        };
        rows[0].sink_date_raw = raw;
        assert!(
            matches!(
                admit_xui20_replay(&binding, &frame_hour()).naval_terminal,
                Xui20SurfaceState::Rejected { .. }
            ),
            "{label}: a raw sink date the shared window cannot convert is not an \
             admissible sinking"
        );
    }
}

/// A sinking dated after the frame it is presented on is refused. This is the
/// half that needs the hour: the row is perfectly well formed and inside the
/// calendar window, and only the frame it arrives on makes it inadmissible.
///
/// Mutation: drop the threaded hour check and this is admitted.
#[test]
fn a_sinking_later_than_the_frame_hour_is_refused() {
    let binding = exact_binding();
    assert_eq!(
        admit_xui20_replay(&binding, &frame_hour()).naval_terminal,
        Xui20SurfaceState::Admitted { row_count: 1 },
        "the control: the fixture sinking happened at the fixture frame hour"
    );
    assert!(
        matches!(
            admit_xui20_replay(
                &binding,
                &Xui20FrameHour::Measured(FIXTURE_FRAME_HOUR - 1)
            )
            .naval_terminal,
            Xui20SurfaceState::Rejected { .. }
        ),
        "one hour earlier the same row is a future sinking"
    );
}

/// The gate and `replay_core` agree on the calendar half for every sink date,
/// not just the ones a hand-written case list happens to name. The predicate is
/// the same function on both sides; this pins that it stays the same function.
#[test]
fn sunk_ship_gate_verdict_tracks_replay_cores_shared_calendar_predicate() {
    let cases: [(&str, i32); 7] = [
        ("in window, at the frame hour", FIXTURE_SINK_DATE_RAW),
        ("in window, an hour before the frame", FIXTURE_SINK_DATE_RAW - 1),
        ("in window, an hour after the frame", FIXTURE_SINK_DATE_RAW + 1),
        ("below the window floor", replay_core::RAW_DATE_MIN - 1),
        ("at the window floor", replay_core::RAW_DATE_MIN),
        ("at the window ceiling", replay_core::RAW_DATE_MAX),
        ("above the window ceiling", replay_core::RAW_DATE_MAX + 1),
    ];

    for (label, raw) in cases {
        let mut binding = exact_binding();
        let Leaf::Measured(rows) = &mut binding.sunk_ships else {
            panic!("sunk ships")
        };
        rows[0].sink_date_raw = raw;
        let row = rows[0].clone();
        let shared = replay_core::sunk_ship_history_rows_are_admissible(
            std::slice::from_ref(&row),
            binding.sunk_ships_completeness,
        )
        .is_ok()
            && replay_core::sunk_ship_event_sink_hour_is_admissible(&row, FIXTURE_FRAME_HOUR);
        let gate_admitted = matches!(
            admit_xui20_replay(&binding, &frame_hour()).naval_terminal,
            Xui20SurfaceState::Admitted { .. }
        );
        assert_eq!(
            shared, gate_admitted,
            "{label}: the XUI20 gate and replay_core disagree about the same D86 \
             row on the same frame"
        );
    }
}

/// A caller with no usable frame hour gets a typed refusal, never an
/// admission. `ExternalInputRequired` is the gate's existing "this surface
/// needs an input it was not given" state, and it is what a negative
/// `hour_index` produces -- the same condition `replay_core` refuses D86 on.
///
/// Mutation: make the no-hour arm admit (or fall through to the content half
/// alone) and this turns red.
#[test]
fn a_binding_with_no_frame_hour_refuses_the_d86_surface_typed() {
    let binding = exact_binding();
    let no_hour = Xui20FrameHour::Unavailable {
        reason: "replay frame hour_index -1 is negative".into(),
    };
    let admission = admit_xui20_replay(&binding, &no_hour);
    let Xui20SurfaceState::ExternalInputRequired { reason } = &admission.naval_terminal else {
        panic!(
            "a D86 surface with no frame hour must refuse typed, got {:?}",
            admission.naval_terminal
        );
    };
    assert!(
        reason.contains("frame hour"),
        "the refusal must name the missing input, got {reason}"
    );
    // The refusal is scoped: families that do not depend on the calendar are
    // untouched, so "no hour" never silently blanks the whole panel.
    assert_eq!(
        admission.land_active,
        Xui20SurfaceState::Admitted { row_count: 1 }
    );
    assert!(
        matches!(
            admission.combat_log_families,
            Xui20SurfaceState::ExternalInputRequired { .. }
        ),
        "the aggregate family state carries the refusal rather than hiding it"
    );
}

/// A MEASURED sunk-ship leaf under `NoHistory` is a contradiction, and the gate
/// refuses it rather than reporting a measured zero.
///
/// The row predicate cannot catch this one: it takes a bare slice, which
/// carries no measured/unobtained state, so an empty slice under `NoHistory`
/// is indistinguishable there from a delivery that legitimately has no rows.
/// Only the gate knows the leaf was measured. Admitting the pairing would
/// publish `Admitted { row_count: 0 }` for a domain that was absent or refused
/// -- "unavailable" collapsed into "measured zero sinkings", which is the one
/// distinction this binding exists to keep.
///
/// Mutation: delete the `NoHistory` arm from `validate_sunk_ships` and the
/// empty case admits with `row_count: 0`.
#[test]
fn a_measured_sunk_ship_leaf_under_no_history_is_refused() {
    for (label, rows) in [
        ("measured empty", Vec::new()),
        ("measured rows", vec![sunk_ship_row()]),
    ] {
        let mut binding = exact_binding();
        binding.sunk_ships = Leaf::Measured(rows);
        binding.sunk_ships_completeness = replay_core::SunkShipHistoryCompleteness::NoHistory;
        let admission = admit_xui20_replay(&binding, &frame_hour());
        assert!(
            matches!(admission.naval_terminal, Xui20SurfaceState::Rejected { .. }),
            "{label}: a measured leaf under NoHistory must be refused, got {:?}",
            admission.naval_terminal
        );
    }

    // The control: the same rows under the completeness the production
    // constructor would derive are admitted, so the refusals are about the
    // declaration and not about the rows.
    let binding = exact_binding();
    assert!(matches!(
        admit_xui20_replay(&binding, &frame_hour()).naval_terminal,
        Xui20SurfaceState::Admitted { .. }
    ));
}
