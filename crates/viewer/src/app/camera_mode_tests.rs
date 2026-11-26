
use super::{Config, Options, Viewer, drop};
use crate::controls::{Frame, flight::Flight, walker::World};
use dashboard_core::protocol::CameraMode;
use eframe::egui;
use std::sync::{Arc, Mutex};
use viewer_gpu::camera::{Camera, MapDefines, MapSize, Pose};

const MAP: MapSize = MapSize {
    width: 5632,
    height: 2048,
};
const AT: [f32; 2] = [1234.0, 567.0];

fn viewer() -> Viewer {
    Viewer::create(
        Config::default(),
        Options::default(),
        None,
        Arc::new(Mutex::new(None)),
        None,
        None,
    )
}

fn map_camera() -> Camera {
    Camera {
        center: [3000.0, 1000.0],
        height: 500.0,
        map: MAP,
        defines: MapDefines::test(),
        pose: Pose::Free(viewer_gpu::camera::Free::new(
            0.0,
            -0.2,
            viewer_gpu::camera::Lens::MAP,
        )),
    }
}

fn run(viewer: &mut Viewer, mode: CameraMode, ctx: &egui::Context) {
    viewer.set_camera_mode(mode, false, ctx);
    viewer.start_pick = drop::Pick::At(AT);
    match mode {
        CameraMode::Walk => {
            let ground = |_: f32, _: f32| 12.0;
            let world = World {
                ground: &ground,
                map: [MAP.x(), MAP.y()],
                ceiling: 3000.0,
            };
            viewer
                .walk
                .camera(&Frame::default(), 0.0, map_camera(), AT, &world);
        }
        CameraMode::Fly => {
            viewer.flight = Some(Flight::new([AT[0], 300.0, AT[1]], 0.0, 0.0, 0.0));
        }
        CameraMode::Map => {}
    }
}

#[test]
fn every_camera_mode_choice_is_taken_from_every_other() {
    let ctx = egui::Context::default();
    let modes = [CameraMode::Map, CameraMode::Fly, CameraMode::Walk];
    for from in modes {
        for to in modes {
            if from == to {
                continue;
            }
            let mut viewer = viewer();
            if from != CameraMode::Map {
                run(&mut viewer, from, &ctx);
                assert_eq!(viewer.free_mode_ground(), Some(AT), "{from:?} runs");
            }
            viewer.set_camera_mode(to, false, &ctx);
            assert_eq!(viewer.camera_mode, to, "{from:?} -> {to:?}");
            match (from, to) {
                (_, CameraMode::Map) => {
                    assert_eq!(viewer.start_pick, drop::Pick::Idle, "{from:?} -> {to:?}");
                }
                (CameraMode::Map, _) => {
                    assert!(viewer.start_pick.choosing(), "{from:?} -> {to:?}");
                }
                _ => assert_eq!(
                    viewer.start_pick.at(),
                    Some(AT),
                    "{from:?} -> {to:?} starts where {from:?} stood"
                ),
            }
            assert!(viewer.flight.is_none(), "{from:?} -> {to:?} keeps no flight");
            assert!(
                viewer.free_cam.is_none(),
                "{from:?} -> {to:?} keeps no free camera"
            );
            assert_eq!(
                viewer.walk.standing(),
                None,
                "{from:?} -> {to:?} keeps no walker"
            );
            if from == CameraMode::Walk && to == CameraMode::Fly {
                assert!(
                    viewer.mode_handover.is_some(),
                    "Walk -> Fly preserves eye pose for handover"
                );
            }
        }
    }
}
