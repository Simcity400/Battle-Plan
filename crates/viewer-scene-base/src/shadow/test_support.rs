use super::*;
use viewer_gpu::camera::{MapDefines, MapSize};
use viewer_gpu::frame::DayNight;

pub(super) const VIEWPORT: [u32; 2] = [2048, 1152];

pub(super) fn reference() -> (Camera, FrameConstants) {
    let camera = Camera {
        center: [3009.83, 1568.00],
        height: 163.934,
        map: MapSize::TEST,
        defines: MapDefines::test(),
        pose: viewer_gpu::camera::Pose::Map,
    };
    let frame =
        FrameConstants::try_new(camera, VIEWPORT, DayNight::at_hour(60828490), 0.0, false)
            .expect("valid reference camera");
    (camera, frame)
}

pub fn assert_depth_map_invalidation(mut create_model_caster: impl FnMut() -> CasterId) {
    let (camera, frame) = reference();
    let light = light_box(camera, VIEWPORT, light_direction(camera, &frame))
        .unwrap()
        .view_projection();
    let (buildings, trees) = (CasterId::next(), CasterId::next());
    let at = |light, camera, viewport, offsets: &[f32], casters, arrows| {
        CasterKey::new(light, camera, viewport, offsets, casters, arrows)
    };
    let key = || at(light, camera, VIEWPORT, &[0.0], vec![buildings, trees], false);
    let mut held = Held::default();
    assert_eq!(held.ready(Some((key(), true))), Ready::Draw, "nothing known yet");
    assert_eq!(held.ready(Some((key(), true))), Ready::Keep);
    assert_eq!(held.ready(Some((key(), false))), Ready::Draw, "a caster drew otherwise");
    assert_eq!(held.ready(Some((key(), true))), Ready::Keep);
    let mut moved_light = light;
    moved_light[3][0] += 1.0;
    let mut moved_camera = camera;
    moved_camera.center[0] += 0.5;
    let mut zoomed = camera;
    zoomed.height += 0.5;
    let both = || vec![buildings, trees];
    let changes = [
        ("the light", at(moved_light, camera, VIEWPORT, &[0.0], both(), false)),
        ("the camera", at(light, moved_camera, VIEWPORT, &[0.0], both(), false)),
        ("the zoom", at(light, zoomed, VIEWPORT, &[0.0], both(), false)),
        ("the viewport", at(light, camera, [VIEWPORT[0], 1080], &[0.0], both(), false)),
        ("the wrap copies", at(light, camera, VIEWPORT, &[0.0, 5632.0], both(), false)),
        ("a caster left", at(light, camera, VIEWPORT, &[0.0], vec![trees], false)),
        ("another order", at(light, camera, VIEWPORT, &[0.0], vec![trees, buildings], false)),
        ("arrows", at(light, camera, VIEWPORT, &[0.0], both(), true)),
    ];
    for (what, changed) in changes {
        assert_eq!(held.ready(Some((changed, true))), Ready::Draw, "{what}");
        assert_eq!(held.ready(Some((key(), true))), Ready::Draw, "{what}, and back");
        assert_eq!(held.ready(Some((key(), true))), Ready::Keep, "{what}, then kept");
    }
    let drew = create_model_caster();
    let player = |caster| at(light, camera, VIEWPORT, &[0.0], vec![caster], false);
    assert_eq!(held.ready(Some((player(drew), true))), Ready::Draw);
    assert_eq!(held.ready(Some((player(drew), true))), Ready::Keep);
    let fresh = create_model_caster();
    assert_eq!(held.ready(Some((player(fresh), true))), Ready::Draw, "a fresh pass");
    assert_eq!(held.ready(None), Ready::Clear, "no casters clear it");
    assert_eq!(held.ready(None), Ready::Nothing, "and it stays clear");
    assert_eq!(held.ready(Some((key(), true))), Ready::Draw, "the casters come back");
    held = Held::Unknown;
    assert_eq!(held.ready(None), Ready::Clear, "a lost frame's clear is done again");
    held = Held::Unknown;
    assert_eq!(held.ready(Some((key(), true))), Ready::Draw, "a lost frame's casters too");
}
