use crate::skeleton_source::tests::*;
use crate::{entity_animation, mesh_pose, AssetKey, ModelMaterial, PaletteJoint, identity, native};
use std::collections::{BTreeMap, BTreeSet};
use viewer_gpu::textures::ColorSpace;

fn refusal(name: &str) -> Option<String> {
    let (mesh, detail) = match name {
        "garbled" => (
            "magic",
            "not a PDX binary asset: the file does not start with the magic \"@@b@\"",
        ),
        "unjointed" => (
            "short_tx",
            "native skeleton joint \"root\" has fewer than12 tx floats",
        ),
        "unlocated" => (
            "bad_locator",
            "binary mesh locator \"mount\": 1 refused q: missing, nonfloat or short payload would cause a native overread",
        ),
        "halfway" => (
            "late",
            "the PDX mesh shape \"second\" has 2 floats in \"p\", not a multiple of 3",
        ),
        "refused" => ("absent", "gfx/models/absent.mesh"),
        "broken" => ("tri", "the PDX mesh shape \"frame\" indexes vertex 1 of 1"),
        "missing" => return Some("no pdxmesh or entity declares it".into()),
        "infantry" | "cavalry" | "fallen" | "rider" => return None,
        _ => panic!("missing fixed outcome for {name}"),
    };
    Some(format!(
        "{name}: pdxmesh {mesh} (gfx/models/{mesh}.mesh) cannot be loaded: {detail}; 0 map effects omitted"
    ))
}

fn expected_add(
    library: &mut ModelLibrary,
    layers: &ResolvedLayers,
    descriptors: &mesh::Descriptors,
    name: &str,
) {
    library
        .layers
        .get_or_insert_with(|| Arc::new(layers.clone()));
    if refusal(name).is_some() {
        return;
    }
    let namespace = library
        .asset_namespace
        .get_or_insert_with(|| identity::namespace(layers))
        .clone();
    if library.textures.is_empty() {
        for space in [ColorSpace::Srgb, ColorSpace::Linear] {
            library
                .paths
                .insert((String::new(), space), library.textures.len());
            library
                .textures
                .push(crate::ModelTexture::held(content_reader_image::dds::Texture::from_rgba8(
                    content_reader_image::Rgba8Image {
                        width: 1,
                        height: 1,
                        pixels: vec![0; 4],
                    },
                )));
            library.spaces.push(space);
        }
        library.materials.push(ModelMaterial {
            diffuse: 0,
            normal: 1,
            specular: 1,
        });
    }
    let geometry_key = ("gfx/models/body.mesh".to_owned(), 3_f32.to_bits(), 0, 0);
    let geometry = library
        .geometry_sources
        .entry(geometry_key.clone())
        .or_insert_with(|| {
            viewer_scene_base::mesh_shader::ModelGeometry::held(viewer_scene_base::mesh_shader::Geometry {
                vertices: vec![
                    0., 0., 0., 0., 1., 0., 3., 0., 0., 0., 1., 0., 0., 0., 10.5, 0., 1., 0.,
                ],
                skin: vec![],
                indices: vec![0, 1, 2],
                uv1: false,
                stride: 24,
            })
        })
        .clone();
    let native_skeleton = [0_f32, -1.5]
        .into_iter()
        .enumerate()
        .map(|(at, x)| mesh::NativeJoint {
            name: ["root", "arm"][at].into(),
            parent: [255, 0][at],
            inverse_bind: [1., 0., 0., 0., 1., 0., 0., 0., 1., x * 3., -0., 6.75],
        })
        .collect::<Vec<_>>();
    let skeleton = mesh::Mesh {
        lod_distances: vec![],
        shapes: vec![],
        skeleton: [0_f32, -1.5]
            .into_iter()
            .enumerate()
            .map(|(at, x)| mesh::Joint {
                name: ["root", "arm"][at].into(),
                index: at as u32,
                parent: (at == 1).then_some(0),
                inverse_bind: Some(mesh::Transform {
                    basis: mesh::Transform::IDENTITY.basis,
                    translation: [x, -0., 2.25],
                }),
            })
            .collect(),
        locators: vec!["mount".into(), "spare".into()],
    };
    let binary_locators = [
        ("mount", [0.5, -0., 1.], [0., 0., 0.6, 0.8], Some(1)),
        ("spare", [1., 2., 3.], [0., 0., 0., 1.], None),
    ]
    .into_iter()
    .map(|(name, translation, rotation, parent)| mesh_pose::Locator {
        name: name.into(),
        parent,
        error: None,
        local: mesh_pose::local(
            mesh::JointPose {
                translation,
                rotation,
                scale: 1.,
            },
            3.,
        ),
    })
    .collect::<Vec<_>>();
    let start = library.palette.len();
    let mut rig = entity_animation::Rig {
        start,
        end: start,
        nodes: vec![],
    };
    let mut native = Vec::new();
    let entities: &[&str] = if name == "cavalry" {
        &["cavalry", "rider"]
    } else {
        &[name]
    };
    for (at, entity) in entities.iter().enumerate() {
        let entity_scale = match *entity {
            "cavalry" => 0.5,
            "rider" => 2.,
            _ => 1.,
        };
        let scale = entity_scale * 3.;
        let (palette_scale, z) = if at == 1 { (3., 1.) } else { (scale, 0.) };
        let rigid = library.palette.len() as u32;
        library.palette.push(PaletteJoint {
            rows: [
                [palette_scale, 0., 0., 0.],
                [0., palette_scale, 0., 0.],
                [0., 0., palette_scale, z],
            ],
        });
        let inherited = descriptors.inherit(entity).unwrap();
        let entity_locators = &descriptors.entities[*entity].locators;
        rig.nodes.push(entity_animation::Node {
            mesh: Arc::new(skeleton.clone()),
            native: Some(Arc::new(mesh_pose::Source {
                pdxmesh: true,
                skeleton: native_skeleton.clone(),
                entity_locators: mesh_pose::entity_locators(
                    entity_locators,
                    &native_skeleton,
                    Some("body"),
                ),
                binary_locators: binary_locators.clone(),
                mesh_scale: 3.,
                entity_scale,
                frozen: None,
                game_data: None,
            })),
            clips: vec![entity_animation::Clip {
                state: mesh::EntityState {
                    name: "idle".into(),
                    looping: true,
                    chance: 1.,
                    chance_weight: Some(0),
                    speed: 1.,
                    ..mesh::EntityState::default()
                },
                animation: None,
            }],
            default: "idle".into(),
            follows_parent: true,
            parent: (at == 1).then(|| (0, "marker".into(), "marker".into())),
            scale,
            mesh_scale: 3.,
            skin_base: None,
            rigid,
            locators: inherited
                .locators
                .iter()
                .map(|(name, locator)| {
                    (
                        (*name).to_owned(),
                        (
                            crate::entity_locator(&locator.transform, 3.),
                            locator.parent_joint.clone(),
                        ),
                    )
                })
                .collect(),
        });
        native.push(native::Descriptor {
            name: (*entity).into(),
            node: at,
            distances: vec![],
            lods: vec![vec![native::Part {
                geometry: geometry.clone(),
                geometry_key: geometry_key.clone(),
                material_index: 0,
                material: mesh::Material {
                    shader: Some("AuthoredShader".into()),
                    ..mesh::Material::default()
                },
            }]],
        });
    }
    rig.end = library.palette.len();
    library.rigs.push(Arc::new(rig));
    library.index.insert(name.into(), library.meshes.len());
    library.meshes.push(ModelMesh {
        asset: AssetKey::selected(name, namespace, library.decode()),
        name: name.into(),
        path: "gfx/models/body.mesh".into(),
        scale: match name {
            "cavalry" => 1.5,
            "rider" => 6.,
            _ => 3.,
        },
        lod_distances: vec![],
        native,
        radius: 0.,
        height: 0.,
        visual: false,
        own_radius: 0.,
        bound: 0.,
        convoy_radius: None,
        attachments: usize::from(name == "cavalry"),
        cull_distance_squared: content_reader_mesh::mesh::DEFAULT_CULL_DISTANCE_SQUARED,
    });
    if name == "fallen" {
        *library
            .attachment_refusals
            .entry(format!("fallen -> broken: {}", refusal("broken").unwrap()))
            .or_default() += 1;
    }
}

#[test]
fn single_parse_bakes_keep_fixed_bytes_and_ordered_refusals() {
    fn node(depth: usize, name: &str) -> Vec<u8> {
        let mut bytes = vec![b'['; depth];
        bytes.extend(name.as_bytes());
        bytes.push(0);
        bytes
    }
    fn property(name: &str, kind: u8, count: usize, payload: impl Iterator<Item = u8>) -> Vec<u8> {
        let mut bytes = vec![b'!', name.len() as u8];
        bytes.extend(name.as_bytes());
        bytes.push(kind);
        bytes.extend((count as u32).to_le_bytes());
        bytes.extend(payload);
        bytes
    }
    fn floats(name: &str, values: &[f32]) -> Vec<u8> {
        let payload = values.iter().flat_map(|value| value.to_le_bytes());
        property(name, b'f', values.len(), payload)
    }
    fn ints(name: &str, values: &[i32]) -> Vec<u8> {
        let payload = values.iter().flat_map(|value| value.to_le_bytes());
        property(name, b'i', values.len(), payload)
    }
    fn string(name: &str, value: &str) -> Vec<u8> {
        let mut payload = (value.len() as u32 + 1).to_le_bytes().to_vec();
        payload.extend(value.as_bytes());
        payload.push(0);
        property(name, b's', 1, payload.into_iter())
    }
    let tx = |x: f32| [1., 0., 0., 0., 1., 0., 0., 0., 1., x, -0., 2.25];
    let body = [
        triangle_mesh(3.5),
        node(3, "skeleton"),
        node(4, "root"),
        ints("ix", &[0]),
        floats("tx", &tx(0.)),
        node(4, "arm"),
        ints("ix", &[1]),
        ints("pa", &[0]),
        floats("tx", &tx(-1.5)),
        node(1, "locator"),
        node(2, "mount"),
        floats("p", &[0.5, -0., 1.]),
        floats("q", &[0., 0., 0.6, 0.8]),
        string("pa", "arm"),
        node(2, "spare"),
        floats("p", &[1., 2., 3.]),
        floats("q", &[0., 0., 0., 1.]),
    ]
    .concat();
    let files = [
        ("body", body),
        (
            "tri",
            [
                b"@@b@[object\0[[frame\0[[[mesh\0".to_vec(),
                floats("p", &[0., 0., 0.]),
                ints("tri", &[0, 1, 2]),
            ]
            .concat(),
        ),
        ("magic", b"nope".to_vec()),
        (
            "short_tx",
            [
                triangle_mesh(1.),
                node(3, "skeleton"),
                node(4, "root"),
                ints("ix", &[0]),
                floats("tx", &[1., 0., 0., 0., 1., 0.]),
                node(2, "bad_positions"),
                node(3, "mesh"),
                floats("p", &[1.]),
            ]
            .concat(),
        ),
        (
            "bad_locator",
            [
                triangle_mesh(2.),
                node(2, "bad_positions"),
                node(3, "mesh"),
                floats("p", &[1.]),
                node(1, "locator"),
                node(2, "mount"),
                floats("p", &[0., 0., 0.]),
            ]
            .concat(),
        ),
        (
            "late",
            [
                triangle_mesh(1.5),
                node(2, "second"),
                node(3, "mesh"),
                floats("p", &[0., 1.]),
            ]
            .concat(),
        ),
    ];
    let root = std::env::temp_dir().join(format!(
        "hoi4_single_parse_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(root.join("gfx/models")).unwrap();
    for (name, bytes) in &files {
        std::fs::write(root.join(format!("gfx/models/{name}.mesh")), bytes).unwrap();
    }
    let descriptors = mesh::parse_descriptors(
        br#"
        objectTypes = {
            pdxmesh = { name = body file = "gfx/models/body.mesh" scale = 3 }
            pdxmesh = { name = tri file = "gfx/models/tri.mesh" }
            pdxmesh = { name = magic file = "gfx/models/magic.mesh" }
            pdxmesh = { name = short_tx file = "gfx/models/short_tx.mesh" }
            pdxmesh = { name = bad_locator file = "gfx/models/bad_locator.mesh" }
            pdxmesh = { name = late file = "gfx/models/late.mesh" scale = 0.75 }
            pdxmesh = { name = absent file = "gfx/models/absent.mesh" }
        }
        entity = { name = infantry pdxmesh = body state = { name = idle } }
        entity = { name = cavalry pdxmesh = body scale = 0.5 attach = { marker = rider }
            locator = { name = marker position = { 0 -0.0 2 } } state = { name = idle } }
        entity = { name = rider pdxmesh = body scale = 2 state = { name = idle } }
        entity = { name = fallen pdxmesh = body attach = { marker = broken }
            locator = { name = marker position = { 1 0 0 } } state = { name = idle } }
        entity = { name = broken pdxmesh = tri state = { name = idle } }
        entity = { name = garbled pdxmesh = magic }
        entity = { name = unjointed pdxmesh = short_tx }
        entity = { name = unlocated pdxmesh = bad_locator }
        entity = { name = halfway pdxmesh = late }
        entity = { name = refused pdxmesh = absent }
    "#,
        "synthetic.asset",
    )
    .unwrap();
    let roster = [
        "infantry",
        "cavalry",
        "fallen",
        "garbled",
        "unjointed",
        "unlocated",
        "halfway",
        "refused",
        "missing",
        "rider",
    ];
    let layers = ResolvedLayers::from_folders_with_coverage_policy(&[(&root, &[])], 2).unwrap();
    let report = layers.report_uncovered_reads();
    let mut actual = ModelLibrary::empty(Decode::default());
    let mut expected = ModelLibrary::empty(Decode::default());
    let mut read_paths = BTreeSet::new();
    let mut successes = BTreeMap::<&str, usize>::new();
    let mut failures = BTreeMap::<&str, usize>::new();
    for hour in 0_usize..96 {
        let mut shard = ModelLibrary::empty(Decode::default());
        let mut reference = ModelLibrary::empty(Decode::default());
        for at in 0..4 {
            let name = roster[(hour * 7 + at * 3) % roster.len()];
            let result = shard
                .add(&layers, &descriptors, name)
                .map_err(|error| (error.kind(), error.to_string()));
            let expected_result = refusal(name).map_or(Ok(()), |error| {
                Err((std::io::ErrorKind::InvalidData, error))
            });
            assert_eq!(result, expected_result, "hour {hour} request {at} {name}");
            expected_add(&mut reference, &layers, &descriptors, name);
            libraries_equal(&shard, &reference);
            if result.is_ok() {
                *successes.entry(name).or_default() += 1;
            } else {
                *failures.entry(name).or_default() += 1;
            }
            let file = match name {
                "infantry" | "cavalry" | "fallen" | "rider" => "body",
                "garbled" => "magic",
                "unjointed" => "short_tx",
                "unlocated" => "bad_locator",
                "halfway" => "late",
                "refused" => "absent",
                "missing" => continue,
                _ => unreachable!(),
            };
            read_paths.insert(format!("gfx/models/{file}.mesh"));
            if name == "fallen" {
                read_paths.insert("gfx/models/tri.mesh".into());
            }
        }
        actual.merge(shard);
        expected.merge(reference);
        libraries_equal(&actual, &expected);
        assert_eq!(
            report.warning(),
            Some(format!(
                "Content: {} file(s) read outside the recorded content coverage policy 2; these bytes are not hash-verified: {}.",
                read_paths.len(),
                read_paths.iter().cloned().collect::<Vec<_>>().join(", ")
            ))
        );
        assert_eq!(report.overlay_warning(), None);
        assert_eq!(report.overlay_refusals(), None);
    }
    assert_eq!(
        successes.keys().copied().collect::<Vec<_>>(),
        ["cavalry", "fallen", "infantry", "rider"]
    );
    assert_eq!(
        failures.keys().copied().collect::<Vec<_>>(),
        [
            "garbled",
            "halfway",
            "missing",
            "refused",
            "unjointed",
            "unlocated"
        ]
    );
    assert_eq!(
        successes.values().sum::<usize>() + failures.values().sum::<usize>(),
        384
    );
    let names = roster
        .iter()
        .chain(roster.iter())
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();
    let mut warnings = Vec::new();
    let loaded = ModelLibrary::load(
        &layers,
        Decode::default(),
        &descriptors,
        &names,
        &mut warnings,
    )
    .unwrap();
    let mut reference = ModelLibrary::empty(Decode::default());
    for name in roster {
        expected_add(&mut reference, &layers, &descriptors, name);
    }
    reference.asset_namespace = None;
    reference.failed = failures
        .keys()
        .map(|name| ((*name).to_owned(), refusal(name).unwrap()))
        .collect();
    libraries_equal(&loaded, &reference);
    let texture_ids = vec!["blank-srgb".to_owned(), "blank-linear".to_owned()];
    for slot in 0..reference.meshes.len() {
        assert_eq!(
            loaded.native_draw_snapshot(slot, &texture_ids),
            reference.native_draw_snapshot(slot, &texture_ids)
        );
    }
    let attachment = format!(
        "Attached entities refused: fallen -> broken: {} (1).",
        refusal("broken").unwrap()
    );
    let missing = reference
        .failed
        .iter()
        .map(|(name, reason)| format!("{name} ({reason})"))
        .collect::<Vec<_>>()
        .join(", ");
    assert_eq!(
        warnings,
        [
            (content_reader_parser::DiagnosticKind::Refused, attachment),
            (
                content_reader_parser::DiagnosticKind::Refused,
                format!("No mesh resolved for {missing}; those objects are not drawn.")
            ),
        ]
    );
    std::fs::remove_dir_all(root).unwrap();
}
