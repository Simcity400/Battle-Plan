use super::*;

fn defines() -> RaidGraphicsDefines {
    RaidGraphicsDefines {
        preparing_color: [0.7, 0.7, 0.7, 1.0],
        ready_color: [0.7, 0.7, 0.0, 0.9],
        active_color: [1.0, 0.0, 0.0, 0.9],
        ballistic_shape: 0.02,
        ballistic_max_height: 30.0,
        ballistic_max_segment_length: 1.0,
        ballistic_max_segments: 100,
        air_height: 10.0,
        air_source_steepness: 0.2,
        air_source_length: 50.0,
        air_target_steepness: 0.2,
        air_target_length: 0.0,
        air_max_segment_length: 1.0,
        air_max_segments: 100,
        naval: PathArrowDefines {
            subdivisions: 2,
            sharp_turn_smoothness: 0.0,
            use_midpoints: true,
        },
        land: PathArrowDefines {
            subdivisions: 20,
            sharp_turn_smoothness: 0.1,
            use_midpoints: true,
        },
        unit_base_scale: 2.0,
        unit_offset: [0.0; 3],
        speed_multiplier_raw: 10_000,
    }
}

#[test]
fn the_colour_is_active_in_flight_preparing_before_and_undecided_when_prepared() {
    let defines = defines();
    assert_eq!(color(4, &defines), Some(defines.active_color));
    assert_eq!(color(5, &defines), Some(defines.active_color));
    assert_eq!(color(1, &defines), Some(defines.preparing_color));
    assert_eq!(color(2, &defines), Some(defines.preparing_color));
    // READY is decided only in phase 3, from execution inputs absent
    // from this fixture.
    assert_eq!(color(3, &defines), None);
}

#[test]
fn a_straight_arrow_wraps_its_ends_and_cuts_ceil_length_segments() {
    // A line is its two ends.
    let line = straight_controls([10.0, 5.0, 10.0], [20.0, 5.0, 10.0], 1000.0, None);
    assert_eq!(line, [[1_000_000, 1_000_000], [2_000_000, 1_000_000]]);
    // Ten world units at one per segment: ten segments, eleven controls.
    let air = straight_controls(
        [10.0, 5.0, 10.0],
        [20.0, 5.0, 10.0],
        1000.0,
        Some((1.0, 100)),
    );
    assert_eq!(air.len(), 11);
    assert_eq!(air[1], [1_100_000, 1_000_000]);
    // The count is capped, and a non-positive length takes the cap.
    let far = straight_controls([0.0; 3], [400.0, 0.0, 0.0], 1000.0, Some((1.0, 100)));
    assert_eq!(far.len(), 101);
    let capped = straight_controls([0.0; 3], [4.0, 0.0, 0.0], 1000.0, Some((0.0, 7)));
    assert_eq!(capped.len(), 8);
    // Across the seam the source moves a map width east.
    let seam = straight_controls([2.0, 0.0, 0.0], [998.0, 0.0, 0.0], 1000.0, None);
    assert_eq!(seam, [[100_200_000, 0], [99_800_000, 0]]);
}

#[test]
fn a_float_point_rounds_half_away_from_zero() {
    assert_eq!(fixed([0.000005, 0.0, -0.000005]), [1, -1]);
    assert_eq!(fixed([12.345_67, 0.0, -1.0]), [1_234_567, -100_000]);
}

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
    // Without midpoints the crossing is left out.
    let plain = path_points(&[1, 2, 3], 1000.0, false, &anchors, &crossing).unwrap();
    assert_eq!(plain.len(), 5);
    // A hop that is not a neighbour is skipped, and the first hop's handle
    // goes with it; the next hop is tried from the same province.
    let skipped = |from: u32, to: u32| match (from, to) {
        (1, 3) => Some(None),
        _ => None,
    };
    let points = path_points(&[1, 2, 3], 1000.0, true, &anchors, &skipped).unwrap();
    assert_eq!(
        points,
        [[10.0, 0.0, 0.0], [30.0, 0.0, 0.0], [35.0, 0.0, 0.0]]
    );
    // Across the seam each point follows the one before.
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

#[test]
fn a_sharp_turn_gets_two_pushed_points_and_a_straight_run_none() {
    let straight = vec![
        [0.0, 0.0, 0.0],
        [10.0, 0.0, 0.0],
        [20.0, 0.0, 0.0],
        [30.0, 0.0, 0.0],
    ];
    let mut points = straight.clone();
    smooth_sharp_turns(&mut points, 1.0);
    assert_eq!(points, straight);
    let mut turn = vec![
        [0.0, 0.0, 0.0],
        [10.0, 0.0, 0.0],
        [20.0, 0.0, 0.0],
        [10.0, 0.0, 1.0],
        [0.0, 0.0, 2.0],
    ];
    smooth_sharp_turns(&mut turn, 1.0);
    assert_eq!(turn.len(), 7);
    // A quarter of the incoming segment back, pushed three units off the
    // turn's side.
    assert_eq!(turn[2], [17.5, 0.0, -3.0]);
    assert_eq!(turn[3], [20.0, 0.0, 0.0]);
}

#[test]
fn a_revisited_point_is_pushed_apart_at_both_visits() {
    // A square loop back through (20, 0), every turn a right angle.
    let mut points = vec![
        [0.0, 0.0, 0.0],
        [10.0, 0.0, 0.0],
        [20.0, 0.0, 0.0],
        [20.0, 0.0, 10.0],
        [30.0, 0.0, 10.0],
        [30.0, 0.0, 0.0],
        [20.0, 0.0, 0.0],
        [20.0, 0.0, -10.0],
        [20.0, 0.0, -20.0],
    ];
    smooth_sharp_turns(&mut points, 1.0);
    assert_eq!(points.len(), 9);
    // The first visit moves across its outgoing leg (+z), side (-z, y, x).
    assert_eq!(points[2], [19.0, 0.0, 0.0]);
    // The revisit moves across the leg after it (-z).
    assert_eq!(points[6], [21.0, 0.0, 0.0]);
}

fn arrow_type() -> ArrowType {
    let mut arrow = ArrowType::default();
    arrow.body_width = [3.0, 8.0];
    arrow
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
    // No subdivisions: the ends only.
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

#[test]
fn clamped_lerp_is_exact_at_its_ends() {
    assert_eq!(clamped_lerp(1.0, 1.0, 0.3), 1.0);
    assert_eq!(clamped_lerp(0.2, 0.9, 1.0), 0.9);
    assert_eq!(clamped_lerp(0.0, 1.0, 0.5), 0.5);
    assert!((clamped_lerp(0.3, 1.0, 0.5) - 0.65).abs() < 1e-6);
}

#[test]
fn the_air_arrow_climbs_from_the_source_to_its_cruise_height() {
    let defines = defines();
    let ends = Ends {
        distance: 100.0,
        first: 10.0,
        last: 12.0,
    };
    // At the source: the terrain.
    assert_eq!(air_height(ends, 0, 10, &defines), 10.0);
    // Past the source slope: the cruise height over the higher end.
    assert_eq!(air_height(ends, 6, 10, &defines), 22.0);
    // A target length of zero: level to the end.
    assert_eq!(air_height(ends, 10, 10, &defines), 22.0);
    assert_eq!(air_height(ends, 3, 0, &defines), 0.0);
    // A short arrow's cruise shrinks with its length.
    let short = Ends {
        distance: 25.0,
        ..ends
    };
    assert_eq!(air_height(short, 10, 10, &defines), 17.0);
}

#[test]
fn the_ballistic_arc_is_refitted_to_its_maximum_height() {
    let mut defines = defines();
    let ends = Ends {
        distance: 100.0,
        first: 0.0,
        last: 0.0,
    };
    // Unrefitted, the apex would be 50: refitted to 30.
    assert!((ballistic_height(ends, 5, 10, &defines) - 30.0).abs() < 1e-3);
    assert!(ballistic_height(ends, 0, 10, &defines).abs() < 1e-6);
    assert!(ballistic_height(ends, 10, 10, &defines).abs() < 1e-3);
    defines.ballistic_max_height = 100.0;
    assert!((ballistic_height(ends, 5, 10, &defines) - 50.0).abs() < 1e-3);
    defines.ballistic_shape = 0.0;
    assert_eq!(ballistic_height(ends, 5, 10, &defines), 0.0);
}

#[test]
fn the_callbacks_measure_the_rounded_ends_across_the_seam() {
    let flat = |_: Point| 0.5;
    let ends = Ends::new(&[[0, 0], [100 * SCALE, 0]], 1000, &flat).unwrap();
    assert!((ends.distance - 100.0).abs() < 1e-2);
    let seam = Ends::new(&[[10 * SCALE, 0], [990 * SCALE, 0]], 1000, &flat).unwrap();
    assert!((seam.distance - 20.0).abs() < 1e-2);
    assert_eq!((seam.first, seam.last), (0.5, 0.5));
    assert_eq!(Ends::new(&[], 1000, &flat), None);
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
    // Convoys: NAVAL_SPEED_MODIFIER * NAVAL_TRANSFER_BASE_SPEED, then the
    // raid multiplier and the type's 1.5.
    assert_eq!(
        raid_unit_speed(SpeedBranch::Convoys, &none, 150_000, &defines),
        75_000
    );
    // Transport planes: the best stat, first on ties, times the land modifier.
    let transports = SpeedInputs {
        transport_speeds: vec![300_000, 500_000, 500_000, 400_000],
        ..SpeedInputs::default()
    };
    // The recorded fallback flag: 100000, unscaled.
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
    // None at all: 100000, unscaled by the land modifier.
    assert_eq!(
        raid_unit_speed(SpeedBranch::TransportPlanes, &none, 100_000, &defines),
        10_000
    );
    // A wing with a live ace: scale min(100 planes / 50, 1.5) = 1.5, so
    // 0.1 * 1.5 * 1.2 = 0.18, plus the country's 0.05 while the ace type
    // has the key: 1.23 * 6.0 * 0.5 * 0.1.
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
    // The wing wins over the army; the army alone; neither is still.
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
    // 1.0 map units at 0.3 an hour, at launch: 0 flown, 4 to go.
    assert_eq!(elapsed_hours(0, 30_000), 0);
    assert_eq!(remaining_hours(100_000, 0, 30_000), 4);
    assert_eq!(phase_progress(0, 4), 0);
    assert_eq!(next_hour_progress(0, 4), 25_000);
    // Two hours in.
    assert_eq!(elapsed_hours(60_000, 30_000), 2);
    assert_eq!(remaining_hours(100_000, 60_000, 30_000), 2);
    assert_eq!(phase_progress(2, 2), 50_000);
    assert_eq!(next_hour_progress(2, 2), 75_000);
    // Overflown: ceil(-0.67) is 0.
    assert_eq!(remaining_hours(100_000, 120_000, 30_000), 0);
    // A still unit: the 0xFFFFFFFF sentinel's floor and ceiling.
    assert_eq!(elapsed_hours(60_000, 0), 42_949);
    assert_eq!(remaining_hours(100_000, 60_000, 0), 42_950);
    assert_eq!(phase_progress(0, 0), 0);
    assert_eq!(next_hour_progress(0, 0), 0);
}

/// A flat heightfield wide enough for the tracks below.
static FLAT: [u8; 256] = [0; 256];
pub(super) fn flat() -> HeightField<'static> {
    HeightField {
        width: 128,
        height: 2,
        values: &FLAT,
    }
}

/// A line arrow's track over a 50-long straight arrow, from `travelled`
/// towards 100_000 at 30_000 an hour with the start `offset`.
pub(super) fn line_track(travelled: i64, offset: i64) -> UnitTrack {
    let controls = [[0, 0], [5_000_000, 0]];
    UnitTrack::new(
        &controls,
        RaidArrowKind::Line,
        &flat(),
        &defines(),
        travelled,
        100_000,
        30_000,
        offset,
    )
    .unwrap()
}

#[test]
fn the_unit_stands_at_its_start_offset_in_the_launch_hour() {
    // Launch hour: progress 0, so exactly the start offset's fraction.
    assert_eq!(line_track(0, 1_500_000).fraction(0.0), 0.3);
    // Two of four hours: halfway from the offset to the end.
    let half = line_track(60_000, 1_500_000).fraction(0.0);
    assert!((half - 0.65).abs() < 1e-6);
    // Half a tick further: progress 0.625.
    let ticked = line_track(60_000, 1_500_000).fraction(0.5);
    assert!((ticked - 0.7375).abs() < 1e-6);
    // An offset past the arrow's end starts at its end.
    assert_eq!(line_track(0, 9_000_000).fraction(0.0), 1.0);
    assert!((stored_length(&[[0, 0], [300_000, 400_000], [300_000, 0]]) - 9.0).abs() < 1e-6);
    let one = [[0, 0]];
    assert_eq!(
        UnitTrack::new(
            &one,
            RaidArrowKind::Line,
            &flat(),
            &defines(),
            0,
            100_000,
            30_000,
            0
        ),
        None
    );
}

#[test]
fn playback_glides_the_unit_by_the_tick_progress_within_the_hour() {
    // Two of four hours: progress 0.5 this hour, 0.75 the next.
    let track = line_track(60_000, 1_500_000);
    let heights = flat();
    let start = track.place(&heights, 0.0).unwrap();
    for tick in [0.25f32, 0.5, 0.75] {
        let pose = track.place(&heights, tick).unwrap();
        let progress = 0.5 + 0.25 * tick;
        assert!((pose.progress - progress).abs() < 1e-6, "{tick}: {pose:?}");
        // The fraction runs from the offset (0.3) to the end by the
        // progress, and the straight arrow puts it that far along 50.
        let fraction = 0.3 + 0.7 * progress;
        let expected = arrow_point(&[[0, 0], [5_000_000, 0]], fraction, &|_, x, z| {
            heights.native_height(x, z)
        })
        .unwrap()
        .0;
        let offset = defines().unit_offset;
        for axis in 0..3 {
            assert!(
                (pose.position[axis] - (expected[axis] + offset[axis])).abs() < 1e-4,
                "{tick}: {pose:?} against {expected:?}"
            );
        }
        assert!(pose.position[0] > start.position[0]);
        assert_eq!(pose.facing, start.facing);
    }
    // Midway, the unit is midway between the hour's two stands.
    let (a, b) = (
        start.position[0],
        track.place(&heights, 1.0).unwrap().position[0],
    );
    assert!((track.place(&heights, 0.5).unwrap().position[0] - (a + b) / 2.0).abs() < 1e-3);
}

#[test]
fn a_stage_threshold_is_crossed_at_its_sub_hour_moment() {
    // Progress runs 0.5 to 0.75 over the hour; a stage at 0.625 is
    // reached halfway through it.
    let track = line_track(60_000, 1_500_000);
    let stages = [
        installed_content::raids::RaidAnimationStage {
            state: "cruise".into(),
            intro: None,
            at_progress_raw: 0,
        },
        installed_content::raids::RaidAnimationStage {
            state: "dive".into(),
            intro: Some("dive_intro".into()),
            at_progress_raw: 62_500,
        },
    ];
    let every = |_: &str| true;
    let stage = |tick: f32| animation_state(&stages, track.progress(tick), &every).unwrap();
    assert_eq!(stage(0.0).state, "cruise");
    assert_eq!(stage(0.49).state, "cruise");
    assert_eq!(stage(0.5).state, "dive");
    assert_eq!(stage(0.75).state, "dive");
    // Playing frame by frame on the viewer's clock, the intro starts on the
    // first frame past the threshold and hands over to the state.
    let rig = crate::map::entity_animation::stage_rig(true);
    let mut player = crate::map::entity_animation::Player::new(&rig);
    let mut started = None;
    for frame in 0..=100 {
        let tick = frame as f32 / 100.0;
        let chosen = stage(tick);
        let code =
            crate::map::entity_animation::named_stage(&chosen.state, chosen.intro.as_deref());
        // A tenth of an hour a second: ten seconds of wall clock an hour.
        let time = 1.0 + tick * 10.0;
        let playing = crate::map::entity_animation::root_state(&mut player, &rig, code, time);
        if playing == "dive_intro" && started.is_none() {
            started = Some(tick);
        }
    }
    assert_eq!(started, Some(0.5));
    let code = crate::map::entity_animation::named_stage("dive", Some("dive_intro"));
    assert_eq!(
        crate::map::entity_animation::root_state(&mut player, &rig, code, 12.0),
        "dive"
    );
}

/// A frame holding only `RaidSpeedInputs`, or without it.
fn speed_frame(rows: Option<Vec<replay::rows::RaidSpeedInputRow>>) -> DecodedFrame {
    use replay::{
        frame::{Metrics, TableView},
        rows::{TableId, TableRows},
    };
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
    let row = replay::rows::RaidSpeedInputRow {
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
    // A present, dead ace adds nothing; an absent one is `None`.
    assert_eq!(inputs.ace_alive, Some(0));
    let absent = replay::rows::RaidSpeedInputRow {
        ace_present: 0,
        ..row.clone()
    };
    assert_eq!(
        SpeedInputs::from_row(&absent, &speed_frame(None))
            .unwrap()
            .ace_alive,
        None
    );
    // An older recording has no distance; a withheld table or a missing row
    // is refused by name.
    let old = RaidRow {
        travelled: None,
        ..raid.clone()
    };
    assert!(plane_inputs(&speed_frame(Some(vec![row])), &old).is_err());
    let withheld = plane_inputs(&speed_frame(None), &raid).unwrap_err();
    assert!(withheld.contains("fixed-key contract"), "{withheld}");
    assert!(plane_inputs(&speed_frame(Some(Vec::new())), &raid).is_err());
}

/// A living ace's country terms are CountryCommandPower's at the same frame,
/// joined by the row's owner and wing-country slots; without that table or
/// its row they are refused, not zero.
#[test]
fn a_living_aces_country_terms_join_country_command_power() {
    use replay::{
        frame::TableView,
        rows::{CountryCommandPowerRow, TableId, TableRows},
    };
    let row = replay::rows::RaidSpeedInputRow {
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
    // A dead ace records no countries and needs no row.
    let dead = replay::rows::RaidSpeedInputRow {
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
    // Phase 4 always shows; the hour's progress lerps to the next hour's.
    assert!(unit_shown(4, 0, 0.0));
    assert!((movement_progress(4, 0.5, 0.75, 0.5) - 0.625).abs() < 1e-6);
    // Phase 5 sets the progress to exactly 1, past the 0.999 cut-off.
    assert_eq!(movement_progress(5, 0.5, 0.75, 0.3), 1.0);
    assert!(!unit_shown(5, 1, movement_progress(5, 0.5, 0.75, 0.3)));
    // Only the frame still holding the last phase-4 progress shows it, and
    // never for a cancelled raid.
    assert!(unit_shown(5, 1, 0.998));
    assert!(!unit_shown(5, 1, 0.999));
    assert!(!unit_shown(5, 5, 0.5));
    assert!(!unit_shown(3, 0, 0.0));
}

#[test]
fn a_wing_ace_with_an_unresolved_wing_scales_by_one() {
    let defines = speed_defines();
    // 0.1 key-2 bonus, scale 1 (wing unresolved), effectiveness 0: factor
    // 1.1, no type bonus: 1.1 * 6.0 = 6.6, * 0.5 land, * 0.1 raid = 0.33.
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
    // 2 x 2 heights, bottom row first in world z.
    let values = [10u8, 20, 30, 40];
    let field = HeightField {
        width: 2,
        height: 2,
        values: &values,
    };
    // Row 1 of the bitmap is world z 0.
    assert_eq!(field.fixed_height(0, 0, 2), 3.0);
    assert!((field.fixed_height(50_000, 0, 2) - 3.5).abs() < 1e-6);
    assert!((field.fixed_height(50_000, 50_000, 2) - 2.5).abs() < 1e-6);
    // x wraps once by the map width before it clamps.
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
    // Archetype tokens 40 and 41 spell the two archetypes.
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
    // Exactly enough: five transports and a hundred infantry equipment.
    assert_eq!(pool_meets(&kind, &pool(500_000, 1), &token), Ok(true));
    // 4.99999 transports are four: short by one whole transport.
    assert_eq!(pool_meets(&kind, &pool(499_999, 1), &token), Ok(false));
    assert_eq!(pool_meets(&kind, &pool(400_000, 1), &token), Ok(false));
    // The additional list's nuke.
    assert_eq!(pool_meets(&kind, &pool(500_000, 0), &token), Ok(false));
    // The essential list's nukes are checked on their own.
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
    // An archetype without a total holds none; the first of two entries
    // answers, as the native scan stops there.
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
    // A token the content cannot spell is refused.
    let odd = RaidPool {
        archetype_totals: vec![(9, 1)],
        nukes: 1,
    };
    assert!(pool_meets(&kind, &odd, &token).is_err());
}

/// A frame holding only `RaidEquipment`, or without it.
fn pool_frame(rows: Option<Vec<replay::rows::RaidEquipmentRow>>) -> DecodedFrame {
    use replay::{
        frame::{Metrics, TableView},
        rows::{TableId, TableRows},
    };
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
    // The variant items, a null one among them, are not what the check
    // reads: the archetype totals are.
    let row = replay::rows::RaidEquipmentRow {
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
    let torn = replay::rows::RaidEquipmentRow {
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
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use installed_content::raids::RaidUnitModel;
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
                .and_then(installed_content::equipment::category_mask)
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
    // The tag first, then the culture, then the bare name.
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
    // The category picks the first primary variant that has it.
    let primary = [(10, 0x40), (11, 0x10_0000), (12, 0x10_0000)];
    assert_eq!(
        choose(
            &model(Some("ship_entity"), None, Some("air_transport")),
            &primary,
            None
        ),
        UnitEntity::Designer(11)
    );
    // Without a variant of it, the names are tried.
    assert_eq!(
        choose(
            &model(Some("ship_entity"), None, Some("air_transport")),
            &primary[..1],
            None
        ),
        UnitEntity::Entity("western_european_gfx_ship_entity".into())
    );
    // Nothing named: the wing, else nothing.
    assert_eq!(
        choose(&model(None, None, None), &[], Some(7)),
        UnitEntity::WingEquipment(7)
    );
    assert_eq!(
        choose(&model(None, None, None), &[], None),
        UnitEntity::None
    );
    // An unknown category has mask zero, so no designer model.
    assert_eq!(
        choose(&model(None, None, Some("zeppelins")), &[], None),
        UnitEntity::None
    );
}

#[test]
fn the_arrow_point_interpolates_controls_heights_and_tangents() {
    let controls = [[0, 0], [10 * SCALE, 0], [10 * SCALE, 10 * SCALE]];
    let height = |i: usize, _: f32, _: f32| i as f32;
    let (at, tangent) = arrow_point(&controls, 0.25, &height).unwrap();
    assert_eq!(at, [5.0, 0.5, 0.0]);
    // Halfway along the first segment: the mean of its direction and the
    // next control's.
    let (a, b) = (normalize([10.0, 1.0, 0.0]), normalize([0.0, 1.0, 10.0]));
    let expected = normalize([
        (a[0] + b[0]) * 0.5,
        (a[1] + b[1]) * 0.5,
        (a[2] + b[2]) * 0.5,
    ]);
    assert!((0..3).all(|axis| (tangent[axis] - expected[axis]).abs() < 1e-6));
    // Past the corner the tangent turns towards +z.
    let (at, _) = arrow_point(&controls, 0.75, &height).unwrap();
    assert_eq!(at, [10.0, 1.5, 5.0]);
    // Outside [0, 1]: the first control.
    assert_eq!(
        arrow_point(&controls, 1.5, &height).unwrap().0,
        [0.0, 0.0, 0.0]
    );
    assert_eq!(arrow_point(&controls[..1], 0.5, &height), None);
}

#[test]
fn the_designer_model_follows_resolve_aircraft_designer_model_with_and_without_the_dlc() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    // An empty type sprite is filled from its archetype.
    let (content, _) = installed_content::equipment::fixture::load(&[(
        "common/units/equipment/planes.txt",
        "equipments = {
            transport_plane_equipment = { is_archetype = yes sprite = transport_plane }
            transport_plane_equipment_1 = { archetype = transport_plane_equipment }
        }",
    )]);
    let variant = |saved: &str| replay::rows::VariantDefinitionRow {
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
    let resolve = |row: &replay::rows::VariantDefinitionRow,
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
    let asked = std::cell::Cell::new(false);
    let untouched = || {
        asked.set(true);
        Ok(None)
    };
    // With the DLC, the declared designer result wins. A set saved model
    // supplies that result; the stub represents any result.
    // `unit_models::tests::aircraft_designer` exercises the resolver.
    assert_eq!(
        resolve(&variant("designer_c47_entity"), 0x100, &pool),
        Ok(Some("pool_c47_entity".into()))
    );
    // An undeclared or absent result goes on to the saved model, then the
    // aircraft ladder.
    assert_eq!(
        resolve(&variant("designer_c47_entity"), 0x100, &undeclared_pool),
        Ok(Some("designer_c47_entity".into()))
    );
    assert_eq!(
        resolve(&variant(""), 0x100, &none),
        Ok(Some("ENG_transport_plane_entity".into()))
    );
    // Without the DLC, do not query the designer model.
    assert_eq!(
        resolve(&variant("designer_c47_entity"), 0, &untouched),
        Ok(Some("designer_c47_entity".into()))
    );
    assert_eq!(
        resolve(&variant(""), 0, &untouched),
        Ok(Some("ENG_transport_plane_entity".into()))
    );
    assert!(!asked.get());
    // What the pools cannot decide is refused with their reason.
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
    // A type the content lacks is refused by name.
    assert_eq!(
        designer_entity(&variant(""), "glider", &content, "ENG", 0, &none, &declared),
        Err("equipment glider is not in the content".to_owned())
    );
}

/// The building arm of `RaidSource`: a null type token (`0x4c17`) or a
/// location of -1 is no building; otherwise the recorded type, location
/// and country, as stored.
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

#[test]
fn the_animation_stage_is_the_last_reached_one_the_entity_has() {
    use installed_content::raids::RaidAnimationStage;
    let stage = |state: &str, at: i64| RaidAnimationStage {
        state: state.into(),
        intro: None,
        at_progress_raw: at,
    };
    let stages = [
        stage("idle", 0),
        stage("idle2", 33_000),
        stage("idle3", 66_000),
    ];
    let all = |_: &str| true;
    let pick = |progress, has: &dyn Fn(&str) -> bool| {
        animation_state(&stages, progress, has).map(|stage| stage.state.as_str())
    };
    assert_eq!(pick(0.0, &all), Some("idle"));
    assert_eq!(pick(0.329, &all), Some("idle"));
    assert_eq!(pick(0.33, &all), Some("idle2"));
    assert_eq!(pick(0.9, &all), Some("idle3"));
    // A state the entity lacks is skipped; the earlier reached one stays.
    let basic = |name: &str| name != "idle3";
    assert_eq!(pick(0.9, &basic), Some("idle2"));
    // Nothing reached or nothing the entity has: the entity keeps its state.
    let none = |_: &str| false;
    assert_eq!(pick(0.9, &none), None);
    assert_eq!(animation_state(&[stage("late", 50_000)], 0.1, &all), None);
    // The launch hour's progress is 0.
    assert_eq!(line_track(0, 0).progress(0.0), 0.0);
}
