use super::*;

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

#[test]
fn native_arrow_tail_preserves_opacity_length_and_scale_offsets() {
    let slot = Slot {
        shadow_factor: 1.,
        normal_map_factor: 1.,
        total_length: 73.,
        uv_to_absolute_length: 19.,
        seethrough_opacity: 0.2,
        ..Default::default()
    };
    let bytes = bytemuck::bytes_of(&slot);
    assert_eq!(&bytes[104..108], &0.2_f32.to_le_bytes());
    assert_eq!(&bytes[108..112], &73_f32.to_le_bytes());
    assert_eq!(&bytes[112..116], &19_f32.to_le_bytes());
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
    // The first definition of a name wins and keeps its bucket.
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
    // The constructor's text defaults.
    assert_eq!(line.text_size, 1.0);
    assert_eq!(line.text_offset, 0.0);
}

#[test]
fn the_global_scale_ramps_from_the_minimum_to_one() {
    let types = types();
    let arrow = types.get("move_thick").unwrap();
    assert!((arrow.global_scale(0) - 0.1).abs() < 1e-6);
    assert!((arrow.global_scale(40) - 1.0).abs() < 1e-6);
    assert!((arrow.global_scale(80) - 1.0).abs() < 1e-6);
    let line = types.get("offence_line").unwrap();
    assert_eq!(line.global_scale(1), 1.0);
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

#[test]
fn a_bound_line_is_identified_by_its_binding_and_styled_by_the_owner() {
    let line = order(TYPE_LINE, 9, 1, &[11, 12]);
    let plan = ArmyOrderPlanRow {
        front_valid: 255,
        owner_override: 8,
        ..Default::default()
    };
    assert_eq!(classify(&line, Some(&plan)), Classified::FrontLine);
    // The bound style ignores the override; the unbound one does not.
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
    // A negative start or end, or an end past the section, is the arrow
    // the native destroys: no arrow, not a refusal.
    for (start, end) in [(-1, 1), (0, -1), (0, 2), (3, 2)] {
        plan.segment_start = start;
        plan.segment_end = end;
        assert_eq!(front_pairs(&plan, &front), Some(FrontSpan::Destroyed));
    }
    // A start after the end, even past the section, is an empty span
    // (14123EEF0 tests only the signs and the end).
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
    // With several successors the sole-child gate does not apply, and the
    // repeated visits reach geometry preparation.
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
    let theatres = [
        LandTheatreRow {
            id: 90,
            owner: 1,
            ordinal: 0,
            ..Default::default()
        },
        LandTheatreRow {
            id: 10,
            owner: 1,
            ordinal: 1,
            ..Default::default()
        },
    ];
    let group = |handle, theatre, ordinal, is_army_group| OrdersGroupRow {
        handle,
        country: 1,
        theatre,
        theatre_ordinal: ordinal,
        is_army_group,
        ..Default::default()
    };
    let mut groups = [
        group(1, 10, Some(0), 0),
        group(2, 90, Some(0), 0),
        group(3, 90, Some(1), 1),
        group(4, 90, Some(0), 1),
        group(5, 90, None, 0),
    ];
    let roots: Vec<_> = groups
        .iter()
        .map(|group| OrdersGroupPlanRow {
            group: group.handle,
            ..Default::default()
        })
        .collect();
    let indexed = groups.iter().map(|group| (group.handle, group)).collect();
    let links = BTreeMap::from([(1, 1)]);
    let mut refused = BTreeMap::new();
    let ordered = ordered_group_roots(&indexed, &theatres, &roots, &links, 1, &mut refused);
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
            1,
            &mut refused
        )
        .is_empty(),
        "a raw country reference is not an unverified canonical slot"
    );
    groups[2].theatre_ordinal = Some(0);
    let indexed = groups.iter().map(|group| (group.handle, group)).collect();
    let mut refused = BTreeMap::new();
    let ordered = ordered_group_roots(&indexed, &theatres, &roots, &links, 1, &mut refused);
    assert_eq!(ordered.len(), 3);
    assert_eq!(refused["duplicate theatre roster position"], 1);

    // A group plan whose group row is missing is counted, not skipped.
    let orphan = [OrdersGroupPlanRow {
        group: 99,
        ..Default::default()
    }];
    let mut refused = BTreeMap::new();
    assert!(ordered_group_roots(&indexed, &theatres, &orphan, &links, 1, &mut refused).is_empty());
    assert_eq!(refused["group plan whose orders group is not recorded"], 1);
}

#[test]
fn every_refusal_reaches_the_warning_area() {
    assert!(OrderReport::default().warnings().is_empty());
    let report = OrderReport {
        refused: BTreeMap::from([
            ("front whose border chain could not be built", 2),
            ("order not reached from its group's recorded roots", 1),
        ]),
        unavailable: Some("no movement map".into()),
        no_arrow: 3,
        too_short: 4,
        labels_unfitted: 5,
        ..Default::default()
    };
    let warnings = report.warnings();
    assert_eq!(
        warnings[..3],
        [
            "Battle plans are not drawn: no movement map.",
            "Battle plan: 2 refused, front whose border chain could not be built.",
            "Battle plan: 1 refused, order not reached from its group's recorded roots.",
        ]
    );
    assert!(warnings[3].starts_with("Battle plan: 3 orders the game draws no arrow for"));
    assert!(warnings[4].starts_with("Battle plan: 4 orders with fewer than two"));
    assert!(warnings[5].starts_with("Battle plan: 5 labels fit no name variant"));
    let controls = OrderReport {
        labels_refused: BTreeMap::from([(
            "it holds the control byte 0x14, which map text does not draw".to_owned(),
            2,
        )]),
        ..Default::default()
    };
    assert_eq!(
        controls.warnings(),
        [
            "Battle plan: 2 labels are not drawn: it holds the control byte 0x14, which map text does not draw."
        ]
    );
}

#[test]
fn the_culling_offset_flips_only_against_a_declared_direction() {
    assert_eq!(culling_offset(1, 1), 0.0);
    assert_eq!(culling_offset(1, -1), 1.0);
    // The arrow constructor's zero side against a declared direction.
    assert_eq!(culling_offset(1, 0), 1.0);
    assert_eq!(culling_offset(0, -1), 0.0);
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
        // A culling flip changes the texture side, never the geometry.
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
fn province_geometry_keeps_integer_centroids_and_directed_borders() {
    let map = MapData {
        width: 4,
        height: 2,
        province_ids: vec![1, 1, 2, 2, 1, 1, 2, 2],
        definitions: BTreeMap::new(),
        country_colors: Default::default(),
    };
    let geometry = ProvinceGeometry::build(&map);
    assert_eq!(geometry.centroid(1), Some([0.0, 0.0]));
    assert_eq!(geometry.centroid(2), Some([2.0, 0.0]));
    assert_eq!(geometry.provinces(), 2);
    // Province 1's own pixels facing 2 (x = 1), and the wrap seam (x = 0).
    let towards_two = &geometry.borders[&(1, 2)];
    assert!(towards_two.iter().all(|p| p[0] == 1 || p[0] == 0));
    assert!(
        geometry.borders[&(2, 1)]
            .iter()
            .all(|p| p[0] == 2 || p[0] == 3)
    );
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
    // 141254C00 truncates each fixed5 product before converting to float.
    assert_eq!(field.vertices[2].position[0], 10.00002);
    assert_eq!(field.vertices[2].position[1], 1.0);
    assert_eq!(field.vertices[4].position[1], 4.0);
    assert_eq!(field.vertices[2].uv_head_variant[0], 10.00002 / 10.0);
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

    // Ground rising towards +z: the head base follows the cut pair's 3D
    // edge, (0, 2, 2) normalised, at the head's half width either side.
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

#[test]
fn the_slot_matches_the_shaders_constant_buffer() {
    assert_eq!(std::mem::size_of::<Slot>(), 128);
    assert_eq!(
        std::mem::size_of::<super::super::arrow_shader::Vertex>(),
        32
    );
}

/// Every character is 10 font pixels square.
struct Square;

impl crate::map::names::GlyphMetrics for Square {
    fn advance(&self, _: char) -> Option<f32> {
        Some(10.0)
    }
    fn line_height(&self) -> f32 {
        20.0
    }
    fn base(&self) -> f32 {
        16.0
    }
    fn glyph(&self, _: char) -> Option<crate::map::names::GlyphBox> {
        Some(crate::map::names::GlyphBox {
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

/// The label operands of the bench: a named move order and an army.
fn bench_localisation() -> installed_content::localisation::Localisation {
    [
        (
            "MAP_ARROWS_LOCALIZATION",
            "$ORDERNAME$$DIVISIONS$ - $GROUP$",
        ),
        ("MAP_ARROWS_LOCALIZATION_EMPTY", "No divisions"),
        ("MAP_ARROWS_LOCALIZATION_EMPTY_SHORT", "No Divs"),
        ("MAP_ARROWS_DIVISIONS_COUNT", "$DIV$ Divs"),
        ("MAP_ARROWS_DIVISIONS_COUNT_SHORT", "$DIV$"),
        ("MAP_ARROWS_PARATROOPERS_COUNT", "$DIV$ Paras"),
        ("o_strat_offensive_ru", "Strategic Offensive Operation"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect()
}

/// A timing bench, not a check: the per-hour cost of a 1943-sized plan's
/// stored controls, its ribbons and its labels, each timed alone. [`build`]
/// pays all three on every hour whose [`TABLES`] changed. 80 fronts of 40 pairs, 80 moves
/// of 9 provinces and 80 offensive lines of 20 provinces (centroid
/// polylines) on a 100 x 100 province grid of 10-pixel cells; the moves'
/// full label is wider than their path, so their layout tries a second
/// variant. Run in release:
///
/// `cargo test --release -p viewer bench_synthetic_1943_plan -- --ignored --nocapture`
#[test]
#[ignore = "timing bench; run with --release -- --ignored --nocapture"]
fn bench_synthetic_1943_plan() {
    const SIDE: i32 = 100;
    const CELL: i32 = 10;
    const ROUNDS: usize = 200;
    let id = |x: i32, z: i32| (z * SIDE + x + 1) as u32;
    let mut neighbours: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    let mut borders: BTreeMap<(u32, u32), Vec<[i32; 2]>> = BTreeMap::new();
    for z in 0..SIDE {
        for x in 0..SIDE {
            let mut list = Vec::new();
            for (dx, dz) in [
                (0, -1),
                (-1, 0),
                (1, 0),
                (0, 1),
                (-1, -1),
                (1, -1),
                (-1, 1),
                (1, 1),
            ] {
                let (nx, nz) = (x + dx, z + dz);
                if !(0..SIDE).contains(&nx) || !(0..SIDE).contains(&nz) {
                    continue;
                }
                list.push(id(nx, nz));
                if dx == 0 || dz == 0 {
                    let pixels = (0..CELL)
                        .map(|i| match (dx, dz) {
                            (1, 0) => [x * CELL + CELL - 1, z * CELL + i],
                            (-1, 0) => [x * CELL, z * CELL + i],
                            (0, 1) => [x * CELL + i, z * CELL + CELL - 1],
                            _ => [x * CELL + i, z * CELL],
                        })
                        .collect();
                    borders.insert((id(x, z), id(nx, nz)), pixels);
                }
            }
            neighbours.insert(id(x, z), list);
        }
    }
    let centroid = |p: u32| {
        let index = p as i32 - 1;
        Some([
            (index % SIDE) * CELL + CELL / 2,
            (index / SIDE) * CELL + CELL / 2,
        ])
    };
    let adjacent = |a: u32, b: u32| neighbours.get(&a).is_some_and(|l| l.contains(&b));
    let kind = |a: u32, b: u32| adjacent(a, b).then_some(0u8);
    let chain_world = chain::ChainWorld {
        borders: &borders,
        kind: &kind,
        raster: None,
    };
    let smoothing = Smoothing {
        smoothness: 50_000,
        passes: 2,
    };
    let values = vec![100u8; 1000 * 1000];
    let heights = HeightField {
        width: 1000,
        height: 1000,
        values: &values,
    };
    let types = types();
    let arrow = types.get("offence_line").unwrap();

    // Every order's stored controls.
    let plan_controls = || {
        let mut plan: Vec<(i32, Vec<Point>)> = Vec::new();
        for index in 0..80 {
            let row = 5 + index;
            // A front along a row boundary.
            let pairs: Vec<chain::Pair> = (0..40)
                .map(|x| chain::Pair {
                    source: id(10 + x, row),
                    target: id(10 + x, row + 1),
                    units: 0,
                })
                .collect();
            let params = chain::ChainParams {
                subdivided: true,
                smoothing,
                extra_units: 0,
                offset: 150_000,
                probe_teeth: false,
            };
            let front = chain::build(&pairs, &chain_world, params).unwrap();
            // A move north onto a line one row up.
            let mut approach: Vec<u32> = (0..9).map(|k| id(30, row - 4 + k / 2)).collect();
            approach.dedup();
            let line: Vec<u32> = (0..20).map(|x| id(20 + x, row)).collect();
            // The line one row up, as its centroid polyline.
            let mut line_controls: Vec<Point> = line
                .iter()
                .filter_map(|&p| centroid(p).map(fixed::from_pixel))
                .collect();
            fixed::curve(&mut line_controls, smoothing, true);
            let mut arrow_controls: Vec<Point> = approach
                .iter()
                .filter_map(|&p| centroid(p).map(fixed::from_pixel))
                .collect();
            fixed::curve(&mut arrow_controls, smoothing, true);
            plan.extend([
                (TYPE_LINE, front.points),
                (TYPE_LINE, line_controls),
                (TYPE_MOVE, arrow_controls),
            ]);
        }
        plan
    };
    let ms = |duration: std::time::Duration| duration.as_secs_f64() * 1000.0;
    // Each round is one hour's work; the minimum and median over the
    // rounds are robust to other load on the machine.
    let summary = |mut rounds: Vec<std::time::Duration>| {
        rounds.sort();
        format!(
            "{:.3} ms minimum, {:.3} ms median of {}",
            ms(rounds[0]),
            ms(rounds[rounds.len() / 2]),
            rounds.len()
        )
    };

    let mut control_rounds = Vec::with_capacity(ROUNDS);
    for _ in 0..ROUNDS {
        let started = std::time::Instant::now();
        std::hint::black_box(plan_controls());
        control_rounds.push(started.elapsed());
    }
    let plan = plan_controls();

    // Ribbons alone, over the stored controls above.
    let ribbon_field = || {
        let mut field = OrderField::default();
        for (_, controls) in &plan {
            let _ = emit(&mut field, arrow, controls, 1.0, [1.0; 4], &|x, z, _| {
                ribbon_height(&heights, x, z)
            });
        }
        field
    };
    let mut ribbon_rounds = Vec::with_capacity(ROUNDS);
    for _ in 0..ROUNDS {
        let started = std::time::Instant::now();
        std::hint::black_box(ribbon_field());
        ribbon_rounds.push(started.elapsed());
    }
    let field = ribbon_field();

    // Labels alone, over the stored controls above.
    let loc = bench_localisation();
    let name = labels::OrderName {
        unique: b"South Sudetenland",
        postfix: b"o_strat_offensive_ru",
        ..Default::default()
    };
    let height = |x: f32, z: f32, _: f32| heights.native_height(x, z);
    let mut label_rounds = Vec::with_capacity(ROUNDS);
    let mut placed = 0usize;
    for _ in 0..ROUNDS {
        let started = std::time::Instant::now();
        placed = 0;
        for (order_type, controls) in &plan {
            let composer = labels::Composer {
                localisation: &loc,
                order_type: *order_type,
                name,
                group_name: b"Army 28",
                army_group: false,
                count: 7,
            };
            let variant = |variant| composer.drawn(variant);
            if let Some(label) =
                labels::place(&variant, controls, &Square, 2.0, 0.0, &height).expect("bench text")
            {
                placed += 1;
                std::hint::black_box(label);
            }
        }
        label_rounds.push(started.elapsed());
    }
    assert_eq!(field.draws.len(), plan.len());
    assert_eq!(placed, plan.len(), "every bench label fits some variant");
    println!(
        "synthetic 1943 plan: {} orders, {} vertices; per hour: controls {}; ribbons {}; labels {}",
        plan.len(),
        field.vertices.len(),
        summary(control_rounds),
        summary(ribbon_rounds),
        summary(label_rounds),
    );
}

/// One hour's plan through [`build`]: a paradrop drawn and lifted, and every
/// refusal and no-arrow case counted once per order.
#[test]
fn one_hour_builds_the_paradrop_and_counts_every_refusal_once() {
    use installed_content::movement::{Neighbour, ProvinceTerrain};
    use replay::{
        frame::{Metrics, TableView},
        rows::{ProvinceRow, TableRows},
    };
    // Provinces 1..=4 in a row of 2x2-pixel blocks, then 5 to keep the
    // hop from 1 to 4 off the map seam; 9 has no pixels.
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
        arrowType = { name = "paradrop" technique = "MapArrowNoHead" body_width = { 1 1 } texture_loop = 2 }
        arrowType = { name = "invasion" technique = "MapArrowNoHead" body_width = { 1 1 } texture_loop = 2 }
        arrowType = { name = "move_thick" technique = "MapArrowNoHead" body_width = { 1 1 } texture_loop = 2 }
        arrowType = { name = "defence_line" technique = "MapArrowNoHead" body_width = { 1 1 } texture_loop = 2 }
        "#,
    )
    .unwrap();
    let smoothing = Smoothing {
        smoothness: 50_000,
        passes: 1,
    };
    let assets = OrderAssets {
        types,
        pages: BTreeMap::new(),
        defines: OrderDefines {
            order_move: smoothing,
            order_front: smoothing,
            root_front: smoothing,
            snapped_off_front: smoothing,
            root_front_offset: 150_000,
            max_offsets: 4,
            ext_text_points_limit: 100,
            virtual_color: [1.0; 4],
            paradrop_height_to_len: 0.5,
            paradrop_height_max: 3.0,
        },
        raid_defines: Err("not read in this test".into()),
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
    let orders = vec![
        // 1: a paradrop from province 1 to 4, drawn and lifted.
        with(1, TYPE_PARADROP, &[4], 1),
        // 2: an invasion, refused by name.
        with(2, TYPE_INVASION, &[3], 1),
        // 3: a move through a province without a centroid, visited twice.
        with(3, TYPE_MOVE, &[2, 9], 0),
        // 4: a front bound to a front the hour does not record.
        with(4, TYPE_LINE, &[2], 0),
        // 5: a move no root reaches.
        with(5, TYPE_MOVE, &[2, 3], 0),
        // 6: a paradrop whose source is -1, visited twice: one no arrow.
        with(6, TYPE_PARADROP, &[4], u32::MAX),
        // 7: an order type no arrow is known for.
        with(7, 9, &[2], 0),
    ];
    let plans: Vec<ArmyOrderPlanRow> = orders
        .iter()
        .map(|order| ArmyOrderPlanRow {
            owner_group: GROUP,
            instance: order.instance,
            front: 77,
            front_valid: u8::from(order.instance == 4),
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
        // A group plan whose group row is missing.
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
        TableView::Complete(TableRows::OrdersGroups(vec![OrdersGroupRow {
            handle: GROUP,
            country: 1,
            theatre: 5,
            theatre_ordinal: Some(0),
            ..Default::default()
        }]));
    tables[TableId::LandTheatres.index()] =
        TableView::Complete(TableRows::LandTheatres(vec![LandTheatreRow {
            id: 5,
            owner: 1,
            ordinal: 0,
            ..Default::default()
        }]));
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
            raids: None,
            geometry: &geometry,
            movement: Some(&movement),
            map: &map,
            heights: &heights,
            ours: 1,
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
    // The paradrop's ribbon rises over flat ground by the sine lift: three
    // world units at the middle of a six-unit hop, capped by the maximum.
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
