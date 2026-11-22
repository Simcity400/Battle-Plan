use super::*;
use std::f32::consts::TAU;
use viewer_gpu::camera::MapSize;
use viewer_gpu::province_admission::side_planes;

const MAP_SIZE_X: f32 = 5632.0;
const TILE: f32 = 256.0;
const MAP_SIZE_Y: f32 = 2048.0;

const TEST_CELL: f32 = MAP_SIZE_X / 11.0;

fn field(indices: &[u8], heights: &[u8]) -> TreeField {
    let image = TreeImage {
        width: 11,
        height: 4,
        indices,
    };
    generate(
        &image,
        &HeightField {
            width: MAP_SIZE_X as u32,
            height: MAP_SIZE_Y as u32,
            values: heights,
        },
    )
    .unwrap()
}

fn flat() -> Vec<u8> {
    vec![100; (MAP_SIZE_X * MAP_SIZE_Y) as usize]
}

#[test]
fn the_fade_matches_the_reference_camera_and_closes_the_pass_above_the_far_distance() {
    let defines = MapDefines::test();
    assert_eq!(tree_fade(&defines, 100.0), 1.0);
    assert_eq!(tree_fade(&defines, 250.0), 1.0);
    assert!((tree_fade(&defines, 310.178) - 0.398_22).abs() < 1e-4);
    assert_eq!(tree_fade(&defines, 350.0), 0.0);
    assert_eq!(tree_fade(&defines, 400.0), 0.0);
    let mut changed = defines;
    changed.tree_fade = [40.0, 80.0];
    assert_eq!(tree_fade(&changed, 40.0), 1.0);
    assert_eq!(tree_fade(&changed, 50.0), 0.75);
    assert_eq!(tree_fade(&changed, 80.0), 0.0);
    assert_eq!(tree_fade(&changed, 81.0), 0.0);
}

fn bitmap(pixels: &[(usize, usize, u8)]) -> Vec<u8> {
    let mut indices = vec![0u8; 11 * 4];
    for &(column, row, palette) in pixels {
        indices[row * 11 + column] = palette;
    }
    indices
}

#[test]
fn every_non_zero_pixel_becomes_one_instance_on_the_brick_lattice() {
    let heights = flat();
    let indices = bitmap(&[(0, 0, 1), (2, 0, 1), (1, 1, 2), (3, 1, 2)]);
    let field = field(&indices, &heights);
    assert_eq!(field.instances.len(), 4);
    assert_eq!(field.palettes, vec![1, 2]);
    assert!((field.cell[0] - TEST_CELL).abs() < 1e-3);
    assert!((field.cell[1] - MAP_SIZE_Y / 4.0).abs() < 1e-3);
    let mut positions = field
        .instances
        .iter()
        .map(|instance| (instance.position[0], instance.position[2]))
        .collect::<Vec<_>>();
    positions.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let expected = [
        (0.0, 3.0 * TEST_CELL),
        (1.5 * TEST_CELL, 2.0 * TEST_CELL),
        (2.0 * TEST_CELL, 3.0 * TEST_CELL),
        (3.5 * TEST_CELL, 2.0 * TEST_CELL),
    ];
    for (got, want) in positions.iter().zip(expected) {
        assert!(
            (got.0 - want.0).abs() < 1e-3 && (got.1 - want.1).abs() < 1e-3,
            "{positions:?} against {expected:?}"
        );
    }
}

#[test]
fn a_flat_height_field_gives_a_flat_lattice_and_no_slope() {
    let heights = flat();
    let field = field(&bitmap(&[(4, 2, 6)]), &heights);
    let instance = field.instances[0];
    assert!((instance.position[1] - 10.0).abs() < 1e-3);
    assert_eq!(instance.slope, [0.0, 0.0]);
    assert!((0.0..TAU).contains(&instance.rotation));
}

#[test]
fn the_slope_follows_the_four_corner_heights() {
    let width = MAP_SIZE_X as usize;
    let mut heights = vec![0u8; width * MAP_SIZE_Y as usize];
    for row in 0..MAP_SIZE_Y as usize {
        for column in 0..width {
            heights[row * width + column] = (column % 256) as u8;
        }
    }
    let image = TreeImage {
        width: 1408,
        height: 512,
        indices: &{
            let mut indices = vec![0u8; 1408 * 512];
            indices[186 * 1408 + 10] = 6;
            indices
        },
    };
    let field = generate(
        &image,
        &HeightField {
            width: MAP_SIZE_X as u32,
            height: MAP_SIZE_Y as u32,
            values: &heights,
        },
    )
    .unwrap();
    let instance = field.instances[0];
    assert!((instance.slope[0] - 0.1).abs() < 1e-3, "{instance:?}");
    assert!(instance.slope[1].abs() < 1e-6, "{instance:?}");
}

#[test]
fn instances_are_bucketed_by_palette_and_tile_and_stay_contiguous() {
    let heights = flat();
    let field = field(
        &bitmap(&[(0, 0, 5), (1, 0, 5), (2, 0, 6), (3, 0, 6)]),
        &heights,
    );
    assert_eq!(field.instances.len(), 4);
    let mut covered = 0;
    for bucket in &field.buckets {
        assert_eq!(bucket.range.start, covered);
        covered = bucket.range.end;
        for index in bucket.range.clone() {
            let instance = field.instances[index as usize];
            assert_eq!((instance.position[0] / TILE) as u32, bucket.tile[0]);
            assert_eq!((instance.position[2] / TILE) as u32, bucket.tile[1]);
        }
    }
    assert_eq!(covered, field.instances.len() as u32);
    assert!(field.buckets.iter().any(|bucket| bucket.palette == 5));
    assert!(field.buckets.iter().any(|bucket| bucket.palette == 6));
}

#[test]
fn height_sampling_interpolates_bytes_before_scaling_and_flips_rows() {
    let heights = HeightField {
        width: 2,
        height: 2,
        values: &[80, 160, 0, 40],
    };
    assert_eq!(tree_height(&heights, 0.0, 0.0), 0.0);
    assert_eq!(tree_height(&heights, 1.0, 1.0), 16.0);
    assert_eq!(tree_height(&heights, 0.5, 0.5), 7.0);
    assert_eq!(tree_height(&heights, 0.25, 0.75), 7.75);
}

#[test]
fn empty_pixels_do_not_consume_rotation_and_buckets_do_not_reseed_it() {
    let heights = flat();
    let field = field(&bitmap(&[(2, 0, 6), (0, 1, 5), (3, 1, 6)]), &heights);
    assert_eq!(field.instances[0].rotation, rotation_for(1));
    assert_eq!(field.instances[1].rotation, rotation_for(2));
    assert_eq!(field.instances[2].rotation, rotation_for(0));
    for index in 0..1000 {
        let degrees = rotation_for(index).to_degrees();
        assert!((degrees - degrees.round()).abs() < 0.0001);
        assert!((0.0..360.0).contains(&degrees));
    }
}

#[test]
fn missing_uv1_aliases_scaled_next_position_until_the_allocation_end() {
    let sub = mesh::SubMesh {
        positions: vec![[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]],
        normals: vec![[0., 1., 0.]; 3],
        tangents: vec![[1., 0., 0., 1.]; 3],
        uv0: vec![[0.; 2]; 3],
        ..Default::default()
    };
    let geometry = viewer_scene_base::mesh_shader::Geometry::from_mesh(&sub, 0.5).unwrap();
    let packed = geometry.packed(true).unwrap();
    assert_eq!(&packed[12..14], &[2., 2.5]);
    assert_eq!(&packed[26..28], &[3.5, 4.]);
    assert_eq!(&packed[40..42], &[0.; 2]);
}

#[test]
fn a_ragged_bitmap_is_refused_but_independent_pitches_are_allowed() {
    let heights = flat();
    let image = TreeImage {
        width: 11,
        height: 4,
        indices: &[1, 2, 3],
    };
    assert!(
        generate(
            &image,
            &HeightField {
                width: MAP_SIZE_X as u32,
                height: MAP_SIZE_Y as u32,
                values: &heights,
            },
        )
        .is_err()
    );
    let indices = vec![0u8; 8];
    let image = TreeImage {
        width: 8,
        height: 1,
        indices: &indices,
    };
    assert!(image.validate().is_ok());
}

#[test]
fn nonmultiple_map_dimensions_use_integer_partitions_and_independent_bitmap_pitches() {
    let mut values = vec![0u8; 770 * 520];
    for z in 230..290 {
        for x in 480..540 {
            values[(519 - z) * 770 + x] = (20 + x - 480 + z - 230) as u8;
        }
    }
    let mut indices = vec![0; 77 * 104];
    indices[51 * 77 + 51] = 3;
    let field = generate(
        &TreeImage {
            width: 77,
            height: 104,
            indices: &indices,
        },
        &HeightField {
            width: 770,
            height: 520,
            values: &values,
        },
    )
    .unwrap();
    assert_eq!(field.cell, [10.0, 5.0]);
    assert_eq!(field.buckets[0].tile, [1, 1]);
    assert_eq!(field.buckets[0].pitch, [256, 260]);
    assert_eq!(field.buckets[0].x_span(), (256.0, 512.0));
    assert_eq!(field.buckets[0].z_span(), (260.0, 520.0));
    assert_eq!(field.instances[0].position, [515.0, 8.5, 260.0]);
    for slope in field.instances[0].slope {
        assert!((slope - 0.1).abs() < 1e-6);
    }
}

#[test]
fn dimensions_without_a_native_partition_are_named_refusals() {
    let error = generate(
        &TreeImage {
            width: 1,
            height: 1,
            indices: &[1],
        },
        &HeightField {
            width: 255,
            height: 256,
            values: &vec![0; 255 * 256],
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("255 x 256"));
}

#[test]
fn the_lod_takes_the_last_threshold_at_or_below_the_distance() {
    let distances = [0.0, 100.0, 200.0, 500.0];
    assert_eq!(lod_for_distance(&distances, 0.0, 4), 0);
    assert_eq!(lod_for_distance(&distances, 99.0, 4), 0);
    assert_eq!(lod_for_distance(&distances, 100.0, 4), 1);
    assert_eq!(lod_for_distance(&distances, 310.0, 4), 2);
    assert_eq!(lod_for_distance(&distances, 500.0, 4), 3);
    assert_eq!(lod_for_distance(&distances, 500.0, 3), 2);
    assert_eq!(lod_for_distance(&[], 10.0, 1), 0);
}

#[test]
fn native_lod_squares_authored_thresholds_and_stops_at_the_first_failure() {
    assert_eq!(lod_for_squared_distance(&[0.0, -100.0], 1.0, 2), 0);
    assert_eq!(lod_for_squared_distance(&[0.0, 100.0, 10.0], 400.0, 3), 0);
    let threshold = 5.0f32;
    let below = f32::from_bits((threshold * threshold).to_bits() - 1);
    assert_eq!(below.sqrt(), threshold);
    assert_eq!(lod_for_squared_distance(&[0.0, threshold], below, 2), 0);
    assert_eq!(lod_for_squared_distance(&[0.0, threshold], 25.0, 2), 1);
}

#[test]
fn the_blend_mesh_crosses_its_last_threshold_inside_the_reference_frame() {
    let dunge = [0.0, 100.0, 200.0, 500.0];
    let blend = [0.0, 100.0, 200.0, 300.0];
    for distance in [260.0, 350.0, 450.0] {
        assert_eq!(lod_for_distance(&dunge, distance, 4), 2);
    }
    assert_eq!(lod_for_distance(&blend, 260.0, 4), 2);
    assert_eq!(lod_for_distance(&blend, 350.0, 4), 3);
}

fn tile(x: u32, z: u32) -> TreeBucket {
    TreeBucket {
        palette: 6,
        tile: [x, z],
        pitch: [256, 256],
        range: 0..1,
    }
}

#[test]
fn the_lod_distance_is_measured_to_the_nearest_point_of_the_tile() {
    let inside = tile(11, 6);
    let camera = [3010.01, 50.178, 1539.65];
    assert!((bucket_distance(&inside, 0.0, camera) - (50.178 - WATER_HEIGHT)).abs() < 1e-3);
    let west = tile(9, 6);
    let gap = 3010.01 - (10.0 * TILE);
    let expected = (gap * gap + (50.178 - WATER_HEIGHT).powi(2)).sqrt();
    assert!((bucket_distance(&west, 0.0, camera) - expected).abs() < 1e-3);
    assert!(
        (bucket_distance(&inside, MAP_SIZE_X, camera)
            - bucket_distance(&inside, 0.0, [camera[0] - MAP_SIZE_X, camera[1], camera[2]]))
        .abs()
            < 1e-3
    );
}

#[test]
fn the_close_cameras_reach_the_detailed_lods_and_the_reference_camera_does_not() {
    let dunge = [0.0, 100.0, 200.0, 500.0];
    let blend = [0.0, 100.0, 200.0, 300.0];
    let under = tile(11, 6);
    let close = [3010.01, 50.178, 1569.65 - 30.0];
    assert_eq!(bucket_lod(&under, 0.0, &dunge, 4, close), 0);
    assert_eq!(bucket_lod(&under, 0.0, &blend, 4, close), 0);
    let mid = [3009.91, 115.293, 1569.65 - 30.0];
    assert_eq!(bucket_lod(&under, 0.0, &dunge, 4, mid), 1);
    assert_eq!(bucket_lod(&under, 0.0, &blend, 4, mid), 1);
    let far = [3010.09, 302.960, 1568.21 - 30.0];
    assert_eq!(bucket_lod(&under, 0.0, &dunge, 4, far), 2);
    assert_eq!(bucket_lod(&under, 0.0, &blend, 4, far), 2);
    assert_eq!(bucket_lod(&tile(9, 6), 0.0, &blend, 4, far), 3);
}

#[test]
fn tree_visibility_uses_native_ceiling_and_only_side_planes() {
    let bucket = tile(4, 2);
    assert!(!visible(&bucket, 0.0, &[[-1.0, 0.0, 0.0, 2000.0]; 6]));
    assert!(visible(&bucket, 0.0, &[[-1.0, 0.0, 0.0, 0.0]; 6]));
    assert!(visible(&bucket, 0.0, &[[0.0, -1.0, 0.0, 25.5]; 6]));
    assert!(!visible(&bucket, 0.0, &[[0.0, -1.0, 0.0, 25.51]; 6]));
    assert!(visible(&bucket, -MAP_SIZE_X, &[[1.0, 0.0, 0.0, 4000.0]; 6]));
}

#[test]
fn tree_visibility_keeps_the_camera_side_margin() {
    let bucket = tile(4, 2);
    let camera = Camera {
        center: [bucket.x_span().0 - 40.0, bucket.z_span().1],
        height: 50.0,
        map: MapSize::TEST,
        defines: MapDefines::test(),
        pose: viewer_gpu::camera::Pose::Map,
    };
    let planes = side_planes(camera, [1024, 768]);
    assert!(visible(&bucket, 0.0, &planes));
    assert!(!visible(&bucket, 20.0, &planes));
}

#[test]
fn one_nearest_visible_wrap_owns_the_tile_and_equal_distance_keeps_first() {
    let bucket = tile(0, 0);
    let camera = Camera {
        center: [0.0, 0.0],
        height: 50.0,
        map: MapSize::TEST,
        defines: MapDefines::test(),
        pose: viewer_gpu::camera::Pose::Map,
    };
    let frame = TreeFrame {
        camera,
        offsets: &[-256.0, 0.0, MAP_SIZE_X],
        planes: [[0.0, 0.0, 0.0, -1.0]; 6],
    };
    assert_eq!(nearest_visible_copy(&bucket, frame).unwrap().0, 0);
    assert_eq!(
        nearest_visible_copy(
            &bucket,
            TreeFrame {
                offsets: &[MAP_SIZE_X, -256.0, 0.0],
                ..frame
            }
        )
        .unwrap()
        .0,
        1
    );
    assert_eq!(
        nearest_visible_copy(
            &bucket,
            TreeFrame {
                planes: [[-1.0, 0.0, 0.0, 1.0]; 6],
                ..frame
            }
        )
        .unwrap()
        .0,
        1
    );
    assert_eq!(
        nearest_visible_copy(
            &bucket,
            TreeFrame {
                planes: [[0.0, -1.0, 0.0, 30.0]; 6],
                ..frame
            }
        ),
        None
    );
}

#[test]
fn the_geometry_builder_bakes_the_mesh_scale_and_ranges_every_lod() {
    let sub = mesh::SubMesh {
        child_index: 0,
        positions: vec![[1.0, 2.0, 3.0], [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
        normals: vec![[0.0, 1.0, 0.0]; 3],
        tangents: vec![[1.0, 0.0, 0.0, 1.0]; 3],
        uv0: vec![[0.0, 0.0]; 3],
        uv1: None,
        uv2: None,
        uv3: None,
        indices: vec![0, 1, 2].into(),
        bounds: None,
        material: None,
        skin: None,
    };
    let assets = TreeAssets {
        meshes: vec![(
            6,
            TreeMeshAssets {
                name: "mapobject_6".into(),
                path: "gfx/models/mapitems/trees/forest_dunge.mesh".into(),
                lod_distances: vec![0.0, 100.0],
                lods: (0..2)
                    .map(|_| {
                        Some(TreeLod {
                            geometry: viewer_scene_base::mesh_shader::Geometry::from_mesh(&sub, 2.)
                                .unwrap(),
                            diffuse: empty_texture(),
                            normal: empty_texture(),
                        })
                    })
                    .collect(),
                scale: 2.0,
            },
        )],
        tint: Rgba8Image {
            width: 1,
            height: 1,
            pixels: vec![0; 4].into(),
        },
        season: Rgba8Image {
            width: 1,
            height: 1,
            pixels: vec![0; 4].into(),
        },
        mask: TreeMask {
            width: 1,
            height: 1,
            data: vec![0],
        },
    };
    let geometry = geometry(&assets).unwrap();
    assert_eq!(geometry.vertices.len(), 6);
    assert_eq!(geometry.vertices[0].position, [2.0, 4.0, 6.0]);
    assert_eq!(geometry.vertices[0].uv1, [0.0, 0.0]);
    assert_eq!(geometry.vertices[1].uv1, [2.0, 0.0]);
    assert_eq!(geometry.vertices[2].uv1, [0.0, 0.0]);
    assert_eq!(geometry.ranges[&(6, 0)], 0..3);
    assert_eq!(geometry.ranges[&(6, 1)], 3..6);
    assert_eq!(geometry.indices[3..6], [3, 4, 5]);
    assert_eq!(geometry.lods[&6], (vec![0.0, 100.0], 2));
}

#[test]
fn first_tree_record_filters_authored_shader_before_overrides_and_stops_at_first_admission() {
    let sub = |index, shader: Option<&str>| mesh::SubMesh {
        child_index: index,
        material: Some(mesh::Material {
            shader: shader.map(str::to_owned),
            ..Default::default()
        }),
        ..Default::default()
    };
    let shape = mesh::Shape {
        name: "canopy".into(),
        meshes: vec![
            sub(0, None),
            sub(1, Some("Collision")),
            sub(2, Some("")),
            sub(3, Some("leaf")),
            sub(4, Some("later")),
        ],
        ..Default::default()
    };
    let definition = mesh::MeshDefinition {
        settings: vec![mesh::MeshSetting {
            name: "canopy".into(),
            index: 3,
            diffuse: Some("selected.dds".into()),
            shader_file: Some("gfx/FX/custom.shader".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut attempted = Vec::new();
    let (selected, material) = first_tree_record(&[&shape], &definition, |material, sub| {
        attempted.push((sub.child_index, material.shader.clone()));
        Ok(sub.child_index != 2)
    })
    .unwrap()
    .unwrap();
    assert_eq!(attempted, [(2, Some("".into())), (3, Some("leaf".into()))]);
    assert_eq!(selected.child_index, 3);
    assert_eq!(material.diffuse, "selected.dds");
    assert_eq!(
        material.shader_file.as_deref(),
        Some("gfx/FX/custom.shader")
    );
    let (selected, _) = first_tree_record(&[&shape], &definition, |_, _| Ok(true))
        .unwrap()
        .unwrap();
    assert_eq!(
        selected.child_index, 2,
        "empty shader can use the native fallback"
    );
    assert!(
        first_tree_record(&[&shape], &definition, |_, _| Err(invalid(
            "compiler unavailable"
        )))
        .is_err()
    );
}

#[test]
fn tree_lookup_uses_direct_pdxmesh_even_when_an_entity_has_the_same_name() {
    let descriptors = mesh::parse_descriptors(
        br#"
        objectTypes = {
            pdxmesh = { name = mapobject_3 file = "gfx/models/canopy.mesh" scale = 0.7 }
            pdxmesh = { name = other file = "gfx/models/other.mesh" scale = 4 }
        }
        entity = { name = mapobject_3 pdxmesh = other scale = 9 }
        entity = { name = mapobject_4 pdxmesh = other }
    "#,
        "gfx/custom/arbitrary.gfx",
    )
    .unwrap();
    let definition = tree_definition(&descriptors, 3).unwrap();
    assert_eq!(definition.file, "gfx/models/canopy.mesh");
    assert_eq!(definition.scale, 0.7);
    assert!(
        tree_definition(&descriptors, 4)
            .unwrap_err()
            .to_string()
            .contains("mapobject_4")
    );
}

#[test]
fn the_tree_mask_box_filters_the_river_pixels() {
    let indices = vec![
        3, 254, 255, 11,
        254, 254, 5, 254,
        255, 254, 0, 1,
        254, 255, 2, 3,
    ];
    let mask = tree_mask(&IndexedImage {
        width: 4,
        height: 4,
        indices: indices.into(),
        palette: [[0; 4]; 256],
    })
    .unwrap();
    assert_eq!((mask.width, mask.height), (2, 2));
    assert_eq!(mask.data, [127, 191, 127, 255]);
    assert!(f32::from(mask.data[0]) / 255.0 > 0.17);
}

#[test]
fn a_river_bitmap_that_is_not_a_bitmap_is_refused() {
    assert!(
        tree_mask(&IndexedImage {
            width: 4,
            height: 4,
            indices: vec![254; 9].into(),
            palette: [[0; 4]; 256],
        })
        .is_err()
    );
}

fn empty_texture() -> dds::Texture {
    dds::Texture {
        format: dds::TextureFormat::Rgba8,
        width: 1,
        height: 1,
        faces: 1,
        mips: vec![vec![dds::MipLevel {
            width: 1,
            height: 1,
            data: (vec![0; 4]).into(),
        }]],
    }
}
