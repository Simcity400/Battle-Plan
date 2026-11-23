use super::*;

fn through(affine: ClipAffine, ndc: [f32; 2], w: f32, out: [u32; 2]) -> [f32; 2] {
    let x = (affine.scale[0] * ndc[0] * w + affine.offset[0] * w) / w;
    let y = (affine.scale[1] * ndc[1] * w + affine.offset[1] * w) / w;
    [
        (x + 1.0) * 0.5 * out[0] as f32,
        (1.0 - y) * 0.5 * out[1] as f32,
    ]
}

fn ndc([x, y]: [f32; 2], size: [u32; 2]) -> [f32; 2] {
    [
        x / size[0] as f32 * 2.0 - 1.0,
        1.0 - y / size[1] as f32 * 2.0,
    ]
}

#[test]
fn the_clip_map_lands_where_the_device_map_does() {
    let placements = [
        Placement::identity([1920, 1080]),
        Placement {
            scale: 0.7,
            pivot: [600.0, 410.0],
            shift: [0.0; 2],
            size: [1920, 1080],
            out: [1920, 1080],
        },
        Placement {
            scale: 0.85,
            pivot: [1500.0, 90.0],
            shift: [-1380.0, 12.0],
            size: [3440, 1440],
            out: [2048, 2048],
        },
    ];
    for placement in placements {
        let affine = placement.clip();
        for point in [[0.0, 0.0], [600.0, 410.0], [1919.0, 3.5], [77.25, 1000.0]] {
            for w in [1.0, 2.5] {
                let landed = through(affine, ndc(point, placement.size), w, placement.out);
                let expected = placement.point(point);
                assert!(
                    (landed[0] - expected[0]).abs() < 1e-2
                        && (landed[1] - expected[1]).abs() < 1e-2,
                    "{placement:?} {point:?}: {landed:?} against {expected:?}"
                );
            }
        }
    }
}

#[test]
fn a_matrix_takes_the_map_after_its_projection() {
    let mut constants = [
        0.01, 0.0, 0.0, 0.0,
        0.0, -0.02, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        -1.0, 1.0, 0.0, 1.0,
        9.0, 9.0,
    ];
    let affine = ClipAffine {
        scale: [0.5, 2.0],
        offset: [0.25, -0.5],
    };
    apply(affine, &mut constants);
    let project = |x: f32, y: f32| {
        [
            constants[0] * x + constants[4] * y + constants[12],
            constants[1] * x + constants[5] * y + constants[13],
        ]
    };
    let [x, y] = project(30.0, 10.0);
    assert!((x - -0.1).abs() < 1e-6 && (y - 1.1).abs() < 1e-6, "{x} {y}");
    assert_eq!(&constants[16..], &[9.0, 9.0]);
}

#[test]
fn scissors_are_placed_and_cut() {
    let placement = Placement {
        scale: 0.5,
        pivot: [100.0, 100.0],
        shift: [10.0, 0.0],
        size: [400, 400],
        out: [400, 400],
    };
    assert_eq!(
        placement.scissor([60, 60, 80, 80], [0, 0, 400, 400]),
        Some([90, 80, 40, 40])
    );
    assert_eq!(
        placement.scissor([60, 60, 80, 80], [100, 90, 120, 100]),
        Some([100, 90, 20, 10])
    );
    assert_eq!(
        placement.scissor([60, 60, 80, 80], [200, 200, 300, 300]),
        None
    );
    assert_eq!(
        cut([1.5, 2.25, 3.5, 4.0], [0, 0, 10, 10]),
        Some([1, 2, 3, 2])
    );
}

#[test]
fn shelves_place_cells_apart_and_open_pages_when_full() {
    let mut shelves = Shelves::new([100, 60]);
    let mut placed = Vec::new();
    for size in [[40, 20], [40, 30], [30, 10], [90, 25], [10, 10]] {
        placed.push((shelves.place(size), size));
    }
    assert_eq!(placed[0].0, (0, [0, 0]));
    assert_eq!(placed[1].0, (0, [42, 0]));
    assert_eq!(placed[2].0, (0, [0, 32]));
    assert_eq!(placed[3].0, (1, [0, 0]));
    assert_eq!(placed[4].0, (1, [0, 27]));
    assert_eq!(shelves.pages, 2);
    assert_eq!(shelves.extents, [[82, 42], [90, 37]]);
    for (i, ((page_a, a), size_a)) in placed.iter().enumerate() {
        for ((page_b, b), size_b) in &placed[i + 1..] {
            if page_a != page_b {
                continue;
            }
            let apart = a[0] + size_a[0] + GAP <= b[0]
                || b[0] + size_b[0] + GAP <= a[0]
                || a[1] + size_a[1] + GAP <= b[1]
                || b[1] + size_b[1] + GAP <= a[1];
            assert!(apart, "{a:?} {size_a:?} and {b:?} {size_b:?}");
        }
    }
    let mut whole = Shelves::new([100, 60]);
    assert_eq!(whole.place([100, 60]), (0, [0, 0]));
    assert_eq!(whole.place([1, 1]), (1, [0, 0]));
    assert_eq!(whole.pages, 2);
    assert_eq!(whole.extents, [[100, 60], [1, 1]]);
}

#[test]
fn a_composite_maps_its_cell_onto_its_frame_rectangle() {
    let corners = composite_quad(
        [0.0, 0.0, 20.0, 10.0],
        [100, 100],
        [50.0, 25.0, 70.0, 35.0],
        [100, 50],
        0.25,
    );
    let near = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6;
    assert!(near(corners[0].position, [0.0, 0.0]));
    assert!(near(corners[0].uv, [0.0, 0.0]));
    assert!(near(corners[2].position, [0.4, -0.4]), "{:?}", corners[2]);
    assert!(near(corners[2].uv, [0.2, 0.1]), "{:?}", corners[2]);
    assert!(corners.iter().all(|corner| corner.alpha == 0.25));
}
