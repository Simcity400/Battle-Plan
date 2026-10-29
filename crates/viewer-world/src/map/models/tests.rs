use super::*;
use std::collections::BTreeSet;
use viewer_foundation::map::camera::MapDefines;
use viewer_foundation::map::camera::MapSize;
use viewer_foundation::map::camera::Placement;

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
    // A newly seen entity loads mid-intro: its mesh, rig and joints are
    // appended, and the raid unit's history stays.
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
    // A library that is not an extension starts every history again:
    // a shorter palette, a changed prefix, fewer meshes.
    let changed = [joint(2.0), joint(1.0)];
    for after in [(&palette[..1], 2), (&changed[..], 2), (&palette[..], 1)] {
        let mut kept = BTreeMap::from([((0, 7, 0), entity_animation::Player::new(&rigs[0]))]);
        retain_players(&mut kept, (&palette, 2), after);
        assert!(kept.is_empty());
    }
    // Nothing changed: every history stays.
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
    // The unit pass shows every group it places, wherever the camera is.
    assert_eq!(step(&mut players, cruise, 10.0, true), 2);
    step(&mut players, dive, 11.0, true);
    assert_eq!(players[&(0, 7, 0)].root_playing(rig), Some("dive_intro"));
    step(&mut players, dive, 12.9, true);
    assert_eq!(players[&(0, 7, 0)].root_playing(rig), Some("dive_intro"));
    step(&mut players, dive, 13.1, true);
    assert_eq!(players[&(0, 7, 0)].root_playing(rig), Some("dive"));
    // A pass that culls (buildings, trains) retires what it does not show.
    assert_eq!(step(&mut players, dive, 13.5, false), 1);
    assert!(players.is_empty());
    step(&mut players, dive, 14.0, true);
    // An unplaced entity is retired.
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

/// An empty roster still keeps the content stack and the client's texture decode, which
/// the pass's compiled mesh effects take their global defines from.
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

/// A pass made over a library that has baked nothing yet takes the content stack the library
/// takes with its first bake, so the entities baked later draw rather than refuse every frame.
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

/// Every palette index a shard carries is rebased onto the merged palette:
/// the rig's range and each node's skin and rigid slots. A slot left behind indexes another entity's
/// joints, and `entity_animation` subtracts `rig.start` from the node's.
#[test]
fn merging_a_shard_rebases_every_palette_slot() {
    let joint = PaletteJoint::from_transform(&mesh::Transform::IDENTITY);
    let shard_mesh = ModelMesh {
        asset: crate::map::models::AssetKey::synthetic("shard"),
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
    };
    let mut shard = ModelLibrary {
        palette: vec![joint; 3],
        meshes: vec![shard_mesh.clone()],
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
    // The node's own slot is inside the rig's range, which is what
    // `Player::evaluate` indexes.
    assert!((rig.start..rig.end).contains(&(rig.nodes[0].rigid as usize)));
}

#[test]
fn a_mesh_setting_overrides_one_shape_and_one_material_index() {
    // This synthetic facility model uses three texture sets,
    // one per material index; applying them all would repaint the model.
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
            pose: viewer_foundation::map::camera::Pose::Map,
        }
        .on(MapSize::TEST, MapDefines::test())
    };
    // The reference pairs at 302.96 draw no sprawl; 50.178 does.
    assert!(!draws_city_sprawl(&at(302.960)));
    assert!(draws_city_sprawl(&at(50.178)));
    // 229.4 - 9.5 = 219.9, just inside; 229.6 - 9.5 = 220.1, just outside.
    assert!(draws_city_sprawl(&at(229.4)));
    assert!(!draws_city_sprawl(&at(229.6)));
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
    // A quarter turn about +Y at scale 2, then a shift: the rows are what
    // a `dot` with `(position, 1)` needs.
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
    // A slot naming a joint outside the palette contributes nothing rather
    // than reading past its end.
    assert_eq!(
        skinned(&[left], [0.0, 1.0, 0.0], &[(0, 1.0), (99, 1.0)], 0),
        [-10.0, 1.0, 0.0]
    );
}

#[test]
fn a_posed_vertex_widens_the_reach_where_the_bind_box_would_not() {
    // `cavalry_horse.mesh` is 6.6 wide bound and 16.0 posed, so a cull on
    // the `[aabb]` would clip it.
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
    // The near LOD spans one unit either way, the far one ten.
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
    // The far LOD's corner bounds the cull, not the first LOD's.
    assert!((tower.radius - 200f32.sqrt()).abs() < 1e-3, "{}", tower.radius);
    assert!((tower.own_radius - 200f32.sqrt()).abs() < 1e-3);
}

fn library_with_one_mesh(radius: f32) -> ModelLibrary {
    ModelLibrary {
        meshes: vec![ModelMesh {
            asset: crate::map::models::AssetKey::synthetic("m"),
            name: "m".into(),
            path: "m.mesh".into(),
            scale: 1.0,
            lod_distances: Vec::new(),
            native: Vec::new(),
            radius,
            height: 1.0,
            visual: false,
            own_radius: radius,
            bound: radius.hypot(1.0),
            convoy_radius: None,
            attachments: 0,
        }],
        ..ModelLibrary::default()
    }
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
            Ok(Arc::new(super::super::mesh_shader::Geometry::from_mesh(
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
    assert_eq!(part.geometry.indices, [0, 1, 2]);
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
    // A meshed parent's attachment names an actual mesh joint. The
    // meshless actor instead uses the authored entity locator below.
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
            == installed_content_core::DiagnosticKind::Warning
            && line.contains("geometry-less entities")));
    }
    let scenario = descriptors.inherit("scenario").unwrap();
    let mut requests = super::super::aircraft_events::scenario_requests(
        &scenario.states["default"][0].events,
        2.,
        library.rig(slot),
        2,
        0,
    );
    // Two scenario seconds of outgoing travel and global speed two;
    // the aircraft's state clocks still advance in entity seconds.
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
        // The game asks the mesh before entity locators. The meshed
        // actor's matching joint has no played palette, so lookup
        // leaves this event at the root; the dummy child still attaches
        // through the authored entity locator.
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
    check_supply(&pass); // Duplicate supply callback must not restart its clock.
    pass.reset_animation();
    pass.animate(900., None, &[0.]);
    check_supply(&pass); // A fresh player crossing the callbacks gives the same state.
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
    // Out-of-range number broadcasts to the whole group, with a new event identity.
    let seeds: BTreeSet<_> = pass
        .events
        .iter()
        .filter(|event| event.definition.particle.as_deref() == Some("drop"))
        .map(|event| event.seed)
        .collect();
    assert_eq!(seeds.len(), 2);
    pass.field.clocks.get_mut(&key).unwrap().time = 0.4;
    pass.animate(903., None, &[0.]);
    assert!(pass.events.is_empty()); // New cycle resets state and callback cursor.
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
    // Sound events are presented for audio, not refused.
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
    let mut names = crate::map::sound_events::Names::default();
    let mut sounds = Vec::new();
    crate::map::sound_events::collect(
        &pass.events,
        &mut names,
        &mut sounds,
        crate::map::entity_animation::Event::in_combat,
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
    crate::map::sound_events::collect(
        &pass.events,
        &mut names,
        &mut sounds,
        crate::map::entity_animation::Event::in_combat,
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
    std::fs::remove_dir_all(&root).unwrap();
    assert_eq!(library.meshes.len(), 2);
    assert_eq!(library.geometry_cache.len(), 1);
    let first = &library.meshes[0].native[0].lods[0][0];
    let second = &library.meshes[1].native[0].lods[0][0];
    assert!(Arc::ptr_eq(&first.geometry, &second.geometry));
    let geometry = &first.geometry;
    assert_eq!(geometry.stride, 24);
    assert_eq!(geometry.vertices.len(), 18);
    assert_eq!(geometry.vertices[6..10], [4., 5., 6., 0.]);
    for (slot, scale) in [(0, 1.0), (1, 2.0)] {
        let actors = super::super::mesh_pose::actors(library.rig(slot), None, true);
        let world = *actors[0].world.as_ref().unwrap();
        for (vertex, raw) in [[1., 2., 3.], [4., 5., 6.], [7., 8., 9.]]
            .into_iter()
            .enumerate()
        {
            // The pre-sharing bake used Transform::transform_point at this scale.
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
    // The wrap copy moves the group, not the rectangle.
    assert!(visible(group, 100.0, Some([100.0, 0.0, 200.0, 20.0])));
}

#[test]
fn unit_lifetime_and_hidden_animation_clock() {
    use {
        super::super::unit_history::History, super::super::unit_history::Kind,
        super::super::unit_history::test_support::GATES,
        super::super::unit_history::test_support::target,
    };
    let mut library = library_with_one_mesh(1.0);
    library.palette = vec![PaletteJoint::IDENTITY];
    library.rigs.push(Arc::new(Rig {
        start: 0,
        end: 1,
        nodes: vec![entity_animation::Node {
            native: None,
            mesh: Arc::new(mesh::Mesh::default()),
            clips: vec![entity_animation::Clip {
                state: mesh::EntityState {
                    name: "idle".into(),
                    chance: 1.0,
                    speed: 1.0,
                    events: vec![mesh::EntityEvent {
                        time: 0.2,
                        trigger_once: true,
                        particle: Some("synthetic".into()),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                animation: None,
            }],
            default: "idle".into(),
            follows_parent: false,
            parent: None,
            scale: 1.0,
            mesh_scale: 1.0,
            skin_base: None,
            rigid: 0,
            locators: BTreeMap::new(),
        }],
    }));
    let mut pass = ModelPass::new(&library, "synthetic unit lifetime");
    let mut target = target(40, Kind::Land);
    target.instance.animation = entity_animation::MOVE; // no reconstructed idle phase
    let mut history = History::default();
    history.set(vec![target], Arc::default(), false);
    {
        let mut frame = |time, shown, in_view| {
            if time == 10.75 {
                // Discovering another mesh in a later hour grows the
                // palette without replacing this live entity.
                library.palette.push(PaletteJoint::IDENTITY);
                pass.sync(&library);
            }
            let placed = history.step(time, GATES, [shown; 2], true, |_| in_view);
            pass.update_placed(placed, 128.0);
            pass.animate(time, None, &[0.0]);
            (
                pass.field.instances.clone(),
                pass.events.iter().map(|e| e.age).collect::<Vec<_>>(),
            )
        };
        let (first, events) = frame(10.0, true, true);
        assert!(
            events.is_empty(),
            "creation clock starts now, not at global time zero"
        );
        let epoch = first[0].animation_epoch;
        let (_, events) = frame(10.5, true, true);
        assert!((events[0] - 0.3).abs() < 1e-5);
        let (hidden, events) = frame(10.75, true, false);
        assert_eq!(hidden[0].animation_epoch, epoch);
        assert!((events[0] - 0.55).abs() < 1e-5, "hidden entity advances");
        let (back, events) = frame(11.0, true, true);
        assert_eq!(back[0].animation_epoch, epoch);
        assert!((events[0] - 0.8).abs() < 1e-5);
        for _ in 0..100 {
            frame(11.0, true, false);
        }
        let (fresh, events) = frame(12.0, true, true);
        assert_ne!(fresh[0].animation_epoch, epoch);
        assert!(events.is_empty());
        frame(12.5, true, true);
        let (faded, _) = frame(12.75, false, true);
        assert_eq!(faded[0].scale, 0.5);
    }
}

#[test]
fn an_entity_s_bound_encloses_every_point_around_its_origin() {
    // A corner up and out reaches farther than the horizontal radius or the height alone, and a
    // point below the origin counts too.
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

/// A state with several variants is found on any node, and through
/// `next_state`, not only on the root's default state.
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
    assert_eq!(super::drawn_variants(&single), None);
    let attached = rig(vec![
        node(vec![clip("idle", None)]),
        node(vec![clip("idle", None), clip("idle", None)]),
    ]);
    assert_eq!(
        super::drawn_variants(&attached),
        Some((1, "idle".into(), 2))
    );
    let later = rig(vec![node(vec![
        clip("idle", Some("burn")),
        clip("burn", None),
        clip("burn", None),
    ])]);
    assert_eq!(super::drawn_variants(&later), Some((0, "burn".into(), 2)));
}

/// A one-triangle mesh reaching `reach` along z, so two stacks' meshes differ in extent.
pub(crate) fn triangle_mesh(reach: f32) -> Vec<u8> {
    let mut bytes = b"@@b@[object\0[[frame\0[[[mesh\0".to_vec();
    for (name, values) in [
        ("p", [0_f32, 0., 0., 1., 0., 0., 0., 0., reach]),
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
    bytes
}

/// A recorded folder `base` and a visual mod `art`, each with `files`, the mod mounted over
/// the recording.
pub(crate) fn visual_stack(
    name: &str,
    files: &[(&str, &[u8])],
) -> (std::path::PathBuf, ResolvedLayers) {
    let root = std::env::temp_dir().join(format!(
        "hoi4_model_{name}_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    for (relative, bytes) in files {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    let art = root.join("art").display().to_string().replace('\\', "/");
    std::fs::create_dir_all(root.join("user/mod")).unwrap();
    std::fs::write(
        root.join("user/mod/art.mod"),
        format!("name=\"Art\"\npath=\"{art}\"\n"),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("steamapps/common/Game")).unwrap();
    let options = installed_content_core::ResolveOptions {
        game_dir: root.join("steamapps/common/Game"),
        user_dir: root.join("user"),
        cache_dir: root.join("cache"),
    };
    let base = root.join("base");
    let mut layers = ResolvedLayers::from_folders(&[(base.as_path(), &[][..])]).unwrap();
    let selected = [installed_content_core::content::visual::Choice {
        id: "mod/art.mod".into(),
        name: "Art".into(),
    }];
    let cancel = std::sync::atomic::AtomicBool::new(false);
    installed_content_core::content::visual::apply(&mut layers, &options, &selected, &[], &cancel).unwrap();
    (root, layers)
}

pub(crate) const MESHES_GFX: &[u8] = br#"objectTypes = {
    pdxmesh = { name = body file = "gfx/models/plane.mesh" scale = 1 }
    pdxmesh = { name = hull file = "gfx/models/tank.mesh" scale = 1 }
}"#;
pub(crate) const PLANE_ASSET: &[u8] =
    br#"entity = { name = plane pdxmesh = body scale = 1 state = { name = idle } }"#;

/// The library `name` bakes to from `layers`, by their own descriptors.
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

/// An entity drawn from a mesh or descriptor a visual mod supplies is the mod's, so its draw
/// refusals count against the mods; the same entity baked from the recording is not.
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
            // The mod changes only the tank's descriptor.
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

/// Each visual mod entity keeps its recorded counterpart, baked from the recording's own files
/// apart from the mod's of the same path; the name keeps the mod's entity.
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
    let _ = std::fs::remove_dir_all(&root);
    let shown = library.slot("plane").unwrap();
    let counterpart = library.alternatives[&shown];
    assert!(library.meshes[shown].visual && !library.meshes[counterpart].visual);
    let geometry = |slot: usize| library.meshes[slot].native[0].lods[0][0].geometry.clone();
    assert!(!Arc::ptr_eq(&geometry(shown), &geometry(counterpart)));
}

/// A refused draw of a visual mod entity draws its recorded counterpart from the next frame,
/// named, with the recording's own shaders, and is not recorded as a refusal; one without a
/// counterpart is. Placements, hidden actors and play positions keyed by the entity's own slot
/// follow it to the counterpart.
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
    // The recorded entity reaches further than the mod's (the fixture meshes declare no
    // bounds of their own).
    library.meshes[shown].radius = 1.0;
    library.meshes[counterpart].radius = 5.0;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
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
    // Culled by the counterpart's own reach, not the refused entity's.
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
    // Later placements, hidden actors and play positions keyed by the entity's own slot follow
    // it to the counterpart.
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

/// An entity whose visual mod mesh does not bake is baked from the recording, by the
/// recording's own files, named; it never vanishes when the recording draws it.
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

/// An animation a visual mod breaks is read from the recording, named; one neither stack reads
/// is refused by name and reported, never taken for an entity without an animation.
#[test]
fn an_animation_a_mod_breaks_is_read_from_the_recording() {
    let clip: &[u8] = b"@@b@[info\0[samples\0";
    // No PDX header: it does not parse.
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
    assert_eq!(reach.collision_radius(), None); // Attached entity sphere unions need their own proof.
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
