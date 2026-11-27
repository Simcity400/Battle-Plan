use content_reader_mesh::mesh::MeshProducts as _;
use crate::test_support::{MESHES_GFX, PLANE_ASSET, library_with_one_mesh, triangle_mesh, visual_stack};
use crate::*;
use std::collections::BTreeSet;
use viewer_gpu::camera::MapDefines;
use viewer_gpu::camera::MapSize;
use viewer_gpu::camera::Placement;

#[test]
fn library_growth_mid_intro_keeps_the_raid_units_history() {
    use entity_animation::{named_stage, stage_rig};
    let joint = |x: f32| {
        PaletteJoint::from_transform(&mesh::Transform::from_trs(
            [x, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
            1.0,
        ))
    };
    let mut rigs = vec![Arc::new(stage_rig(true))];
    let mut palette = vec![joint(0.0)];
    let dive = named_stage("dive", Some("dive_intro"));
    let raid = ModelInstance {
        animation: named_stage("cruise", None),
        animation_seed: 7,
        scale: 1.0,
        ..ModelInstance::default()
    };
    let field = |instance: ModelInstance| {
        ModelField::build(&ModelLibrary::default(), vec![(0, instance)], 256.0)
    };
    let mut players = BTreeMap::new();
    let mut events = Vec::new();
    let mut step = |players: &mut BTreeMap<_, _>,
                    rigs: &[Arc<Rig>],
                    palette: &[PaletteJoint],
                    animation: u32,
                    time: f32| {
        let mut field = field(ModelInstance { animation, ..raid });
        animate_players(
            players,
            rigs,
            palette,
            &mut field,
            time,
            &|_| true,
            &mut events,
        );
    };
    step(&mut players, &rigs, &palette, raid.animation, 10.0);
    step(&mut players, &rigs, &palette, dive, 11.0);
    assert_eq!(
        players[&(0, 7, 0)].root_playing(&rigs[0]),
        Some("dive_intro")
    );
    let mut grown = palette.clone();
    grown.push(joint(1.0));
    let mut appended = stage_rig(false);
    (appended.start, appended.end) = (1, 2);
    appended.nodes[0].rigid = 1;
    retain_players(
        &mut players,
        (&palette, rigs.len()),
        (&grown, rigs.len() + 1),
    );
    assert!(players.contains_key(&(0, 7, 0)));
    rigs.push(Arc::new(appended));
    palette = grown;
    step(&mut players, &rigs, &palette, dive, 12.0);
    assert_eq!(
        players[&(0, 7, 0)].root_playing(&rigs[0]),
        Some("dive_intro")
    );
    step(&mut players, &rigs, &palette, dive, 13.1);
    assert_eq!(players[&(0, 7, 0)].root_playing(&rigs[0]), Some("dive"));
    let changed = [joint(2.0), joint(1.0)];
    for after in [(&palette[..1], 2), (&changed[..], 2), (&palette[..], 1)] {
        let mut kept = BTreeMap::from([((0, 7, 0), entity_animation::Player::new(&rigs[0]))]);
        retain_players(&mut kept, (&palette, 2), after);
        assert!(kept.is_empty());
    }
    retain_players(&mut players, (&palette, 2), (&palette, 2));
    assert!(players.contains_key(&(0, 7, 0)));
}

#[test]
fn a_placed_raid_unit_continues_its_intro_and_retires_only_when_unplaced() {
    use entity_animation::{named_stage, stage_rig};
    let rigs = vec![Arc::new(stage_rig(true))];
    let rig = &rigs[0];
    let palette = vec![PaletteJoint::from_transform(&mesh::Transform::IDENTITY)];
    let at = |stage: u32| ModelInstance {
        animation: stage,
        animation_seed: 7,
        scale: 1.0,
        ..ModelInstance::default()
    };
    let cruise = named_stage("cruise", None);
    let dive = named_stage("dive", Some("dive_intro"));
    let mut players = BTreeMap::new();
    let mut events = Vec::new();
    let mut step = |players: &mut BTreeMap<_, _>, stage: u32, time: f32, shown: bool| {
        let mut field = ModelField::build(&ModelLibrary::default(), vec![(0, at(stage))], 256.0);
        animate_players(
            players,
            &rigs,
            &palette,
            &mut field,
            time,
            &|_| shown,
            &mut events,
        )
        .len()
    };
    assert_eq!(step(&mut players, cruise, 10.0, true), 2);
    step(&mut players, dive, 11.0, true);
    assert_eq!(players[&(0, 7, 0)].root_playing(rig), Some("dive_intro"));
    step(&mut players, dive, 12.9, true);
    assert_eq!(players[&(0, 7, 0)].root_playing(rig), Some("dive_intro"));
    step(&mut players, dive, 13.1, true);
    assert_eq!(players[&(0, 7, 0)].root_playing(rig), Some("dive"));
    assert_eq!(step(&mut players, dive, 13.5, false), 1);
    assert!(players.is_empty());
    step(&mut players, dive, 14.0, true);
    let mut gone = ModelField::default();
    animate_players(
        &mut players,
        &rigs,
        &palette,
        &mut gone,
        14.5,
        &|_| true,
        &mut events,
    );
    assert!(players.is_empty());
}

#[test]
fn an_empty_authored_actor_roster_keeps_the_content_stack() {
    let layers = ResolvedLayers::default();
    let mut warnings = Vec::new();
    let decode = Decode {
        force_pow2_textures: true,
    };
    let library =
        ModelLibrary::load(&layers, decode, &Descriptors::default(), &[], &mut warnings).unwrap();
    assert!(library.meshes.is_empty());
    assert!(library.layers.is_some());
    assert!(warnings.is_empty());
    assert_eq!(library.decode(), decode);
    assert_eq!(ModelPass::new(&library, "test").decode, decode);
    let mut empty = ModelLibrary::empty(decode);
    empty.merge(library);
    assert_eq!(empty.decode(), decode);
}

#[test]
fn a_pass_over_an_empty_library_takes_the_stack_of_its_later_bakes() {
    let decode = Decode::default();
    let mut library = ModelLibrary::empty(decode);
    let mut pass = ModelPass::new(&library, "unit models");
    assert!(pass.layers.is_none());
    let mut warnings = Vec::new();
    library.merge(
        ModelLibrary::load(
            &ResolvedLayers::default(),
            decode,
            &Descriptors::default(),
            &[],
            &mut warnings,
        )
        .unwrap(),
    );
    pass.replace_field(&library, &mut ModelField::default());
    assert!(pass.layers.is_some());
}

#[test]
fn merging_a_shard_rebases_every_palette_slot() {
    let joint = PaletteJoint::from_transform(&mesh::Transform::IDENTITY);
    let shard_mesh = ModelMesh {
        asset: crate::AssetKey::synthetic("shard"),
        name: "shard".to_owned(),
        path: "shard.mesh".to_owned(),
        scale: 1.0,
        lod_distances: Vec::new(),
        native: Vec::new(),
        radius: 0.0,
        height: 0.0,
        visual: false,
        own_radius: 0.0,
        bound: 0.0,
        convoy_radius: None,
        attachments: 0,
        cull_distance_squared: content_reader_mesh::mesh::DEFAULT_CULL_DISTANCE_SQUARED,
    };
    let mut shard = ModelLibrary {
        palette: vec![joint; 3],
        meshes: vec![shard_mesh],
        rigs: vec![Arc::new(Rig {
            nodes: vec![entity_animation::Node {
                native: None,
                mesh: Arc::new(mesh::Mesh::default()),
                clips: Vec::new(),
                default: "idle".to_owned(),
                follows_parent: true,
                parent: None,
                scale: 1.0,
                mesh_scale: 1.0,
                skin_base: Some(0),
                rigid: 1,
                locators: BTreeMap::new(),
            }],
            start: 0,
            end: 3,
        })],
        ..ModelLibrary::default()
    };
    shard.index.insert("shard".to_owned(), 0);
    let mut library = ModelLibrary {
        palette: vec![joint; 5],
        ..ModelLibrary::default()
    };
    library.merge(shard);
    let rig = &library.rigs[0];
    assert_eq!((rig.start, rig.end), (5, 8));
    assert_eq!(rig.nodes[0].rigid, 6);
    assert_eq!(rig.nodes[0].skin_base, Some(5));
    assert!((rig.start..rig.end).contains(&(rig.nodes[0].rigid as usize)));
}

#[test]
fn a_mesh_setting_overrides_one_shape_and_one_material_index() {
    let setting = |name: &str, index: i32| mesh::MeshSetting {
        name: name.to_owned(),
        index,
        ..mesh::MeshSetting::default()
    };
    assert!(setting("facility_landShape", 1).applies("facility_landShape", 1));
    assert!(!setting("facility_landShape", 1).applies("facility_landShape", 2));
    assert!(!setting("facility_landShape", 1).applies("collisionShape", 1));
    assert!(setting("", 0).applies("anyShape", 0));
}

#[test]
fn the_sprawl_gate_is_water_relative_and_independent_of_transient_map_objects() {
    let at = |height| {
        Placement {
            center: [0.0, 0.0],
            height,
            pose: viewer_gpu::camera::Pose::Map,
        }
        .on(MapSize::TEST, MapDefines::test())
    };
    let edge = viewer_gpu::camera::WATER_HEIGHT + MapDefines::test().sprawl_cutoff;
    assert!(!draws_city_sprawl(&at(302.960)));
    assert!(draws_city_sprawl(&at(50.178)));
    assert!(draws_city_sprawl(&at(edge - 0.1)));
    assert!(!draws_city_sprawl(&at(edge + 0.1)));
    let mut camera = at(50.178);
    camera.defines.map_objects_cutoff = 1.0;
    assert!(
        draws_city_sprawl(&camera),
        "The building update cutoff does not gate city-sprawl actors"
    );
}

#[test]
fn an_entity_locator_ignores_the_baked_pdxmesh_scale() {
    let entity_scale = 0.8;
    let mesh_scale = 0.14;
    let node = mesh::Transform::from_trs(
        [10.0, 0.0, 20.0],
        [0.0, 0.0, 0.0, 1.0],
        entity_scale * mesh_scale,
    );
    let mut locator = mesh::Transform::IDENTITY;
    locator.translation = [0.0, 1.1, -1.0];
    let world = node.compose(&entity_locator(&locator, mesh_scale));
    let at = world.transform_point([0.0; 3]);
    let expected = [10.0, 1.1 * entity_scale, 20.0 - entity_scale];
    for axis in 0..3 {
        assert!((at[axis] - expected[axis]).abs() < 1e-5, "{at:?}");
    }
    assert!((world.uniform_scale() - entity_scale).abs() < 1e-5);
}

#[test]
fn a_palette_joint_is_the_transform_transposed_into_rows() {
    let half = std::f32::consts::FRAC_PI_4;
    let transform =
        mesh::Transform::from_trs([1.0, 2.0, 3.0], [0.0, half.sin(), 0.0, half.cos()], 2.0);
    let joint = PaletteJoint::from_transform(&transform);
    for point in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.3, -0.4, 0.5]] {
        let want = transform.transform_point(point);
        let got = skinned(&[joint], point, &[(0, 1.0)], 0);
        assert!(
            got.iter()
                .zip(want)
                .all(|(value, want)| (value - want).abs() < 1e-5),
            "{got:?} against {want:?}"
        );
    }
}

#[test]
fn a_vertex_with_no_weight_keeps_its_own_position() {
    let point = [1.25, -0.5, 7.0];
    assert_eq!(skinned(&[], point, &[], 0), point);
    assert_eq!(skinned(&[PaletteJoint::IDENTITY], point, &[], 0), point);
}

#[test]
fn weights_blend_the_joints_they_name() {
    let left = PaletteJoint::from_transform(&mesh::Transform {
        translation: [-10.0, 0.0, 0.0],
        ..mesh::Transform::IDENTITY
    });
    let right = PaletteJoint::from_transform(&mesh::Transform {
        translation: [10.0, 0.0, 0.0],
        ..mesh::Transform::IDENTITY
    });
    let at = |weights: [f32; 4]| {
        skinned(
            &[left, right],
            [0.0, 1.0, 0.0],
            &[(0, weights[0]), (1, weights[1])],
            0,
        )
    };
    assert_eq!(at([1.0, 0.0, 0.0, 0.0]), [-10.0, 1.0, 0.0]);
    assert_eq!(at([0.0, 1.0, 0.0, 0.0]), [10.0, 1.0, 0.0]);
    assert_eq!(at([0.5, 0.5, 0.0, 0.0]), [0.0, 1.0, 0.0]);
    assert_eq!(
        skinned(&[left], [0.0, 1.0, 0.0], &[(0, 1.0), (99, 1.0)], 0),
        [-10.0, 1.0, 0.0]
    );
}

#[test]
fn a_posed_vertex_widens_the_reach_where_the_bind_box_would_not() {
    let mut reach = Reach::default();
    reach.widen(
        &mesh::Transform::IDENTITY,
        &mesh::Bounds {
            min: [-3.3, 0.0, -3.3],
            max: [3.3, 8.0, 3.3],
        },
        true,
    );
    let bound = reach.radius;
    reach.widen_point([8.0, 9.0, 0.0], true);
    assert!(reach.radius > bound);
    assert_eq!(reach.radius, 8.0);
    assert_eq!(reach.height, 9.0);
    assert_eq!(reach.own_radius, 8.0);
}

#[test]
fn culling_reaches_around_every_lod_a_model_may_draw() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_model_lod_reach_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let floats = |name: &str, values: &[f32]| {
        let mut out = vec![b'!', name.len() as u8];
        out.extend(name.as_bytes());
        out.push(b'f');
        out.extend((values.len() as u32).to_le_bytes());
        out.extend(values.iter().flat_map(|value| value.to_le_bytes()));
        out
    };
    let mut bytes = b"@@b@[object\0".to_vec();
    for (lod, extent) in [(0_i32, 1.0_f32), (1, 10.0)] {
        bytes.extend(format!("[[shape{lod}\0").as_bytes());
        bytes.extend(b"!\x03lodi\x01\0\0\0");
        bytes.extend(lod.to_le_bytes());
        bytes.extend(b"[[[mesh\0");
        bytes.extend(floats("p", &[0., 0., 0., 1., 0., 0., 0., 1., 0.]));
        bytes.extend(floats("n", &[0., 1., 0., 0., 1., 0., 0., 1., 0.]));
        bytes.extend(b"!\x03trii\x03\0\0\0\0\0\0\0\x01\0\0\0\x02\0\0\0");
        bytes.extend(b"[[[[aabb\0");
        bytes.extend(floats("min", &[-extent, 0., -extent]));
        bytes.extend(floats("max", &[extent, 1., extent]));
        bytes.extend(b"[[[[material\0!\x06shaders\x01\0\0\0\x0e\0\0\0AuthoredShader");
    }
    std::fs::write(root.join("lods.mesh"), bytes).unwrap();
    let descriptors = mesh::parse_descriptors(
        br#"
            objectTypes = { pdxmesh = { name = body file = "lods.mesh" scale = 1 } }
            entity = { name = tower pdxmesh = body state = { name = idle } }
        "#,
        "fixture.asset",
    )
    .unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    let mut warnings = Vec::new();
    let library = ModelLibrary::load(
        &layers,
        Decode::default(),
        &descriptors,
        &["tower".into()],
        &mut warnings,
    )
    .unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    let tower = &library.meshes[library.slot("tower").unwrap()];
    assert_eq!(tower.native[0].lods.len(), 2);
    assert!((tower.radius - 200f32.sqrt()).abs() < 1e-3, "{}", tower.radius);
    assert!((tower.own_radius - 200f32.sqrt()).abs() < 1e-3);
}

#[test]
fn custom_material_source_reaches_native_geometry() {
    let layers = ResolvedLayers::default();
    let mut library = library_with_one_mesh(1.);
    let definition = mesh::MeshDefinition {
        scale: 1.,
        ..Default::default()
    };
    let resolved = mesh::ResolvedMesh {
        definition: &definition,
        scale: 1.,
    };
    let decoded = mesh::Mesh {
        shapes: vec![mesh::Shape {
            name: "shape".into(),
            lod: None,
            meshes: vec![mesh::SubMesh {
                positions: vec![[0.; 3]; 3],
                normals: vec![[0., 1., 0.]; 3],
                tangents: vec![[1., 0., 0., 1.]; 3],
                uv0: vec![[0.; 2]; 3],
                indices: vec![0, 1, 2],
                material: Some(mesh::Material {
                    shader: Some("AuthoredTechnique".into()),
                    shader_file: Some("gfx/FX/authored.lua".into()),
                    ..Default::default()
                }),
                ..Default::default()
            }],
        }],
        ..Default::default()
    };
    let descriptor = native::descriptor(
        &decoded,
        &resolved,
        0,
        "synthetic_actor",
        |_, sub| {
            Ok(viewer_scene_base::mesh_shader::ModelGeometry::held(viewer_scene_base::mesh_shader::Geometry::from_mesh(
                sub, 1.,
            )?))
        },
        |shape, sub| library.material(&layers, "gfx/models", shape, sub, &resolved),
    )
    .unwrap();
    let part = &descriptor.lods[0][0];
    assert_eq!(part.material.shader.as_deref(), Some("AuthoredTechnique"));
    assert_eq!(
        part.material.shader_file.as_deref(),
        Some("gfx/FX/authored.lua")
    );
    assert_eq!((part.geometry.load)(&mut None).unwrap().indices, [0, 1, 2]);
    assert_eq!(library.materials.len(), 1);
}

fn aircraft_callbacks_with_geometry(meshed: bool) {
    let root = std::env::temp_dir().join(format!(
        "hoi4_aircraft_callback_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let mut bytes = b"@@b@[object\0[[frame\0[[[mesh\0".to_vec();
    for (name, values) in [
        ("p", [0_f32, 0., 0., 1., 0., 0., 0., 1., 0.]),
        ("n", [0., 1., 0., 0., 1., 0., 0., 1., 0.]),
    ] {
        bytes.extend([b'!', name.len() as u8]);
        bytes.extend(name.as_bytes());
        bytes.push(b'f');
        bytes.extend((values.len() as u32).to_le_bytes());
        bytes.extend(values.into_iter().flat_map(f32::to_le_bytes));
    }
    bytes.extend(b"!\x03trii\x03\0\0\0\0\0\0\0\x01\0\0\0\x02\0\0\0");
    bytes.extend(b"[[[[material\0!\x06shaders\x01\0\0\0\x0e\0\0\0AuthoredShader");
    bytes.extend(b"[[[skeleton\0[[[[mount\0!\x02ixi\x01\0\0\0\0\0\0\0!\x02txf\x0c\0\0\0");
    bytes.extend(
        [1_f32, 0., 0., 0., 1., 0., 0., 0., 1., -1., 0., 0.]
            .into_iter()
            .flat_map(f32::to_le_bytes),
    );
    std::fs::write(root.join("plane.mesh"), bytes).unwrap();
    let binding = if meshed { "pdxmesh = body" } else { "" };
    let descriptors = mesh::parse_descriptors(format!(r#"
            objectTypes = {{ pdxmesh = {{ name = body file = "plane.mesh" scale = 1 }} }}
            entity = {{ name = plane {binding} scale = 2
                locator = {{ name = mount position = {{ 1 0 0 }} }}
                attach = {{ mount = child }}
                state = {{ name = idle }}
                state = {{ name = supply animation_speed = 2
                    event = {{ time = 0.2 node = mount particle = drop light = lamp trigger_once = yes keep_particle = yes }}
                    event = {{ sound = {{ soundeffect = hum }} }}
                }}
            }}
            entity = {{ name = child
                state = {{ name = idle }}
                state = {{ name = supply event = {{ time = 0.1 particle = glow }} }}
            }}
            entity = {{ name = scenario state = {{ name = default
                event = {{ time = 2 id = a_1_supply }}
                event = {{ time = 2.05 id = a_1_supply }}
                event = {{ time = 3 id = a_1_idle }}
                event = {{ time = 4 id = d_supply }}
                event = {{ time = 5 id = a_2_supply }}
                event = {{ time = 6 id = a_9_supply }}
            }} }}
        "#).as_bytes(), "fixture.asset").unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    let mut warnings = Vec::new();
    let library = ModelLibrary::load(
        &layers,
        Decode::default(),
        &descriptors,
        &["plane".into()],
        &mut warnings,
    )
    .unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    let slot = library.slot("plane").unwrap();
    assert_eq!(library.meshes[slot].native.is_empty(), !meshed);
    assert!(warnings.iter().all(|(_, line)| !line.contains("sound")));
    if !meshed {
        assert!(warnings.iter().any(|(kind, line)| *kind
            == content_reader_parser::DiagnosticKind::Warning
            && line.contains("geometry-less entities")));
    }
    let scenario = descriptors.inherit("scenario").unwrap();
    let mut requests = crate::aircraft_events::scenario_requests(
        &scenario.states["default"][0].events,
        2.,
        library.rig(slot),
        2,
        0,
    );
    for request in &mut requests {
        request.time = (2. + request.time) / 2.;
    }
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[0].time, 1.5);
    let requests = Arc::new(requests);
    let instance = ModelInstance {
        position: [10., 20., 30.],
        scale: 1.,
        animation_seed: 7,
        animation_epoch: 1,
        native_flags: 4,
        native_basis: [[0., 1., 0.], [-1., 0., 0.], [0., 0., 1.]],
        ..Default::default()
    };
    let mut pass = ModelPass::new(&library, "flying aircraft");
    pass.update_placed(vec![(slot, instance)], 128.);
    let key = (slot, 7, 1);
    pass.field.clocks.insert(
        key,
        entity_animation::Clock {
            time: 1.4,
            phase: None,
            requests,
        },
    );
    pass.animate(100., None, &[0.]);
    assert!(pass.events.is_empty());
    let check_supply = |pass: &ModelPass| {
        let drops: Vec<_> = pass
            .events
            .iter()
            .filter(|event| event.definition.particle.is_some())
            .collect();
        assert_eq!(drops.len(), 2);
        assert_eq!(drops[0].definition.particle.as_deref(), Some("drop"));
        assert_eq!(drops[0].definition.light.as_deref(), Some("lamp"));
        let drop_position = if meshed {
            [10., 20., 30.]
        } else {
            [10., 22., 30.]
        };
        assert_eq!(drops[0].transform.translation, drop_position);
        assert!((drops[0].age - 0.05).abs() < 1e-5, "{}", drops[0].age);
        assert_eq!(drops[1].definition.particle.as_deref(), Some("glow"));
        assert_eq!(drops[1].transform.translation, [10., 22., 30.]);
    };
    pass.field.clocks.get_mut(&key).unwrap().time = 1.65;
    pass.animate(900., None, &[0.]);
    check_supply(&pass);
    pass.reset_animation();
    pass.animate(900., None, &[0.]);
    check_supply(&pass);
    pass.field.clocks.get_mut(&key).unwrap().time = 1.8;
    pass.animate(901., None, &[0.]);
    assert_eq!(pass.events.len(), 1);
    assert_eq!(pass.events[0].definition.particle.as_deref(), Some("drop"));
    assert!((pass.events[0].emission_end.unwrap() - 0.15).abs() < 1e-5);
    assert!((pass.events[0].age - 0.2).abs() < 1e-5);
    pass.field.clocks.get_mut(&key).unwrap().time = 2.65;
    pass.animate(902., None, &[0.]);
    assert_eq!(
        pass.events
            .iter()
            .filter(|event| event.emission_end.is_none() && event.definition.particle.is_some())
            .count(),
        2
    );
    assert_eq!(
        pass.events
            .iter()
            .filter(|event| event.emission_end.is_some())
            .count(),
        1
    );
    let seeds: BTreeSet<_> = pass
        .events
        .iter()
        .filter(|event| event.definition.particle.as_deref() == Some("drop"))
        .map(|event| event.seed)
        .collect();
    assert_eq!(seeds.len(), 2);
    pass.field.clocks.get_mut(&key).unwrap().time = 0.4;
    pass.animate(903., None, &[0.]);
    assert!(pass.events.is_empty());
}

#[test]
fn aircraft_scenario_callbacks_animate_a_meshed_actor() {
    aircraft_callbacks_with_geometry(true);
}

#[test]
fn aircraft_scenario_callbacks_animate_a_meshless_actor() {
    aircraft_callbacks_with_geometry(false);
}

#[test]
fn meshless_buildings_use_shared_states_events_and_attachments() {
    let mut descriptors = mesh::parse_descriptors(
        br#"
            entity = { name = emitter scale = 2
                locator = { name = mount position = { 1 0 0 } }
                attach = { mount = child }
                attach = { absent = child }
                state = { name = idle
                    event = { particle = smoke light = lamp }
                    event = { sound = { soundeffect = hum } }
                }
                state = { name = damaged event = { particle = fire } }
            }
            entity = { name = child
                state = { name = idle event = { particle = glow } }
                state = { name = damaged event = { particle = sparks } }
            }
        "#,
        "fixture.asset",
    )
    .unwrap();
    descriptors
        .entities
        .get_mut("emitter")
        .unwrap()
        .unbound_mesh = Some("lost".into());
    let mut warnings = Vec::new();
    let library = ModelLibrary::load(
        &ResolvedLayers::default(),
        Decode::default(),
        &descriptors,
        &["emitter".into()],
        &mut warnings,
    )
    .unwrap();
    let slot = library.slot("emitter").unwrap();
    assert!(library.meshes[slot].native.is_empty());
    assert_eq!(library.meshes[slot].attachments, 2);
    assert!(
        warnings[0]
            .1
            .contains("entity emitter references pdxmesh lost")
    );
    assert!(
        warnings.iter().all(|(_, line)| !line.contains("sound")),
        "{warnings:?}"
    );
    let mut pass = ModelPass::new(&library, "buildings");
    let placed = ModelInstance {
        position: [3., 4., 5.],
        scale: 3.,
        animation: entity_animation::BUILDING_IDLE,
        animation_epoch: 1,
        ..Default::default()
    };
    pass.update_placed(vec![(slot, placed)], 128.);
    pass.animate(20., None, &[0.]);
    assert_eq!(pass.events.len(), 4);
    let effect = &pass.events[0];
    assert_eq!(effect.definition.particle.as_deref(), Some("smoke"));
    assert_eq!(effect.definition.light.as_deref(), Some("lamp"));
    assert_eq!(effect.transform.translation, placed.position);
    assert_eq!(effect.transform.uniform_scale(), 6.);
    assert_eq!(effect.age, 0.);
    assert_eq!(pass.events[2].definition.particle.as_deref(), Some("glow"));
    assert_eq!(pass.events[2].transform.translation, [9., 4., 5.]);
    assert_eq!(pass.events[3].transform.translation, placed.position);
    let mut names = crate::sound_events::Names::default();
    let mut sounds = Vec::new();
    crate::sound_events::collect(
        &pass.events,
        &mut names,
        &mut sounds,
        crate::entity_animation::Event::in_combat,
    );
    assert_eq!(sounds.len(), 1);
    assert_eq!(&*sounds[0].effect, "hum");
    assert_eq!(sounds[0].position, placed.position);
    assert_eq!(sounds[0].age, 0.);
    assert!(!sounds[0].combat);
    let key = sounds[0].key;
    pass.animate(20.5, None, &[0.]);
    assert_eq!(pass.events[0].age, 0.5);
    sounds.clear();
    crate::sound_events::collect(
        &pass.events,
        &mut names,
        &mut sounds,
        crate::entity_animation::Event::in_combat,
    );
    assert_eq!(
        (sounds[0].key, sounds[0].age),
        (key, 0.5),
        "a present event keeps its key"
    );
    pass.update_placed(
        vec![(
            slot,
            ModelInstance {
                animation: entity_animation::BUILDING_DAMAGED,
                ..placed
            },
        )],
        128.,
    );
    pass.animate(21., None, &[0.]);
    assert_eq!(pass.events.len(), 3);
    assert_eq!(pass.events[0].definition.particle.as_deref(), Some("fire"));
    assert_eq!(
        pass.events[2].definition.particle.as_deref(),
        Some("sparks")
    );
    assert_eq!(pass.events[0].age, 0.);
}

#[test]
fn undeclared_and_unloadable_meshes_keep_named_refusals() {
    let descriptors = mesh::parse_descriptors(
        br#"
            objectTypes = { pdxmesh = { name = broken_mesh file = absent.mesh } }
            entity = { name = broken pdxmesh = broken_mesh
                state = { name = idle event = { particle = smoke } }
            }
        "#,
        "fixture.asset",
    )
    .unwrap();
    let mut library = ModelLibrary::default();
    let layers = ResolvedLayers::default();
    assert_eq!(
        library
            .add(&layers, &descriptors, "undeclared")
            .unwrap_err()
            .to_string(),
        "no pdxmesh or entity declares it"
    );
    let error = library
        .add(&layers, &descriptors, "broken")
        .unwrap_err()
        .to_string();
    assert!(
        error.starts_with("broken: pdxmesh broken_mesh (absent.mesh) cannot be loaded: "),
        "{error}"
    );
    assert!(error.ends_with("; 1 map effects omitted"));
    assert!(library.meshes.is_empty() && library.rigs.is_empty() && library.palette.is_empty());
}

#[test]
fn two_entities_share_a_partial_stream_mesh_and_keep_placed_vertices() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_partial_mesh_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir(&root).unwrap();
    let mut bytes = b"@@b@[object\0[[frame\0[[[mesh\0".to_vec();
    for (name, values) in [
        ("p", [1_f32, 2., 3., 4., 5., 6., 7., 8., 9.]),
        ("n", [0., 1., 0., 0., 1., 0., 0., 1., 0.]),
    ] {
        bytes.extend([b'!', name.len() as u8]);
        bytes.extend(name.as_bytes());
        bytes.push(b'f');
        bytes.extend((values.len() as u32).to_le_bytes());
        bytes.extend(values.into_iter().flat_map(f32::to_le_bytes));
    }
    bytes.extend(b"!\x03trii\x03\0\0\0\0\0\0\0\x01\0\0\0\x02\0\0\0");
    bytes.extend(b"[[[[material\0!\x06shaders\x01\0\0\0\x0e\0\0\0AuthoredShader");
    std::fs::write(root.join("frame.mesh"), &bytes).unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    let descriptors = mesh::parse_descriptors(
        br#"objectTypes = {
            pdxmesh = { name = "frame_mesh" file = "frame.mesh" scale = 1 }
        }
        entity = { name = "actor" pdxmesh = "frame_mesh" }
        entity = { name = "actor_large" pdxmesh = "frame_mesh" scale = 2 }"#,
        "fixture.asset",
    )
    .unwrap();
    let library = ModelLibrary::load(
        &layers,
        Decode::default(),
        &descriptors,
        &["actor".into(), "actor_large".into()],
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(library.meshes.len(), 2);
    assert_eq!(library.geometry_sources.len(), 1);
    let first = &library.meshes[0].native[0].lods[0][0];
    let second = &library.meshes[1].native[0].lods[0][0];
    assert!(Arc::ptr_eq(&first.geometry, &second.geometry));
    let geometry = (first.geometry.load)(&mut None).unwrap();
    assert_eq!(geometry.stride, 24);
    assert_eq!(geometry.vertices.len(), 18);
    assert_eq!(geometry.vertices[6..10], [4., 5., 6., 0.]);
    for (slot, scale) in [(0, 1.0), (1, 2.0)] {
        let actors = crate::mesh_pose::actors(library.rig(slot), None, true);
        let world = *actors[0].world.as_ref().unwrap();
        for (vertex, raw) in [[1., 2., 3.], [4., 5., 6.], [7., 8., 9.]]
            .into_iter()
            .enumerate()
        {
            let old = mesh::Transform {
                basis: [[scale, 0., 0.], [0., scale, 0.], [0., 0., scale]],
                translation: [0.; 3],
            }
            .transform_point(raw);
            let start = vertex * 6;
            let local: [f32; 3] = geometry.vertices[start..start + 3].try_into().unwrap();
            let placed: [f32; 3] = std::array::from_fn(|axis| {
                world[0][axis] * local[0]
                    + world[1][axis] * local[1]
                    + world[2][axis] * local[2]
                    + world[3][axis]
            });
            assert_eq!(placed, old);
        }
    }
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn an_attachment_cycle_is_counted_without_a_fixed_depth_limit() {
    let descriptors = mesh::parse_descriptors(
        br#"entity = { name = a attach = { root = b } }
            entity = { name = b attach = { root = a } }"#,
        "fixture.asset",
    )
    .unwrap();
    let mut warnings = Vec::new();
    let library = ModelLibrary::load(
        &ResolvedLayers::default(),
        Decode::default(),
        &descriptors,
        &["a".into()],
        &mut warnings,
    )
    .unwrap();
    assert_eq!(library.meshes[0].attachments, 1);
    assert_eq!(
        library.attachment_refusals["b -> a: attachment cycle through a"],
        1
    );
    assert!(
        warnings
            .iter()
            .any(|(_, warning)| warning.contains("attachment cycle through a"))
    );
}

fn at(x: f32, z: f32) -> ModelInstance {
    ModelInstance {
        position: [x, 0.0, z],
        rotation: 0.0,
        scale: 1.0,
        pitch: 0.0,
        train_clip: [0.0; 4],
        ..ModelInstance::default()
    }
}

#[test]
fn the_field_groups_by_mesh_and_tile_and_widens_by_the_radius() {
    let library = library_with_one_mesh(2.0);
    let field = ModelField::build(
        &library,
        vec![
            (0, at(10.0, 10.0)),
            (0, at(20.0, 10.0)),
            (0, at(300.0, 10.0)),
        ],
        256.0,
    );
    assert_eq!(field.instances.len(), 3);
    assert_eq!(field.groups.len(), 2);
    assert_eq!(field.groups[0].bounds, [8.0, 8.0, 22.0, 12.0]);
    assert_eq!(field.groups[0].centroid, [15.0, 0.0, 10.0]);
    assert_eq!(field.groups[1].range, 2..3);
}

#[test]
fn native_submission_order_survives_grouping_and_field_rebuild() {
    let library = library_with_one_mesh(2.);
    let mut field = ModelField::build(
        &library,
        vec![
            (0, at(300., 10.)),
            (0, at(20., 10.)),
            (0, at(310., 10.)),
            (0, at(10., 10.)),
        ],
        256.,
    );
    let positions = |field: &ModelField| {
        field
            .native_order
            .iter()
            .map(|&(group, instance)| {
                assert!(field.groups[group].range.contains(&(instance as u32)));
                field.instances[instance].position[0]
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(positions(&field), vec![300., 20., 310., 10.]);
    field.rebuild(
        &library,
        &mut vec![(0, at(500., 10.))],
        &mut Vec::new(),
        256.,
    );
    assert_eq!(positions(&field), vec![500.]);
    field.rebuild(&library, &mut Vec::new(), &mut Vec::new(), 256.);
    assert!(field.native_order.is_empty());
}

#[test]
fn a_group_outside_the_visible_rectangle_is_culled() {
    let library = library_with_one_mesh(0.0);
    let field = ModelField::build(&library, vec![(0, at(10.0, 10.0))], 256.0);
    let group = &field.groups[0];
    assert!(visible(group, 0.0, None));
    assert!(visible(group, 0.0, Some([0.0, 0.0, 20.0, 20.0])));
    assert!(!visible(group, 0.0, Some([100.0, 0.0, 200.0, 20.0])));
    assert!(visible(group, 100.0, Some([100.0, 0.0, 200.0, 20.0])));
}

#[test]
fn a_figure_flying_above_the_rectangle_is_not_culled_by_its_ground_position() {
    let library = library_with_one_mesh(0.0);
    let flying = ModelInstance {
        position: [10.0, 60.0, 10.0],
        ..at(10.0, 10.0)
    };
    let field = ModelField::build(&library, vec![(0, flying)], 256.0);
    let group = &field.groups[0];
    assert!(visible(group, 0.0, Some([-50.0, -50.0, -10.0, 50.0])));
    assert!(!visible(group, 0.0, Some([-500.0, -50.0, -400.0, 50.0])));
}

#[test]
fn an_entity_s_bound_encloses_every_point_around_its_origin() {
    let mut reach = Reach::default();
    reach.widen_point([100.0, 100.0, 0.0], true);
    reach.widen_point([0.0, -120.0, 30.0], true);
    assert_eq!((reach.radius, reach.height), (100.0, 100.0));
    assert!(
        (reach.bound - 100.0 * 2f32.sqrt()).abs() < 1e-3,
        "{}",
        reach.bound
    );
    reach.widen_point([0.0, -150.0, 0.0], true);
    assert_eq!(reach.bound, 150.0);
}

#[test]
fn drawn_variants_are_found_on_attached_entities_and_later_states() {
    let clip = |name: &str, next: Option<&str>| entity_animation::Clip {
        state: mesh::EntityState {
            name: name.into(),
            chance: 1.0,
            next_state: next.map(Into::into),
            ..Default::default()
        },
        animation: None,
    };
    let node = |clips: Vec<entity_animation::Clip>| entity_animation::Node {
        native: None,
        mesh: Arc::new(mesh::Mesh::default()),
        clips,
        default: "idle".into(),
        follows_parent: false,
        parent: None,
        scale: 1.0,
        mesh_scale: 1.0,
        skin_base: None,
        rigid: 0,
        locators: BTreeMap::new(),
    };
    let rig = |nodes| Rig {
        start: 0,
        end: 1,
        nodes,
    };
    let single = rig(vec![node(vec![clip("idle", None)])]);
    assert_eq!(crate::drawn_variants(&single), None);
    let attached = rig(vec![
        node(vec![clip("idle", None)]),
        node(vec![clip("idle", None), clip("idle", None)]),
    ]);
    assert_eq!(
        crate::drawn_variants(&attached),
        Some((1, "idle".into(), 2))
    );
    let later = rig(vec![node(vec![
        clip("idle", Some("burn")),
        clip("burn", None),
        clip("burn", None),
    ])]);
    assert_eq!(crate::drawn_variants(&later), Some((0, "burn".into(), 2)));
}

fn baked(layers: &ResolvedLayers, name: &str) -> ModelLibrary {
    let roster = layers.descriptor_roster().unwrap();
    let mut warnings = Vec::new();
    let library = ModelLibrary::load(
        layers,
        Decode::default(),
        &roster.descriptors,
        &[name.to_owned()],
        &mut warnings,
    )
    .unwrap();
    assert!(
        library.slot(name).is_some(),
        "{name} does not bake: {warnings:?}; {:?} {:?}",
        roster.logged,
        roster.descriptors.entities.keys().collect::<Vec<_>>()
    );
    library
}

#[test]
fn an_entity_drawn_from_a_visual_mods_files_is_the_mods() {
    let tank = br#"entity = { name = tank pdxmesh = hull scale = 2 state = { name = idle } }"#;
    let (root, layers) = visual_stack(
        "origin",
        &[
            ("base/gfx/models/meshes.gfx", MESHES_GFX),
            ("base/gfx/entities/plane.asset", PLANE_ASSET),
            ("base/gfx/models/plane.mesh", &triangle_mesh(1.)),
            ("art/gfx/models/plane.mesh", &triangle_mesh(1.)),
            ("base/gfx/entities/tank.asset", tank),
            ("art/gfx/entities/tank.asset", tank),
            ("base/gfx/models/tank.mesh", &triangle_mesh(1.)),
        ],
    );
    let presentation = layers.presentation().clone();
    let visual = |layers: &ResolvedLayers, name: &str| {
        let library = baked(layers, name);
        library.meshes[library.slot(name).unwrap()].visual
    };
    assert!(presentation.supplied_by_visual_mod("gfx/models/plane.mesh"));
    assert!(visual(&presentation, "plane"));
    assert!(visual(&presentation, "tank"));
    assert!(!layers.supplied_by_visual_mod("gfx/models/plane.mesh"));
    assert!(!visual(&layers, "plane") && !visual(&layers, "tank"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_visual_mod_entity_keeps_its_recorded_counterpart() {
    let (root, layers) = visual_stack(
        "counterpart",
        &[
            ("base/gfx/models/meshes.gfx", MESHES_GFX),
            ("base/gfx/entities/plane.asset", PLANE_ASSET),
            ("base/gfx/models/plane.mesh", &triangle_mesh(1.)),
            ("art/gfx/models/plane.mesh", &triangle_mesh(5.)),
        ],
    );
    let library = baked(layers.presentation(), "plane");
    let shown = library.slot("plane").unwrap();
    let counterpart = library.alternatives[&shown];
    assert!(library.meshes[shown].visual && !library.meshes[counterpart].visual);
    let geometry = |slot: usize| (library.meshes[slot].native[0].lods[0][0].geometry.load)(&mut None).unwrap();
    assert_eq!(geometry(shown).vertices[8], 5.);
    assert_eq!(geometry(counterpart).vertices[8], 1.);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_recorded_counterparts_mesh_read_reaches_the_presentation_trace() {
    use content_reader_layers::content::{ContentRead, ReadOutcome};
    let (root, layers) = visual_stack(
        "counterpart_trace",
        &[
            ("base/gfx/models/meshes.gfx", MESHES_GFX),
            ("base/gfx/entities/plane.asset", PLANE_ASSET),
            ("base/gfx/models/plane.mesh", &triangle_mesh(1.)),
            ("art/gfx/models/plane.mesh", &triangle_mesh(5.)),
        ],
    );
    let traced = |stack: &ResolvedLayers| {
        let reads = Arc::new(std::sync::Mutex::new(Vec::<ContentRead>::new()));
        let log = Arc::clone(&reads);
        let stack = stack
            .with_read_trace(Arc::new(move |read| log.lock().unwrap().push(read)))
            .with_read_purpose("display/buildings");
        (stack, reads)
    };
    let mesh = |reads: &[ContentRead], layer: usize| -> Vec<ReadOutcome> {
        reads
            .iter()
            .filter(|read| read.path == b"gfx/models/plane.mesh" && read.layer == Some(layer))
            .inspect(|read| assert_eq!(read.purpose, "display/buildings"))
            .map(|read| read.outcome.clone())
            .collect()
    };
    let (recorded, recorded_reads) = traced(&layers);
    recorded.read_uncovered("gfx/models/plane.mesh").unwrap();
    let recorded_mesh = mesh(&recorded_reads.lock().unwrap()[..], 0);
    assert_eq!(recorded_mesh.len(), 1);
    let (presentation, reads) = traced(layers.presentation());
    let library = baked(&presentation, "plane");
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        library
            .alternatives
            .contains_key(&library.slot("plane").unwrap())
    );
    let reads = reads.lock().unwrap();
    let counterpart = mesh(&reads[..], 0);
    assert!(!counterpart.is_empty(), "{reads:?}");
    assert!(
        counterpart.iter().all(|read| *read == recorded_mesh[0]),
        "{reads:?}"
    );
    let shown = mesh(&reads[..], 1);
    assert!(!shown.is_empty() && !shown.contains(&recorded_mesh[0]));
}

#[test]
fn a_refused_visual_mod_entity_is_drawn_from_the_recording() {
    let (root, layers) = visual_stack(
        "redirect",
        &[
            ("base/gfx/models/meshes.gfx", MESHES_GFX),
            ("base/gfx/entities/plane.asset", PLANE_ASSET),
            ("base/gfx/models/plane.mesh", &triangle_mesh(5.)),
            ("art/gfx/models/plane.mesh", &triangle_mesh(1.)),
        ],
    );
    let mut library = baked(layers.presentation(), "plane");
    let _ = std::fs::remove_dir_all(&root);
    let shown = library.slot("plane").unwrap();
    let counterpart = library.alternatives[&shown];
    library.meshes[shown].radius = 1.0;
    library.meshes[counterpart].radius = 5.0;
    let instance = shader_pipeline::gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, _queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let arena = shader_pipeline::gpu::Arena::new(&device);
    let mut pass = ModelPass::new(&library, "unit models");
    let placed = ModelInstance {
        scale: 1.0,
        ..ModelInstance::default()
    };
    pass.update_placed(vec![(shown, placed)], 256.0);
    pass.refuse_actor(
        &arena,
        Some(shown),
        0,
        usize::MAX,
        "plane",
        "skin: no played clip",
    );
    assert!(arena.refusals().is_empty());
    pass.animate(0.0, None, &[0.0]);
    assert_eq!(pass.field.groups[0].mesh, counterpart);
    let reach = library.meshes[counterpart].radius;
    assert!(reach > library.meshes[shown].radius);
    assert_eq!(pass.field.groups[0].bounds, [-reach, -reach, reach, reach]);
    let lines = pass.refusal_lines(arena.frame());
    assert!(
        lines.iter().any(|line| line
            == "Map shader unit models: visual mod entity plane is drawn from the recording (refused actor plane: skin: no played clip)"),
        "{lines:?}"
    );
    assert_eq!(
        (pass.shader_library(shown), pass.shader_library(counterpart)),
        (0, 1)
    );
    pass.update_placed(vec![(shown, ModelInstance::default())], 256.0);
    assert_eq!(pass.field.groups[0].mesh, counterpart);
    let (seed, identity) = ModelInstance::default().animation_key();
    pass.set_hidden([(shown, seed, identity)].into());
    assert!(
        pass.hides(pass.actor_address((shown, seed, identity)))
            && pass.hides(pass.actor_address((counterpart, seed, identity)))
    );
    assert_eq!(
        pass.root_play_position((shown, seed, identity)),
        pass.root_play_position((counterpart, seed, identity))
    );
    let mut alone = ModelPass::new(&library, "unit models");
    alone.alternatives.clear();
    alone.refuse_actor(
        &arena,
        Some(shown),
        0,
        usize::MAX,
        "plane",
        "skin: no played clip",
    );
    assert_eq!(
        arena.refusals(),
        ["Map shader unit models: refused actor plane".to_owned()].into()
    );
}

#[test]
fn an_entity_a_visual_mod_breaks_is_baked_from_the_recording() {
    let (root, layers) = visual_stack(
        "broken",
        &[
            ("base/gfx/models/meshes.gfx", MESHES_GFX),
            ("base/gfx/entities/plane.asset", PLANE_ASSET),
            ("base/gfx/models/plane.mesh", &triangle_mesh(1.)),
            ("art/gfx/models/plane.mesh", b"not a mesh"),
        ],
    );
    let presentation = layers.presentation().clone();
    let library = baked(&presentation, "plane");
    let _ = std::fs::remove_dir_all(&root);
    let slot = library.slot("plane").unwrap();
    assert!(!library.meshes[slot].visual);
    assert!(library.alternatives.is_empty());
    let refusals = presentation.read_warnings().overlay_refusals().unwrap();
    assert!(refusals.contains("model plane ("), "{refusals}");
}

#[test]
fn a_visual_mod_texture_stays_deferred_and_keeps_its_recorded_counterpart() {
    let tga = |b: u8, g: u8, r: u8| {
        let mut bytes = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 2, 0, 32, 0x28];
        bytes.extend([b, g, r, 255].repeat(4));
        bytes
    };
    let meshes_gfx = br#"objectTypes = {
        pdxmesh = {
            name = body
            file = "gfx/models/plane.mesh"
            scale = 1
            meshsettings = { texture_diffuse = "plane.tga" }
        }
    }"#;
    let base_tga = tga(10, 20, 30);
    let art_tga = tga(40, 50, 60);
    let (root, layers) = visual_stack(
        "visual_texture",
        &[
            ("base/gfx/models/meshes.gfx", meshes_gfx),
            ("base/gfx/entities/plane.asset", PLANE_ASSET),
            ("base/gfx/models/plane.mesh", &triangle_mesh(1.)),
            ("base/gfx/models/plane.tga", &base_tga),
            ("art/gfx/models/plane.tga", &art_tga),
        ],
    );
    let presentation = layers.presentation().clone();
    let library = baked(&presentation, "plane");
    let slot = library.slot("plane").unwrap();
    assert!(library.meshes[slot].visual);
    let counterpart = library.alternatives[&slot];
    let diffuse = |mesh_slot: usize| {
        let material = library.meshes[mesh_slot].native[0].lods[0][0].material_index;
        &library.textures[library.materials[material].diffuse]
    };
    let visual_tex = diffuse(slot);
    let recorded_tex = diffuse(counterpart);
    assert!(visual_tex.source().is_none());
    assert!(recorded_tex.source().is_none());
    assert_eq!(visual_tex.path(), Some("gfx/models/plane.tga"));
    assert_eq!(recorded_tex.path(), Some("gfx/models/plane.tga"));
    let visual_decoded = visual_tex.decode(Decode::default()).unwrap();
    let recorded_decoded = recorded_tex.decode(Decode::default()).unwrap();
    assert_eq!(&visual_decoded.base().data[..4], &[60, 50, 40, 255]);
    assert_eq!(&recorded_decoded.base().data[..4], &[30, 20, 10, 255]);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_animation_a_mod_breaks_is_read_from_the_recording() {
    let clip: &[u8] = b"@@b@[info\0[samples\0";
    let broken: &[u8] = b"not an animation";
    let (root, layers) = visual_stack(
        "animation",
        &[
            ("base/gfx/models/idle.anim", clip),
            ("art/gfx/models/idle.anim", broken),
            ("base/gfx/models/lost.anim", broken),
        ],
    );
    let presentation = layers.presentation().clone();
    let mut library = ModelLibrary::default();
    let idle = library
        .animation(&presentation, "gfx/models/idle.anim")
        .is_some();
    let lost = library
        .animation(&presentation, "gfx/models/lost.anim")
        .is_some();
    let _ = std::fs::remove_dir_all(&root);
    assert!(idle && !lost);
    let refusals = presentation.read_warnings().overlay_refusals().unwrap();
    assert!(
        refusals.contains("animation gfx/models/idle.anim ("),
        "{refusals}"
    );
    assert!(
        presentation
            .refused()
            .contains("model animation gfx/models/lost.anim")
    );
    assert!(
        library
            .animation_refusals
            .contains_key("gfx/models/lost.anim")
    );
    assert!(
        !library
            .animation_refusals
            .contains_key("gfx/models/idle.anim")
    );
}

#[test]
fn convoy_collision_radius_is_the_runtime_bounds_three_dimensional_half_diagonal() {
    let mut reach = Reach::default();
    reach.collision_point([1., 2., 3.]);
    reach.collision_point([7., 10., 3.]);
    assert_eq!(reach.collision_radius(), Some(5.));
    reach.attachments = 1;
    assert_eq!(reach.collision_radius(), None);
}

#[test]
fn a_moved_part_is_written_where_it_stands_and_leaves_the_rest_of_the_field_alone() {
    let library = library_with_one_mesh(2.0);
    let standing = vec![
        (0, at(300.0, 10.0)),
        (0, at(20.0, 10.0)),
        (0, at(310.0, 10.0)),
        (0, at(10.0, 10.0)),
    ];
    for shift in [30.0, 400.0] {
        let mut pass = ModelPass::new(&library, "moved parts");
        pass.update_placed(standing.clone(), 256.0);
        let held = (
            pass.field.instances.as_ptr(),
            pass.field.native_order.clone(),
            pass.field.groups.iter().map(|group| (group.mesh, group.range.clone())).collect::<Vec<_>>(),
        );
        let moved = at(300.0 + shift, 10.0);
        pass.update_moving(
            crate::Moving {
                standing: None,
                moved: vec![(0, moved)],
            },
            256.0,
        );
        assert_eq!(
            pass.field.native_order, held.1,
            "a part that moves keeps the field's walk order"
        );
        assert_eq!(
            pass.field.groups.iter().map(|group| (group.mesh, group.range.clone())).collect::<Vec<_>>(),
            held.2,
            "a part that moves keeps its group and the parts in it"
        );
        assert_eq!(
            pass.field.instances.as_ptr(),
            held.0,
            "the field's storage is written in place"
        );
        let (group, index) = pass.field.native_order[0];
        assert_eq!(pass.field.instances[index].position, moved.position);
        for (at, instance) in pass.field.instances.iter().enumerate() {
            if at == index {
                continue;
            }
            assert!(
                standing.iter().any(|(_, held)| held.position == instance.position),
                "a part the frame did not move stands where its placement put it"
            );
        }
        let bounds = pass.field.groups[group].bounds;
        assert!(
            bounds[0] <= moved.position[0] - 2.0
                && bounds[2] >= moved.position[0] + 2.0
                && bounds[1] <= moved.position[2] - 2.0
                && bounds[3] >= moved.position[2] + 2.0,
            "the group a moved part stands in reaches it: {bounds:?} for {:?}",
            moved.position
        );
        let mut whole = standing.clone();
        whole[0].1 = moved;
        let afresh = ModelField::build_meshes(&pass.meshes, whole, 256.0);
        for (ordinal, &(group, index)) in pass.field.native_order.iter().enumerate() {
            let (built_group, built) = afresh.native_order[ordinal];
            assert_eq!(
                pass.field.instances[index].position, afresh.instances[built].position,
                "ordinal {ordinal} is placed where a whole placement places it"
            );
            let (held, built) = (
                pass.field.groups[group].bounds,
                afresh.groups[built_group].bounds,
            );
            assert!(
                held[0] <= built[0] && held[1] <= built[1] && held[2] >= built[2] && held[3] >= built[3],
                "ordinal {ordinal}'s group reaches it: {held:?} against {built:?}"
            );
        }
    }
}

#[test]
fn retained_placement_storage_keeps_group_ties_order_and_replacement_state() {
    let library = library_with_one_mesh(2.0);
    let mut pass = ModelPass::new(&library, "retained placements");
    let placed = vec![
        (0, at(300.0, 10.0)),
        (0, at(20.0, 10.0)),
        (0, at(310.0, 10.0)),
        (0, at(10.0, 10.0)),
        (0, at(20.0, 10.0)),
    ];
    pass.update_placed(placed.clone(), 256.0);
    let storage = (
        pass.field.instances.as_ptr(),
        pass.field.native_order.as_ptr(),
        pass.keyed.as_ptr(),
    );
    for placements in [placed[..3].to_vec(), Vec::new(), placed] {
        pass.field.default_states.insert((0, 1, 2), 3);
        let expected = ModelField::build_meshes(&pass.meshes, placements.clone(), 256.0);
        pass.update_placed(placements, 256.0);
        assert_eq!(pass.field, expected);
        assert_eq!(
            storage,
            (
                pass.field.instances.as_ptr(),
                pass.field.native_order.as_ptr(),
                pass.keyed.as_ptr()
            )
        );
    }
}

#[test]
fn visual_fallback_and_refusal_identities_survive_opposite_demand_orders() {
    let (root, layers) = visual_stack(
        "stable_identity",
        &[
            (
                "base/gfx/models/meshes.gfx",
                br#"objectTypes = {
                pdxmesh = { name = body file = "gfx/models/plane.mesh" scale = 1 }
                pdxmesh = { name = hull file = "gfx/models/tank.mesh" scale = 1 }
                pdxmesh = { name = absent file = "gfx/models/absent.mesh" scale = 1 }
            }"#,
            ),
            ("base/gfx/models/plane.mesh", &triangle_mesh(1.)),
            (
                "base/gfx/entities/actors.asset",
                br#"
            entity = { name = alpha pdxmesh = body }
            entity = { name = zeta pdxmesh = body }
            entity = { name = retry pdxmesh = body }
            entity = { name = refused pdxmesh = absent }
        "#,
            ),
            ("art/gfx/models/plane.mesh", &triangle_mesh(2.)),
            (
                "art/gfx/entities/actors.asset",
                br#"
            entity = { name = alpha pdxmesh = body }
            entity = { name = zeta pdxmesh = body }
            entity = { name = retry pdxmesh = hull }
            entity = { name = refused pdxmesh = absent }
        "#,
            ),
        ],
    );
    let presentation = layers.presentation();
    let roster = presentation.descriptor_roster().unwrap();
    let mut reference = None;
    for names in [
        ["zeta", "retry", "alpha", "refused"],
        ["refused", "alpha", "retry", "zeta"],
    ] {
        let mut warnings = Vec::new();
        let library = ModelLibrary::load(
            presentation,
            Decode::default(),
            &roster.descriptors,
            &names.map(str::to_owned),
            &mut warnings,
        )
        .unwrap();
        let mut identities = BTreeMap::new();
        for name in ["alpha", "retry", "zeta"] {
            let slot = library.slot(name).unwrap();
            let counterpart = library
                .alternatives
                .get(&slot)
                .map(|&at| library.meshes[at].asset.clone());
            if name != "retry" {
                let counterpart = counterpart
                    .as_ref()
                    .expect("a visual mesh has its recorded counterpart");
                assert_ne!(&library.meshes[slot].asset, counterpart);
                let recorded = presentation.without_overlays().unwrap();
                let mut expected =
                    AssetKey::selected(name, identity::namespace(&recorded), Decode::default());
                expected.recorded(true);
                assert_eq!(counterpart, &expected);
            } else {
                assert!(counterpart.is_none());
            }
            identities.insert(name, (library.meshes[slot].asset.clone(), counterpart));
        }
        assert!(library.slot("refused").is_none());
        assert!(library.failed["refused"].contains("absent"));
        let observed = (identities, library.failed.clone());
        if let Some(expected) = &reference {
            assert_eq!(&observed, expected);
        } else {
            reference = Some(observed);
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_game_cull_tests_the_eye_s_distance_to_the_sphere_s_centre_then_the_four_side_planes() {
    let wide = 1.0e4;
    let cull = GameCull {
        eye: [0., 100., 0.],
        sides: [[1., 0., 0., wide], [-1., 0., 0., wide], [0., 0., 1., wide], [0., 0., -1., wide]],
        x: 0.,
        pad: 0.,
    };
    let default = content_reader_mesh::mesh::DEFAULT_CULL_DISTANCE_SQUARED;
    assert_eq!(cull.test([400., 0., 0.], 1., default), None);
    assert_eq!(cull.test([420., 0., 0.], 1., default), Some(Culled::Distance));
    assert_eq!(cull.test([420., 0., 0.], 1., 600. * 600.), None, "a mesh's own cull distance");
    assert_eq!(cull.test([-wide - 2., 0., 0.], 1., 1.0e12), Some(Culled::Planes), "wholly outside a side plane");
    assert_eq!(cull.test([-wide - 0.5, 0., 0.], 1., 1.0e12), None, "a sphere crossing a side plane is inside");
}

#[test]
fn a_figure_walked_on_a_camera_moved_onto_another_map_copy_is_culled_where_it_stands_for_the_cull() {
    let wide = 1.0e9;
    let width = MapSize::TEST.x();
    let eye = viewer_gpu::camera::Camera { center: [-10., 0.], height: 100., map: MapSize::TEST, defines: MapDefines::test(), pose: viewer_gpu::camera::Pose::Map };
    let moved = viewer_gpu::camera::Camera { center: [width - 10., 0.], ..eye };
    let pass = ModelPass::new(&library_with_one_mesh(1.), "unit models");
    let cull = GameCull { eye: [-10., 100., 0.], sides: [[1., 0., 0., wide], [-1., 0., 0., wide], [0., 0., 1., wide], [0., 0., -1., wide]], x: -10., pad: 0. };
    pass.set_cull(Some(cull), Some(cull));
    let instance = ModelInstance { position: [width - 20., 0., 0.], scale: 1.0, ..ModelInstance::default() };
    assert_eq!(pass.game_culled(0, &instance, 0., &moved), None, "beside the moved eye: drawn");
    assert_eq!(pass.game_culled(0, &instance, 0., &eye), Some(Culled::Distance), "walked on the eye where it stands, the figure is a map width away");
}

#[test]
fn a_pan_draws_a_moved_figure_by_its_own_camera_s_cull_and_its_held_one_only_gathers() {
    let wide = 1.0e9;
    let pass = ModelPass::new(&library_with_one_mesh(1.), "unit models");
    let sides = [[1., 0., 0., wide], [-1., 0., 0., wide], [0., 0., 1., wide], [0., 0., -1., wide]];
    let pad = viewer_gpu::camera::PAD;
    let held = GameCull { eye: [1000., 0.5, 0.], sides, x: 1000., pad };
    let own = |x: f32| GameCull { eye: [x, 0.5, 0.], sides, x, pad: 0. };
    let camera = |x: f32| viewer_gpu::camera::Camera { center: [x, 0.], height: 100., map: MapSize::TEST, defines: MapDefines::test(), pose: viewer_gpu::camera::Pose::Map };
    let at = |x: f32| ModelInstance { position: [x, 0., 0.], scale: 1.0, ..ModelInstance::default() };
    let default = content_reader_mesh::mesh::DEFAULT_CULL_DISTANCE_SQUARED;
    assert_eq!(held.test([1434., 0.5, 0.], 1., default), None, "the held cull, moved by the pan, admits it");
    pass.set_cull(Some(held), Some(own(1000.)));
    assert_eq!(pass.game_culled(0, &at(1427.), 0., &camera(1000.)), None, "drawn while in range");
    pass.set_cull(Some(held), Some(own(1006.)));
    assert_eq!(pass.game_culled(0, &at(1435.), 0., &camera(1006.)), None, "429 from the panned eye: drawn");
    assert_eq!(pass.game_culled(0, &at(1440.), 0., &camera(1006.)), Some(Culled::Distance), "434 from it: culled, as the game does");
}

#[test]
fn a_pose_batch_of_a_history_that_ended_never_replaces_the_new_historys_poses() {
    use crate::poses;
    let mut library = library_with_one_mesh(1.0);
    library.palette = vec![PaletteJoint::IDENTITY];
    library.rigs.push(Arc::new(entity_animation::stage_rig(false)));
    let mut pass = ModelPass::new(&library, "synthetic poses");
    let mut worker = poses::Worker::new();
    assert!(worker.refusal().is_none(), "the pose worker must run: {:?}", worker.refusal());
    let mut jobs = poses::Jobs::default();
    let place = |pass: &mut ModelPass, time: f32| {
        pass.update_placed(vec![(0, at(10.0, 10.0))], 128.0);
        pass.animate(time, None, &[0.0]);
    };
    let made = |worker: &mut poses::Worker, jobs: &mut poses::Jobs| loop {
        if let Some(made) = worker.take(jobs) {
            return made;
        }
        std::thread::yield_now();
    };
    place(&mut pass, 1.0);
    let rig = Arc::clone(&library.rigs[0]);
    let key = (0, 0, 0);
    pass.pose_jobs(&mut jobs, None, &[0.0]);
    assert_eq!(jobs.len(), 1, "the placed figure's pose is asked for");
    worker.give(1.0, &mut jobs);
    let first = made(&mut worker, &mut jobs);
    pass.take_poses(&first);
    assert_eq!(pass.posed.len(), 1, "the batch is this pass's own history's, so its set is installed");
    let worked = pass.actors(0, key, &rig, pass.players.get(&key), false);
    assert!(
        pass.posed.get((key, 0)).is_some_and(|posed| Arc::ptr_eq(&posed, &worked)),
        "a figure the set holds is drawn with the worker's limbs"
    );
    pass.pose_jobs(&mut jobs, None, &[0.0]);
    worker.give(2.0, &mut jobs);
    let stale = made(&mut worker, &mut jobs);
    let ended = pass.pose_pass();
    pass.reset_animation();
    place(&mut pass, 3.0);
    pass.take_poses(&stale);
    assert_eq!(pass.posed.len(), 0, "a set of the history that ended is not installed");
    let posed = pass.actors(0, key, &rig, pass.players.get(&key), false);
    assert!(
        !Arc::ptr_eq(&posed, &stale.set_for(ended).get((key, 0)).expect("the stale batch posed it")),
        "the new history's figure is posed where it is read, not with the pre-seek limbs"
    );
    pass.pose_jobs(&mut jobs, None, &[0.0]);
    worker.give(4.0, &mut jobs);
    let own = made(&mut worker, &mut jobs);
    pass.pose_on_the_frame(true);
    pass.take_poses(&own);
    assert_eq!(pass.posed.len(), 0, "a stopped picture installs no set of another frame");
    pass.pose_jobs(&mut jobs, None, &[0.0]);
    assert_eq!(jobs.len(), 0, "a stopped picture asks the worker for nothing");
}

#[test]
fn the_depth_map_is_drawn_again_whenever_anything_it_was_drawn_from_changes() {
    let library = ModelLibrary::default();
    let mut passes = Vec::new();
    viewer_scene_base::shadow::test_support::assert_depth_map_invalidation(|| {
        let pass = ModelPass::new(&library, "player");
        let caster = pass.caster_id();
        passes.push(pass);
        caster
    });
}
