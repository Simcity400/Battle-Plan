use super::*;
use std::cell::Cell;
use viewer_gpu::unit_pose::test_support::defines;

fn anchors(province: u32) -> Result<[f32; 3], String> {
    Ok(match province {
        1 => [10.0, 3.0, 0.0],
        2 => [20.0, 3.0, 0.0],
        3 => [30.0, 3.0, 0.0],
        7 => [995.0, 0.0, 4.0],
        8 => [5.0, 0.0, 4.0],
        _ => return Err(format!("province {province} has no anchor")),
    })
}

#[test]
fn path_points_add_handles_anchors_and_admitted_crossings() {
    let crossing = |from: u32, to: u32| match (from, to) {
        (1, 2) => Some(Some([15, 1])),
        (2, 3) | (1, 3) | (7, 8) => Some(None),
        _ => None,
    };
    let points = path_points(&[1, 2, 3], 1000.0, true, &anchors, &crossing).unwrap();
    assert_eq!(
        points,
        [
            [7.5, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [15.0, 0.0, 1.0],
            [20.0, 0.0, 0.0],
            [30.0, 0.0, 0.0],
            [32.5, 0.0, 0.0]
        ]
    );
    let plain = path_points(&[1, 2, 3], 1000.0, false, &anchors, &crossing).unwrap();
    assert_eq!(plain.len(), 5);
    let skipped = |from: u32, to: u32| match (from, to) {
        (1, 3) => Some(None),
        _ => None,
    };
    let points = path_points(&[1, 2, 3], 1000.0, true, &anchors, &skipped).unwrap();
    assert_eq!(
        points,
        [[10.0, 0.0, 0.0], [30.0, 0.0, 0.0], [35.0, 0.0, 0.0]]
    );
    let seam = path_points(&[7, 8], 1000.0, true, &anchors, &crossing).unwrap();
    assert_eq!(
        seam,
        [
            [992.5, 0.0, 4.0],
            [995.0, 0.0, 4.0],
            [1005.0, 0.0, 4.0],
            [1007.5, 0.0, 4.0]
        ]
    );
}

fn arrow_type() -> ArrowType {
    ArrowType {
        body_width: [3.0, 8.0],
        ..Default::default()
    }
}

#[test]
fn the_path_spline_keeps_the_ends_and_samples_each_inner_segment() {
    let points = vec![
        [0.0, 0.0, 0.0],
        [10.0, 0.0, 0.0],
        [20.0, 0.0, 0.0],
        [30.0, 0.0, 0.0],
    ];
    let spline = defines().naval;
    let controls = spline_controls(points.clone(), spline, &arrow_type()).unwrap();
    assert_eq!(
        controls,
        [[0, 0], [1_000_000, 0], [1_500_000, 0], [3_000_000, 0]]
    );
    let bare = PathArrowDefines {
        subdivisions: 0,
        ..spline
    };
    assert_eq!(
        spline_controls(points, bare, &arrow_type()).unwrap(),
        [[0, 0], [3_000_000, 0]]
    );
    assert_eq!(spline_controls(Vec::new(), spline, &arrow_type()), None);
}

fn speed_defines() -> SpeedDefines {
    SpeedDefines {
        land_speed_modifier: 50_000,
        naval_speed_modifier: 50_000,
        naval_transfer_base_speed: 1_000_000,
        raid_unit_speed_multiplier: 10_000,
        ace_wing_size: 10_000_000,
        ace_wing_size_max_bonus: 150_000,
    }
}

#[test]
fn the_raid_unit_speed_takes_each_native_branch() {
    let defines = speed_defines();
    let none = SpeedInputs::default();
    assert_eq!(
        raid_unit_speed(SpeedBranch::Convoys, &none, 150_000, &defines),
        75_000
    );
    let transports = SpeedInputs {
        transport_speeds: vec![300_000, 500_000, 500_000, 400_000],
        ..SpeedInputs::default()
    };
    let fallback = SpeedInputs {
        transport_fallback: 1,
        ..transports.clone()
    };
    assert_eq!(
        raid_unit_speed(SpeedBranch::TransportPlanes, &fallback, 100_000, &defines),
        10_000
    );
    assert_eq!(
        raid_unit_speed(SpeedBranch::TransportPlanes, &transports, 100_000, &defines),
        25_000
    );
    assert_eq!(
        raid_unit_speed(SpeedBranch::TransportPlanes, &none, 100_000, &defines),
        10_000
    );
    let wing = SpeedInputs {
        wing_max_speed: Some(600_000),
        ace_alive: Some(1),
        ace_speed: 10_000,
        ace_type_speed: 1,
        ace_wing_planes: Some(50),
        ace_effectiveness: 20_000,
        ace_bonuses: 5_000,
        ..SpeedInputs::default()
    };
    assert_eq!(
        raid_unit_speed(SpeedBranch::Unit, &wing, 100_000, &defines),
        36_900
    );
    let dead = SpeedInputs {
        ace_alive: Some(0),
        ..wing.clone()
    };
    assert_eq!(
        raid_unit_speed(SpeedBranch::Unit, &dead, 100_000, &defines),
        30_000
    );
    let army = SpeedInputs {
        army_max_speed: Some(400_000),
        ..SpeedInputs::default()
    };
    assert_eq!(
        raid_unit_speed(SpeedBranch::Unit, &army, 100_000, &defines),
        20_000
    );
    assert_eq!(
        raid_unit_speed(SpeedBranch::Unit, &none, 100_000, &defines),
        0
    );
    assert_eq!(ace_wing_scale(0, 10_000_000, 150_000), 100_000);
    assert_eq!(ace_wing_scale(200, 10_000_000, 150_000), 50_000);
}

#[test]
fn the_hours_floor_elapsed_and_ceil_remaining() {
    assert_eq!(elapsed_hours(0, 30_000), 0);
    assert_eq!(remaining_hours(100_000, 0, 30_000), 4);
    assert_eq!(phase_progress(0, 4), 0);
    assert_eq!(next_hour_progress(0, 4), 25_000);
    assert_eq!(elapsed_hours(60_000, 30_000), 2);
    assert_eq!(remaining_hours(100_000, 60_000, 30_000), 2);
    assert_eq!(phase_progress(2, 2), 50_000);
    assert_eq!(next_hour_progress(2, 2), 75_000);
    assert_eq!(remaining_hours(100_000, 120_000, 30_000), 0);
    assert_eq!(elapsed_hours(60_000, 0), 42_949);
    assert_eq!(remaining_hours(100_000, 60_000, 0), 42_950);
    assert_eq!(phase_progress(0, 0), 0);
    assert_eq!(next_hour_progress(0, 0), 0);
}

fn speed_frame(rows: Option<Vec<replay_data::rows::RaidSpeedInputRow>>) -> DecodedFrame {
    use replay_data::{rows::TableId, rows::TableRows};
    use replay_frame::{frame::Metrics, frame::TableView};
    let mut views = std::array::from_fn(|_| TableView::Unavailable {
        reason: "withheld by the fixed-key contract".into(),
    });
    if let Some(rows) = rows {
        views[TableId::RaidSpeedInputs.index()] =
            TableView::Complete(TableRows::RaidSpeedInputs(rows));
    }
    DecodedFrame {
        branch: 1,
        hour: 1,
        tables: views.map(Into::into),
        metrics: Metrics::default(),
    }
}

#[test]
fn plane_inputs_read_the_travelled_distance_and_the_raids_speed_row() {
    let raid = RaidRow {
        id: 9,
        reference_type: 84,
        phase: 4,
        travelled: Some(60_000),
        ..RaidRow::default()
    };
    let row = replay_data::rows::RaidSpeedInputRow {
        id: 9,
        reference_type: 84,
        phase: 4,
        wing_max_speed: Some(600_000),
        ace_present: 1,
        ace_alive: 0,
        ace_speed: 0,
        ace_wing_planes: None,
        ..Default::default()
    };
    let (travelled, inputs) = plane_inputs(&speed_frame(Some(vec![row.clone()])), &raid).unwrap();
    assert_eq!(travelled, 60_000);
    assert_eq!(inputs.wing_max_speed, Some(600_000));
    assert_eq!(inputs.ace_alive, Some(0));
    let absent = replay_data::rows::RaidSpeedInputRow {
        ace_present: 0,
        ..row.clone()
    };
    assert_eq!(
        SpeedInputs::from_row(&absent, &speed_frame(None))
            .unwrap()
            .ace_alive,
        None
    );
    let old = RaidRow {
        travelled: None,
        ..raid.clone()
    };
    assert!(plane_inputs(&speed_frame(Some(vec![row])), &old).is_err());
    let withheld = plane_inputs(&speed_frame(None), &raid).unwrap_err();
    assert!(withheld.contains("fixed-key contract"), "{withheld}");
    assert!(plane_inputs(&speed_frame(Some(Vec::new())), &raid).is_err());
}

#[test]
fn a_living_aces_country_terms_join_country_command_power() {
    use replay_data::{rows::CountryCommandPowerRow, rows::TableId, rows::TableRows};
    use replay_frame::frame::TableView;
    let row = replay_data::rows::RaidSpeedInputRow {
        id: 9,
        reference_type: 84,
        phase: 4,
        wing_max_speed: Some(600_000),
        ace_present: 1,
        ace_alive: 1,
        ace_speed: 10_000,
        ace_owner: Some(3),
        wing_country: Some(1),
        ..Default::default()
    };
    let mut frame = speed_frame(Some(vec![row.clone()]));
    let power = |frame: &mut DecodedFrame, rows: Vec<CountryCommandPowerRow>| {
        *frame.tables[TableId::CountryCommandPower.index()] =
            TableView::Complete(TableRows::CountryCommandPower(rows));
    };
    let withheld = SpeedInputs::from_row(&row, &frame).unwrap_err();
    assert!(withheld.contains("fixed-key contract"), "{withheld}");
    power(
        &mut frame,
        vec![
            CountryCommandPowerRow {
                canonical_slot: 1,
                ace_effectiveness_factor: Some(1),
                air_ace_bonuses_factor: Some(5_000),
                ..Default::default()
            },
            CountryCommandPowerRow {
                canonical_slot: 3,
                ace_effectiveness_factor: Some(20_000),
                air_ace_bonuses_factor: Some(2),
                ..Default::default()
            },
        ],
    );
    let inputs = SpeedInputs::from_row(&row, &frame).unwrap();
    assert_eq!(
        (inputs.ace_effectiveness, inputs.ace_bonuses),
        (20_000, 5_000)
    );
    let dead = replay_data::rows::RaidSpeedInputRow {
        ace_alive: 0,
        ace_owner: None,
        wing_country: None,
        ..row.clone()
    };
    power(&mut frame, Vec::new());
    let inputs = SpeedInputs::from_row(&dead, &frame).unwrap();
    assert_eq!((inputs.ace_effectiveness, inputs.ace_bonuses), (0, 0));
    assert!(SpeedInputs::from_row(&row, &frame).is_err());
}

#[test]
fn an_ended_raids_unit_leaves_with_its_first_phase_5_update() {
    assert!(unit_shown(4, 0, 0.0));
    assert!((movement_progress(4, 0.5, 0.75, 0.5) - 0.625).abs() < 1e-6);
    assert_eq!(movement_progress(5, 0.5, 0.75, 0.3), 1.0);
    assert!(!unit_shown(5, 1, movement_progress(5, 0.5, 0.75, 0.3)));
    assert!(unit_shown(5, 1, 0.998));
    assert!(!unit_shown(5, 1, 0.999));
    assert!(!unit_shown(5, 5, 0.5));
    assert!(!unit_shown(3, 0, 0.0));
}

#[test]
fn a_wing_ace_with_an_unresolved_wing_scales_by_one() {
    let defines = speed_defines();
    let inputs = SpeedInputs {
        wing_max_speed: Some(600_000),
        ace_alive: Some(1),
        ace_speed: 10_000,
        ace_type_speed: 0,
        ace_wing_planes: None,
        ace_bonuses: 99_999,
        ..SpeedInputs::default()
    };
    assert_eq!(
        raid_unit_speed(SpeedBranch::Unit, &inputs, 100_000, &defines),
        33_000
    );
}

#[test]
fn the_speed_branch_follows_the_essential_categories() {
    let kind = |equipment: &[&str]| RaidType {
        essential_equipment: equipment.iter().map(|name| (name.to_string(), 1)).collect(),
        ..RaidType::default()
    };
    let category = |name: &str| match name {
        "transport_plane" => Ok(0x10_0000),
        "convoy" => Ok(1),
        "infantry_equipment" => Ok(0x40),
        _ => Err(format!("{name} is unknown")),
    };
    assert_eq!(
        SpeedBranch::of(&kind(&["convoy", "transport_plane"]), &category),
        Ok(SpeedBranch::TransportPlanes)
    );
    assert_eq!(
        SpeedBranch::of(&kind(&["infantry_equipment", "convoy"]), &category),
        Ok(SpeedBranch::Convoys)
    );
    assert_eq!(
        SpeedBranch::of(&kind(&["infantry_equipment"]), &category),
        Ok(SpeedBranch::Unit)
    );
    assert!(SpeedBranch::of(&kind(&["zeppelin"]), &category).is_err());
}

#[test]
fn the_callback_height_samples_the_fixed_point_bilinearly() {
    let values = [10u8, 20, 30, 40];
    let field = HeightField {
        width: 2,
        height: 2,
        values: &values,
    };
    assert_eq!(field.fixed_height(0, 0, 2), 3.0);
    assert!((field.fixed_height(50_000, 0, 2) - 3.5).abs() < 1e-6);
    assert!((field.fixed_height(50_000, 50_000, 2) - 2.5).abs() < 1e-6);
    assert_eq!(field.fixed_height(-100_000, 0, 2), 4.0);
}

#[test]
fn the_pool_meets_both_lists_in_whole_units_and_nukes() {
    let kind = RaidType {
        essential_equipment: vec![("transport_plane".into(), 5)],
        additional_equipment: vec![("infantry_equipment".into(), 100)],
        nukes: 0,
        additional_nukes: 1,
        ..RaidType::default()
    };
    let token = |token: u32| -> Result<String, String> {
        Ok(match token {
            40 => "transport_plane",
            41 => "infantry_equipment",
            _ => return Err(format!("archetype token {token} is unknown")),
        }
        .to_owned())
    };
    let pool = |transports: i64, nukes: i32| RaidPool {
        archetype_totals: vec![(40, transports), (41, 10_000_000)],
        nukes,
    };
    assert_eq!(pool_meets(&kind, &pool(500_000, 1), &token), Ok(true));
    assert_eq!(pool_meets(&kind, &pool(499_999, 1), &token), Ok(false));
    assert_eq!(pool_meets(&kind, &pool(400_000, 1), &token), Ok(false));
    assert_eq!(pool_meets(&kind, &pool(500_000, 0), &token), Ok(false));
    let essential_nuke = RaidType {
        nukes: 2,
        ..kind.clone()
    };
    assert_eq!(
        pool_meets(&essential_nuke, &pool(500_000, 1), &token),
        Ok(false)
    );
    assert_eq!(
        pool_meets(&essential_nuke, &pool(500_000, 2), &token),
        Ok(true)
    );
    let missing = RaidPool {
        archetype_totals: vec![(41, 10_000_000)],
        nukes: 1,
    };
    assert_eq!(pool_meets(&kind, &missing, &token), Ok(false));
    let twice = RaidPool {
        archetype_totals: vec![(40, 500_000), (41, 10_000_000), (40, 0)],
        nukes: 1,
    };
    assert_eq!(pool_meets(&kind, &twice, &token), Ok(true));
    let odd = RaidPool {
        archetype_totals: vec![(9, 1)],
        nukes: 1,
    };
    assert!(pool_meets(&kind, &odd, &token).is_err());
}

fn pool_frame(rows: Option<Vec<replay_data::rows::RaidEquipmentRow>>) -> DecodedFrame {
    use replay_data::{rows::TableId, rows::TableRows};
    use replay_frame::{frame::Metrics, frame::TableView};
    let mut views = std::array::from_fn(|_| TableView::Unavailable {
        reason: "withheld by the fixed-key contract".into(),
    });
    if let Some(rows) = rows {
        views[TableId::RaidEquipment.index()] = TableView::Complete(TableRows::RaidEquipment(rows));
    }
    DecodedFrame {
        branch: 1,
        hour: 1,
        tables: views.map(Into::into),
        metrics: Metrics::default(),
    }
}

#[test]
fn the_pool_is_the_raids_recorded_row_or_refused_by_name() {
    let raid = RaidRow {
        id: 7,
        reference_type: 3,
        phase: 3,
        ..RaidRow::default()
    };
    let row = replay_data::rows::RaidEquipmentRow {
        id: 7,
        reference_type: 3,
        phase: 3,
        end_hour: None,
        variants: vec![0x0000_0005_0000_0001, 0],
        amounts: vec![900_000, 100_000],
        nukes: 1,
        archetype_tokens: vec![40, 41],
        archetype_amounts: vec![500_000, 0],
    };
    assert_eq!(
        raid_pool(&pool_frame(Some(vec![row.clone()])), &raid),
        Ok(RaidPool {
            archetype_totals: vec![(40, 500_000), (41, 0)],
            nukes: 1,
        })
    );
    let other = RaidRow {
        id: 8,
        ..raid.clone()
    };
    assert!(
        raid_pool(&pool_frame(Some(vec![row.clone()])), &other)
            .unwrap_err()
            .contains("no RaidEquipment row")
    );
    assert!(
        raid_pool(&pool_frame(None), &raid)
            .unwrap_err()
            .contains("withheld")
    );
    let torn = replay_data::rows::RaidEquipmentRow {
        archetype_amounts: vec![1],
        ..row
    };
    assert!(
        raid_pool(&pool_frame(Some(vec![torn])), &raid)
            .unwrap_err()
            .contains("2 archetype tokens and 1 archetype amounts")
    );
}

#[test]
fn the_unit_entity_follows_the_native_ladder() {
    content_reader_script::test_game::enter();
    use content_reader_raids::raids::RaidUnitModel;
    let declared = |name: &str| {
        [
            "ENG_truck_entity",
            "western_european_gfx_ship_entity",
            "ship_entity",
            "fallback_entity",
        ]
        .contains(&name)
    };
    let model =
        |entity: Option<&str>, default: Option<&str>, equipment: Option<&str>| RaidUnitModel {
            entity: entity.map(str::to_owned),
            default_entity: default.map(str::to_owned),
            equipment: equipment
                .and_then(content_reader_script::equipment_stats::category_mask)
                .unwrap_or(0),
            ..RaidUnitModel::default()
        };
    let choose = |model: &RaidUnitModel, primary: &[(u64, u64)], wing| {
        unit_entity(
            model,
            "ENG",
            "western_european_gfx",
            primary,
            wing,
            &declared,
        )
    };
    assert_eq!(
        choose(&model(Some("truck_entity"), None, None), &[], None),
        UnitEntity::Entity("ENG_truck_entity".into())
    );
    assert_eq!(
        choose(&model(Some("ship_entity"), None, None), &[], None),
        UnitEntity::Entity("western_european_gfx_ship_entity".into())
    );
    assert_eq!(
        choose(
            &model(Some("missing_entity"), Some("fallback_entity"), None),
            &[],
            None
        ),
        UnitEntity::Entity("fallback_entity".into())
    );
    let category = content_reader_script::test_game::category;
    let primary = [
        (10, category("capital_ship")),
        (11, category("air_transport")),
        (12, category("air_transport")),
    ];
    assert_eq!(
        choose(
            &model(Some("ship_entity"), None, Some("air_transport")),
            &primary,
            None
        ),
        UnitEntity::Designer(11)
    );
    assert_eq!(
        choose(
            &model(Some("ship_entity"), None, Some("air_transport")),
            &primary[..1],
            None
        ),
        UnitEntity::Entity("western_european_gfx_ship_entity".into())
    );
    assert_eq!(
        choose(&model(None, None, None), &[], Some(7)),
        UnitEntity::WingEquipment(7)
    );
    assert_eq!(
        choose(&model(None, None, None), &[], None),
        UnitEntity::None
    );
    assert_eq!(
        choose(&model(None, None, Some("zeppelins")), &[], None),
        UnitEntity::None
    );
}

#[test]
fn the_designer_model_follows_resolve_aircraft_designer_model_with_and_without_the_dlc() {
    content_reader_script::test_game::enter();
    let (content, _) = content_reader_key_names::equipment::fixture::load(&[(
        "common/units/equipment/planes.txt",
        "equipments = {
            transport_plane_equipment = { is_archetype = yes sprite = transport_plane }
            transport_plane_equipment_1 = { archetype = transport_plane_equipment }
        }",
    )]);
    let variant = |saved: &str| replay_data::rows::VariantDefinitionRow {
        id: 5,
        explicit_entity: saved.as_bytes().to_vec(),
        ..Default::default()
    };
    let declared = |name: &str| {
        [
            "ENG_transport_plane_entity",
            "designer_c47_entity",
            "pool_c47_entity",
        ]
        .contains(&name)
    };
    let resolve = |row: &replay_data::rows::VariantDefinitionRow,
                   dlc: u32,
                   pools: &dyn Fn() -> Result<Option<String>, String>| {
        designer_entity(
            row,
            "transport_plane_equipment_1",
            &content,
            "ENG",
            dlc,
            pools,
            &declared,
        )
    };
    let pool = || Ok(Some("pool_c47_entity".to_owned()));
    let undeclared_pool = || Ok(Some("missing_entity".to_owned()));
    let none = || Ok(None);
    let asked = Cell::new(false);
    let untouched = || {
        asked.set(true);
        Ok(None)
    };
    assert_eq!(
        resolve(&variant("designer_c47_entity"), 0x100, &pool),
        Ok(Some("pool_c47_entity".into()))
    );
    assert_eq!(
        resolve(&variant("designer_c47_entity"), 0x100, &undeclared_pool),
        Ok(Some("designer_c47_entity".into()))
    );
    assert_eq!(
        resolve(&variant(""), 0x100, &none),
        Ok(Some("ENG_transport_plane_entity".into()))
    );
    assert_eq!(
        resolve(&variant("designer_c47_entity"), 0, &untouched),
        Ok(Some("designer_c47_entity".into()))
    );
    assert_eq!(
        resolve(&variant(""), 0, &untouched),
        Ok(Some("ENG_transport_plane_entity".into()))
    );
    assert!(!asked.get());
    let failing = || Err("a stored value is missing".to_owned());
    assert!(resolve(&variant(""), 0x100, &failing).is_err());
    let undeclared = |_: &str| false;
    assert_eq!(
        designer_entity(
            &variant(""),
            "transport_plane_equipment_1",
            &content,
            "ENG",
            0,
            &none,
            &undeclared
        ),
        Ok(None)
    );
    assert_eq!(
        designer_entity(&variant(""), "glider", &content, "ENG", 0, &none, &declared),
        Err("equipment glider is not in the content".to_owned())
    );
}

#[test]
fn a_building_source_reads_the_recorded_building_arm() {
    let mut raid = RaidRow::default();
    raid.source.building_type = NO_BUILDING_TYPE;
    raid.source.building_location = 10364;
    assert!(building_source(&raid).is_err());
    raid.source.building_type = 77;
    raid.source.building_location = -1;
    assert!(building_source(&raid).is_err());
    raid.source.building_location = 7301;
    raid.source.building_country = -1;
    assert_eq!(building_source(&raid), Ok((77, 7301, -1)));
}
