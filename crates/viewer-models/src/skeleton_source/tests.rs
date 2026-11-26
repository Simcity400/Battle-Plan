use crate::skeleton_source::*;
use crate::{ModelLibrary, ModelMesh, test_support::triangle_mesh};
use content_reader_layers::ResolvedLayers;
use content_reader_image::native::Decode;
use std::sync::Arc;

fn transform_bits(value: &mesh::Transform) -> [[u32; 3]; 4] {
    [
        value.basis[0],
        value.basis[1],
        value.basis[2],
        value.translation,
    ]
    .map(|axis| axis.map(f32::to_bits))
}

fn skeleton_equal(left: &mesh::Mesh, right: &mesh::Mesh) {
    let mesh::Mesh {
        lod_distances,
        shapes,
        skeleton,
        locators,
    } = left;
    assert_eq!(
        lod_distances
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        right
            .lod_distances
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>()
    );
    assert!(shapes.is_empty());
    assert_eq!(locators, &right.locators);
    assert_eq!(skeleton.len(), right.skeleton.len());
    for (left, right) in skeleton.iter().zip(&right.skeleton) {
        assert_eq!(
            (&left.name, left.index, left.parent),
            (&right.name, right.index, right.parent)
        );
        assert_eq!(
            left.inverse_bind.as_ref().map(transform_bits),
            right.inverse_bind.as_ref().map(transform_bits)
        );
    }
}

#[test]
fn skeleton_copy_keeps_every_retained_bit() {
    let values = [
        0.,
        -0.,
        f32::from_bits(0x7fc0_0123),
        f32::from_bits(0xffc0_4567),
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::from_bits(1),
        2.5,
    ];
    for count in 0..48 {
        let decoded = mesh::Mesh {
            lod_distances: values[..count % (values.len() + 1)].to_vec(),
            shapes: vec![mesh::Shape {
                name: "discarded geometry".into(),
                meshes: vec![mesh::SubMesh {
                    positions: vec![[1., -0., 3.]; count],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            skeleton: (0..count)
                .map(|at| mesh::Joint {
                    name: format!("joint {at}"),
                    index: (at / 2) as u32,
                    parent: at.checked_sub(1).map(|parent| parent as u32),
                    inverse_bind: (!at.is_multiple_of(3)).then(|| mesh::Transform {
                        basis: std::array::from_fn(|c| {
                            std::array::from_fn(|r| values[(at + c * 3 + r) % values.len()])
                        }),
                        translation: std::array::from_fn(|r| values[(at + r) % values.len()]),
                    }),
                })
                .collect(),
            locators: vec!["mount".into(), "mount".into(), "?".into()],
        };
        skeleton_equal(&crate::skeleton_source::skeleton(&decoded), &decoded);
        assert_eq!(decoded.shapes.len(), 1);
    }
}

fn geometry_equal(
    left: &viewer_scene_base::mesh_shader::Geometry,
    right: &viewer_scene_base::mesh_shader::Geometry,
) {
    assert_eq!((left.uv1, left.stride), (right.uv1, right.stride));
    assert_eq!(
        bytemuck::cast_slice::<_, u8>(&left.vertices),
        bytemuck::cast_slice::<_, u8>(&right.vertices)
    );
    assert_eq!(
        bytemuck::cast_slice::<_, u8>(&left.skin),
        bytemuck::cast_slice::<_, u8>(&right.skin)
    );
    assert_eq!(
        bytemuck::cast_slice::<_, u8>(&left.indices),
        bytemuck::cast_slice::<_, u8>(&right.indices)
    );
}

fn bits(values: &[f32]) -> Vec<u32> {
    values.iter().map(|value| value.to_bits()).collect()
}

type AnimationBits = (
    u32,
    u32,
    Vec<(String, String, Vec<u32>)>,
    [Vec<u32>; 3],
    Vec<Vec<u32>>,
);

fn animation_bits(animation: &mesh::Animation) -> AnimationBits {
    let joints = animation
        .joints
        .iter()
        .map(|joint| {
            let mesh::AnimationJoint {
                name,
                channels,
                translation,
                rotation,
                scale,
            } = joint;
            let rest = [&translation[..], &rotation[..], &[*scale][..]].concat();
            (name.clone(), channels.clone(), bits(&rest))
        })
        .collect();
    let banks = animation
        .sample_storage()
        .map(|(samples, _capacity)| bits(samples));
    let frames = (0..=animation.samples)
        .map(|frame| {
            animation
                .frame_trs_iter(frame)
                .flat_map(|pose| {
                    let mesh::JointPose {
                        translation,
                        rotation,
                        scale,
                    } = pose;
                    bits(&[&translation[..], &rotation[..], &[scale][..]].concat())
                })
                .collect()
        })
        .collect();
    (
        animation.fps.to_bits(),
        animation.samples,
        joints,
        banks,
        frames,
    )
}

fn same_animation(left: Option<&Arc<mesh::Animation>>, right: Option<&Arc<mesh::Animation>>) {
    assert_eq!(
        left.map(|animation| animation_bits(animation)),
        right.map(|animation| animation_bits(animation))
    );
}

fn state_bits(state: &mesh::EntityState) -> impl PartialEq + std::fmt::Debug {
    let mesh::EntityState {
        name,
        animation,
        looping,
        next_state,
        chance,
        chance_weight,
        blend,
        speed,
        start,
        random_start,
        propagate,
        events,
        naval_hitmiss_effects,
        naval_hitmiss_error,
    } = state;
    let events: Vec<_> = events
        .iter()
        .map(|event| {
            let mesh::EntityEvent {
                id,
                time,
                node,
                light,
                particle,
                sound,
                keep_particle,
                trigger_once,
            } = event;
            (
                (id.clone(), node.clone(), light.clone()),
                (particle.clone(), sound.clone()),
                (time.to_bits(), *keep_particle, *trigger_once),
            )
        })
        .collect();
    let hitmiss: Vec<_> = naval_hitmiss_effects
        .iter()
        .map(|mesh::NavalHitMissEffect { time, kind }| (time.to_bits(), *kind))
        .collect();
    (
        (
            name.clone(),
            animation.clone(),
            *looping,
            next_state.clone(),
        ),
        bits(&[*chance, *blend, *speed, *start, *random_start]),
        (*chance_weight, propagate.clone()),
        (events, hitmiss, naval_hitmiss_error.clone()),
    )
}

fn sharing(library: &ModelLibrary) -> [Vec<usize>; 5] {
    use std::collections::BTreeMap;
    fn class<T>(classes: &mut BTreeMap<*const T, usize>, item: &Arc<T>) -> usize {
        let next = classes.len();
        *classes.entry(Arc::as_ptr(item)).or_insert(next)
    }
    let mut animations = BTreeMap::new();
    let mut geometry = BTreeMap::new();
    let mut meshes = BTreeMap::new();
    let mut sources = BTreeMap::new();
    let mut rigs = BTreeMap::new();
    let mut walk: [Vec<usize>; 5] = Default::default();
    for animation in library.animations.values() {
        walk[0].push(
            animation
                .as_ref()
                .map_or(usize::MAX, |animation| class(&mut animations, animation)),
        );
    }
    for cached in library.geometry_cache.values() {
        walk[1].push(class(&mut geometry, cached));
    }
    for model in &library.meshes {
        for part in model
            .native
            .iter()
            .flat_map(|native| native.lods.iter().flatten())
        {
            walk[1].push(class(&mut geometry, &part.geometry));
        }
    }
    for rig in &library.rigs {
        walk[4].push(class(&mut rigs, rig));
        for node in &rig.nodes {
            walk[2].push(class(&mut meshes, &node.mesh));
            for clip in &node.clips {
                walk[0].push(
                    clip.animation
                        .as_ref()
                        .map_or(usize::MAX, |animation| class(&mut animations, animation)),
                );
            }
            if let Some(source) = &node.native {
                walk[3].push(class(&mut sources, source));
                walk[0].push(
                    source
                        .frozen
                        .as_ref()
                        .map_or(usize::MAX, |animation| class(&mut animations, animation)),
                );
            }
        }
    }
    walk
}

fn meshes_equal(left: &ModelMesh, right: &ModelMesh) {
    let ModelMesh {
        asset,
        name,
        path,
        scale,
        lod_distances,
        native,
        radius,
        height,
        visual,
        own_radius,
        bound,
        convoy_radius,
        attachments,
        cull_distance_squared,
    } = left;
    assert_eq!(cull_distance_squared.to_bits(), right.cull_distance_squared.to_bits());
    assert_eq!(asset, &right.asset);
    assert_eq!(
        (name, path, visual, attachments),
        (&right.name, &right.path, &right.visual, &right.attachments)
    );
    assert_eq!(
        bits(&[*scale, *radius, *height, *own_radius, *bound]),
        bits(&[
            right.scale,
            right.radius,
            right.height,
            right.own_radius,
            right.bound
        ])
    );
    assert_eq!(
        convoy_radius.map(f32::to_bits),
        right.convoy_radius.map(f32::to_bits)
    );
    assert_eq!(bits(lod_distances), bits(&right.lod_distances));
    assert_eq!(native.len(), right.native.len());
    for (left, right) in native.iter().zip(&right.native) {
        let crate::native::Descriptor {
            name,
            node,
            distances,
            lods,
        } = left;
        assert_eq!((name, node), (&right.name, &right.node));
        assert_eq!(bits(distances), bits(&right.distances));
        assert_eq!(
            lods.iter().map(Vec::len).collect::<Vec<_>>(),
            right.lods.iter().map(Vec::len).collect::<Vec<_>>()
        );
        for (left, right) in lods.iter().flatten().zip(right.lods.iter().flatten()) {
            let crate::native::Part {
                geometry,
                geometry_key,
                material,
                material_index,
            } = left;
            assert_eq!(
                (geometry_key, material, material_index),
                (&right.geometry_key, &right.material, &right.material_index)
            );
            geometry_equal(geometry, &right.geometry);
        }
    }
}

fn nodes_equal(
    left: &crate::entity_animation::Node,
    right: &crate::entity_animation::Node,
) {
    let crate::entity_animation::Node {
        mesh,
        native,
        clips,
        default,
        follows_parent,
        parent,
        scale,
        mesh_scale,
        skin_base,
        rigid,
        locators,
    } = left;
    assert!(mesh.shapes.is_empty() && right.mesh.shapes.is_empty());
    skeleton_equal(mesh, &right.mesh);
    assert_eq!(
        (default, follows_parent, parent, skin_base, rigid),
        (
            &right.default,
            &right.follows_parent,
            &right.parent,
            &right.skin_base,
            &right.rigid
        )
    );
    assert_eq!(
        bits(&[*scale, *mesh_scale]),
        bits(&[right.scale, right.mesh_scale])
    );
    assert_eq!(
        locators.keys().collect::<Vec<_>>(),
        right.locators.keys().collect::<Vec<_>>()
    );
    for (name, (transform, joint)) in locators {
        let (other, other_joint) = &right.locators[name];
        assert_eq!(joint, other_joint);
        assert_eq!(transform_bits(transform), transform_bits(other));
    }
    assert_eq!(clips.len(), right.clips.len());
    for (left, right) in clips.iter().zip(&right.clips) {
        assert_eq!(state_bits(&left.state), state_bits(&right.state));
        same_animation(left.animation.as_ref(), right.animation.as_ref());
    }
    assert_eq!(native.is_some(), right.native.is_some());
    let (Some(left), Some(right)) = (native, &right.native) else {
        return;
    };
    let crate::mesh_pose::Source {
        pdxmesh,
        skeleton,
        entity_locators,
        binary_locators,
        mesh_scale,
        entity_scale,
        frozen,
        game_data,
    } = &**left;
    assert_eq!(pdxmesh, &right.pdxmesh);
    assert_eq!(
        bits(&[*mesh_scale, *entity_scale]),
        bits(&[right.mesh_scale, right.entity_scale])
    );
    same_animation(frozen.as_ref(), right.frozen.as_ref());
    let game_data_bits = |data: &Option<Result<mesh::EntityGameData, String>>| {
        data.clone().map(|data| {
            data.map(|data| {
                let mesh::EntityGameData {
                    texture_anim_speed,
                    atlas_size,
                } = data;
                (texture_anim_speed.to_bits(), atlas_size)
            })
        })
    };
    assert_eq!(game_data_bits(game_data), game_data_bits(&right.game_data));
    assert_eq!(skeleton.len(), right.skeleton.len());
    for (left, right) in skeleton.iter().zip(&right.skeleton) {
        let mesh::NativeJoint {
            name,
            parent,
            inverse_bind,
        } = left;
        assert_eq!((name, parent), (&right.name, &right.parent));
        assert_eq!(bits(inverse_bind), bits(&right.inverse_bind));
    }
    for (left, right) in [
        (entity_locators, &right.entity_locators),
        (binary_locators, &right.binary_locators),
    ] {
        assert_eq!(left.len(), right.len());
        for (left, right) in left.iter().zip(right) {
            let crate::mesh_pose::Locator {
                name,
                local,
                parent,
                error,
            } = left;
            assert_eq!(
                (name, parent, error),
                (&right.name, &right.parent, &right.error)
            );
            assert_eq!(
                local.map(|row| row.map(f32::to_bits)),
                right.local.map(|row| row.map(f32::to_bits))
            );
        }
    }
}

fn libraries_equal(left: &ModelLibrary, right: &ModelLibrary) {
    assert_eq!(left.decode(), right.decode());
    assert_eq!(left.layers.is_some(), right.layers.is_some());
    assert_eq!(left.asset_namespace, right.asset_namespace);
    assert_eq!(
        left.animations.keys().collect::<Vec<_>>(),
        right.animations.keys().collect::<Vec<_>>()
    );
    for (path, animation) in &left.animations {
        same_animation(animation.as_ref(), right.animations[path].as_ref());
    }
    assert_eq!(left.index, right.index);
    assert_eq!(left.materials, right.materials);
    assert_eq!(left.textures, right.textures);
    assert_eq!(left.spaces, right.spaces);
    assert_eq!(left.paths, right.paths);
    assert_eq!(left.failed, right.failed);
    assert_eq!(left.animation_refusals, right.animation_refusals);
    assert_eq!(left.attachment_refusals, right.attachment_refusals);
    assert_eq!(left.alternatives, right.alternatives);
    assert_eq!(
        (left.alternative, left.composing_visual),
        (right.alternative, right.composing_visual)
    );
    assert_eq!(
        bytemuck::cast_slice::<_, u8>(&left.palette),
        bytemuck::cast_slice::<_, u8>(&right.palette)
    );
    assert_eq!(left.meshes.len(), right.meshes.len());
    for (left, right) in left.meshes.iter().zip(&right.meshes) {
        meshes_equal(left, right);
    }
    assert_eq!(
        left.geometry_cache.keys().collect::<Vec<_>>(),
        right.geometry_cache.keys().collect::<Vec<_>>()
    );
    for (key, geometry) in &left.geometry_cache {
        geometry_equal(geometry, &right.geometry_cache[key]);
    }
    assert_eq!(left.rigs.len(), right.rigs.len());
    for (left, right) in left.rigs.iter().zip(&right.rigs) {
        assert_eq!(
            (left.start, left.end, left.nodes.len()),
            (right.start, right.end, right.nodes.len())
        );
        for (left, right) in left.nodes.iter().zip(&right.nodes) {
            nodes_equal(left, right);
        }
    }
    assert_eq!(sharing(left), sharing(right));
}

#[test]
fn merged_textures_keep_their_storage_and_first_seen_slots() {
    use crate::ModelMaterial;
    use content_reader_image::dds::{MipLevel, Texture, TextureFormat};
    use viewer_gpu::textures::ColorSpace::{Linear, Srgb};

    let texture = |fill: u8| Texture {
        format: TextureFormat::Rgba8,
        width: 2,
        height: 2,
        faces: 1,
        mips: vec![vec![
            MipLevel {
                width: 2,
                height: 2,
                data: (vec![fill; 16]).into(),
            },
            MipLevel {
                width: 1,
                height: 1,
                data: (vec![fill + 1; 4]).into(),
            },
        ]],
    };
    let mut library = ModelLibrary::empty(Decode::default());
    library.textures.push(crate::ModelTexture::held(texture(10)));
    library.spaces.push(Srgb);
    library.paths.insert(("shared.dds".into(), Srgb), 0);
    let mut shard = ModelLibrary::empty(Decode::default());
    shard.textures = [texture(20), texture(30), texture(40)].map(crate::ModelTexture::held).into();
    shard.spaces = vec![Srgb, Linear, Srgb];
    shard.paths = std::collections::BTreeMap::from([
        (("shared.dds".into(), Srgb), 0),
        (("shared.dds".into(), Linear), 1),
        (("failed.dds".into(), Srgb), 2),
    ]);
    shard.materials = vec![ModelMaterial {
        diffuse: 0,
        normal: 1,
        specular: 0,
    }];
    shard
        .failed
        .insert("refused".into(), "synthetic refusal".into());
    let expected = vec![texture(10), texture(30), texture(40)];
    let storage = |texture: &crate::ModelTexture| {
        texture.source().unwrap().mips[0]
            .iter()
            .map(|mip| mip.data.as_ptr())
            .collect::<Vec<_>>()
    };
    let sources = |library: &ModelLibrary| library.textures.iter().map(|texture| texture.source().unwrap()).collect::<Vec<_>>();
    let retained_storage = storage(&library.textures[0]);
    let moved_storage = [storage(&shard.textures[1]), storage(&shard.textures[2])];
    library.merge(shard);
    assert_eq!(sources(&library), expected);
    assert_eq!(library.spaces, [Srgb, Linear, Srgb]);
    assert_eq!(library.paths[&("shared.dds".into(), Srgb)], 0);
    assert_eq!(library.paths[&("shared.dds".into(), Linear)], 1);
    assert_eq!(library.paths[&("failed.dds".into(), Srgb)], 2);
    assert_eq!(
        library.materials,
        [ModelMaterial {
            diffuse: 0,
            normal: 1,
            specular: 0,
        }]
    );
    assert_eq!(library.failed["refused"], "synthetic refusal");
    assert_eq!(storage(&library.textures[0]), retained_storage);
    assert_eq!(storage(&library.textures[1]), moved_storage[0]);
    assert_eq!(storage(&library.textures[2]), moved_storage[1]);
    library.merge(ModelLibrary::empty(Decode::default()));
    assert_eq!(sources(&library), expected);
    assert_eq!(storage(&library.textures[1]), moved_storage[0]);
    assert_eq!(storage(&library.textures[2]), moved_storage[1]);
}

#[test]
fn failed_compositions_keep_unique_resources_through_repeated_and_empty_merges() {
    use crate::{AssetKey, ModelMaterial, identity};
    use content_reader_image::dds::{MipLevel, Texture, TextureFormat};
    use std::collections::BTreeMap;
    use viewer_gpu::textures::ColorSpace::{Linear, Srgb};

    let root = std::env::temp_dir().join(format!(
        "hoi4_failed_merge_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(root.join("gfx/models")).unwrap();
    for (name, bytes) in [
        (
            "partial.mesh",
            textured_mesh(1.5, ["crack.dds", "absent_n.dds", ""]),
        ),
        ("plane.mesh", textured_mesh(1., ["plane.dds", "", ""])),
        ("crack.dds", dds_file(2, 5)),
        ("plane.dds", dds_file(2, 9)),
        ("crack.anim", sampled_animation()),
    ] {
        std::fs::write(root.join("gfx/models").join(name), bytes).unwrap();
    }
    let layers = ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    let descriptors = mesh::parse_descriptors(
        br#"
        objectTypes = {
            pdxmesh = { name = partial file = "gfx/models/partial.mesh" scale = 2
                animation = { id = "idle" type = "crack_anim" } }
            pdxmesh = { name = plain file = "gfx/models/plane.mesh" scale = 1 }
        }
        entity = { name = cracked pdxmesh = partial state = { name = idle animation = idle } }
        entity = { name = plane pdxmesh = plain state = { name = idle } }
        animation = { name = "crack_anim" file = "gfx/models/crack.anim" }
    "#,
        "synthetic.asset",
    )
    .unwrap();
    let decode = Decode {
        force_pow2_textures: true,
    };
    let texture = |side: u32, fill: u8| Texture {
        format: TextureFormat::Rgba8,
        width: side,
        height: side,
        faces: 1,
        mips: vec![vec![MipLevel {
            width: side,
            height: side,
            data: (vec![fill; (side * side * 4) as usize]).into(),
        }]],
    };
    let failure = concat!(
        "gfx/models/absent_n.dds: no layer holds any of [",
        "\"gfx/models/absent_n.dds\", \"gfx/models/absent_n.tga\", ",
        "\"gfx/models/absent_n.dds\"]"
    );
    let expected_failed = BTreeMap::from([("cracked".to_owned(), failure.to_owned())]);
    let expected_animation: AnimationBits = (
        30_f32.to_bits(),
        2,
        vec![(
            "root".into(),
            "t".into(),
            bits(&[0., 0., 0., 0., 0., 0., 1., 1.]),
        )],
        [bits(&[0., 1., 2., 3., 4., 5.5]), vec![], vec![]],
        vec![
            bits(&[0., 1., 2., 0., 0., 0., 1., 1.]),
            bits(&[3., 4., 5.5, 0., 0., 0., 1., 1.]),
            bits(&[0., 0., 0., 0., 0., 0., 1., 1.]),
        ],
    );
    let assert_crack = |library: &ModelLibrary, slot: usize| {
        assert_eq!(library.failed, expected_failed);
        assert_eq!(library.paths[&("gfx/models/crack.dds".into(), Srgb)], slot);
        assert_eq!(library.textures[slot], texture(2, 5));
        assert_eq!(
            library
                .animations
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["gfx/models/crack.anim"]
        );
        assert_eq!(
            animation_bits(
                library.animations["gfx/models/crack.anim"]
                    .as_ref()
                    .unwrap()
            ),
            expected_animation
        );
        assert!(library.animation_refusals.is_empty());
    };
    let mut library = ModelLibrary::empty(decode);
    library.add(&layers, &descriptors, "plane").unwrap();
    assert_eq!(library.textures, [texture(2, 9), texture(1, 0)]);
    assert_eq!(
        library.paths,
        BTreeMap::from([
            (("gfx/models/plane.dds".into(), Srgb), 0),
            ((String::new(), Linear), 1),
        ])
    );
    assert!(library.animations.is_empty());
    assert!(library.failed.is_empty());
    let expected_asset = AssetKey::selected("plane", identity::namespace(&layers), decode);
    for names in [&["cracked"][..], &[], &["cracked"], &[]] {
        let mut warnings = Vec::new();
        let shard = ModelLibrary::load(
            &layers,
            decode,
            &descriptors,
            &names
                .iter()
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>(),
            &mut warnings,
        )
        .unwrap();
        assert!(shard.index.is_empty());
        assert!(shard.meshes.is_empty());
        assert!(shard.rigs.is_empty());
        assert!(shard.palette.is_empty());
        assert!(shard.materials.is_empty());
        assert!(shard.geometry_cache.is_empty());
        if names.is_empty() {
            assert!(warnings.is_empty());
            assert!(shard.failed.is_empty());
            assert!(shard.paths.is_empty());
            assert!(shard.textures.is_empty());
            assert!(shard.spaces.is_empty());
            assert!(shard.animations.is_empty());
        } else {
            assert_eq!(
                warnings,
                [(
                    content_reader_parser::DiagnosticKind::Refused,
                    format!(
                        "No mesh resolved for cracked ({failure}); those objects are not drawn."
                    )
                )]
            );
            assert_crack(&shard, 0);
            assert_eq!(
                shard.paths,
                BTreeMap::from([(("gfx/models/crack.dds".into(), Srgb), 0)])
            );
            assert_eq!(shard.textures, [texture(2, 5)]);
            assert_eq!(shard.spaces, [Srgb]);
        }
        library.merge(shard);
        assert_crack(&library, 2);
        assert_eq!(
            library.paths,
            BTreeMap::from([
                (("gfx/models/plane.dds".into(), Srgb), 0),
                ((String::new(), Linear), 1),
                (("gfx/models/crack.dds".into(), Srgb), 2),
            ])
        );
        assert_eq!(
            library.textures,
            [texture(2, 9), texture(1, 0), texture(2, 5)]
        );
        assert_eq!(library.spaces, [Srgb, Linear, Srgb]);
        assert_eq!(
            library.materials,
            [ModelMaterial {
                diffuse: 0,
                normal: 1,
                specular: 1,
            }]
        );
        assert_eq!(library.index, BTreeMap::from([("plane".to_owned(), 0)]));
        assert_eq!(library.meshes.len(), 1);
        assert_eq!(library.meshes[0].asset, expected_asset);
        assert_eq!(library.rigs.len(), 1);
        assert_eq!(library.palette.len(), 1);
        assert_eq!(library.geometry_cache.len(), 1);
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn skeleton_only_bakes_keep_geometry_palette_rig_and_merge_bytes() {
    let root =
        std::env::temp_dir().join(format!("hoi4_skeleton_bytes_{}_{}", std::process::id(), 8));
    std::fs::create_dir_all(&root).unwrap();
    let mut bytes = triangle_mesh(3.5);
    bytes.extend(b"[[[skeleton\0[[[[mount\0!\x02ixi\x01\0\0\0\0\0\0\0!\x02txf\x0c\0\0\0");
    bytes.extend(
        [1_f32, 0., 0., 0., 1., 0., 0., 0., 1., -1., 0., 0.]
            .into_iter()
            .flat_map(f32::to_le_bytes),
    );
    std::fs::write(root.join("body.mesh"), bytes).unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    let descriptors = mesh::parse_descriptors(
        br#"
        objectTypes = { pdxmesh = { name = body file = "body.mesh" scale = 3 } }
        entity = { name = parent pdxmesh = body scale = 2 attach = { marker = child }
            locator = { name = marker position = { 0 -0.0 2 } }
            state = { name = idle } }
        entity = { name = child pdxmesh = body scale = 0.5 state = { name = idle } }
        entity = { name = other pdxmesh = body scale = 1.25 state = { name = idle } }
    "#,
        "synthetic.asset",
    )
    .unwrap();
    let mut new = ModelLibrary::empty(Decode::default());
    for names in [["parent", "child"], ["other", "missing"]] {
        let mut shard = ModelLibrary::empty(Decode::default());
        for name in names {
            let result = shard.add(&layers, &descriptors, name);
            assert_eq!(result.is_ok(), name != "missing", "{name}");
            if let Err(error) = result {
                assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
                assert_eq!(error.to_string(), "no pdxmesh or entity declares it");
            }
        }
        new.merge(shard);
    }
    assert_eq!(
        new.index,
        std::collections::BTreeMap::from([
            ("parent".to_owned(), 0),
            ("child".to_owned(), 1),
            ("other".to_owned(), 2),
        ])
    );
    assert_eq!(
        new.rigs
            .iter()
            .map(|rig| rig.nodes.len())
            .collect::<Vec<_>>(),
        [2, 1, 1]
    );
    assert_eq!(new.palette.len(), 4);
    assert_eq!(new.index.len(), 3);
    assert!(
        new.meshes
            .iter()
            .map(|mesh| mesh.attachments)
            .sum::<usize>()
            > 0
    );
    assert!(!new.geometry_cache.is_empty());
    assert!(
        new.rigs
            .iter()
            .flat_map(|rig| &rig.nodes)
            .any(|node| !node.mesh.skeleton.is_empty())
    );
    let mut warnings = Vec::new();
    let names = ["parent", "child", "parent", "other", "missing", "missing"].map(str::to_owned);
    let before = viewer_runtime::worker_priority::current_priority();
    let actual = ModelLibrary::load(
        &layers,
        Decode::default(),
        &descriptors,
        &names,
        &mut warnings,
    )
    .unwrap();
    assert_eq!(
        viewer_runtime::worker_priority::current_priority(),
        before
    );
    new.failed
        .insert("missing".into(), "no pdxmesh or entity declares it".into());
    libraries_equal(&actual, &new);
    assert_eq!(
        warnings,
        vec![(
            content_reader_parser::DiagnosticKind::Refused,
            "No mesh resolved for missing (no pdxmesh or entity declares it); those objects are not drawn.".into(),
        )]
    );
    std::fs::remove_file(root.join("body.mesh")).unwrap();
    std::fs::remove_dir(root).unwrap();
}

#[test]
fn repeated_hour_bakes_keep_fixed_names_bytes_and_source_ordered_refusals() {
    let root = std::env::temp_dir().join(format!("hoi4_hour_bakes_{}", std::process::id()));
    std::fs::create_dir_all(root.join("gfx/models")).unwrap();
    std::fs::write(root.join("gfx/models/body.mesh"), triangle_mesh(3.5)).unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    let report = layers.report_uncovered_reads();
    let descriptors = mesh::parse_descriptors(
        br#"
        objectTypes = { pdxmesh = { name = body file = "gfx/models/body.mesh" scale = 3 }
            pdxmesh = { name = absent file = "gfx/models/absent.mesh" } }
        entity = { name = infantry pdxmesh = body state = { name = idle } }
        entity = { name = cavalry pdxmesh = body scale = 0.5 state = { name = idle } }
        entity = { name = destroyer pdxmesh = body scale = 1.25 state = { name = idle } }
        entity = { name = refused pdxmesh = absent }
    "#,
        "synthetic.asset",
    )
    .unwrap();
    let mut warm = ModelLibrary::empty(Decode::default());
    let mut cold = ModelLibrary::empty(Decode::default());
    let mut failures = 0;
    for hour in 0_usize..96 {
        let names = [
            if hour.is_multiple_of(3) {
                "infantry"
            } else {
                "cavalry"
            },
            if hour.is_multiple_of(5) {
                "refused"
            } else {
                "destroyer"
            },
            if hour.is_multiple_of(7) {
                "missing"
            } else {
                "infantry"
            },
        ];
        let fresh = ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
        let fresh_report = fresh.report_uncovered_reads();
        let mut warm_shard = ModelLibrary::empty(Decode::default());
        let mut cold_shard = ModelLibrary::empty(Decode::default());
        for name in names {
            let a = warm_shard
                .add(&layers, &descriptors, name)
                .map_err(|error| (error.kind(), error.to_string()));
            let b = cold_shard
                .add(&fresh, &descriptors, name)
                .map_err(|error| (error.kind(), error.to_string()));
            assert_eq!(a, b, "ordered result hour {hour} name {name}");
            assert_eq!(a.is_err(), matches!(name, "refused" | "missing"));
            if let Err((kind, why)) = &a {
                assert_eq!(*kind, std::io::ErrorKind::InvalidData);
                if name == "missing" {
                    assert_eq!(why, "no pdxmesh or entity declares it");
                } else {
                    assert!(why.starts_with(
                        "refused: pdxmesh absent (gfx/models/absent.mesh) cannot be loaded:"
                    ));
                }
            }
            failures += usize::from(a.is_err());
            libraries_equal(&warm_shard, &cold_shard);
        }
        warm.merge(warm_shard);
        cold.merge(cold_shard);
        libraries_equal(&warm, &cold);
        assert!(report.warning().unwrap().contains("gfx/models/body.mesh"));
        assert!(
            fresh_report
                .warning()
                .unwrap()
                .contains("gfx/models/body.mesh")
        );
        assert_eq!(report.overlay_warning(), fresh_report.overlay_warning());
        assert_eq!(report.overlay_refusals(), fresh_report.overlay_refusals());
    }
    assert_eq!(failures, 34);
    assert_eq!(warm.meshes.len(), 254);
    assert_eq!(warm.rigs.len(), 254);
    assert_eq!(warm.palette.len(), 254);
    assert_eq!(
        warm.index,
        std::collections::BTreeMap::from([
            ("infantry".to_owned(), 253),
            ("cavalry".to_owned(), 252),
            ("destroyer".to_owned(), 250),
        ])
    );
    assert!(!warm.geometry_cache.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

fn pdx_property(
    name: &str,
    kind: u8,
    count: usize,
    payload: impl IntoIterator<Item = u8>,
) -> Vec<u8> {
    let mut bytes = vec![b'!', name.len() as u8];
    bytes.extend(name.as_bytes());
    bytes.push(kind);
    bytes.extend((count as u32).to_le_bytes());
    bytes.extend(payload);
    bytes
}

fn pdx_string(name: &str, value: &str) -> Vec<u8> {
    let payload: Vec<u8> = (value.len() as u32)
        .to_le_bytes()
        .into_iter()
        .chain(value.bytes())
        .collect();
    pdx_property(name, b's', 1, payload)
}

fn pdx_floats(name: &str, values: &[f32]) -> Vec<u8> {
    pdx_property(
        name,
        b'f',
        values.len(),
        values.iter().flat_map(|value| value.to_le_bytes()),
    )
}

fn textured_mesh(reach: f32, [diffuse, normal, specular]: [&str; 3]) -> Vec<u8> {
    let mut bytes = triangle_mesh(reach);
    for (name, file) in [("diff", diffuse), ("n", normal), ("spec", specular)] {
        bytes.extend(pdx_string(name, file));
    }
    bytes
}

fn sampled_animation() -> Vec<u8> {
    [
        b"@@b@[info\0".to_vec(),
        pdx_floats("fps", &[30.]),
        pdx_property("sa", b'i', 1, 2_i32.to_le_bytes()),
        b"[[root\0".to_vec(),
        pdx_string("sa", "t"),
        pdx_floats("t", &[0., 0., 0.]),
        pdx_floats("q", &[0., 0., 0., 1.]),
        pdx_floats("s", &[1.]),
        b"[samples\0".to_vec(),
        pdx_floats("t", &[0., 1., 2., 3., 4., 5.5]),
    ]
    .concat()
}

fn dds_file(side: u32, fill: u8) -> Vec<u8> {
    let mut bytes = vec![0_u8; 128];
    bytes[..4].copy_from_slice(b"DDS ");
    for (offset, value) in [
        (4, 124),
        (12, side),
        (16, side),
        (28, 1),
        (76, 32),
        (80, 0x41),
        (88, 32),
        (92, 0xFF_0000),
        (96, 0xFF00),
        (100, 0xFF),
        (104, 0xFF00_0000),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&u32::to_le_bytes(value));
    }
    bytes.extend(std::iter::repeat_n(fill, (side * side * 4) as usize));
    bytes
}

fn rest_animation(with_samples: bool) -> Vec<u8> {
    let mut bytes = [
        b"@@b@[info\0".to_vec(),
        pdx_floats("fps", &[30.]),
        pdx_property("sa", b'i', 1, 2_i32.to_le_bytes()),
        b"[[root\0".to_vec(),
        pdx_string("sa", "t"),
        pdx_floats("t", &[0., 0., 0.]),
        pdx_floats("q", &[0., 0., 0., 1.]),
        pdx_floats("s", &[1.]),
    ]
    .concat();
    if with_samples {
        bytes.extend(b"[samples\0");
        bytes.extend(pdx_floats("t", &[0.; 6]));
    }
    bytes
}

fn paired(name: &str, animation: &[u8]) -> ModelLibrary {
    let root = std::env::temp_dir().join(format!(
        "hoi4_comparator_{name}_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("gfx/models")).unwrap();
    std::fs::write(root.join("gfx/models/body.mesh"), triangle_mesh(1.)).unwrap();
    std::fs::write(root.join("gfx/models/rest.anim"), animation).unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let descriptors = mesh::parse_descriptors(
        br#"
        objectTypes = { pdxmesh = { name = body file = "gfx/models/body.mesh" scale = 1
            animation = { id = "idle" type = "rest_anim" }
            animation = { id = "walk" type = "rest_anim" } } }
        entity = { name = paired pdxmesh = body
            state = { name = idle animation = idle } state = { name = walk animation = walk } }
        animation = { name = "rest_anim" file = "gfx/models/rest.anim" }
    "#,
        "synthetic.asset",
    )
    .unwrap();
    let mut library = ModelLibrary {
        asset_namespace: Some(Arc::from([])),
        ..ModelLibrary::empty(Decode::default())
    };
    library.add(&layers, &descriptors, "paired").unwrap();
    let _ = std::fs::remove_dir_all(&root);
    library
}

fn refused(left: &ModelLibrary, right: &ModelLibrary) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        libraries_equal(left, right)
    }))
    .is_err()
}

fn first_state(library: &mut ModelLibrary) -> &mut mesh::EntityState {
    &mut Arc::get_mut(&mut library.rigs[0])
        .expect("a baked rig is the library's own")
        .nodes[0]
        .clips[0]
        .state
}

#[test]
fn the_comparison_tells_apart_what_derived_equality_does_not() {
    let rest = rest_animation(false);
    let (base, mut same) = (paired("base", &rest), paired("same", &rest));
    assert!(!refused(&base, &same));
    let mut reidentified = paired("reidentified", &rest);
    reidentified.meshes[0].asset = crate::AssetKey::synthetic("another_entity");
    assert!(refused(&base, &reidentified));
    let shared = base.rigs[0].nodes[0].clips.clone();
    assert_eq!(shared.len(), 2);
    assert!(Arc::ptr_eq(
        shared[0].animation.as_ref().unwrap(),
        shared[1].animation.as_ref().unwrap()
    ));
    let mut signed = paired("signed", &rest);
    first_state(&mut same).blend = 0.0;
    first_state(&mut signed).blend = -0.0;
    assert_eq!(first_state(&mut signed).blend, first_state(&mut same).blend);
    assert!(refused(&same, &signed));
    let (mut left, mut right) = (paired("nan_left", &rest), paired("nan_right", &rest));
    first_state(&mut left).speed = f32::from_bits(0x7fc0_1234);
    first_state(&mut right).speed = f32::from_bits(0x7fc0_1234);
    assert!(!refused(&left, &right));
    first_state(&mut right).speed = f32::from_bits(0x7fc0_4321);
    assert!(refused(&left, &right));
    let banked = paired("banked", &rest_animation(true));
    let (poses, banks) = (
        animation_bits(base.animations["gfx/models/rest.anim"].as_ref().unwrap()),
        animation_bits(banked.animations["gfx/models/rest.anim"].as_ref().unwrap()),
    );
    assert_eq!(poses.4, banks.4);
    assert_ne!(poses.3, banks.3);
    assert!(refused(&base, &banked));
    let mut separate = paired("separate", &rest);
    let clip = &mut Arc::get_mut(&mut separate.rigs[0]).unwrap().nodes[0].clips[1];
    clip.animation = clip
        .animation
        .as_ref()
        .map(|animation| Arc::new((**animation).clone()));
    assert!(refused(&base, &separate));
}

#[path = "projection_tests.rs"]
mod projection;
