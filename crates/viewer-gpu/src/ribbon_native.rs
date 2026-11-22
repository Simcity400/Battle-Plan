use crate::ribbon_content::{ArrowType, OrderDraw, OrderVertex};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};
use crate::{
    camera::Camera,
    province_admission::{Planes, admits_box, side_planes},
    ribbon_content::{Mode, Vertex},
};

#[derive(Debug, PartialEq)]
pub struct Draw {
    pub index: usize,
    pub offsets: Vec<f32>,
}

#[derive(Default)]
pub struct View {
    pub buckets: Vec<Vec<Draw>>,
    pub labels: Vec<usize>,
}

fn offsets(bounds: [[f32; 3]; 2], width: f32, planes: &Planes) -> Vec<f32> {
    let seam = if bounds[0][0] < 0. {
        Some(width)
    } else if bounds[1][0] > width {
        Some(-width)
    } else {
        None
    };
    std::iter::once(0.)
        .chain(seam)
        .filter(|offset| admits_box(bounds, *offset, planes))
        .collect()
}

pub fn plans<'a>(
    draws: &[OrderDraw],
    camera: Camera,
    viewport: [u32; 2],
    wraps: &[f32],
    select: impl Fn(&OrderDraw, Mode) -> Option<&'a ArrowType>,
) -> BTreeMap<Mode, Vec<View>> {
    let mut seen = BTreeSet::new();
    [Mode::Shadow, Mode::Main, Mode::Seethrough, Mode::Late]
        .into_iter()
        .map(|mode| {
            if matches!(mode, Mode::Shadow | Mode::Main) {
                seen.clear();
            }
            let views = wraps
                .iter()
                .enumerate()
                .map(|(view, wrap)| {
                    let mut camera = camera;
                    camera.center[0] -= wrap;
                    let planes = side_planes(camera, viewport);
                    let mut result = View::default();
                    let mut base = 0;
                    for bucket in draws.chunk_by(|a, b| a.declaration_index == b.declaration_index)
                    {
                        let mut admitted = Vec::new();
                        for (local, draw) in bucket.iter().enumerate() {
                            let index = base + local;
                            if view != 0 && seen.contains(&index) {
                                continue;
                            }
                            let Some(arrow) = select(draw, mode) else {
                                continue;
                            };
                            let offsets = offsets(draw.bounds, camera.map.x(), &planes);
                            if offsets.is_empty() {
                                continue;
                            }
                            let inner = arrow.mask_inner != [0.; 3];
                            let outer = arrow.mask_outer != [0.; 3];
                            if inner || outer {
                                seen.insert(index);
                            }
                            if mode != Mode::Shadow
                                && inner
                                && result.labels.len() < 0x200
                                && let Some(label) = draw.label
                            {
                                result.labels.push(label);
                            }
                            admitted.push(Draw { index, offsets });
                        }
                        base += bucket.len();
                        if !admitted.is_empty() {
                            result.buckets.push(admitted);
                        }
                    }
                    result
                })
                .collect();
            (mode, views)
        })
        .collect()
}

pub fn bounds(vertices: &[OrderVertex]) -> [[f32; 3]; 2] {
    let mut bounds = [vertices[0].position; 2];
    for vertex in &vertices[1..] {
        extend_bounds(&mut bounds, [vertex.position; 2]);
    }
    bounds
}

pub fn extend_bounds(bounds: &mut [[f32; 3]; 2], other: [[f32; 3]; 2]) {
    for axis in 0..3 {
        bounds[0][axis] = bounds[0][axis].min(other[0][axis]);
        bounds[1][axis] = bounds[1][axis].max(other[1][axis]);
    }
}

pub fn vertices(
    source: &[OrderVertex],
    controls: usize,
    head: bool,
) -> (Vec<Vertex>, Vec<Range<u32>>) {
    let head_count = if head { 4 } else { 0 };
    let body = source.len() - head_count;
    let actual = controls * 2 + head_count;
    let capacity = (actual / 512 + 1) * 512;
    let mut vertices = Vec::with_capacity(capacity);
    let convert = |v: &OrderVertex| Vertex {
        position: v.position,
        padding: 0,
        uv: v.uv_head_variant,
    };
    vertices.extend(source[body..].iter().rev().map(convert));
    for _ in body / 2..controls {
        vertices.extend(source[body - 2..body].iter().rev().map(convert));
    }
    vertices.extend(source[..body].iter().rev().map(convert));
    vertices.resize(capacity, *vertices.last().expect("two controls"));
    let strips = if head {
        vec![0..4, 3..actual as u32]
    } else {
        std::iter::once(0..capacity as u32).collect()
    };
    (vertices, strips)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        camera::{MapDefines, MapSize, Pose},
        ribbon_content::OrderField,
        ribbon_mesh::emit,
    };

    #[test]
    fn native_stream_has_reverse_pairs_head_split_and_allocation_slack() {
        let source: Vec<_> = (0..8)
            .map(|i| OrderVertex {
                position: [i as f32, 2., 3.],
                uv_head_variant: [4., 5., 6., 7.],
            })
            .collect();
        let (output, strips) = vertices(&source, 4, true);
        assert_eq!(strips, [0..4, 3..12]);
        assert_eq!(output.len(), 512);
        assert_eq!(
            output[..12]
                .iter()
                .map(|v| v.position[0])
                .collect::<Vec<_>>(),
            [7., 6., 5., 4., 3., 2., 3., 2., 3., 2., 1., 0.]
        );
        assert!(output[12..].iter().all(|v| *v == output[11]));
        let raw: &[f32] = bytemuck::cast_slice(&output[..1]);
        assert_eq!(raw, [7., 2., 3., 0., 4., 5., 6., 7.]);
        let (output, strips) = vertices(&source[..4], 2, false);
        assert_eq!(strips, std::iter::once(0..512).collect::<Vec<_>>());
        assert_eq!(output[0].position[0], 3.);
        assert!(output[4..].iter().all(|v| v.position[0] == 0.));
    }

    #[test]
    fn seam_boxes_keep_both_copies_and_use_negative_edge_first() {
        let all = [[0.; 4]; 6];
        assert_eq!(
            offsets([[-2., 0., 0.], [2., 1., 1.]], 100., &all),
            [0., 100.]
        );
        assert_eq!(
            offsets([[98., 0., 0.], [102., 1., 1.]], 100., &all),
            [0., -100.]
        );
        assert_eq!(
            offsets([[-2., 0., 0.], [102., 1., 1.]], 100., &all),
            [0., 100.]
        );
        let near_right = [
            [-1., 0., 0., 90.],
            [1., 0., 0., -110.],
            [0.; 4],
            [0.; 4],
            [0.; 4],
            [0.; 4],
        ];
        assert_eq!(
            offsets([[-2., 0., 0.], [2., 1., 1.]], 100., &near_right),
            [100.]
        );
        assert!(admits_box([[90., 0., 0.]; 2], 0., &near_right));
        assert!(!admits_box([[89., 0., 0.]; 2], 0., &near_right));
    }

    #[test]
    fn arrows_keep_the_camera_side_margin() {
        let arrow = ArrowType::default();
        let mut field = OrderField::default();
        emit(
            &mut field,
            &arrow,
            &[[0, 0], [100_000, 0]],
            1.,
            [1.; 4],
            &|_, _, _| 0.,
        )
        .unwrap();
        let camera = Camera {
            center: [3000., 1000.],
            height: 50.,
            map: MapSize::TEST,
            defines: MapDefines::test(),
            pose: Pose::Map,
        };
        for (x, expected) in [(3040., 1), (3060., 0)] {
            field.draws[0].bounds = [[x, 0., 999.], [x + 2., 1., 1001.]];
            let plan = plans(&field.draws, camera, [1024, 768], &[0.], |_, _| {
                Some(&arrow)
            });
            for mode in [Mode::Shadow, Mode::Main, Mode::Seethrough, Mode::Late] {
                assert_eq!(plan[&mode][0].buckets.len(), expected);
            }
        }
    }

    #[test]
    fn labels_follow_arrow_admission_masks_and_retained_seen_list() {
        let arrow = ArrowType {
            head_length: 3.,
            ..Default::default()
        };
        let mut field = OrderField::default();
        emit(
            &mut field,
            &arrow,
            &[[9_000_000, 100_000_000], [11_000_000, 100_000_000]],
            1.,
            [1.; 4],
            &|_, _, _| 10.,
        )
        .unwrap();
        field.draws[0].label = Some(17);
        let camera = Camera {
            center: [100., 1000.],
            height: 300.,
            map: MapSize::TEST,
            defines: MapDefines::test(),
            pose: Pose::Map,
        };
        let plan = plans(&field.draws, camera, [1920, 1080], &[0., 0.], |_, _| {
            Some(&arrow)
        });
        for mode in [Mode::Main, Mode::Seethrough, Mode::Late] {
            assert_eq!(plan[&mode][0].labels, [17]);
            assert!(plan[&mode][1].buckets.is_empty());
        }
        let hidden = ArrowType {
            mask_inner: [0.; 3],
            mask_outer: [0.; 3],
            ..arrow
        };
        let plan = plans(&field.draws, camera, [1920, 1080], &[0., 0.], |_, _| {
            Some(&hidden)
        });
        for view in &plan[&Mode::Main] {
            assert_eq!(view.buckets.len(), 1);
            assert!(view.labels.is_empty());
        }
    }
}
