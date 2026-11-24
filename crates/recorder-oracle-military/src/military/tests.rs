use super::*;
use replay_data::{
    rows::DivisionRow, rows::FleetRow, rows::LandCombatRow, rows::NavalCombatParticipantRow,
    rows::NavalCombatRow, rows::ShipRow, rows::TableRows, rows::TaskForceRow,
};
use replay_frame::frame::TableView;

fn membership() -> content_reader_save::PartialMapMembership {
    super::super::fixtures::membership(
        "state={id=1 provinces={1} history={IF={limit={} free_operative={operative=a}}}} \
         state={id=2 provinces={2 3}}",
        3,
    )
}

fn set(frame: &mut DecodedFrame, table: TableId, rows: TableRows) {
    *frame.tables[table.index()] = TableView::Complete(rows);
}

fn zero() -> content_reader_save::Decimal {
    content_reader_save::Decimal::parse(b"0").unwrap()
}

#[test]
fn division_movement_style_compares_writer_truth_priority_and_local_refusals() {
    use replay_data::{rows::DivisionMovementRow, rows::Recorded};
    let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
    let id = (7_u64 << 32) | 51;
    saved.divisions.clear();
    saved.divisions.insert(
        id,
        content_reader_save::Division {
            id,
            roster_owner: b"GER".to_vec(),
            logical_owner: b"GER".to_vec(),
            expeditionary_owner: None,
            province: None,
            template: None,
            alternate_template: None,
            separate_counter: false,
            hit_points: zero(),
            organisation: zero(),
            country_intel: Vec::new(),
            presentation: Default::default(),
        },
    );
    let base = DivisionMovementRow {
        division: id,
        withdrawal: Recorded::Value(Some(255)),
        movement_priority: Recorded::Value(Some(1)),
        queued_actions: Recorded::Value(Some(0)),
        ..Default::default()
    };
    for (raw, expected) in [
        (0, Some(0)),
        (1, None),
        (2, Some(2)),
        (3, Some(3)),
        (i32::MIN, Some(0)),
    ] {
        saved
            .divisions
            .get_mut(&id)
            .unwrap()
            .presentation
            .withdrawal = true;
        saved
            .divisions
            .get_mut(&id)
            .unwrap()
            .presentation
            .movement_priority = expected;
        set(
            &mut frame,
            TableId::DivisionMovement,
            TableRows::DivisionMovement(vec![DivisionMovementRow {
                movement_priority: Recorded::Value(Some(raw)),
                ..base.clone()
            }]),
        );
        let mut report = Report::default();
        compare_division_movement(&frame, &saved, &mut report).unwrap();
        assert!(
            report.differences.is_empty(),
            "raw {raw}: {:?}",
            report.differences
        );
        assert!(
            !report
                .limitations
                .keys()
                .any(|reason| reason.contains("DivisionMovement.fresh")),
            "{:?}",
            report.limitations
        );
        if raw == i32::MIN {
            assert!(
                report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains("unknown priority word"))
            );
        }
    }
    saved
        .divisions
        .get_mut(&id)
        .unwrap()
        .presentation
        .movement_priority = Some(2);
    let mut report = Report::default();
    compare_division_movement(&frame, &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("movement_priority"));

    saved
        .divisions
        .get_mut(&id)
        .unwrap()
        .presentation
        .movement_priority = None;
    for (value, differences) in [
        (Recorded::NotRecorded, 0),
        (Recorded::Value(None), 1),
        (Recorded::Value(Some(0)), 1),
    ] {
        set(
            &mut frame,
            TableId::DivisionMovement,
            TableRows::DivisionMovement(vec![DivisionMovementRow {
                withdrawal: value,
                ..base.clone()
            }]),
        );
        let mut report = Report::default();
        compare_division_movement(&frame, &saved, &mut report).unwrap();
        assert_eq!(
            report.differences.len(),
            differences,
            "{:?}",
            report.differences
        );
    }
    for (value, differences) in [
        (Recorded::NotRecorded, 0),
        (Recorded::Value(None), 1),
        (Recorded::Value(Some(2)), 1),
    ] {
        set(
            &mut frame,
            TableId::DivisionMovement,
            TableRows::DivisionMovement(vec![DivisionMovementRow {
                movement_priority: value,
                source: 99,
                ..base.clone()
            }]),
        );
        let mut report = Report::default();
        compare_division_movement(&frame, &saved, &mut report).unwrap();
        assert_eq!(
            report.differences.len(),
            differences + 1,
            "{:?}",
            report.differences
        );
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains("source"))
        );
    }
    for (value, saved_count, differences) in [
        (Recorded::NotRecorded, 0, 0),
        (Recorded::Value(None), 0, 1),
        (Recorded::Value(Some(2)), 2, 0),
        (Recorded::Value(Some(2)), 1, 1),
        (Recorded::Value(Some(0)), 1, 1),
    ] {
        saved
            .divisions
            .get_mut(&id)
            .unwrap()
            .presentation
            .queued_actions = saved_count;
        set(
            &mut frame,
            TableId::DivisionMovement,
            TableRows::DivisionMovement(vec![DivisionMovementRow {
                queued_actions: value,
                ..base.clone()
            }]),
        );
        let mut report = Report::default();
        compare_division_movement(&frame, &saved, &mut report).unwrap();
        assert_eq!(
            report.differences.len(),
            differences,
            "{:?}",
            report.differences
        );
    }
}

#[test]
fn stored_air_wing_name_oracle_compares_bytes_presence_and_capture_refusals() {
    use replay_data::{rows::AirWingRow, rows::Recorded};
    let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
    let id = (1 << 32) | 69;
    let country = frame.complete::<tables::Countries>().unwrap()[0].clone();
    let links = BTreeMap::from([(101, &country)]);
    saved.air.wings.push(content_reader_save::air::Wing {
        id,
        pool_type: b"fixture_plane".to_vec(),
        name: None,
        accidents: None,
        experience: zero(),
        equipment: content_reader_save::air::EquipmentPool {
            entries: Vec::new(),
            allow_zero: false,
        },
        ace: None,
        raid: None,
    });
    for (name, expected, differences) in [
        (
            Recorded::Value(Some(vec![0xff, b'\\', b'"', b'W'])),
            Some(vec![0xff, b'\\', b'"', b'W']),
            0,
        ),
        (
            Recorded::Value(Some(vec![0xff, 0, b'W'])),
            Some(vec![0xff]),
            0,
        ),
        (Recorded::Value(Some(Vec::new())), Some(Vec::new()), 0),
        (Recorded::Value(Some(Vec::new())), None, 1),
        (Recorded::Value(None), Some(Vec::new()), 1),
        (Recorded::NotRecorded, Some(b"Old save name".to_vec()), 0),
        (Recorded::NotRecorded, None, 1),
    ] {
        let suffix = matches!(&name, Recorded::Value(Some(bytes)) if bytes.contains(&0));
        saved.air_wings.insert(
            id,
            content_reader_save::AirWing {
                government_in_exile_tag: None,
                manpower: Some(40),

                id,
                owner: b"GER".to_vec(),
                reinforcement_setting: Some(0),
                name: expected,
                country_override: b"---".to_vec(),
                air_base: 0,
                planes: 1,
                mission_mask: 0,
                mission_region: None,
                mission_active: None,
                stop_training_at_max_xp: false,
                region_change_penalty: None,
                transfer_progress: None,
                transferring_to: None,
                pool_ordinal: 0,
            },
        );
        set(
            &mut frame,
            TableId::AirWings,
            TableRows::AirWings(vec![AirWingRow {
                id,
                owner: 101,
                planes: 1,
                acquired_manpower: Some(40).into(),
                exile_country: Some(0).into(),
                name,
                replacement_setting: Recorded::Value(Some(0)),
                ..Default::default()
            }]),
        );
        let mut report = Report::default();
        compare_air_wings(&frame, &saved, &links, &mut report).unwrap();
        if suffix {
            assert!(
                report
                    .limitations
                    .values()
                    .flatten()
                    .any(|field| field.ends_with(".name"))
            );
        }
        assert_eq!(
            report.differences.len(),
            differences,
            "{:?}",
            report.differences
        );
        if differences != 0 {
            assert!(report.differences[0].contains(".name"));
        }
    }
}

#[test]
fn wing_replacement_oracle_checks_raw_bytes_and_never_waives_missing_or_unavailable_values() {
    use replay_data::rows::{AirWingRow, Recorded};
    let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
    let id = (1 << 32) | 69;
    let country = frame.complete::<tables::Countries>().unwrap()[0].clone();
    let links = BTreeMap::from([(101, &country)]);
    saved.air.wings.push(content_reader_save::air::Wing {
        id,
        pool_type: b"fixture_plane".to_vec(),
        name: Some(Vec::new()),
        accidents: None,
        experience: zero(),
        equipment: content_reader_save::air::EquipmentPool {
            entries: Vec::new(),
            allow_zero: false,
        },
        ace: None,
        raid: None,
    });
    for (actual, expected, differences) in [
        (Recorded::Value(Some(255)), Some(255), 0),
        (Recorded::Value(Some(254)), Some(255), 1),
        (Recorded::Value(None), Some(255), 1),
        (Recorded::NotRecorded, Some(0), 1),
        (Recorded::Value(Some(0)), None, 1),
    ] {
        saved.air_wings.insert(
            id,
            content_reader_save::AirWing {
                government_in_exile_tag: None,
                manpower: Some(40),
                stop_training_at_max_xp: false,
                transfer_progress: None,
                transferring_to: None,
                id,
                owner: b"GER".to_vec(),
                reinforcement_setting: expected,
                name: Some(Vec::new()),
                country_override: b"---".to_vec(),
                air_base: 0,
                planes: 1,
                mission_mask: 0,
                mission_region: None,
                mission_active: None,
                region_change_penalty: None,
                pool_ordinal: 0,
            },
        );
        set(
            &mut frame,
            TableId::AirWings,
            TableRows::AirWings(vec![AirWingRow {
                id,
                owner: 101,
                planes: 1,
                name: Recorded::Value(Some(Vec::new())),
                acquired_manpower: Some(40).into(),
                exile_country: Some(0).into(),
                replacement_setting: actual,
                ..Default::default()
            }]),
        );
        let mut report = Report::default();
        compare_air_wings(&frame, &saved, &links, &mut report).unwrap();
        assert_eq!(
            report.differences.len(),
            differences,
            "{:?}",
            report.differences
        );
        if differences != 0 {
            assert!(report.differences[0].contains("reinforcement_setting"));
        }
    }
}

#[test]
fn division_country_ordinal_compares_the_full_saved_roster() {
    let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
    let division = |id: u64| content_reader_save::Division {
        id: (id << 32) | 4713,
        roster_owner: b"GER".to_vec(),
        logical_owner: b"GER".to_vec(),
        expeditionary_owner: None,
        province: None,
        template: None,
        alternate_template: None,
        separate_counter: false,
        hit_points: zero(),
        organisation: zero(),
        country_intel: Vec::new(),
        presentation: content_reader_save::DivisionPresentation {
            max_supply: Some(zero()),
            army_current_supply_ratio: Some(zero()),
            ..Default::default()
        },
    };
    let first = (1 << 32) | 4713;
    let second = (2 << 32) | 4713;
    saved.divisions.insert(first, division(1));
    saved.divisions.insert(second, division(2));
    saved
        .counter_inputs
        .division_orders
        .insert(b"GER".to_vec(), vec![second, first]);
    let mut rows = vec![
        replay_data::rows::DivisionStatusRow {
            officer_experience: Some(0).into(),
            career_counter: Some(0).into(),
            out_of_supply_days: replay_data::rows::Recorded::NotRecorded,
            division: first,
            experience: replay_data::rows::Recorded::NotRecorded,
            pending_template: replay_data::rows::Recorded::NotRecorded,
            country_ordinal: replay_data::rows::Recorded::Value(Some(1)),
            supply: replay_data::rows::Recorded::NotRecorded,
            supply_ratio_exact: replay_data::rows::Recorded::Value(Some(0)),
            supply_grace_exact: replay_data::rows::Recorded::Value(Some(0)),
            ..Default::default()
        },
        replay_data::rows::DivisionStatusRow {
            officer_experience: Some(0).into(),
            career_counter: Some(0).into(),
            out_of_supply_days: replay_data::rows::Recorded::NotRecorded,
            division: second,
            experience: replay_data::rows::Recorded::NotRecorded,
            pending_template: replay_data::rows::Recorded::NotRecorded,
            country_ordinal: replay_data::rows::Recorded::Value(Some(0)),
            supply: replay_data::rows::Recorded::NotRecorded,
            supply_ratio_exact: replay_data::rows::Recorded::Value(Some(0)),
            supply_grace_exact: replay_data::rows::Recorded::Value(Some(0)),
            ..Default::default()
        },
    ];
    set(
        &mut frame,
        TableId::DivisionStatus,
        TableRows::DivisionStatus(rows.clone()),
    );
    let mut report = Report::default();
    compare_division_status(
        &frame,
        &saved,
        &BTreeMap::new(),
        &Ok(50_000),
        &Ok(72),
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    rows[0].country_ordinal = replay_data::rows::Recorded::Value(Some(0));
    set(
        &mut frame,
        TableId::DivisionStatus,
        TableRows::DivisionStatus(rows),
    );
    compare_division_status(
        &frame,
        &saved,
        &BTreeMap::new(),
        &Ok(50_000),
        &Ok(72),
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("country_ordinal"));
}

#[test]
fn division_status_naming_takes_the_recorded_member_template() {
    use replay_data::{
        rows::DivisionStatusRow, rows::NameOperands, rows::Recorded, rows::Witnessed,
    };
    let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
    let id = (1_u64 << 32) | 4713;
    let (standard, deleted) = (1_u64 << 32 | 52, 999_u64 << 32 | 52);
    saved.division_templates.insert(
        standard,
        content_reader_save::DivisionTemplate {
            origin_type: None,
            template: 0,
            country: b"GER".to_vec(),
            name: Vec::new(),
            name_present: false,
            name_key: Vec::new(),
            name_key_present: false,
            custom_icon: 0,
            custom_icon_nato: false,
            foreign_template_tag: None,
            composition: BTreeMap::new(),
            names_group: Some(b"GER_INF_01".to_vec()),
            ..Default::default()
        },
    );
    saved.divisions.insert(
        id,
        content_reader_save::Division {
            id,
            roster_owner: b"GER".to_vec(),
            logical_owner: b"GER".to_vec(),
            expeditionary_owner: None,
            province: None,
            template: Some(standard),
            alternate_template: None,
            separate_counter: false,
            hit_points: zero(),
            organisation: zero(),
            country_intel: Vec::new(),
            presentation: content_reader_save::DivisionPresentation {
                max_supply: Some(zero()),
                army_current_supply_ratio: Some(zero()),
                division_name: Some(content_reader_save::DivisionName {
                    programmatic: Some(true),
                    custom: Some(b"1. Infanterie".to_vec()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        },
    );
    let naming_report = |frame: &DecodedFrame, template: Option<u64>| {
        let mut frame = frame.clone();
        set(
            &mut frame,
            TableId::DivisionStatus,
            TableRows::DivisionStatus(vec![DivisionStatusRow {
                officer_experience: Some(0).into(),
                career_counter: Some(0).into(),
                division: id,
                naming: Recorded::Value(Some(NameOperands {
                    ordered: 1,
                    programmatic: 1,
                    custom: b"1. Infanterie".to_vec(),
                    ..Default::default()
                })),
                name_template: Recorded::Value(Witnessed(template)),
                supply: Recorded::NotRecorded,
                ..Default::default()
            }]),
        );
        let mut report = Report::default();
        compare_division_status(
            &frame,
            &saved,
            &BTreeMap::new(),
            &Ok(50_000),
            &Ok(72),
            &mut report,
        )
        .unwrap();
        let differences = report
            .differences
            .into_iter()
            .filter(|difference| difference.contains("override") || difference.contains("name "))
            .collect::<Vec<_>>();
        let not_in_recording = report
            .limitations
            .keys()
            .any(|reason| reason.starts_with("this recording does not record the name member"));
        (differences, not_in_recording)
    };
    let naming_differences =
        |frame: &DecodedFrame, template: Option<u64>| naming_report(frame, template).0;
    assert_eq!(
        naming_report(&frame, Some(deleted)),
        (Vec::<String>::new(), false)
    );
    assert_eq!(naming_report(&frame, None), (Vec::<String>::new(), true));
    set(
        &mut frame,
        TableId::DivisionTemplates,
        TableRows::DivisionTemplates(vec![replay_data::rows::DivisionTemplateRow {
            template: deleted,
            ..Default::default()
        }]),
    );
    assert_eq!(naming_differences(&frame, Some(deleted)).len(), 1);
}

#[test]
fn division_disrupted_supply_oracle_checks_positive_presence_at_display_precision() {
    use replay_data::{rows::DivisionStatusRow, rows::DivisionSupplyFooter, rows::Recorded};
    let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
    let id = (1_u64 << 32) | 51;
    saved.divisions.clear();
    saved.divisions.insert(
        id,
        content_reader_save::Division {
            id,
            roster_owner: b"X01".to_vec(),
            logical_owner: b"X01".to_vec(),
            expeditionary_owner: None,
            province: None,
            template: None,
            alternate_template: None,
            separate_counter: false,
            hit_points: zero(),
            organisation: zero(),
            country_intel: Vec::new(),
            presentation: content_reader_save::DivisionPresentation {
                max_supply: Some(zero()),
                army_current_supply_ratio: Some(zero()),
                ..Default::default()
            },
        },
    );
    let value = |disruption| {
        Recorded::Value(Some(DivisionSupplyFooter {
            disruption,
            origins: None,
            ..Default::default()
        }))
    };
    for (supply, text, differences) in [
        (value(1_000), Some("0.00001"), 0),
        (value(1_000), Some("0.01499"), 0),
        (value(1_000), Some("0.01500"), 1),
        (value(0), Some("0.00001"), 1),
        (value(0), None, 0),
        (value(-1_000), None, 0),
        (value(1_000), None, 1),
        (Recorded::Value(None), None, 1),
        (Recorded::Value(None), Some("0.00001"), 1),
        (Recorded::NotRecorded, Some("0.00001"), 0),
    ] {
        saved
            .divisions
            .get_mut(&id)
            .unwrap()
            .presentation
            .disrupted_supply =
            text.map(|text| content_reader_save::Decimal::parse(text.as_bytes()).unwrap());
        set(
            &mut frame,
            TableId::DivisionStatus,
            TableRows::DivisionStatus(vec![DivisionStatusRow {
                officer_experience: Some(0).into(),
                career_counter: Some(0).into(),
                out_of_supply_days: replay_data::rows::Recorded::NotRecorded,
                division: id,
                supply_grace_maximum: Recorded::Value(0),
                experience: Recorded::NotRecorded,
                pending_template: Recorded::NotRecorded,
                supply_grace_remaining: Recorded::Value(0),
                supply: supply.clone(),
                supply_ratio_exact: Recorded::Value(Some(0)),
                supply_grace_exact: Recorded::Value(Some(0)),
                ..Default::default()
            }]),
        );
        let mut report = Report::default();
        compare_division_status(
            &frame,
            &saved,
            &BTreeMap::new(),
            &Ok(50_000),
            &Ok(72),
            &mut report,
        )
        .unwrap();
        assert_eq!(
            report.differences.len(),
            differences,
            "{supply:?} {text:?} {:?}",
            report.differences
        );
        assert!(
            report
                .differences
                .iter()
                .all(|line| line.contains("supply.disrupted_supply"))
        );
        if matches!(supply, Recorded::NotRecorded) {
            assert!(
                report
                    .limitations
                    .contains_key("DivisionStatus.supply not recorded")
            );
        }
        if text.is_some() && matches!(supply, Recorded::Value(Some(_))) {
            assert!(
                report.display_precision_lines().iter().any(|line| line
                    == "  DivisionStatus.supply.disruption: 0.01 (fixed5, half away from zero, nonzero kept)")
            );
        }
    }
    let presentation = &mut saved.divisions.get_mut(&id).unwrap().presentation;
    presentation.dig_in = Some(content_reader_save::Decimal::parse(b"1").unwrap());
    presentation.disrupted_supply = None;
    set(
        &mut frame,
        TableId::DivisionStatus,
        TableRows::DivisionStatus(vec![DivisionStatusRow {
            officer_experience: Some(0).into(),
            career_counter: Some(0).into(),
            out_of_supply_days: replay_data::rows::Recorded::NotRecorded,
            division: id,
            supply_grace_remaining: Recorded::NotRecorded,
            experience: Recorded::NotRecorded,
            pending_template: Recorded::NotRecorded,
            supply: Recorded::Value(None),
            supply_ratio_exact: Recorded::Value(Some(0)),
            supply_grace_exact: Recorded::Value(Some(0)),
            ..Default::default()
        }]),
    );
    let mut report = Report::default();
    compare_division_status(
        &frame,
        &saved,
        &BTreeMap::new(),
        &Ok(50_000),
        &Ok(72),
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains(".entrenchment"))
    );
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains("supply.disrupted_supply"))
    );
    assert!(
        report
            .limitations
            .contains_key("DivisionStatus.supply_grace_remaining not recorded")
    );
}

#[test]
fn division_status_checks_the_combat_movement_branch_and_undefined_move_flags() {
    use replay_data::rows::{DivisionMovementRow, DivisionStatusRow, Recorded};
    let (mut frame, saved, _, _) = super::super::fixtures::fixture();
    let id = (7_u64 << 32) | 51;
    let all_flags = DivisionStatusRow::MOVE_EXILED
        | DivisionStatusRow::MOVE_CAN_EXFILTRATE_FROM_COAST
        | DivisionStatusRow::MOVE_NONCORE_SUPPLY_BYPASS
        | DivisionStatusRow::MOVE_NONCORE_AIR;
    let mut differences = |combat_movement: Option<i64>,
                           move_flags: u8,
                           combat_count: u32,
                           withdrawal: u8|
     -> Vec<String> {
        set(
            &mut frame,
            TableId::DivisionStatus,
            TableRows::DivisionStatus(vec![DivisionStatusRow {
                division: id,
                out_of_supply_days: Recorded::NotRecorded,
                supply: Recorded::NotRecorded,
                combat_movement: Recorded::Value(Some(combat_movement)),
                move_flags,
                ..Default::default()
            }]),
        );
        set(
            &mut frame,
            TableId::DivisionMovement,
            TableRows::DivisionMovement(vec![DivisionMovementRow {
                division: id,
                combat_count,
                withdrawal: Recorded::Value(Some(withdrawal)),
                ..Default::default()
            }]),
        );
        let mut report = Report::default();
        compare_division_status(
            &frame,
            &saved,
            &BTreeMap::new(),
            &Ok(50_000),
            &Ok(72),
            &mut report,
        )
        .unwrap();
        assert!(
            report.limitations.keys().any(|reason| {
                reason.starts_with("combat_movement is the movement word")
                    && reason.contains("the save omits")
            }),
            "{:?}",
            report.limitations
        );
        report
            .differences
            .into_iter()
            .filter(|line| line.contains(".combat_movement") || line.contains(".move_flags"))
            .collect()
    };
    assert_eq!(
        differences(Some(-13_000), all_flags, 2, 0),
        Vec::<String>::new()
    );
    assert_eq!(differences(None, 0, 2, 0), Vec::<String>::new());
    assert_eq!(differences(None, 0, 0, 1), Vec::<String>::new());
    for (combat_count, withdrawal) in [(0, 0), (2, 1)] {
        let lines = differences(Some(-13_000), 0, combat_count, withdrawal);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains(".combat_movement"), "{lines:?}");
    }
    let lines = differences(None, all_flags | 0x80, 0, 0);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains(".move_flags"), "{lines:?}");
}

#[test]
fn land_combats_compare_locations_and_missing_rows_despite_membership_exclusions() {
    let membership = membership();
    let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
    for (id, province) in [(11, 1), (12, 1), (13, 2), (14, 2), (17, 2)] {
        saved.land_combats.insert(
            id,
            content_reader_save::LandCombat {
                id,
                province,
                day: 0,
                duration: 0,
                attackers: Vec::new(),
                defenders: Vec::new(),
                details: Default::default(),
            },
        );
    }
    set(
        &mut frame,
        TableId::LandCombats,
        TableRows::LandCombats(
            [(11, 1), (13, 1), (14, 2), (15, 1), (16, 2)]
                .into_iter()
                .map(|(id, province)| LandCombatRow {
                    id,
                    province,
                    day: 1,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    let mut report = Report::default();
    compare_land_combats(&frame, &saved, &membership, &mut report).unwrap();
    assert_eq!(report.land_combats, 3);
    assert_differences(
        &report,
        &[
            "type=11 id=0.day",
            "type=12 id=0",
            "type=13 id=0.province",
            "type=13 id=0.day",
            "type=14 id=0.day",
            "type=15 id=0",
            "type=16 id=0",
            "type=17 id=0",
        ],
    );
    assert!(report.membership_skips.is_empty());
}

#[test]
fn land_combat_classes_follow_the_saved_roster_of_each_participant() {
    let membership = membership();
    let (frame, mut saved, _, _) = super::super::fixtures::fixture();
    let owner = saved.countries.keys().next().unwrap().clone();
    let (division, gun, task_force, stray) = (
        21_u64 << 32 | 4713,
        22_u64 << 32 | 4713,
        23_u64 << 32 | 61,
        24_u64 << 32 | 4713,
    );
    saved.divisions.insert(
        division,
        content_reader_save::Division {
            id: division,
            roster_owner: owner.clone(),
            logical_owner: owner.clone(),
            expeditionary_owner: None,
            province: Some(1),
            template: None,
            alternate_template: None,
            separate_counter: false,
            hit_points: zero(),
            organisation: zero(),
            country_intel: Vec::new(),
            presentation: Default::default(),
        },
    );
    saved.railway_guns.insert(
        gun,
        content_reader_save::RailwayGun {
            id: gun,
            name: None,
            roster_owner: owner.clone(),
            logical_owner: owner.clone(),
            expeditionary_owner: None,
            province: Some(1),
            path: Vec::new(),
            support_attack: None,
            full_path: Vec::new(),
            strategic_redeployment: false,
            exile: false,
            withdrawal: false,
            strength: zero(),
            combats: Vec::new(),
            country_intel: Vec::new(),
            queued_actions: 0,
        },
    );
    saved.task_forces.insert(
        task_force,
        content_reader_save::TaskForce {
            id: task_force,
            name: None,
            fleet: 11,
            roster_owner: owner.clone(),
            logical_owner: owner,
            expeditionary_owner: None,
            province: Some(1),
            mission: 0,
            country_intel: Vec::new(),
            path: Vec::new(),
            target: None,
            alternate_path: Vec::new(),
            movement_progress: None,
            retreat: false,
            withdrawal: false,
            exile: false,
        },
    );
    let attackers = vec![division, gun];
    let combat = |defenders: Vec<u64>| content_reader_save::LandCombat {
        id: 11,
        province: 1,
        day: 0,
        duration: 0,
        attackers: attackers.clone(),
        defenders,
        details: Default::default(),
    };
    let compare =
        |saved: &content_reader_save::Save, defenders: Vec<u64>, defender_classes: Vec<u32>| {
            let mut frame = frame.clone();
            set(
                &mut frame,
                TableId::LandCombats,
                TableRows::LandCombats(vec![LandCombatRow {
                    id: 11,
                    province: 1,
                    attackers: attackers.clone(),
                    defenders,
                    attacker_classes: vec![0, 13],
                    defender_classes,
                    ..Default::default()
                }]),
            );
            let mut report = Report::default();
            compare_land_combats(&frame, saved, &membership, &mut report).unwrap();
            report
        };
    saved.land_combats.insert(11, combat(vec![task_force]));
    assert_differences(&compare(&saved, vec![task_force], vec![1]), &[]);
    assert_differences(
        &compare(&saved, vec![task_force], vec![0]),
        &["type=11 id=0.defender_classes"],
    );
    saved.land_combats.insert(11, combat(vec![stray]));
    assert_differences(&compare(&saved, vec![stray], vec![13]), &[]);
    assert_differences(
        &compare(&saved, vec![stray], vec![]),
        &["type=11 id=0.defender_classes"],
    );
}

#[test]
fn divisions_compare_locations_strength_and_missing_rows_despite_membership_exclusions() {
    content_reader_script::test_game::enter();
    let membership = membership();
    let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
    let owner = saved.countries.keys().next().unwrap().clone();
    for (id, province) in [(11, 1), (12, 2), (13, 2)] {
        saved.divisions.insert(
            id,
            content_reader_save::Division {
                id,
                roster_owner: owner.clone(),
                logical_owner: owner.clone(),
                expeditionary_owner: None,
                province: Some(province),
                template: None,
                alternate_template: None,
                separate_counter: false,
                hit_points: zero(),
                organisation: zero(),
                country_intel: Vec::new(),
                presentation: Default::default(),
            },
        );
    }
    set(
        &mut frame,
        TableId::Divisions,
        TableRows::Divisions(
            [(12, 1), (13, 2), (14, 1)]
                .into_iter()
                .map(|(id, province)| DivisionRow {
                    id,
                    province,
                    roster_owner: 101,
                    logical_owner: 101,
                    hit_points: 1,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    let context = super::super::country_context(&frame).unwrap();
    let mut report = Report::default();
    compare_divisions(&frame, &saved, &membership, &context.links, &mut report).unwrap();
    assert_eq!(report.divisions, 2);
    assert_differences(
        &report,
        &[
            "type=11 id=0",
            "type=12 id=0.province",
            "type=12 id=0.hit_points",
            "type=13 id=0.hit_points",
            "type=14 id=0",
        ],
    );
    assert!(report.membership_skips.is_empty());
}

#[test]
fn naval_units_compare_locations_and_missing_rows_despite_membership_exclusions() {
    let membership = membership();
    let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
    let owner = saved.countries.keys().next().unwrap().clone();
    for (id, home_base) in [(11, 1), (12, 2), (17, 1)] {
        saved.fleets.insert(
            id,
            content_reader_save::Fleet {
                id,
                owner: owner.clone(),
                home_base: Some(home_base),
                name: Some(Vec::new()),
                leader: None,
            },
        );
    }
    for (id, province) in [(13, 2), (14, 1), (15, 1)] {
        saved.task_forces.insert(
            id,
            content_reader_save::TaskForce {
                id,
                name: None,
                fleet: 11,
                roster_owner: owner.clone(),
                logical_owner: owner.clone(),
                expeditionary_owner: None,
                province: Some(province),
                mission: 0,
                country_intel: Vec::new(),
                path: Vec::new(),
                target: None,
                alternate_path: Vec::new(),
                movement_progress: None,
                retreat: false,
                withdrawal: false,
                exile: false,
            },
        );
    }
    saved.ships.insert(
        15,
        content_reader_save::Ship {
            ship_name: Some(content_reader_save::ShipName {
                kind: 1,
                name_order: None,
                is_name_ordered: None,
                override_name: None,
                override_set_programmatically: None,
                equipment: None,
            }),
            manpower: Some(0),
            max_manpower: Some(0),

            has_criticals: false,
            id: 15,
            roster_ordinal: 0,
            task_force: 14,
            strength: zero(),
            organisation: zero(),
            experience: zero(),
            sunk_convoys: 0,
            equipment: None,
        },
    );
    set(
        &mut frame,
        TableId::Fleets,
        TableRows::Fleets(vec![
            FleetRow {
                id: 11,
                owner: 101,
                home_base: 1,
                name: Some(Vec::new()).into(),
                leader: Some(0).into(),
            },
            FleetRow {
                id: 12,
                owner: 101,
                home_base: 1,
                name: Some(Vec::new()).into(),
                leader: Some(0).into(),
            },
            FleetRow {
                id: 18,
                owner: 101,
                home_base: 1,
                name: Some(Vec::new()).into(),
                leader: Some(0).into(),
            },
        ]),
    );
    set(
        &mut frame,
        TableId::TaskForces,
        TableRows::TaskForces(
            [(13, 1), (14, 1), (16, 1)]
                .into_iter()
                .map(|(id, province)| TaskForceRow {
                    id,
                    fleet: 11,
                    province,
                    roster_owner: 101,
                    logical_owner: 101,
                    mission: 1,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        &mut frame,
        TableId::Ships,
        TableRows::Ships(vec![ShipRow {
            naming: Some(replay_data::rows::ShipNamingOperands {
                kind: 1,
                ordered: 1,
                ..Default::default()
            })
            .into(),
            acquired_manpower: Some(0).into(),
            required_manpower: Some(0).into(),
            id: 15,
            task_force: 14,
            strength: 1,
            ..Default::default()
        }]),
    );
    let context = super::super::country_context(&frame).unwrap();
    let mut report = Report::default();
    compare_naval_units(&frame, &saved, &membership, &context.links, &mut report).unwrap();
    assert_eq!((report.fleets, report.task_forces, report.ships), (2, 2, 1));
    assert_differences(
        &report,
        &[
            "type=12 id=0.home_base",
            "fleet type=17 id=0",
            "fleet 18",
            "type=13 id=0.province",
            "type=13 id=0.mission",
            "type=14 id=0.mission",
            "task force type=15 id=0",
            "task force 16",
            "type=15 id=0.strength",
        ],
    );
    assert!(report.membership_skips.is_empty());
}

#[test]
fn naval_combats_compare_locations_and_missing_rows_despite_membership_exclusions() {
    let membership = membership();
    let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
    for (id, province) in [(11, 1), (12, 2), (13, 1), (14, 2)] {
        saved.naval_combats.insert(
            id,
            content_reader_save::NavalCombat {
                id,
                province,
                attackers: Vec::new(),
                defenders: Vec::new(),
                attacker_positioning: zero(),
                defender_positioning: zero(),
                attacker_anti_air: zero(),
                defender_anti_air: zero(),
                details: Default::default(),
                transfers: Vec::new(),
            },
        );
    }
    saved.naval_combat_participants.insert(
        (11, 0, 0),
        content_reader_save::NavalCombatParticipant {
            combat: 11,
            side: 0,
            battle_slot: 0,
            ship: None,
            convoy: None,
            owner: saved.countries.keys().next().unwrap().clone(),
            state: 0,
            details: Default::default(),
        },
    );
    set(
        &mut frame,
        TableId::NavalCombats,
        TableRows::NavalCombats(
            [(11, 1), (12, 1), (15, 1), (16, 2)]
                .into_iter()
                .enumerate()
                .map(|(ordinal, (id, province))| NavalCombatRow {
                    id,
                    province,
                    attacker_positioning: 1,
                    registry_ordinal: ordinal as u32,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        &mut frame,
        TableId::NavalCombatParticipants,
        TableRows::NavalCombatParticipants(vec![NavalCombatParticipantRow {
            combat: 11,
            side: 0,
            battle_slot: 0,
            owner: 101,
            state: 1,
            ..Default::default()
        }]),
    );
    let context = super::super::country_context(&frame).unwrap();
    let mut report = Report::default();
    compare_naval_combats(&frame, &saved, &membership, &context.links, &mut report).unwrap();
    assert_eq!(
        (report.naval_combats, report.naval_combat_participants),
        (2, 1)
    );
    assert_differences(
        &report,
        &[
            "type=11 id=0.attacker_positioning",
            "type=12 id=0.province",
            "type=12 id=0.attacker_positioning",
            "naval combat type=13 id=0",
            "naval combat type=14 id=0",
            "naval combat 15",
            "naval combat 16",
            "naval participant combat=11 side=0 slot=0.state",
        ],
    );
    assert!(report.membership_skips.is_empty());
}

fn ship_experience_report(recorded: i64) -> Report {
    ship_report(recorded, Some(0).into(), Some(0))
}

fn ship_report(recorded: i64, crew: Recorded<Option<i32>>, saved_crew: Option<i32>) -> Report {
    let membership = membership();
    let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
    let owner = saved.countries.keys().next().unwrap().clone();
    saved.fleets.insert(
        11,
        content_reader_save::Fleet {
            id: 11,
            owner: owner.clone(),
            home_base: Some(1),
            name: Some(Vec::new()),
            leader: None,
        },
    );
    saved.task_forces.insert(
        14,
        content_reader_save::TaskForce {
            id: 14,
            name: None,
            fleet: 11,
            roster_owner: owner.clone(),
            logical_owner: owner,
            expeditionary_owner: None,
            province: Some(1),
            mission: 0,
            country_intel: Vec::new(),
            path: Vec::new(),
            target: None,
            alternate_path: Vec::new(),
            movement_progress: None,
            retreat: false,
            withdrawal: false,
            exile: false,
        },
    );
    saved.ships.insert(
        15,
        content_reader_save::Ship {
            ship_name: Some(content_reader_save::ShipName {
                kind: 1,
                name_order: None,
                is_name_ordered: None,
                override_name: None,
                override_set_programmatically: None,
                equipment: None,
            }),
            manpower: saved_crew,
            max_manpower: saved_crew,

            has_criticals: false,
            id: 15,
            roster_ordinal: 0,
            task_force: 14,
            strength: zero(),
            organisation: zero(),
            experience: content_reader_save::Decimal::parse(b"30.00999").unwrap(),
            sunk_convoys: 0,
            equipment: None,
        },
    );
    set(
        &mut frame,
        TableId::Fleets,
        TableRows::Fleets(vec![FleetRow {
            id: 11,
            owner: 101,
            home_base: 1,
            name: Some(Vec::new()).into(),
            leader: Some(0).into(),
        }]),
    );
    set(
        &mut frame,
        TableId::TaskForces,
        TableRows::TaskForces(vec![TaskForceRow {
            id: 14,
            fleet: 11,
            province: 1,
            roster_owner: 101,
            logical_owner: 101,
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableId::Ships,
        TableRows::Ships(vec![ShipRow {
            naming: Some(replay_data::rows::ShipNamingOperands {
                kind: 1,
                ordered: 1,
                ..Default::default()
            })
            .into(),
            acquired_manpower: crew,
            required_manpower: crew,
            id: 15,
            task_force: 14,
            experience: recorded,
            ..Default::default()
        }]),
    );
    let context = super::super::country_context(&frame).unwrap();
    let mut report = Report::default();
    compare_naval_units(&frame, &saved, &membership, &context.links, &mut report).unwrap();
    assert_eq!(report.ships, 1);
    report
}

#[test]
fn ship_experience_compares_at_its_display_step() {
    let report = ship_experience_report(3_000_000);
    assert_differences(&report, &[]);
    assert_eq!(
        report.display_precision_lines()[1..],
        ["  Ships.experience: 0.01 (fixed5, toward zero)".to_owned()]
    );
    assert_differences(
        &ship_experience_report(2_990_000),
        &["type=15 id=0.experience"],
    );
}

#[test]
fn ship_crew_compares_with_the_save_and_a_not_recorded_crew_is_a_limitation_with_its_reason() {
    let equal = ship_report(3_000_000, Some(255).into(), Some(255));
    assert_differences(&equal, &[]);
    assert_eq!(equal.compared_tables.get("Ships.manpower"), Some(&1));
    assert_eq!(equal.compared_tables.get("Ships.max_manpower"), Some(&1));
    assert!(!equal.limitations.contains_key(CREW_NOT_RECORDED));
    assert_differences(
        &ship_report(3_000_000, Some(254).into(), Some(255)),
        &["type=15 id=0.manpower", "type=15 id=0.max_manpower"],
    );
    for (crew, saved_crew) in [
        (Recorded::Value(None), Some(255)),
        (Recorded::Value(None), None),
        (Recorded::NotRecorded, Some(255)),
    ] {
        let missing = ship_report(3_000_000, crew, saved_crew);
        assert_differences(&missing, &[]);
        assert_eq!(missing.compared_tables.get("Ships.manpower"), None);
        assert_eq!(
            missing.limitations.get(CREW_NOT_RECORDED),
            Some(&vec![
                "ship type=15 id=0.manpower".to_owned(),
                "ship type=15 id=0.max_manpower".to_owned(),
            ])
        );
    }
}

fn assert_differences(report: &Report, fields: &[&str]) {
    assert_eq!(
        report.differences.len(),
        fields.len(),
        "{:?}",
        report.differences
    );
    for field in fields {
        assert!(
            report.differences.iter().any(|line| line.contains(field)),
            "{field}: {:?}",
            report.differences
        );
    }
}

#[test]
fn fleet_name_is_mandatory_in_ordinary_save_even_for_historical_recordings() {
    let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
    let membership = membership();
    let owner = saved.countries.keys().next().unwrap().clone();
    saved.fleets.clear();
    saved.task_forces.clear();
    saved.ships.clear();
    set(
        &mut frame,
        TableId::TaskForces,
        TableRows::TaskForces(Vec::new()),
    );
    set(&mut frame, TableId::Ships, TableRows::Ships(Vec::new()));
    for name in [Recorded::NotRecorded, Recorded::Value(Some(Vec::new()))] {
        set(
            &mut frame,
            TableId::Fleets,
            TableRows::Fleets(vec![FleetRow {
                id: 17,
                owner: 101,
                home_base: 1,
                name,
                leader: Recorded::Value(Some(0)),
            }]),
        );
        saved.fleets.insert(
            17,
            content_reader_save::Fleet {
                id: 17,
                owner: owner.clone(),
                home_base: Some(1),
                name: Some(Vec::new()),
                leader: None,
            },
        );
        let context = super::super::country_context(&frame).unwrap();
        let mut report = Report::default();
        compare_naval_units(&frame, &saved, &membership, &context.links, &mut report).unwrap();
        assert!(report.differences.is_empty());
        saved.fleets.get_mut(&17).unwrap().name = None;
        assert!(
            compare_naval_units(
                &frame,
                &saved,
                &membership,
                &context.links,
                &mut Report::default()
            )
            .unwrap_err()
            .to_string()
            .contains("fleet stored name missing")
        );
    }
}

#[test]
fn manpower_wing_oracle_compares_exile_tags_omission_aliases_and_unavailable_capture() {
    use replay_data::rows::{AirWingRow, Recorded};
    let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
    let id = (1 << 32) | 69;
    let country = frame.complete::<tables::Countries>().unwrap()[0].clone();
    let mut host = country.clone();
    host.id = 201;
    host.tag = b"HST".to_vec();
    let links = BTreeMap::from([(101, &country), (102, &country), (201, &host)]);
    saved.air.wings.push(content_reader_save::air::Wing {
        id,
        pool_type: b"fixture_plane".to_vec(),
        name: None,
        accidents: None,
        experience: zero(),
        equipment: content_reader_save::air::EquipmentPool {
            entries: Vec::new(),
            allow_zero: false,
        },
        ace: None,
        raid: None,
    });
    for (raw, expected, differences) in [
        (Some(0).into(), None, 0),
        (Some(-1).into(), None, 0),
        (Some(102).into(), Some(b"GER".as_slice()), 0),
        (Some(201).into(), Some(b"HST".as_slice()), 0),
        (Some(201).into(), None, 1),
        (Some(0).into(), Some(b"HST".as_slice()), 1),
        (Recorded::Value(None), None, 0),
        (Recorded::NotRecorded, None, 0),
    ] {
        saved.air_wings.insert(
            id,
            content_reader_save::AirWing {
                reinforcement_setting: Some(0),
                stop_training_at_max_xp: false,
                transfer_progress: None,
                transferring_to: None,
                id,
                government_in_exile_tag: expected.map(Vec::from),
                manpower: Some(23),
                owner: b"GER".to_vec(),
                name: Some(Vec::new()),
                country_override: b"---".to_vec(),
                air_base: 0,
                planes: 1,
                mission_mask: 0,
                mission_region: None,
                mission_active: None,
                region_change_penalty: None,
                pool_ordinal: 0,
            },
        );
        set(
            &mut frame,
            TableId::AirWings,
            TableRows::AirWings(vec![AirWingRow {
                id,
                owner: 101,
                planes: 1,
                acquired_manpower: Some(23).into(),
                exile_country: raw,
                replacement_setting: Some(0).into(),
                name: Some(Vec::new()).into(),
                ..Default::default()
            }]),
        );
        let mut report = Report::default();
        compare_air_wings(&frame, &saved, &links, &mut report).unwrap();
        assert_eq!(
            report.differences.len(),
            differences,
            "{:?}",
            report.differences
        );
        assert!(
            report
                .differences
                .iter()
                .all(|line| line.contains("government_in_exile_tag"))
        );
    }
}

fn wing_report(
    manpower: Recorded<Option<i32>>,
    exile: Recorded<Option<i32>>,
    saved: Option<i32>,
) -> Report {
    use replay_data::rows::AirWingRow;
    let (mut frame, mut saved_world, _, _) = super::super::fixtures::fixture();
    let id = (1 << 32) | 69;
    let country = frame.complete::<tables::Countries>().unwrap()[0].clone();
    let links = BTreeMap::from([(101, &country)]);
    saved_world.air.wings.push(content_reader_save::air::Wing {
        id,
        pool_type: b"fixture_plane".to_vec(),
        name: None,
        accidents: None,
        experience: zero(),
        equipment: content_reader_save::air::EquipmentPool {
            entries: Vec::new(),
            allow_zero: false,
        },
        ace: None,
        raid: None,
    });
    saved_world.air_wings.insert(
        id,
        content_reader_save::AirWing {
            reinforcement_setting: Some(0),
            stop_training_at_max_xp: false,
            transfer_progress: None,
            transferring_to: None,
            id,
            government_in_exile_tag: None,
            manpower: saved,
            owner: b"GER".to_vec(),
            name: Some(Vec::new()),
            country_override: b"---".to_vec(),
            air_base: 0,
            planes: 1,
            mission_mask: 0,
            mission_region: None,
            mission_active: None,
            region_change_penalty: None,
            pool_ordinal: 0,
        },
    );
    set(
        &mut frame,
        TableId::AirWings,
        TableRows::AirWings(vec![AirWingRow {
            id,
            owner: 101,
            planes: 1,
            acquired_manpower: manpower,
            exile_country: exile,
            replacement_setting: Some(0).into(),
            name: Some(Vec::new()).into(),
            ..Default::default()
        }]),
    );
    let mut report = Report::default();
    compare_air_wings(&frame, &saved_world, &links, &mut report).unwrap();
    assert_eq!(report.air_wings, 1);
    report
}

#[test]
fn wing_manpower_compares_with_the_save_and_a_not_recorded_crew_is_a_limitation_with_its_reason() {
    let equal = wing_report(Some(480).into(), Some(0).into(), Some(480));
    assert_differences(&equal, &[]);
    assert_eq!(equal.compared_tables.get("AirWings.acquired_manpower"), Some(&1));
    assert!(!equal.limitations.contains_key(CREW_NOT_RECORDED));
    assert_differences(
        &wing_report(Some(479).into(), Some(0).into(), Some(480)),
        &["type=69 id=1.manpower"],
    );
    for missing in [Recorded::Value(None), Recorded::NotRecorded] {
        let missing = wing_report(missing, missing, Some(480));
        assert_differences(&missing, &[]);
        assert_eq!(missing.compared_tables.get("AirWings.acquired_manpower"), None);
        assert_eq!(
            missing.limitations.get(CREW_NOT_RECORDED),
            Some(&vec![
                "air wing type=69 id=1.government_in_exile_tag".to_owned(),
                "air wing type=69 id=1.manpower".to_owned(),
            ])
        );
    }
}

#[test]
fn division_experience_compares_stored_value_and_rejects_refused_capture() {
    use replay_data::rows::{DivisionStatusRow, Recorded};
    let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
    let id = (77_u64 << 32) | 51;
    saved.divisions.clear();
    saved.divisions.insert(
        id,
        content_reader_save::Division {
            id,
            roster_owner: b"MOD".to_vec(),
            logical_owner: b"MOD".to_vec(),
            expeditionary_owner: None,
            province: None,
            template: None,
            alternate_template: None,
            separate_counter: false,
            hit_points: zero(),
            organisation: zero(),
            country_intel: Vec::new(),
            presentation: content_reader_save::DivisionPresentation {
                experience: Some(content_reader_save::Decimal::parse(b"123.456").unwrap()),
                pending_template: Some(555),
                ..Default::default()
            },
        },
    );
    for (experience, pending_template, count, transition_count) in [
        (Some(12_345_600), Some(555), 0, 0),
        (Some(12_445_600), Some(555), 1, 0),
        (None, Some(555), 1, 0),
        (Some(12_345_600), Some(0), 0, 1),
        (Some(12_345_600), None, 0, 1),
    ] {
        set(
            &mut frame,
            TableId::DivisionStatus,
            TableRows::DivisionStatus(vec![DivisionStatusRow {
                officer_experience: Some(0).into(),
                career_counter: Some(0).into(),
                division: id,
                experience: Recorded::Value(experience),
                pending_template: Recorded::Value(pending_template),
                ..Default::default()
            }]),
        );
        let mut report = Report::default();
        compare_division_status(
            &frame,
            &saved,
            &BTreeMap::new(),
            &Ok(50_000),
            &Ok(72),
            &mut report,
        )
        .unwrap();
        let differences: Vec<_> = report
            .differences
            .iter()
            .filter(|text| text.contains("experience"))
            .collect();
        assert_eq!(differences.len(), count, "{differences:?}");
        assert_eq!(
            report
                .differences
                .iter()
                .filter(|text| text.contains("pending_template"))
                .count(),
            transition_count
        );
    }
}

#[test]
fn experience_history_oracle_compares_order_medals_and_refused_roots() {
    use replay_data::rows::{DivisionHistoryRow, SharedDivisionHistoryRow};
    use std::sync::Arc;
    let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
    let id = (7_u64 << 32) | 51;
    let entry = content_reader_save::DivisionHistoryEntry {
        name: Some(b"subject".to_vec()),
        custom_name: None,
        kind: 11,
        target_country: b"---".to_vec(),
        hour: frame.hour,
        medal: Some(b"mod_medal".to_vec()),
        medal_count: false,
        inherit: true,
        multiplier: None,
        orders: Some(-2),
        provinces: vec![9, 3, 9],
        sunk_ship: None,
    };
    saved.divisions.insert(
        id,
        content_reader_save::Division {
            id,
            roster_owner: b"GER".to_vec(),
            logical_owner: b"GER".to_vec(),
            expeditionary_owner: None,
            province: None,
            template: None,
            alternate_template: None,
            separate_counter: false,
            hit_points: zero(),
            organisation: zero(),
            country_intel: Vec::new(),
            presentation: content_reader_save::DivisionPresentation {
                history: Some(content_reader_save::DivisionHistory {
                    amount: Some(3),
                    career: vec![entry.clone(), entry.clone()],
                    awards: vec![entry],
                }),
                ..Default::default()
            },
        },
    );
    let row = DivisionHistoryRow {
        division: id,
        list: DivisionHistoryRow::CAREER,
        available: 1,
        name: b"subject".to_vec(),
        kind: 11,
        hour: frame.hour,
        medal_token: Some(55),
        medal_definition: Some(9),
        medal_enabled: 1,
        inherit: 1,
        multiplier: 100_000,
        orders: -2,
        provinces: vec![9, 3, 9],
        ..Default::default()
    };
    let rows: Vec<SharedDivisionHistoryRow> = vec![
        Arc::new(DivisionHistoryRow {
            division: id,
            available: 1,
            amount: 3,
            ..Default::default()
        }),
        Arc::new(row.clone()),
        Arc::new(DivisionHistoryRow {
            ordinal: 1,
            ..row.clone()
        }),
        Arc::new(DivisionHistoryRow {
            list: DivisionHistoryRow::AWARDS,
            ..row
        }),
    ];
    set(
        &mut frame,
        TableId::DivisionHistory,
        TableRows::DivisionHistory(rows.clone()),
    );
    let tokens = BTreeMap::from([(55, b"mod_medal".as_slice())]);
    let mut report = Report::default();
    division_history::compare(&frame, &saved, &tokens, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    for medal_count in [0, 1, 2, u8::MAX] {
        for inherit in [0, 1, 2, u8::MAX] {
            let mut flags = rows.clone();
            for actual in flags.iter_mut().skip(1) {
                let actual = Arc::make_mut(actual);
                actual.medal_count = medal_count;
                actual.inherit = inherit;
            }
            let history = saved
                .divisions
                .get_mut(&id)
                .unwrap()
                .presentation
                .history
                .as_mut()
                .unwrap();
            for expected in history.career.iter_mut().chain(&mut history.awards) {
                expected.medal_count = medal_count != 0;
                expected.inherit = inherit != 0;
            }
            set(
                &mut frame,
                TableId::DivisionHistory,
                TableRows::DivisionHistory(flags.clone()),
            );
            let mut report = Report::default();
            division_history::compare(&frame, &saved, &tokens, &mut report).unwrap();
            assert!(report.differences.is_empty(), "{:?}", report.differences);
            for field in ["medal_count", "inherit"] {
                let mut mismatch = flags.clone();
                let actual = Arc::make_mut(&mut mismatch[1]);
                if field == "medal_count" {
                    actual.medal_count = u8::from(medal_count == 0);
                } else {
                    actual.inherit = u8::from(inherit == 0);
                }
                set(
                    &mut frame,
                    TableId::DivisionHistory,
                    TableRows::DivisionHistory(mismatch),
                );
                let mut report = Report::default();
                division_history::compare(&frame, &saved, &tokens, &mut report).unwrap();
                assert_eq!(report.differences.len(), 1);
                assert!(report.differences[0].contains(field));
            }
        }
    }
    let history = saved
        .divisions
        .get_mut(&id)
        .unwrap()
        .presentation
        .history
        .as_mut()
        .unwrap();
    for expected in history.career.iter_mut().chain(&mut history.awards) {
        expected.medal_count = false;
        expected.inherit = true;
    }
    set(
        &mut frame,
        TableId::DivisionHistory,
        TableRows::DivisionHistory(rows.clone()),
    );
    saved
        .divisions
        .get_mut(&id)
        .unwrap()
        .presentation
        .history
        .as_mut()
        .unwrap()
        .career[0]
        .name = None;
    let mut report = Report::default();
    division_history::compare(&frame, &saved, &tokens, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("army_names"));
    saved
        .divisions
        .get_mut(&id)
        .unwrap()
        .presentation
        .history
        .as_mut()
        .unwrap()
        .career[0]
        .name = Some(b"subject".to_vec());
    saved
        .divisions
        .get_mut(&id)
        .unwrap()
        .presentation
        .history
        .as_mut()
        .unwrap()
        .career[0]
        .kind = 16;
    let mut custom = rows.clone();
    Arc::make_mut(&mut custom[1]).kind = 16;
    Arc::make_mut(&mut custom[1]).custom_name = b"mod_event".to_vec();
    set(
        &mut frame,
        TableId::DivisionHistory,
        TableRows::DivisionHistory(custom),
    );
    let mut report = Report::default();
    division_history::compare(&frame, &saved, &tokens, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("custom_lockey"));
    saved
        .divisions
        .get_mut(&id)
        .unwrap()
        .presentation
        .history
        .as_mut()
        .unwrap()
        .career[0]
        .kind = 11;
    let mut changed = rows.clone();
    Arc::make_mut(&mut changed[2]).provinces.swap(0, 1);
    set(
        &mut frame,
        TableId::DivisionHistory,
        TableRows::DivisionHistory(changed),
    );
    let mut report = Report::default();
    division_history::compare(&frame, &saved, &tokens, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("location"));
    let mut refused = rows;
    Arc::make_mut(&mut refused[0]).available = 0;
    Arc::make_mut(&mut refused[0]).reason = b"history unreadable".to_vec();
    set(
        &mut frame,
        TableId::DivisionHistory,
        TableRows::DivisionHistory(refused),
    );
    let mut report = Report::default();
    division_history::compare(&frame, &saved, &tokens, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("history unreadable"));
}

#[test]
fn division_panel_stats_checks_its_own_roster_and_every_operand() {
    use replay_data::rows::DivisionPanelStatsRow;
    let (mut frame, mut save, _, _) = super::super::fixtures::division_fixture();
    let id = *save.divisions.keys().next().unwrap();
    save.divisions.retain(|key, _| *key == id);
    let mut row = DivisionPanelStatsRow {
        division: id,
        panel_stats: [Some(i64::MIN); 22],
        suppression: Some(i64::MIN).into(),
        status_maxima: [None; 2],
    };
    set(
        &mut frame,
        TableId::DivisionPanelStats,
        TableRows::DivisionPanelStats(vec![row.clone()]),
    );
    let mut report = Report::default();
    compare_division_panel_stats(&frame, &save, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    row.panel_stats[21] = None;
    set(
        &mut frame,
        TableId::DivisionPanelStats,
        TableRows::DivisionPanelStats(vec![row.clone()]),
    );
    let mut report = Report::default();
    compare_division_panel_stats(&frame, &save, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("panel_stats[21]"));
    row.panel_stats[21] = Some(i64::MIN);
    row.suppression = None.into();
    set(
        &mut frame,
        TableId::DivisionPanelStats,
        TableRows::DivisionPanelStats(vec![row]),
    );
    let mut report = Report::default();
    compare_division_panel_stats(&frame, &save, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("suppression"));
    set(
        &mut frame,
        TableId::DivisionPanelStats,
        TableRows::DivisionPanelStats(vec![]),
    );
    let mut report = Report::default();
    compare_division_panel_stats(&frame, &save, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains(&id.to_string()));
}

#[test]
fn country_template_oracle_checks_selection_and_lock_without_owned_templates() {
    let (mut frame, mut save, _, _) = super::super::fixtures::fixture();
    save.production.template_states.insert(
        b"GER".to_vec(),
        content_reader_save::production::TemplateState {
            garrison_priority: Some(1),
            selected_staff: Some(None),
            lock_all: true,
            garrisons: Some(BTreeMap::new()),
            ..Default::default()
        },
    );
    set(
        &mut frame,
        TableId::CountryTemplateState,
        TableRows::CountryTemplateState(vec![replay_data::rows::CountryTemplateStateRow {
            garrison_priority: Recorded::Value(Some(1)),
            country: 102,
            selected_staff: (99_u64 << 32) | 52,
            next_hq_order: -19,
            lock_all: 7,
            garrisons: Recorded::Value(Some(Vec::new().into())),
        }]),
    );
    let countries = super::super::country_context(&frame).unwrap();
    let mut report = Report::default();
    compare_country_template_state(&frame, &save, &countries, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    save.production
        .template_states
        .get_mut(b"GER".as_slice())
        .unwrap()
        .garrison_priority = Some(-3);
    let mut priority_report = Report::default();
    compare_country_template_state(&frame, &save, &countries, &mut priority_report).unwrap();
    assert_eq!(priority_report.differences.len(), 1);
    assert!(priority_report.differences[0].contains("occupation_status.priority"));
    save.production
        .template_states
        .get_mut(b"GER".as_slice())
        .unwrap()
        .garrison_priority = Some(1);
    save.production
        .template_states
        .get_mut(b"GER".as_slice())
        .unwrap()
        .selected_staff = Some(Some((1_u64 << 32) | 52));
    save.production
        .template_states
        .get_mut(b"GER".as_slice())
        .unwrap()
        .lock_all = false;
    let mut report = Report::default();
    compare_country_template_state(&frame, &save, &countries, &mut report).unwrap();
    assert_eq!(report.differences.len(), 2);
}

#[test]
fn division_status_exact_movement_organisation_compares_saved_subdisplay_words() {
    let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
    let id = (7_u64 << 32) | 51;
    saved.divisions.insert(
        id,
        content_reader_save::Division {
            id,
            roster_owner: b"GER".to_vec(),
            logical_owner: b"GER".to_vec(),
            expeditionary_owner: None,
            province: None,
            template: None,
            alternate_template: None,
            separate_counter: false,
            hit_points: zero(),
            organisation: content_reader_save::Decimal::parse(b"9.24554").unwrap(),
            country_intel: Vec::new(),
            presentation: Default::default(),
        },
    );
    for (value, expected) in [
        (Some(Some([924_554, 3_640_123])), 0),
        (Some(Some([920_000, 3_640_123])), 1),
        (Some(Some([924_554, 3_630_123])), 1),
        (Some(None), 0),
        (None, 1),
    ] {
        let mut panel = replay_data::rows::DivisionPanelStatsRow {
            division: id,
            ..Default::default()
        };
        panel.panel_stats[1] = Some(3_640_000);
        set(
            &mut frame,
            TableId::DivisionPanelStats,
            TableRows::DivisionPanelStats(vec![panel]),
        );
        set(
            &mut frame,
            TableId::DivisionStatus,
            TableRows::DivisionStatus(vec![DivisionStatusRow {
                division: id,
                organisation_exact: Recorded::Value(value.flatten().map(|pair| pair[0])),
                maximum_organisation_exact: Recorded::Value(value.flatten().map(|pair| pair[1])),
                move_flags: match value {
                    Some(Some(_)) => DivisionStatusRow::MOVE_LOW_ORGANISATION,
                    Some(None) => 0,
                    None => DivisionStatusRow::MOVE_ORGANISATION_REFUSED,
                },
                ..Default::default()
            }]),
        );
        let mut report = Report::default();
        compare_division_status(
            &frame,
            &saved,
            &BTreeMap::new(),
            &Ok(50_000),
            &Ok(72),
            &mut report,
        )
        .unwrap();
        let differences: Vec<_> = report
            .differences
            .iter()
            .filter(|line| line.contains(".movement_organisation"))
            .collect();
        assert_eq!(differences.len(), expected, "{differences:?}");
    }
}

#[test]
fn training_stop_oracle_limits_unrecorded_and_rejects_mismatched_demanded_flags() {
    use replay_data::rows::{AirWingRow, Recorded};
    let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
    let id = (1 << 32) | 69;
    let country = frame.complete::<tables::Countries>().unwrap()[0].clone();
    let links = BTreeMap::from([(101, &country)]);
    saved.air.wings.push(content_reader_save::air::Wing {
        id,
        pool_type: b"fixture_plane".to_vec(),
        name: None,
        accidents: None,
        experience: zero(),
        equipment: content_reader_save::air::EquipmentPool {
            entries: Vec::new(),
            allow_zero: false,
        },
        ace: None,
        raid: None,
    });
    saved.air_wings.insert(
        id,
        content_reader_save::AirWing {
            government_in_exile_tag: None,
            manpower: Some(40),
            reinforcement_setting: Some(0),
            id,
            owner: b"GER".to_vec(),
            name: Some(Vec::new()),
            country_override: b"---".to_vec(),
            air_base: 0,
            planes: 1,
            mission_mask: 0x800,
            mission_region: None,
            mission_active: None,
            stop_training_at_max_xp: true,
            region_change_penalty: None,
            transfer_progress: None,
            transferring_to: None,
            pool_ordinal: 0,
        },
    );
    for (flags, differences, recorded) in [
        (None, 0, false),
        (Some(0), 0, false),
        (Some(1), 1, true),
        (Some(3), 0, true),
        (Some(15), 0, true),
    ] {
        set(
            &mut frame,
            TableId::AirWings,
            TableRows::AirWings(vec![AirWingRow {
                id,
                owner: 101,
                planes: 1,
                mission_mask: 0x800,
                name: Recorded::Value(Some(Vec::new())),
                map_flags: flags.into(),
                acquired_manpower: Some(40).into(),
                exile_country: Some(0).into(),
                replacement_setting: Some(0).into(),
                ..Default::default()
            }]),
        );
        let mut report = Report::default();
        compare_air_wings(&frame, &saved, &links, &mut report).unwrap();
        assert_eq!(
            report.differences.len(),
            differences,
            "{:?}",
            report.differences
        );
        assert!(
            report
                .differences
                .iter()
                .all(|difference| difference.contains("stop_training_at_max_xp"))
        );
        assert_eq!(
            report.compared_tables.get("AirWings.training_stop").copied(),
            recorded.then_some(1),
            "{flags:?}"
        );
        assert_eq!(
            report
                .limitations
                .get(TRAINING_STOP_NOT_RECORDED)
                .map(Vec::len),
            (!recorded).then_some(1),
            "{flags:?}"
        );
    }
}

#[test]
fn acclimatization_categories_compare_their_exact_bytes() {
    let weight = content_reader_save::Decimal::parse(b"0.5").unwrap();
    let tokens = BTreeMap::from([(1, b"\x80".as_slice())]);
    let row = replay_data::rows::DivisionStatusRow {
        acclimatization_maximum: Some(content_reader_save::Acclimatization::OMITTED_MAXIMUM_RAW),
        acclimatization: vec![replay_data::rows::DivisionAcclimatization {
            category: 1,
            weight: weight.raw,
        }],
        ..Default::default()
    };
    let saved = |name: u8| content_reader_save::Acclimatization {
        entries: vec![(vec![name], weight.clone())],
        maximum: None,
    };
    let mut report = Report::default();
    compare_acclimatization("division", &row, Some(&saved(0x80)), &tokens, &mut report);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    compare_acclimatization("division", &row, Some(&saved(0x81)), &tokens, &mut report);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
}

#[test]
fn division_status_accepts_always_exact_supply_at_zero_days_and_historical_absence() {
    use content_reader_save::Decimal;
    use replay_data::rows::{DivisionStatusRow, Recorded, TableId, TableRows};
    let (frame, mut saved, _, _) = super::super::fixtures::fixture();
    let id = (7_u64 << 32) | 51;
    saved.divisions.insert(
        id,
        content_reader_save::Division {
            id,
            roster_owner: b"GER".to_vec(),
            logical_owner: b"GER".to_vec(),
            expeditionary_owner: None,
            province: None,
            template: None,
            alternate_template: None,
            separate_counter: false,
            hit_points: zero(),
            organisation: zero(),
            country_intel: Vec::new(),
            presentation: content_reader_save::DivisionPresentation {
                max_supply: Some(Decimal::parse(b"72").unwrap()),
                army_current_supply_ratio: Some(Decimal::parse(b"36.35999").unwrap()),
                ..Default::default()
            },
        },
    );
    let base = DivisionStatusRow {
        division: id,
        officer_experience: Some(0).into(),
        career_counter: Some(0).into(),
        experience: Recorded::NotRecorded,
        pending_template: Recorded::NotRecorded,
        out_of_supply_days: Recorded::Value(Some(0)),
        supply: Recorded::NotRecorded,
        supply_grace_maximum: Recorded::Value(7_200_000),
        supply_grace_remaining: Recorded::Value(3_600_000),
        supply_ratio: None,
        supply_ratio_exact: Recorded::Value(Some(35_499)),
        supply_grace_exact: Recorded::Value(Some(3_635_999)),
        ..Default::default()
    };
    let run = |row: DivisionStatusRow| {
        let mut frame = frame.clone();
        set(&mut frame, TableId::DivisionStatus, TableRows::DivisionStatus(vec![row]));
        let mut report = Report::default();
        compare_division_status(&frame, &saved, &BTreeMap::new(), &Ok(50_000), &Ok(72), &mut report)
            .unwrap();
        report
    };
    let report = run(base.clone());
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    let wrong_grace = run(DivisionStatusRow {
        supply_grace_exact: Recorded::Value(Some(3_636_000)),
        ..base.clone()
    });
    assert_eq!(wrong_grace.differences.len(), 1, "{:?}", wrong_grace.differences);
    assert!(wrong_grace.differences[0].contains("supply_exact.grace_remaining"));
    let duplicated = run(DivisionStatusRow {
        supply_ratio: Some(35_000),
        ..base.clone()
    });
    assert_eq!(duplicated.differences.len(), 1, "{:?}", duplicated.differences);
    assert!(duplicated.differences[0].contains("supply_exact shown"));
    let historical = run(DivisionStatusRow {
        supply_ratio: Some(35_000),
        supply_ratio_exact: Recorded::Value(None),
        supply_grace_exact: Recorded::Value(None),
        ..base.clone()
    });
    assert!(historical.differences.is_empty(), "{:?}", historical.differences);
    let partial = run(DivisionStatusRow {
        supply_grace_exact: Recorded::Value(None),
        ..base.clone()
    });
    assert_eq!(partial.differences.len(), 1, "{:?}", partial.differences);
    assert!(partial.differences[0].contains("supply_exact"));
    let missing_all = run(DivisionStatusRow {
        supply_ratio: None,
        supply_ratio_exact: Recorded::Value(None),
        supply_grace_exact: Recorded::Value(None),
        ..base.clone()
    });
    assert_eq!(missing_all.differences.len(), 1, "{:?}", missing_all.differences);
    assert!(missing_all.differences[0].contains("supply_exact"));
    let half_recorded = run(DivisionStatusRow {
        supply_ratio_exact: Recorded::NotRecorded,
        ..base.clone()
    });
    assert_eq!(half_recorded.differences.len(), 1, "{:?}", half_recorded.differences);
    assert!(half_recorded.differences[0].contains("supply_exact"));
    let pre_format = run(DivisionStatusRow {
        supply_ratio: Some(35_000),
        supply_ratio_exact: Recorded::NotRecorded,
        supply_grace_exact: Recorded::NotRecorded,
        ..base.clone()
    });
    assert!(pre_format.differences.is_empty(), "{:?}", pre_format.differences);
    assert!(
        pre_format
            .limitations
            .keys()
            .any(|reason| reason.contains("DivisionStatus.supply_ratio_exact")),
        "{:?}",
        pre_format.limitations
    );
    let pre_format_missing_legacy = run(DivisionStatusRow {
        supply_ratio: None,
        supply_ratio_exact: Recorded::NotRecorded,
        supply_grace_exact: Recorded::NotRecorded,
        ..base
    });
    assert_eq!(
        pre_format_missing_legacy.differences.len(),
        1,
        "{:?}",
        pre_format_missing_legacy.differences
    );
    assert!(pre_format_missing_legacy.differences[0].contains("supply_ratio"));
}
