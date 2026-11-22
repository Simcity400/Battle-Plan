use super::*;
use viewer_gpu::{
    camera::{Free, Lens, MapDefines, MapSize, Placement, Pose},
    province_map::WATER_HEIGHT,
};
use crate::sprites::{self, layer};

const VIEWPORT: [u32; 2] = [1600, 900];
const LENS: Lens = Lens {
    fov_y: 0.9,
    near: 0.5,
    far: 3000.0,
};

fn free(eye: [f32; 3], target: [f32; 3]) -> Camera {
    Placement {
        center: [eye[0], eye[2]],
        height: eye[1],
        pose: Pose::Free(Free::looking_at(eye, target, LENS)),
    }
    .on(MapSize::TEST, MapDefines::test())
}

fn anchored(world: [f32; 3], cutoff: f32, layer: u64, rect: [f32; 4]) -> Quad {
    Quad {
        texture: 0,
        rect,
        uv: [0.0, 0.0, 1.0, 1.0],
        color: [1.0; 4],
        rotation: None,
        layer,
        shade: 0.0,
        exact: None,
        anchor: Some(IconAnchor {
            world,
            cutoff,
            family: IconFamily::Columns,
            moves: None,
        }),
        present: None,
    }
}

fn batch(quads: Vec<Quad>) -> Batch {
    let mut batch = Batch::default();
    batch.quads.extend(quads);
    batch
}

#[test]
fn distance_keeps_size_near_then_shrinks_gently_and_fades_by_the_cutoff() {
    let range = 1100.0;
    let (scale, alpha) = distance_look(20.0, 900.0, range);
    assert_eq!((scale, alpha), (1.0, 1.0));
    let (scale, alpha) = distance_look(SHRINK_TO * range, 900.0, range);
    assert!((scale - SMALLEST).abs() < 1e-6, "{scale}");
    assert_eq!(alpha, 1.0);
    let middle = 900.0 * (1.0 - FADE_SPAN * 0.5);
    let (_, alpha) = distance_look(middle, 900.0, range);
    assert!((alpha - 0.5).abs() < 1e-4, "{alpha}");
    assert_eq!(distance_look(900.0, 900.0, range).1, 0.0);
    assert_eq!(distance_look(range, 1500.0, range).1, 0.0);
    let mut last = 1.0;
    for step in 0..=110 {
        let (scale, _) = distance_look(step as f32 * 10.0, 900.0, range);
        assert!(scale <= last + 1e-6);
        last = scale;
    }
}

#[test]
fn the_game_s_camera_keeps_every_quad_as_laid_out() {
    let camera = Placement::map([2000.0, 1000.0], 300.0).on(MapSize::TEST, MapDefines::test());
    let laid_out = batch(vec![
        anchored(
            [2000.0, 10.0, 1000.0],
            900.0,
            layer(10, 0, 0),
            [1.0, 2.0, 3.0, 4.0],
        ),
        anchored(
            [9000.0, 10.0, 1000.0],
            900.0,
            layer(10, 1, 0),
            [5.0, 6.0, 7.0, 8.0],
        ),
    ]);
    let mut out = Batch::default();
    let looks = present(&camera, VIEWPORT, 1.0, None, &laid_out, Head::default(), &mut out);
    assert!(looks.is_empty());
    assert_eq!(out.quads, laid_out.quads);
    assert_eq!(camera.icon_distance([0.0, 0.0, 0.0]), 300.0);
}

#[test]
fn a_free_eye_ranks_icons_farthest_first_and_drops_what_it_cannot_show() {
    let eye = [1000.0, 40.0, 1000.0];
    let camera = free(eye, [1000.0, 10.0, 1400.0]);
    let near = [1000.0, 10.0, 1100.0];
    let far = [1000.0, 10.0, 1500.0];
    let behind = [1000.0, 10.0, 800.0];
    let beyond = [1000.0, 10.0, 2000.0];
    let laid_out = batch(vec![
        Quad {
            anchor: None,
            ..anchored(near, 900.0, sprites::TOOLTIP_LAYER, [0.0; 4])
        },
        anchored(near, 900.0, layer(20, 0, 0), [10.0, 10.0, 5.0, 5.0]),
        anchored(far, 900.0, layer(3, 0, 0), [20.0, 20.0, 5.0, 5.0]),
        anchored(far, 900.0, layer(3, 0, 1), [20.0, 25.0, 5.0, 5.0]),
        anchored(behind, 900.0, layer(10, 0, 0), [30.0, 30.0, 5.0, 5.0]),
        anchored(beyond, 900.0, layer(10, 1, 0), [40.0, 40.0, 5.0, 5.0]),
    ]);
    let mut out = Batch::default();
    let looks = present(&camera, VIEWPORT, 1.0, None, &laid_out, Head::default(), &mut out);
    let ranks: Vec<_> = out
        .quads
        .iter()
        .map(|quad| quad.present.map(|present| present.rank))
        .collect();
    assert_eq!(ranks, [Some(1), Some(0), Some(0), None]);
    assert_eq!(out.quads[0].layer, layer(20, 0, 0));
    assert_eq!(out.quads[3].layer, sprites::TOOLTIP_LAYER);
    let near_look = looks.of(IconFamily::Columns, layer(20, 0, 0)).unwrap();
    let far_look = looks.of(IconFamily::Columns, layer(3, 0, 1)).unwrap();
    assert_eq!(far_look.rank, 0);
    assert!(near_look.scale > far_look.scale);
    assert!(looks.of(IconFamily::Columns, layer(10, 0, 0)).is_none());
    assert!(looks.of(IconFamily::Columns, layer(10, 1, 0)).is_none());
    assert!((near_look.pivot[0] - VIEWPORT[0] as f32 / 2.0).abs() < 1.0);
}

#[test]
fn hits_reach_back_into_the_icon_s_own_layout() {
    let present = Present {
        pivot: [100.0, 200.0],
        scale: 0.5,
        alpha: 0.8,
        rank: 3,
    };
    assert_eq!(present.unscale([110.0, 190.0]), [120.0, 180.0]);
    let mut looks = Looks::default();
    looks.show(IconFamily::Columns, layer(10, 4, 0), present);
    looks.show(
        IconFamily::Columns,
        layer(10, 5, 0),
        Present {
            alpha: 0.2,
            ..present
        },
    );
    assert_eq!(
        looks.hit(IconFamily::Columns, layer(10, 4, 7), [110.0, 190.0]),
        Some(([120.0, 180.0], 3))
    );
    assert_eq!(
        looks.hit(IconFamily::Columns, layer(10, 5, 0), [110.0, 190.0]),
        None
    );
    assert_eq!(
        looks.hit(IconFamily::Columns, layer(10, 6, 0), [110.0, 190.0]),
        None
    );
    assert_eq!(
        looks.hit(IconFamily::Combats, layer(10, 4, 0), [110.0, 190.0]),
        None
    );
    looks.show(
        IconFamily::Combats,
        layer(10, 4, 0),
        Present {
            scale: 1.0,
            rank: 9,
            ..present
        },
    );
    assert_eq!(
        looks.hit(IconFamily::Combats, layer(10, 4, 0), [110.0, 190.0]),
        Some(([110.0, 190.0], 9))
    );
    assert_eq!(
        looks.hit(IconFamily::Columns, layer(10, 4, 7), [110.0, 190.0]),
        Some(([120.0, 180.0], 3))
    );
}

#[test]
fn terrain_hiding_the_anchor_fades_the_whole_icon() {
    let (width, rows) = (MapSize::TEST.width, MapSize::TEST.height);
    let mut values = vec![95u8; (width * rows) as usize];
    for bottom in 0..rows {
        let row = (rows - 1 - bottom) as usize;
        for column in 1240..1250 {
            values[row * width as usize + column] = 220;
        }
    }
    let field = HeightField {
        width,
        height: rows,
        values: &values,
    };
    let sight = TerrainSight::new(&field);
    let ground = Ground {
        field,
        sight: &sight,
    };
    let eye = [1200.0, 12.0, 1000.0];
    let camera = free(eye, [1400.0, WATER_HEIGHT, 1000.0]);
    let before = [1230.0, WATER_HEIGHT, 1000.0];
    let hidden = [1300.0, WATER_HEIGHT, 1000.0];
    assert_eq!(sight_alpha(&ground, eye, before, 30.0), 1.0);
    assert_eq!(sight_alpha(&ground, eye, hidden, 100.0), 0.0);
    let laid_out = batch(vec![
        anchored(before, 900.0, layer(10, 0, 0), [0.0, 0.0, 5.0, 5.0]),
        anchored(hidden, 900.0, layer(10, 1, 0), [0.0, 0.0, 5.0, 5.0]),
    ]);
    let mut out = Batch::default();
    let looks = present(&camera, VIEWPORT, 1.0, Some(&ground), &laid_out, Head::default(), &mut out);
    assert_eq!(out.quads.len(), 1);
    assert_eq!(
        looks
            .of(IconFamily::Columns, layer(10, 0, 0))
            .map(|look| look.alpha),
        Some(1.0)
    );
    assert!(looks.of(IconFamily::Columns, layer(10, 1, 0)).is_none());
    let high = [1200.0, 300.0, 1000.0];
    assert_eq!(sight_alpha(&ground, high, hidden, 316.0), 1.0);
}
