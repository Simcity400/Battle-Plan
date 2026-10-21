use super::*;
use crate::map::models::{ModelLibrary, tests::triangle_mesh};
use installed_content_core::ResolvedLayers;
use installed_content_image::native::Decode;

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
    // Exhaustive: a new retained Mesh field must extend this comparison.
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
fn skeleton_copy_keeps_every_retained_bit_and_first_duplicate_index() {
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
        let mut actual = super::skeleton(&decoded);
        skeleton_equal(&actual, &decoded);
        let mut seen = std::collections::BTreeSet::new();
        actual.skeleton.retain(|joint| seen.insert(joint.index));
        assert_eq!(actual.skeleton.len(), count.div_ceil(2));
        for joint in &actual.skeleton {
            let expected = &decoded.skeleton[joint.index as usize * 2];
            assert_eq!(
                (&joint.name, joint.index, joint.parent),
                (&expected.name, expected.index, expected.parent)
            );
            assert_eq!(
                joint.inverse_bind.as_ref().map(transform_bits),
                expected.inverse_bind.as_ref().map(transform_bits)
            );
        }
        assert_eq!(decoded.shapes.len(), 1);
    }
}

fn geometry_equal(
    left: &crate::map::mesh_shader::Geometry,
    right: &crate::map::mesh_shader::Geometry,
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

fn libraries_equal(left: &ModelLibrary, right: &ModelLibrary) {
    assert_eq!(left.index, right.index);
    assert_eq!(left.materials, right.materials);
    assert_eq!(left.textures, right.textures); // All mip metadata AND encoded/texel bytes.
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
    assert!(left.animations.is_empty() && right.animations.is_empty());
    assert_eq!(
        bytemuck::cast_slice::<_, u8>(&left.palette),
        bytemuck::cast_slice::<_, u8>(&right.palette)
    );
    assert_eq!(left.meshes, right.meshes);
    for (a, b) in left.meshes.iter().zip(&right.meshes) {
        assert_eq!(
            [a.scale, a.radius, a.height, a.own_radius, a.bound].map(f32::to_bits),
            [b.scale, b.radius, b.height, b.own_radius, b.bound].map(f32::to_bits)
        );
        assert_eq!(
            a.convoy_radius.map(f32::to_bits),
            b.convoy_radius.map(f32::to_bits)
        );
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&a.lod_distances),
            bytemuck::cast_slice::<_, u8>(&b.lod_distances)
        );
        for (a, b) in a.native.iter().zip(&b.native) {
            assert_eq!(
                bytemuck::cast_slice::<_, u8>(&a.distances),
                bytemuck::cast_slice::<_, u8>(&b.distances)
            );
            for (a, b) in a.lods.iter().flatten().zip(b.lods.iter().flatten()) {
                geometry_equal(&a.geometry, &b.geometry);
            }
        }
    }
    assert_eq!(left.geometry_cache.len(), right.geometry_cache.len());
    for (key, a) in &left.geometry_cache {
        geometry_equal(a, &right.geometry_cache[key]);
    }
    assert_eq!(left.rigs.len(), right.rigs.len());
    for (a, b) in left.rigs.iter().zip(&right.rigs) {
        assert_eq!(
            (a.start, a.end, a.nodes.len()),
            (b.start, b.end, b.nodes.len())
        );
        for (a, b) in a.nodes.iter().zip(&b.nodes) {
            skeleton_equal(&a.mesh, &b.mesh);
            assert_eq!(
                (
                    &a.default,
                    a.follows_parent,
                    &a.parent,
                    a.skin_base,
                    a.rigid
                ),
                (
                    &b.default,
                    b.follows_parent,
                    &b.parent,
                    b.skin_base,
                    b.rigid
                )
            );
            assert_eq!(
                [a.scale, a.mesh_scale].map(f32::to_bits),
                [b.scale, b.mesh_scale].map(f32::to_bits)
            );
            assert_eq!(a.locators.len(), b.locators.len());
            for (name, (transform, joint)) in &a.locators {
                let (other, other_joint) = &b.locators[name];
                assert_eq!(joint, other_joint);
                assert_eq!(transform_bits(transform), transform_bits(other));
            }
            assert_eq!(a.clips.len(), b.clips.len());
            for (a, b) in a.clips.iter().zip(&b.clips) {
                assert_eq!(a.state, b.state);
                assert!(a.animation.is_none() && b.animation.is_none());
            }
            assert_eq!(a.native.is_some(), b.native.is_some());
            if let (Some(a), Some(b)) = (&a.native, &b.native) {
                assert_eq!(a.pdxmesh, b.pdxmesh);
                assert_eq!(
                    [a.mesh_scale, a.entity_scale].map(f32::to_bits),
                    [b.mesh_scale, b.entity_scale].map(f32::to_bits)
                );
                assert!(a.frozen.is_none() && b.frozen.is_none());
                assert!(a.game_data.is_none() && b.game_data.is_none());
                assert_eq!(a.skeleton.len(), b.skeleton.len());
                for (a, b) in a.skeleton.iter().zip(&b.skeleton) {
                    assert_eq!((&a.name, a.parent), (&b.name, b.parent));
                    assert_eq!(
                        a.inverse_bind.map(f32::to_bits),
                        b.inverse_bind.map(f32::to_bits)
                    );
                }
                for (a, b) in [
                    (&a.entity_locators, &b.entity_locators),
                    (&a.binary_locators, &b.binary_locators),
                ] {
                    assert_eq!(a.len(), b.len());
                    for (a, b) in a.iter().zip(b) {
                        assert_eq!((&a.name, a.parent, &a.error), (&b.name, b.parent, &b.error));
                        assert_eq!(
                            a.local.map(|c| c.map(f32::to_bits)),
                            b.local.map(|c| c.map(f32::to_bits))
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn skeleton_only_bakes_keep_geometry_palette_rig_and_merge_bytes() {
    // Explicit source-order construction keeps attachment and merge expectations visible.
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
    // Direct add appends duplicates too: 96 * 3 requests minus 34 failures.
    // The latest occurrence supplies each name's slot, unlike load's deduped input.
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
