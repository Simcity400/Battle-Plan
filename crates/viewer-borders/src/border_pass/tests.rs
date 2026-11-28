use super::*;
use content_reader_map::{adjacencies::SEA_CROSSING, map::{CountryColors, ProvinceDefinition}};

#[test]
fn a_covered_border_wakes_the_map_only_while_its_draw_pulses() {
    use crate::border_shader::{Batch, Border, Inputs, Run};
    use shader_pipeline::{Buffer, Compiled, Shader};
    use viewer_gpu::gpu_contract::{SCENE_FORMAT, SeaBed};

    viewer_scene_base::test_game::enter();
    let (_guard, device, queue) = viewer_gpu::test_support::gpu::device();
    let source = content_reader_gui::pdxshader::program::read_with(b"test.shader", &|path| {
        Some(if path == b"test.shader" {
            b"Effect border { }".to_vec()
        } else {
            Vec::new()
        })
    })
    .unwrap();
    let stage = |wgsl: &str, buffers| Shader {
        wgsl: wgsl.into(),
        buffers,
        clock: Default::default(),
        independent_draws: false,
        textures: Vec::new(),
        samplers: Vec::new(),
        inputs: Vec::new(),
        sampling_pairs: Vec::new(),
    };
    let vertex = stage(
        "@vertex fn main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
        let xy = array(vec2f(-1,-1), vec2f(1,-1), vec2f(0,1));
        return vec4f(xy[i],0,1); }",
        vec![],
    );
    let pixel = stage(
        "struct Inputs { selection: vec4f, clock: vec4f }
        @group(0) @binding(5) var<uniform> paint: Inputs;
        @fragment fn main() -> @location(0) vec4f {
            return vec4f(clamp(sin(paint.clock.x), 0.0, 1.0) * paint.selection.x); }",
        vec![Buffer {
            binding: 5,
            size: 32,
            reads: vec![0..4, 16..20],
        }],
    );
    let effect = shader_pipeline::library::Effect::from_compiled(
        Arc::new(source),
        vec![],
        vec![Ok(Arc::new(Compiled { vertex, pixel }))],
    );
    let border = Border::new(&device, &effect, 1, SCENE_FORMAT, DEPTH_FORMAT).unwrap();
    let mut targets = Targets::new(&device, [8, 8], [8, 8], 1, SeaBed::Full);
    let arena = shader_pipeline::gpu::Arena::new(&device);
    let vertices = buffer(
        &device,
        "fixture border vertices",
        wgpu::BufferUsages::VERTEX,
        &[0; 48],
    )
    .unwrap();
    let indices = index_buffer(&device, &[0, 1, 2, 2, 1, 0]).unwrap();
    for (selection, live) in [([0.; 4], false), ([1.; 4], true), ([0.; 4], false)] {
        targets.surface_motion.begin(&device, true, 1, true);
        let mut encoder = device.create_command_encoder(&Default::default());
        let texture = &targets.refraction_view;
        border
            .draw(
                &arena,
                &mut encoder,
                &targets,
                Inputs {
                    fog: false,
                    retained: None,
                    frames: &[&[0; 416]],
                    snow_mud: texture,
                    intel: texture,
                    shadow: texture,
                    time: 1.25,
                },
                &[Batch {
                    texture,
                    alpha: 1.,
                    selection,
                    runs: vec![Run {
                        vertices: &vertices,
                        indices: &indices,
                        index_format: wgpu::IndexFormat::Uint16,
                        range: 0..6,
                    }],
                }],
                true,
            )
            .unwrap();
        targets.surface_motion.finish(&mut encoder);
        arena.flush(&queue);
        queue.submit([encoder.finish()]);
        targets.surface_motion.submitted();
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        targets.surface_motion.begin(&device, false, 1, true);
        assert_eq!(targets.surface_motion.live(), live);
    }
}

#[test]
fn faction_hover_color_uses_the_content_index_and_refuses_missing_quads() {
    let defines = content_reader_defines::defines::Defines::new().unwrap();
    defines
        .load(
            "fixture",
            br#"NDefines = {
        NMapMode = { FACTION_THEATER_HIGHLIGHT_COLOR_INDEX = 1 },
        NGraphics = { BORDER_COLOR_CUSTOM_HIGHLIGHTS = {
            0.1, 0.2, 0.3, 0.4, 0.7, 0.6, 0.5, 0.9
        } }
    }"#,
        )
        .unwrap();
    assert_eq!(
        faction_highlight_color(&defines).unwrap(),
        [0.7, 0.6, 0.5, 0.9]
    );
    for index in [-1, 2, 246] {
        defines
            .load(
                "fixture index",
                format!("NDefines.NMapMode.FACTION_THEATER_HIGHLIGHT_COLOR_INDEX = {index}")
                    .as_bytes(),
            )
            .unwrap();
        assert!(faction_highlight_color(&defines).is_err());
    }
}

fn map() -> MapData {
    let kinds = [
        (1, ProvinceKind::Land),
        (2, ProvinceKind::Land),
        (3, ProvinceKind::Land),
        (4, ProvinceKind::Sea),
        (5, ProvinceKind::Sea),
        (6, ProvinceKind::Lake),
    ];
    MapData {
        width: 6,
        height: 1,
        province_ids: vec![1, 2, 3, 4, 5, 6],
        definitions: kinds
            .into_iter()
            .map(|(id, kind)| {
                (
                    id,
                    ProvinceDefinition {
                        id,
                        rgb: [id as u8, 0, 0],
                        kind,
                        continent: 0,
                    },
                )
            })
            .collect(),
        country_colors: CountryColors::default(),
    }
}

fn state(id: u32, provinces: &[u32], impassable: bool) -> (u32, StateDefinition) {
    (
        id,
        StateDefinition {
            id,
            impassable,
            provinces: provinces.to_vec(),
            ..Default::default()
        },
    )
}

fn region(id: u32, provinces: &[u32]) -> (u32, StrategicRegion) {
    (
        id,
        StrategicRegion {
            id,
            name_key: format!("STRATEGICREGION_{id}"),
            provinces: provinces.to_vec(),
            naval_terrain: None,
            static_modifiers: false,
            static_modifier_names: Vec::new(),
            ..Default::default()
        },
    )
}

fn topology_of(adjacencies: &[Adjacency]) -> Topology {
    Topology::new(
        &map(),
        &[state(1, &[1, 2], false), state(2, &[3], true)]
            .into_iter()
            .collect(),
        &[region(10, &[1, 2, 3, 4, 6]), region(11, &[5])]
            .into_iter()
            .collect(),
        adjacencies,
    )
    .unwrap()
}

fn adjacency(from: u32, to: u32, kind: u8) -> Adjacency {
    Adjacency {
        from,
        to,
        kind,
        through: -1,
        start: (-1, -1),
        stop: (-1, -1),
        rule: None,
    }
}

#[test]
fn water_edges_take_the_sea_classes_and_a_coast_takes_none() {
    let topology = topology_of(&[]);
    assert_eq!(topology.static_class(4, 6), Some(BorderClass::Sea));
    assert_eq!(topology.static_class(4, 5), Some(BorderClass::SeaRegion));
    assert_eq!(topology.static_class(1, 4), None);
    assert_eq!(topology.static_class(3, 5), None);
    assert_eq!(topology.trace_class(1, 4), None);
    assert_eq!(topology.trace_class(3, 5), None);
    assert_eq!(topology.static_class(4, 4), None);
    assert_eq!(topology.trace_class(2, 2), None);
}

#[test]
fn impassable_comes_from_the_state_flag_and_from_an_adjacency_row() {
    let topology = topology_of(&[]);
    assert_eq!(topology.static_class(2, 3), Some(BorderClass::Impassable));
    assert_eq!(topology.static_class(1, 2), None);
    assert_eq!(topology.trace_class(1, 2), Some(BorderClass::Province));
    let rows = [adjacency(2, 1, IMPASSABLE)];
    assert_eq!(
        topology_of(&rows).static_class(1, 2),
        Some(BorderClass::Impassable)
    );
    let rows = [adjacency(1, 2, SEA_CROSSING)];
    assert_eq!(topology_of(&rows).static_class(1, 2), None);
    let rows = [adjacency(1, 2, IMPASSABLE), adjacency(2, 1, SEA_CROSSING)];
    assert_eq!(topology_of(&rows).static_class(1, 2), None);
    let rows = [adjacency(1, 2, SEA_CROSSING), adjacency(2, 1, IMPASSABLE)];
    assert_eq!(
        topology_of(&rows).static_class(1, 2),
        Some(BorderClass::Impassable)
    );
}

#[test]
fn isolated_land_is_hidden_before_authored_adjacencies_are_loaded() {
    let mut map = map();
    map.province_ids = vec![1, 4, 2, 5, 3, 6];
    assert_eq!(
        hidden_provinces(&map, 7),
        [false, true, true, true, false, false, false]
    );
    for kind in [SEA_CROSSING, IMPASSABLE] {
        let topology = Topology::new(
            &map,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &[adjacency(1, 2, kind)],
        )
        .unwrap();
        assert_eq!(topology.trace_class(1, 2), None);
        assert_eq!(topology.trace_class(1, 4), None);
        assert_eq!(topology.trace_class(4, 5), Some(BorderClass::Sea));
    }
    map.province_ids = vec![1, 4, 5, 6, 3, 2];
    assert!(!hidden_provinces(&map, 7)[1]);
    map.width = 2;
    map.height = 2;
    map.province_ids = vec![4, 1, 2, 5];
    let hidden = hidden_provinces(&map, 7);
    assert!(!hidden[1] && !hidden[2]);
}

fn regions() -> Regions {
    Regions {
        country_of: vec![0, 2, 1, 2, 2, 0].into(),
        owner_of: vec![0, 1, 1, 1, 2, 0].into(),
        state_of: vec![0, 7, 7, 8, 9, 0].into(),
        country_colors: Default::default(),
        warnings: Vec::new(),
        strategic_attributes: Default::default(),
        mode: None,
    }
}

#[test]
fn the_land_class_is_country_then_state_then_province() {
    let regions = regions();
    assert_eq!(land_class(&regions, 1, 2), BorderClass::Province);
    assert_eq!(land_class(&regions, 2, 3), BorderClass::State);
    assert_eq!(land_class(&regions, 3, 4), BorderClass::Country);
    assert_eq!(land_class(&regions, 1, 5), BorderClass::Country);
    assert_eq!(land_class(&regions, 1, 900), BorderClass::Country);
}

#[test]
fn the_quad_pattern_matches_the_engine_and_offsets_by_the_base_vertex() {
    let mut indices = Vec::new();
    run_indices(0, 3, &mut indices);
    assert_eq!(indices, [0, 2, 1, 2, 3, 1, 2, 4, 3, 4, 5, 3]);
    let mut offset = Vec::new();
    run_indices(10, 2, &mut offset);
    assert_eq!(offset, [10, 12, 11, 12, 13, 11]);
    let mut none = Vec::new();
    run_indices(0, 1, &mut none);
    run_indices(0, 0, &mut none);
    assert!(none.is_empty());
}

fn land(province: u32) -> bool {
    province != 9
}

fn strait(from: u32, to: u32, start: (i32, i32), stop: (i32, i32)) -> Adjacency {
    Adjacency {
        from,
        to,
        kind: SEA_CROSSING,
        through: 9,
        start,
        stop,
        rule: None,
    }
}

#[test]
fn a_strait_becomes_one_quad_on_the_water_plane_tiled_a_whole_number_of_times() {
    let rows = [strait(1, 2, (10, 4), (22, 4))];
    let quad = strait_quads(&rows, &grid(1, &[&[1]]), &land).unwrap();
    assert_eq!(quad.len(), 6);
    let xs = quad.iter().map(|v| v.position[0]).collect::<Vec<_>>();
    let zs = quad.iter().map(|v| v.position[2]).collect::<Vec<_>>();
    assert_eq!(xs.iter().cloned().fold(f32::MAX, f32::min), 10.0);
    assert_eq!(xs.iter().cloned().fold(f32::MIN, f32::max), 22.0);
    let (low, high) = (
        zs.iter().cloned().fold(f32::MAX, f32::min),
        zs.iter().cloned().fold(f32::MIN, f32::max),
    );
    assert!((high - low - STRAIT_WIDTH).abs() < 1e-4, "{low} .. {high}");
    assert!(((low + high) / 2.0 - 4.0).abs() < 1e-4);
    assert!(quad.iter().all(|v| v.position[1] == WATER_HEIGHT));
    let vs = quad.iter().map(|v| v.uv[1]).collect::<Vec<_>>();
    assert_eq!(vs.iter().cloned().fold(f32::MIN, f32::max), 3.0);
    assert!(quad.iter().all(|v| v.uv[0] == 0.0 || v.uv[0] == 1.0));
    let short = strait_quads(&[strait(1, 2, (10, 4), (11, 4))], &grid(1, &[&[1]]), &land).unwrap();
    assert_eq!(short.iter().map(|v| v.uv[1]).fold(f32::MIN, f32::max), 1.0);
}

fn grid(width: u32, rows: &[&[u32]]) -> MapData {
    MapData {
        width,
        height: rows.len() as u32,
        province_ids: rows.iter().flat_map(|row| row.iter().copied()).collect(),
        definitions: Default::default(),
        country_colors: CountryColors::default(),
    }
}

#[test]
fn records_with_a_through_on_land_become_quads() {
    let map = grid(1, &[&[1]]);
    let mut impassable = strait(3, 4, (1, 1), (3, 3));
    impassable.kind = IMPASSABLE;
    impassable.through = -1;
    let rows = [
        strait(1, 2, (1, 1), (3, 3)),
        impassable,
        adjacency(5, 6, 0),
        strait(7, 8, (5, 5), (5, 5)),
    ];
    assert_eq!(strait_quads(&rows, &map, &land).unwrap().len(), 12);
    let canal = [strait(9, 9, (1, 1), (3, 3))];
    assert!(strait_quads(&canal, &map, &land).unwrap().is_empty());
    let coastal = [strait(1, 9, (1, 1), (3, 3))];
    assert!(strait_quads(&coastal, &map, &land).unwrap().is_empty());
    let mut rule_only = strait(1, 2, (1, 1), (3, 3));
    rule_only.kind = 0;
    assert_eq!(strait_quads(&[rule_only], &map, &land).unwrap().len(), 6);
    let mut unset = strait(1, 2, (1, 1), (3, 3));
    unset.through = 0;
    assert!(strait_quads(&[unset], &map, &land).unwrap().is_empty());
}

#[test]
fn a_missing_end_is_the_nearest_pixel_and_the_higher_id_end_leads() {
    let map = grid(
        8,
        &[
            &[2, 2, 9, 9, 9, 5, 5, 9],
            &[2, 2, 9, 9, 9, 9, 5, 9],
            &[2, 9, 9, 9, 9, 9, 5, 9],
        ],
    );
    let mut derived = strait(2, 5, (0, 0), (0, 0));
    derived.start = (-1, -1);
    derived.stop = (0, 0);
    let quad = strait_quads(&[derived], &map, &land).unwrap();
    assert_eq!(quad.len(), 6);
    let (a0, a1) = (quad[0].position, quad[1].position);
    let centre = [(a0[0] + a1[0]) / 2.0, (a0[2] + a1[2]) / 2.0];
    assert_eq!(centre, [5.0, 2.0]);
    let b = [(quad[5].position[0] + quad[2].position[0]) / 2.0];
    assert_eq!(b, [1.0]);
    let mut half = strait(2, 5, (0, 0), (6, 0));
    half.start = (-1, 5);
    let quad = strait_quads(&[half], &map, &land).unwrap();
    let (a0, a1) = (quad[0].position, quad[1].position);
    assert_eq!([(a0[0] + a1[0]) / 2.0, (a0[2] + a1[2]) / 2.0], [6.0, 0.0]);
}

#[test]
fn touching_provinces_take_their_csv_ends_the_other_way_round() {
    let apart = grid(4, &[&[2, 9, 5, 9]]);
    let touching = grid(2, &[&[2, 5]]);
    let row = [strait(2, 5, (10, 0), (20, 0))];
    let lead = |map: &MapData| {
        let quad = strait_quads(&row, map, &land).unwrap();
        (quad[0].position[0] + quad[1].position[0]) / 2.0
    };
    assert_eq!(lead(&apart), 20.0);
    assert_eq!(lead(&touching), 10.0);
    assert_eq!(lead(&grid(3, &[&[2, 9, 5]])), 10.0);
}

#[test]
fn updated_raster_records_keep_their_raster_order() {
    let map = grid(4, &[&[2, 5, 3, 9]]);
    let rows = [
        strait(3, 5, (30, 0), (31, 0)),
        strait(2, 5, (20, 0), (21, 0)),
    ];
    let quad = strait_quads(&rows, &map, &land).unwrap();
    let lead = |at: usize| (quad[at].position[0] + quad[at + 1].position[0]) / 2.0;
    assert_eq!((lead(0), lead(6)), (20.0, 30.0));
}

#[test]
fn a_repeated_row_updates_one_pair_of_records() {
    let apart = grid(4, &[&[2, 9, 5, 9]]);
    let rows = [
        strait(2, 5, (10, 0), (20, 0)),
        strait(2, 5, (30, 0), (40, 0)),
    ];
    let quad = strait_quads(&rows, &apart, &land).unwrap();
    assert_eq!(quad.len(), 6);
    assert_eq!((quad[0].position[0] + quad[1].position[0]) / 2.0, 30.0);
}
