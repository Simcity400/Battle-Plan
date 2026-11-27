use super::*;
use std::cell::RefCell;
use std::time::Duration;
use std::time::Instant;
use viewer_native_gui::text::GlyphBox;
use viewer_native_gui::text::GlyphMetrics;

#[test]
fn native_arrow_modes_use_authored_flags_and_constructor_defaults() {
    let types = ArrowTypes::parse(
        br#"
        arrowType = { name = ordinary technique = MapArrowNoHead }
        arrowType = { name = late technique = Custom draw_last = yes draw_shadows = no }
        arrowType = { name = overwritten draw_last = yes draw_last = no }
        arrowType = { name = invalid draw_last = "yes" draw_last = YES draw_shadows = YES }
        arrowText = { technique = CustomText }
    "#,
    )
    .unwrap();
    let ordinary = types.get("ordinary").unwrap();
    assert!(!ordinary.draw_last);
    assert!(ordinary.draw_shadows);
    let late = types.get("late").unwrap();
    assert!(late.draw_last);
    assert!(!late.draw_shadows);
    assert_eq!(late.shader_technique, "Custom");
    assert!(!types.get("overwritten").unwrap().draw_last);
    let invalid = types.get("invalid").unwrap();
    assert!(invalid.draw_last);
    assert!(invalid.draw_shadows);
    assert_eq!(types.text_technique, "CustomText");
    assert!(Mode::Main.admits(ordinary.draw_last));
    assert!(!Mode::Main.admits(late.draw_last));
    assert!(Mode::Late.admits(late.draw_last));
    assert!(!Mode::Late.admits(ordinary.draw_last));
    assert!(Mode::Seethrough.admits(late.draw_last));
    assert!(Mode::Seethrough.admits(ordinary.draw_last));
    assert_eq!(
        [Mode::Main, Mode::Seethrough, Mode::Late].map(Mode::lighting),
        [1., 1., 0.]
    );
}

fn types() -> ArrowTypes {
    ArrowTypes::parse(
        br#"
        colors = {
            leader_group = { 0.50 0.00 0.00 1.0 }
            leader_group = { 0.00 0.50 0.41 1.0 }
        }
        arrowText = {
            font = "some_arrow_font"
            effect = "gfx/FX/mapname.lua"
        }
        arrowType = {
            name = "move_thick"
            technique = "MapArrowDefault"
            texture_masks = { "a_mask.dds" "a_unassigned_mask.dds" }
            texture_patterns = { "a.dds" "a.dds" }
            body_uv_scale = 0.4
            body_width = { 20.5 6 }
            head_length = 19.5
            text_size = 1
            text_offset = 5
            fade_in = 0.2
            global_scale_min_size = 0.1
            global_scale_max_points = 40
            texture_anim_speed = 0.1
        }
        arrowType = {
            name = "offence_line"
            technique = "MapArrowNoHead"
            texture_masks = { "b_mask.dds" }
            texture_patterns = { "b.dds" }
            body_width = { 1.1 1.1 }
            texture_loop = 2
            culling_direction = 1
        }
        arrowType = { name = "move_thick" body_width = { 80 90 } }
        "#,
    )
    .unwrap()
}

#[test]
fn the_arrow_file_parses_into_the_engines_own_fields() {
    let types = types();
    assert_eq!(types.leader_colors.len(), 2);
    assert_eq!(types.leader_colors[1], [0.0, 0.5, 0.41, 1.0]);
    assert_eq!(types.text_font.as_deref(), Some("some_arrow_font"));
    let move_thick = types.get("move_thick").unwrap();
    assert_eq!(move_thick.shader_technique, "MapArrowDefault");
    assert_eq!(move_thick.body_width, [20.5, 6.0]);
    assert_eq!(move_thick.declaration_index, 0);
    assert_eq!(move_thick.head_length, 19.5);
    assert_eq!(move_thick.masks.len(), 2);
    assert_eq!(move_thick.text_offset, 5.0);
    assert_eq!(move_thick.culling_direction, 0);
    let line = types.get("offence_line").unwrap();
    assert_eq!(line.shader_technique, "MapArrowNoHead");
    assert_eq!(line.texture_loop, 2.0);
    assert_eq!(line.culling_direction, 1);
    assert_eq!(line.declaration_index, 1);
    assert_eq!(line.text_size, 1.0);
    assert_eq!(line.text_offset, 0.0);
}

fn order(kind: i32, group: u64, instance: u32, path: &[u32]) -> ArmyOrderRow {
    ArmyOrderRow {
        owner_group: group,
        instance,
        country: 1,
        order_type: kind,
        active: 1,
        path: path.to_vec(),
        target_states: Vec::new(),
        paradrop_target: 0,
    }
}

fn theatre(id: u64, owner: u32, ordinal: u32) -> LandTheatreRow {
    LandTheatreRow {
        id,
        owner,
        ordinal,
        volunteers_theatre: 0,
    }
}

fn group(handle: u64, country: u32, theatre: u64, ordinal: Option<u32>) -> OrdersGroupRow {
    OrdersGroupRow {
        handle,
        country,
        theatre,
        theatre_ordinal: ordinal,
        ..Default::default()
    }
}

fn group_roots(groups: &[OrdersGroupRow]) -> Vec<OrdersGroupPlanRow> {
    groups
        .iter()
        .map(|group| OrdersGroupPlanRow {
            group: group.handle,
            ..Default::default()
        })
        .collect()
}

fn fixture_defines(smoothing: Smoothing) -> OrderDefines {
    OrderDefines {
        order_move: smoothing,
        order_front: smoothing,
        root_front: smoothing,
        snapped_off_front: smoothing,
        root_front_offset: 170_000,
        root_front_max_intersection_tests_order: 21,
        root_front_max_intersection_tests_front: 27,
        max_offsets: 4,
        ext_text_points_limit: 100,
        virtual_color: [1.0; 4],
        allied_color: [0.5; 4],
        paradrop_height_to_len: 0.5,
        paradrop_height_max: 3.0,
    }
}

#[test]
fn callback_colour_uses_virtual_then_owner_identity_and_display_group() {
    let defines = fixture_defines(Smoothing {
        smoothness: 50_000,
        passes: 1,
    });
    assert_eq!(callback_color(&defines, true, None, None), Some([1.0; 4]));
    assert_eq!(
        callback_color(&defines, false, Some(false), None),
        Some([0.5; 4])
    );
    let display = OrdersGroupPlanRow {
        red: 0.1_f32.to_bits(),
        green: 0.2_f32.to_bits(),
        blue: 0.3_f32.to_bits(),
        alpha: 0.4_f32.to_bits(),
        ..Default::default()
    };
    assert_eq!(
        callback_color(&defines, false, Some(true), Some(&display)),
        Some([0.1, 0.2, 0.3, 0.4])
    );
    assert_eq!(callback_color(&defines, false, None, Some(&display)), None);
    assert_eq!(callback_color(&defines, false, Some(true), None), None);
    let nonfinite = OrderDefines {
        allied_color: [f32::NAN; 4],
        ..defines
    };
    assert_eq!(callback_color(&nonfinite, false, Some(false), None), None);
}

#[test]
fn a_bound_line_is_identified_by_its_binding_and_styled_by_the_owner() {
    let line = order(TYPE_LINE, 9, 1, &[11, 12]);
    let plan = ArmyOrderPlanRow {
        front_valid: 255,
        owner_override: 8,
        ..Default::default()
    };
    assert_eq!(classify(&line, Some(&plan)), Classified::FrontLine);
    assert_eq!(
        Classified::FrontLine.arrow_type(&plan, true),
        Some("ag_defence_line_no_child")
    );
    assert_eq!(
        Classified::OffensiveLine.arrow_type(&plan, true),
        Some("offence_line")
    );
    assert_eq!(
        classify(&line, Some(&ArmyOrderPlanRow::default())),
        Classified::OffensiveLine
    );
}

#[test]
fn front_span_uses_section_ids_and_inclusive_pairs_without_clamping() {
    let front = FrontRow {
        front: 7,
        section_ids: vec![80, 20],
        section_ends: vec![2, 6],
        province_pairs: vec![9, 10, 1, 2, 2, 3],
    };
    let mut plan = ArmyOrderPlanRow {
        front: 7,
        front_valid: 1,
        section: 20,
        segment_start: 0,
        segment_end: 1,
        ..Default::default()
    };
    let pairs = |pairs: Vec<[u32; 2]>| Some(FrontSpan::Pairs(pairs));
    assert_eq!(front_pairs(&plan, &front), pairs(vec![[1, 2], [2, 3]]));
    plan.segment_start = 1;
    assert_eq!(front_pairs(&plan, &front), pairs(vec![[2, 3]]));
    for (start, end) in [(-1, 1), (0, -1), (0, 2), (3, 2)] {
        plan.segment_start = start;
        plan.segment_end = end;
        assert_eq!(front_pairs(&plan, &front), Some(FrontSpan::Destroyed));
    }
    for start in [1, 5] {
        plan.segment_start = start;
        plan.segment_end = 0;
        assert_eq!(front_pairs(&plan, &front), pairs(Vec::new()));
    }
    plan.section = 1;
    assert!(front_pairs(&plan, &front).is_none());
}

#[test]
fn transports_and_garrisons_have_no_move_chain() {
    let mut invasion = order(TYPE_INVASION, 1, 1, &[10, 11]);
    invasion.paradrop_target = 12;
    assert!(chain(&invasion, Classified::Invasion).is_empty());
    let mut drop = order(TYPE_PARADROP, 1, 2, &[20]);
    drop.paradrop_target = 21;
    assert!(chain(&drop, Classified::Paradrop).is_empty());
    let garrison = order(TYPE_AREA_DEFENSE, 1, 3, &[]);
    assert!(chain(&garrison, Classified::GarrisonArea).is_empty());
}

#[test]
fn long_moves_keep_the_first_two_last_two_and_alternate_interior_provinces() {
    let short = order(TYPE_MOVE, 1, 1, &[1, 2, 3, 4]);
    assert_eq!(chain(&short, Classified::Move), short.path);
    let long = order(TYPE_MOVE, 1, 2, &[1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(chain(&long, Classified::Move), vec![1, 2, 4, 6, 7, 8]);
}

#[test]
fn endpoint_snap_picks_a_stored_control_wraps_longitude_and_keeps_the_first_tie() {
    let p = |x: i64, z: i64| [x * SCALE, z * SCALE];
    let width = 100 * SCALE;
    assert_eq!(
        nearest_control(&[p(0, 0), p(10, 0)], p(4, 2), width),
        Some(p(0, 0))
    );
    assert_eq!(
        nearest_control(&[p(98, 0), p(5, 0)], p(1, 0), width),
        Some(p(-2, 0))
    );
    assert_eq!(
        nearest_control(&[p(10, 0), p(0, 0)], p(5, 0), width),
        Some(p(10, 0))
    );
    assert_eq!(nearest_control(&[p(1_000, 1_000)], p(0, 0), 0), None);
}

#[test]
fn linked_type_selection_uses_native_order_without_assignment_guesses() {
    let line = order(TYPE_LINE, 1, 2, &[1, 2]);
    let movement = order(TYPE_MOVE, 1, 3, &[8, 9, 2]);
    let first = order(TYPE_MOVE, 1, 4, &[6, 7, 8, 2]);
    let foreign = order(TYPE_MOVE, 2, 4, &[6, 7, 8, 2]);
    let indexed = BTreeMap::from([
        ((1, 2), &line),
        ((1, 3), &movement),
        ((1, 4), &first),
        ((2, 4), &foreign),
    ]);
    assert_eq!(
        first_linked(1, &[2, 4, 3], TYPE_MOVE, &indexed),
        Some(&first)
    );
    assert_eq!(
        first_linked(1, &[2, 3, 4], TYPE_MOVE, &indexed),
        Some(&movement)
    );
    assert_eq!(first_linked(1, &[], TYPE_MOVE, &indexed), None);
    assert_eq!(first_linked(2, &[4], TYPE_MOVE, &indexed), Some(&foreign));
}

#[test]
fn root_and_child_order_control_native_shared_child_suppression() {
    let make = |id| ArmyOrderRow {
        owner_group: 9,
        instance: id,
        ..Default::default()
    };
    let rows = [make(1), make(2), make(3), make(4)];
    let indexed: BTreeMap<_, _> = rows.iter().map(|row| ((9, row.instance), row)).collect();
    let one = ArmyOrderPlanRow {
        successors: vec![3],
        ..Default::default()
    };
    let two = one.clone();
    let child = ArmyOrderPlanRow {
        predecessors: vec![2, 1],
        ..Default::default()
    };
    let fallback = ArmyOrderPlanRow::default();
    let mut plans = BTreeMap::from([
        ((9, 1), &one),
        ((9, 2), &two),
        ((9, 3), &child),
        ((9, 4), &fallback),
    ]);
    let group = OrdersGroupPlanRow {
        group: 9,
        primary_orders: vec![1, 2],
        fallback_orders: vec![4],
        ..Default::default()
    };
    let ids = |result: Vec<&ArmyOrderRow>| {
        result
            .into_iter()
            .map(|row| row.instance)
            .collect::<Vec<_>>()
    };
    let mut refused = BTreeMap::new();
    assert_eq!(
        ids(drawing_order(&[&group], &indexed, &plans, &mut refused)),
        [1, 2, 3, 4]
    );
    let multiple = ArmyOrderPlanRow {
        successors: vec![3, 4],
        ..Default::default()
    };
    plans.insert((9, 1), &multiple);
    assert_eq!(
        ids(drawing_order(&[&group], &indexed, &plans, &mut refused)),
        [1, 3, 4, 2, 3, 4]
    );
    assert!(refused.is_empty());
}

#[test]
fn malformed_graphs_are_refused_not_inferred() {
    let row = order(TYPE_MOVE, 9, 1, &[1]);
    let indexed = BTreeMap::from([((9, 1), &row)]);
    let group = OrdersGroupPlanRow {
        group: 9,
        primary_orders: vec![1],
        ..Default::default()
    };
    let cycle = ArmyOrderPlanRow {
        successors: vec![1],
        ..Default::default()
    };
    let mut refused = BTreeMap::new();
    let plans = BTreeMap::from([((9, 1), &cycle)]);
    assert!(drawing_order(&[&group], &indexed, &plans, &mut refused).is_empty());
    let missing = ArmyOrderPlanRow {
        successors: vec![2],
        ..Default::default()
    };
    let plans = BTreeMap::from([((9, 1), &missing)]);
    assert!(drawing_order(&[&group], &indexed, &plans, &mut refused).is_empty());
    assert_eq!(refused.values().sum::<usize>(), 2);
}

#[test]
fn group_roots_follow_theatre_then_subtype_roster_order() {
    let theatres = [theatre(90, 1, 0), theatre(10, 1, 1)];
    let mut groups = [
        group(1, 1, 10, Some(0)),
        group(2, 1, 90, Some(0)),
        group(3, 1, 90, Some(1)),
        group(4, 1, 90, Some(0)),
        group(5, 1, 90, None),
    ];
    groups[2].is_army_group = 1;
    groups[3].is_army_group = 1;
    let roots = group_roots(&groups);
    let indexed = groups.iter().map(|group| (group.handle, group)).collect();
    let links = BTreeMap::from([(1, 1)]);
    let mut refused = BTreeMap::new();
    let ordered = ordered_group_roots(
        &indexed,
        &theatres,
        &roots,
        &links,
        &[1],
        false,
        &mut refused,
    );
    assert_eq!(
        ordered.iter().map(|group| group.group).collect::<Vec<_>>(),
        [4, 3, 2, 1]
    );
    assert_eq!(refused.values().sum::<usize>(), 1);
    assert!(
        ordered_group_roots(
            &indexed,
            &theatres,
            &roots,
            &BTreeMap::new(),
            &[1],
            false,
            &mut refused
        )
        .is_empty(),
        "a raw country reference is not an unverified canonical slot"
    );
    groups[2].theatre_ordinal = Some(0);
    let indexed = groups.iter().map(|group| (group.handle, group)).collect();
    let mut refused = BTreeMap::new();
    let ordered = ordered_group_roots(
        &indexed,
        &theatres,
        &roots,
        &links,
        &[1],
        false,
        &mut refused,
    );
    assert_eq!(ordered.len(), 3);
    assert_eq!(refused["duplicate theatre roster position"], 1);

    let orphan = [OrdersGroupPlanRow {
        group: 99,
        ..Default::default()
    }];
    let mut refused = BTreeMap::new();
    assert!(
        ordered_group_roots(
            &indexed,
            &theatres,
            &orphan,
            &links,
            &[1],
            false,
            &mut refused
        )
        .is_empty()
    );
    assert_eq!(refused["group plan whose orders group is not recorded"], 1);
}

#[test]
fn allied_roots_follow_country_occurrences_and_allow_cross_country_groups() {
    let theatres = [theatre(10, 1, 0), theatre(20, 2, 0)];
    let groups = [
        group(100, 1, 10, Some(0)),
        group(200, 3, 20, Some(0)),
    ];
    let indexed = groups.iter().map(|group| (group.handle, group)).collect();
    let roots = group_roots(&groups);
    let links = BTreeMap::from([(1, 1), (2, 2), (3, 3)]);
    let mut refused = BTreeMap::new();
    let ordered = ordered_group_roots(
        &indexed,
        &theatres,
        &roots,
        &links,
        &[1, 2, 2],
        true,
        &mut refused,
    );
    assert_eq!(
        ordered.iter().map(|root| root.group).collect::<Vec<_>>(),
        [100, 200, 200]
    );
    assert!(refused.is_empty(), "{refused:?}");
    assert!(
        ordered_group_roots(
            &indexed,
            &theatres,
            &roots,
            &links,
            &[2],
            false,
            &mut refused
        )
        .is_empty()
    );
}

#[test]
fn theatre_admission_skips_known_foreign_and_refuses_unknown_identities() {
    let theatres = [theatre(10, 1, 0), theatre(20, 2, 0), theatre(30, 88, 0)];
    let groups = [
        group(100, 1, 10, Some(0)),
        group(200, 77, 10, Some(1)),
        group(300, 1, 20, None),
        group(400, 1, 30, Some(0)),
        group(500, 1, 99, Some(0)),
    ];
    let indexed = groups.iter().map(|group| (group.handle, group)).collect();
    let roots = group_roots(&groups);
    let links = BTreeMap::from([(1, 1), (2, 2)]);
    let mut refused = BTreeMap::new();
    let ordered = ordered_group_roots(
        &indexed,
        &theatres,
        &roots,
        &links,
        &[1],
        false,
        &mut refused,
    );
    assert_eq!(
        ordered.iter().map(|root| root.group).collect::<Vec<_>>(),
        [100]
    );
    assert_eq!(
        refused,
        BTreeMap::from([
            ("group country has no CountryLinks row", 1),
            ("group whose theatre owner has no CountryLinks row", 1),
            ("group whose theatre roster is not recorded", 1),
        ])
    );
    refused.clear();
    let ordered = ordered_group_roots(
        &indexed,
        &theatres,
        &roots,
        &links,
        &[1],
        true,
        &mut refused,
    );
    assert_eq!(
        ordered.iter().map(|root| root.group).collect::<Vec<_>>(),
        [100, 200]
    );
    assert!(!refused.contains_key("group country has no CountryLinks row"));
    assert!(!refused.contains_key("group without a theatre roster position"));
}

#[test]
fn ribbon_v_zero_is_on_the_native_left_edge_in_both_directions() {
    let arrow = ArrowType {
        body_width: [2.0; 2],
        texture_loop: 4.0,
        ..Default::default()
    };
    for (samples, expected_z) in [
        ([[2.0, 4.0], [6.0, 4.0]], 6.0),
        ([[6.0, 4.0], [2.0, 4.0]], 2.0),
    ] {
        let mut field = OrderField::default();
        emit(
            &mut field,
            &arrow,
            &samples.map(|p| p.map(|v| (v * SCALE as f32) as i64)),
            1.0,
            [1.0; 4],
            &|_, _, _| 10.0,
        )
        .unwrap();
        let edge = &field.vertices[0];
        assert_eq!(edge.uv_head_variant[1], 0.0);
        assert_eq!(edge.position[2], expected_z);
        assert_eq!(field.vertices[1].uv_head_variant[1], 1.0);
        assert_eq!(culling_offset(1, -1) - edge.uv_head_variant[1], 1.0);
    }
}

#[test]
fn ribbon_height_interpolates_wraps_and_clamps_to_the_sea() {
    let heights = HeightField {
        width: 2,
        height: 2,
        values: &[200, 240, 100, 140],
    };
    assert!((ribbon_height(&heights, 0.25, 0.5) - 16.01).abs() < 0.00001);
    assert_eq!(
        ribbon_height(&heights, 0.25, 0.5),
        ribbon_height(&heights, 2.25, 0.5)
    );
    assert_eq!(
        ribbon_height(&heights, 0.25, 0.5),
        ribbon_height(&heights, -1.75, 0.5)
    );
    let sea = HeightField {
        width: 2,
        height: 2,
        values: &[0; 4],
    };
    assert!((ribbon_height(&sea, 0.5, 0.5) - 9.51).abs() < 0.00001);
}

#[test]
fn ribbon_uses_fixed_smoothed_controls_and_interpolates_between_them() {
    let mut field = OrderField::default();
    let arrow = ArrowType {
        body_width: [1.0; 2],
        texture_loop: 10.0,
        ..Default::default()
    };
    let controls = [[0, 0], [1_000_003, 0], [2_000_007, 0]];
    emit(&mut field, &arrow, &controls, 1.0, [1.0; 4], &|_, _, i| {
        (i * i) as f32
    })
    .unwrap();
    assert_eq!(field.vertices.len(), 6);
    assert_eq!(
        field.draws[0].native_strips,
        std::iter::once(0..512).collect::<Vec<_>>()
    );
    assert_eq!(field.vertices[2].position[0], 10.00002);
    assert_eq!(field.vertices[2].position[1], 1.0);
    assert_eq!(field.vertices[4].position[1], 4.0);
    assert_eq!(field.vertices[2].uv_head_variant[0], 10.00002 / 10.0);
}

#[test]
fn paradrop_strip_lift_uses_control_indices_even_at_a_head_cut() {
    let controls = [[0, 0], [10 * SCALE, 0], [30 * SCALE, 0]];
    let lift = transport::Lift {
        from: 2.0,
        to: 8.0,
        amplitude: 0.0,
    };
    for (head_length, expected_indices, body_end_height, tip_height) in [
        (0.0, vec![0, 0, 1, 1, 2, 2], 6.0, 6.0),
        (8.0, vec![0, 0, 1, 1, 2, 2, 4, 4, 5, 5], 4.4, 8.0),
    ] {
        let arrow = ArrowType {
            body_width: [2.0; 2],
            head_length,
            ..Default::default()
        };
        let observed = RefCell::new(Vec::new());
        let mut field = OrderField::default();
        emit(
            &mut field,
            &arrow,
            &controls,
            1.0,
            [1.0; 4],
            &|_, _, index| {
                observed.borrow_mut().push(index);
                lift.at_control(index, controls.len(), head_length > 0.0)
            },
        )
        .unwrap();
        assert_eq!(observed.into_inner(), expected_indices);
        assert!((field.vertices[4].position[1] - body_end_height).abs() < 1e-5);
        assert!((field.vertices.last().unwrap().position[1] - tip_height).abs() < 1e-5);
    }
}

#[test]
fn ribbon_head_cut_keeps_the_crossed_controls_width_uv_and_height() {
    let arrow = ArrowType {
        body_width: [4.0, 2.0],
        body_uv_scale: 0.5,
        head_length: 8.0,
        ..Default::default()
    };
    let mut field = OrderField::default();
    emit(
        &mut field,
        &arrow,
        &[[0, 0], [1_000_000, 0], [3_000_000, 0]],
        1.0,
        [1.0; 4],
        &|_, _, i| i as f32,
    )
    .unwrap();
    assert_eq!(field.vertices.len(), 10);
    let body = &field.vertices[4];
    assert_eq!(body.position, [22.0, 2.0, 1.0]);
    assert_eq!(body.uv_head_variant, [1.0, 0.0, 0.0, 0.0]);
    assert_eq!(field.vertices[6].position, [22.0, 2.0, 2.0]);
    assert_eq!(field.vertices[8].position, [30.0, 5.0, 2.0]);
    assert_eq!(field.draws[0].uv_to_length, 30.0);

    let mut field = OrderField::default();
    emit(
        &mut field,
        &arrow,
        &[[0, 0], [1_000_000, 0], [3_000_000, 0]],
        1.0,
        [1.0; 4],
        &|_, z, i| i as f32 + z,
    )
    .unwrap();
    assert_eq!(field.vertices[4].position, [22.0, 3.0, 1.0]);
    assert_eq!(field.vertices[5].position, [22.0, 1.0, -1.0]);
    let root = std::f32::consts::SQRT_2;
    for (vertex, side) in [(6, 1.0), (7, -1.0)] {
        let expected = [22.0, 2.0 + root * side, root * side];
        let actual = field.vertices[vertex].position;
        for (actual, expected) in actual.into_iter().zip(expected) {
            assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
        }
    }
}

struct Square;

impl GlyphMetrics for Square {
    fn advance(&self, _: char) -> Option<f32> {
        Some(10.0)
    }
    fn line_height(&self) -> f32 {
        20.0
    }
    fn base(&self) -> f32 {
        16.0
    }
    fn glyph(&self, _: char) -> Option<GlyphBox> {
        Some(GlyphBox {
            width: 10.0,
            height: 10.0,
            xoffset: 0.0,
            yoffset: 5.0,
            xadvance: 10.0,
            uv: [0.0, 0.0, 0.1, 0.1],
        })
    }
}

impl labels::LabelFont for Square {
    fn page(&self, _: char) -> Option<usize> {
        Some(0)
    }
}

fn bench_localisation() -> content_reader_localisation::localisation::Localisation {
    [
        (
            "MAP_ARROWS_LOCALIZATION",
            "$ORDERNAME$$DIVISIONS$ of $GROUP$",
        ),
        ("MAP_ARROWS_LOCALIZATION_EMPTY", "No divisions"),
        ("MAP_ARROWS_LOCALIZATION_EMPTY_SHORT", "No Divs"),
        ("MAP_ARROWS_DIVISIONS_COUNT", "$DIV$ Divs"),
        ("MAP_ARROWS_DIVISIONS_COUNT_SHORT", "$DIV$"),
        ("MAP_ARROWS_PARATROOPERS_COUNT", "$DIV$ Paras"),
        ("o_strat_offensive_ru", "Fixture Push Order"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect()
}

#[test]
fn one_hour_builds_the_paradrop_and_counts_every_refusal_once() {
    use content_reader_map::movement::{Neighbour, ProvinceTerrain};
    use replay_data::{
        rows::OrderExecutionInputs, rows::ProvinceRow, rows::Recorded, rows::TableRows,
    };
    use replay_frame::{frame::Metrics, frame::TableView};
    let width = 16u32;
    let map = MapData {
        width,
        height: 2,
        province_ids: [[1, 1, 2, 2, 3, 3, 4, 4], [5; 8]].concat().repeat(2),
        definitions: BTreeMap::new(),
        country_colors: Default::default(),
    };
    let geometry = ProvinceGeometry::build(&map);
    let land = ProvinceTerrain { flags: 1, index: 1 };
    let edge = |to| Neighbour {
        to,
        distance: Some(100_000),
        ..Default::default()
    };
    let movement = MovementMap::from_parts(&[
        (1, land, vec![edge(2)]),
        (2, land, vec![edge(1), edge(3)]),
        (3, land, vec![edge(2), edge(4)]),
        (4, land, vec![edge(3)]),
    ]);
    let types = ArrowTypes::parse(
        br#"
        arrowType = { name = "paradrop" technique = "MapArrowNoHead" body_width = { 1 1 } texture_loop = 2 texture_masks = { "assigned.dds" "unassigned.dds" } texture_patterns = { "assigned.dds" "unassigned.dds" } }
        arrowType = { name = "invasion" technique = "MapArrowNoHead" body_width = { 1 1 } texture_loop = 2 texture_masks = { "assigned.dds" "unassigned.dds" } texture_patterns = { "assigned.dds" "unassigned.dds" } }
        arrowType = { name = "move_thick" technique = "MapArrowNoHead" body_width = { 1 1 } texture_loop = 2 texture_masks = { "assigned.dds" "unassigned.dds" } texture_patterns = { "assigned.dds" "unassigned.dds" } }
        arrowType = { name = "defence_line" technique = "MapArrowNoHead" body_width = { 1 1 } texture_loop = 2 texture_masks = { "assigned.dds" "unassigned.dds" } texture_patterns = { "assigned.dds" "unassigned.dds" } }
        "#,
    )
    .unwrap();
    let smoothing = Smoothing {
        smoothness: 50_000,
        passes: 1,
    };
    let assets = OrderAssets {
        air_transfer: Err("unused".into()),
        air_range: Err("not read in this test".into()),
        radar: Err("not read in this test".into()),
        types,
        pages: BTreeMap::new(),
        defines: fixture_defines(smoothing),
        raid_defines: Err("not read in this test".into()),
        execution: ExecutionDefines {
            naval_days: Ok(1),
            air_days: Ok(1),
            unit_weight: Ok(100_000),
            minimum_planes: Ok(1),
        },
    };
    let heights = vec![0u8; (width * 2) as usize];
    let heights = HeightField {
        width,
        height: 2,
        values: &heights,
    };
    let links = BTreeMap::from([(1, 1)]);
    const GROUP: u64 = 100;
    let with = |instance, kind, path: &[u32], source| ArmyOrderRow {
        paradrop_target: source,
        ..order(kind, GROUP, instance, path)
    };
    let mut orders = vec![
        with(1, TYPE_PARADROP, &[4], 1),
        with(2, TYPE_INVASION, &[3], 1),
        with(3, TYPE_MOVE, &[2, 9], 0),
        with(4, TYPE_LINE, &[2], 0),
        with(5, TYPE_MOVE, &[2, 3], 0),
        with(6, TYPE_PARADROP, &[4], u32::MAX),
        with(7, 9, &[2], 0),
    ];
    orders[2].country = 2;
    let plans: Vec<ArmyOrderPlanRow> = orders
        .iter()
        .map(|order| ArmyOrderPlanRow {
            owner_group: GROUP,
            instance: order.instance,
            front: 77,
            front_valid: u8::from(order.instance == 4),
            execution: Recorded::Value((order.order_type == TYPE_PARADROP).then_some(
                OrderExecutionInputs {
                    route_is_ok: Some(0),
                    paradrop: None,
                },
            )),
            ..Default::default()
        })
        .collect();
    let color = |value: f32| value.to_bits();
    let group_plans = vec![
        OrdersGroupPlanRow {
            group: GROUP,
            red: color(1.0),
            green: color(0.0),
            blue: color(0.0),
            alpha: color(1.0),
            primary_orders: vec![1, 2, 3, 3, 4, 6, 6, 7],
            ..Default::default()
        },
        OrdersGroupPlanRow {
            group: 200,
            ..Default::default()
        },
    ];
    let mut tables = std::array::from_fn(|_| TableView::Unavailable {
        reason: "not read by the plan".into(),
    });
    tables[TableId::ArmyOrders.index()] = TableView::Complete(TableRows::ArmyOrders(orders));
    tables[TableId::ArmyOrderPlans.index()] = TableView::Complete(TableRows::ArmyOrderPlans(plans));
    tables[TableId::OrdersGroupPlans.index()] =
        TableView::Complete(TableRows::OrdersGroupPlans(group_plans));
    tables[TableId::OrdersGroups.index()] =
        TableView::Complete(TableRows::OrdersGroups(vec![group(GROUP, 1, 5, Some(0))]));
    tables[TableId::LandTheatres.index()] =
        TableView::Complete(TableRows::LandTheatres(vec![theatre(5, 1, 0)]));
    tables[TableId::Fronts.index()] = TableView::Complete(TableRows::Fronts(Vec::new()));
    tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(
        (1..=4)
            .map(|id| ProvinceRow {
                id,
                controller: 1,
                ..Default::default()
            })
            .collect(),
    ));
    let frame = DecodedFrame {
        branch: 1,
        hour: 0,
        tables: tables.map(Into::into),
        metrics: Metrics::default(),
    };
    let field = build(
        &OrderInputs {
            assets: &assets,
            reads: None,
            transport: None,
            geometry: &geometry,
            movement: Some(&movement),
            map: &map,
            heights: &heights,
            ours: 1,
            show_allied: false,
            links: &links,
            labels: None,
        },
        &frame,
    )
    .unwrap();
    let report = &field.report;
    assert_eq!(report.drawn, 1);
    assert_eq!(report.per_kind.get("paradrop arrow"), Some(&1));
    assert_eq!(report.no_arrow, 1, "{report:?}");
    assert_eq!(
        report.invasion_refused,
        BTreeMap::from([("invasion movement content is unavailable".into(), 1)])
    );
    assert_eq!(
        report.refused,
        BTreeMap::from([
            ("order path province without a map centroid", 1),
            ("front binding outside its recorded section", 1),
            ("order not reached from its group's recorded roots", 1),
            ("order of a type or binding no arrow is known for", 1),
            ("group plan whose orders group is not recorded", 1),
        ]),
    );
    let floor = heights.native_height(0.0, 0.0);
    let top = field
        .vertices
        .iter()
        .map(|vertex| vertex.position[1])
        .fold(f32::MIN, f32::max);
    assert!(
        top > floor + 2.5 && top <= floor + 3.0 + 1e-4,
        "{floor} {top}"
    );
    assert!(
        report
            .warnings()
            .iter()
            .any(|line| line.contains("invasion")),
        "{:?}",
        report.warnings()
    );
}

#[test]
fn selected_texture_index_never_substitutes_a_missing_unassigned_resource() {
    let mut arrow = ArrowType {
        name: "modded".into(),
        ..Default::default()
    };
    arrow.masks = vec!["assigned.dds".into(), "unassigned.dds".into()];
    arrow.patterns = vec!["assigned.dds".into(), "unassigned.dds".into()];
    assert_eq!(texture_choice(&arrow, false), Ok(0));
    assert_eq!(texture_choice(&arrow, true), Ok(1));
    arrow.patterns.pop();
    assert_eq!(texture_choice(&arrow, false), Ok(0));
    assert!(texture_choice(&arrow, true).is_err());
    arrow.patterns.push("unassigned.dds".into());
    arrow.masks.pop();
    assert!(texture_choice(&arrow, true).is_err());
    arrow.masks.clear();
    assert!(texture_choice(&arrow, false).is_err());
}

#[test]
fn assignment_count_preserves_virtual_and_empty_paths_but_requires_demanded_hq() {
    let mut plan = ArmyOrderPlanRow {
        assigned_divisions: vec![10, 11, 10],
        ..Default::default()
    };
    assert!(assigned_count(&plan, None).is_err());
    plan.virtual_order = 1;
    assert_eq!(assigned_count(&plan, None), Ok(0));
    plan.virtual_order = 0;
    let mut group = OrdersGroupRow {
        leader: 20,
        hq_provider: 10,
        ..Default::default()
    };
    assert_eq!(assigned_count(&plan, Some(&group)), Ok(1));
    group.leader = 0;
    assert_eq!(assigned_count(&plan, Some(&group)), Ok(3));
    plan.assigned_divisions.clear();
    assert_eq!(assigned_count(&plan, None), Ok(0));
}

#[test]
fn own_order_count_preserves_decoded_admission_and_table_gaps() {
    use replay_data::{rows::CountryLinkRow, rows::TableRows};
    use replay_frame::frame::TableView;

    let mut frame = DecodedFrame {
        branch: 1,
        hour: 42,
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
        }),
        metrics: Default::default(),
    };
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        [(0, 1), (11, 1), (12, 1), (21, 2), (22, 2), (30, 0), (31, 0)]
            .into_iter()
            .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
            .collect(),
    ));
    let links = table_gate::rows::<tables::CountryLinks>(&frame)
        .unwrap()
        .rows()
        .iter()
        .map(|link| (link.id, link.canonical_slot))
        .collect();
    *frame.tables[TableId::OrdersGroups.index()] =
        TableView::Complete(TableRows::OrdersGroups(vec![
            group(100, 11, 10, Some(0)),
            group(200, 21, 20, Some(0)),
            group(300, 77, 10, Some(1)),
            group(400, 11, 99, Some(0)),
            group(500, 11, 30, Some(0)),
            group(600, 11, 20, Some(1)),
            group(700, 0, 40, Some(0)),
            group(800, 30, 50, Some(0)),
        ]));
    *frame.tables[TableId::LandTheatres.index()] =
        TableView::Complete(TableRows::LandTheatres(vec![
            theatre(10, 12, 0),
            theatre(20, 22, 0),
            theatre(30, 88, 0),
            theatre(40, 0, 0),
            theatre(50, 31, 1),
        ]));
    *frame.tables[TableId::OrdersGroupPlans.index()] =
        TableView::Complete(TableRows::OrdersGroupPlans(
            [100, 200, 300, 400, 500, 600, 700, 800, 900]
                .into_iter()
                .map(|group| OrdersGroupPlanRow {
                    group,
                    primary_orders: vec![1],
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::ArmyOrders.index()] = TableView::Complete(TableRows::ArmyOrders(vec![
        order(TYPE_MOVE, 100, 1, &[]),
        order(TYPE_LINE, 100, 2, &[]),
        order(TYPE_MOVE, 200, 1, &[]),
        order(TYPE_MOVE, 300, 1, &[]),
        order(TYPE_MOVE, 400, 1, &[]),
        order(TYPE_MOVE, 500, 1, &[]),
        order(TYPE_MOVE, 600, 1, &[]),
        order(TYPE_MOVE, 700, 1, &[]),
        order(TYPE_MOVE, 800, 1, &[]),
        order(TYPE_MOVE, 900, 1, &[]),
    ]));
    assert_eq!(own_order_count(&frame, &links, 1).unwrap(), 2);
    assert_eq!(own_order_count(&frame, &links, 0).unwrap(), 2);

    let complete_orders = std::mem::replace(
        &mut *frame.tables[TableId::ArmyOrders.index()],
        TableView::Truncated {
            rows: TableRows::ArmyOrders(vec![
                order(TYPE_MOVE, 100, 1, &[]),
                order(TYPE_LINE, 100, 2, &[]),
            ]),
            missing: 8,
        },
    );
    assert_eq!(own_order_count(&frame, &links, 1).unwrap(), 2);
    assert!(table_gate::limitations(&frame).lines().iter().any(|line| line ==
        "Table ArmyOrders truncated (8 missing rows): the battle plan covers only recorded orders; missing orders are not drawn"));
    *frame.tables[TableId::ArmyOrders.index()] = TableView::Unavailable {
        reason: "order census refused".into(),
    };
    assert_eq!(own_order_count(&frame, &links, 1).unwrap(), 0);
    assert!(table_gate::limitations(&frame).lines().iter().any(|line| line ==
        "Table ArmyOrders unavailable: order census refused: the battle plan covers only recorded orders; missing orders are not drawn"));
    *frame.tables[TableId::ArmyOrders.index()] = complete_orders;
    let complete_roots = std::mem::replace(
        &mut *frame.tables[TableId::OrdersGroupPlans.index()],
        TableView::Unavailable {
            reason: "group roots refused".into(),
        },
    );
    assert_eq!(own_order_count(&frame, &links, 1).unwrap(), 0);
    assert!(table_gate::limitations(&frame).lines().iter().any(|line| line ==
        "Table OrdersGroupPlans unavailable: group roots refused: battle-plan colour and root order lists; absent colours are not guessed and a missing group's orders stay unresolved"));
    *frame.tables[TableId::OrdersGroupPlans.index()] = complete_roots;
    *frame.tables[TableId::ArmyOrders.index()] = TableView::Complete(TableRows::Clock(Vec::new()));
    let error = own_order_count(&frame, &links, 1).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        error.to_string(),
        "army orders holds rows of another kind"
    );
}
