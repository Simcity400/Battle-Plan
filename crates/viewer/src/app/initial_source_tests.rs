use super::render_failure_tests::{target, until};
use super::*;
use crate::map::scene_failure_tests::Fixture;
use viewer_playback::map_source::Current;
use eframe::egui;
use std::sync::{Arc, Mutex};
use viewer_gpu::camera::Placement;

#[test]
#[ignore = "verified replay/content and GPU; requires BATTLEPLAN_SCENE_FAILURE_INPUT"]
fn the_window_lets_go_of_its_load_time_frame_after_its_first_complete_draw() -> std::io::Result<()>
{
    let fixture = Fixture::load("initial-source")?;
    let document = Document::from_shared_content(
        fixture.document.content.clone(),
        fixture.document.playback.replay_handle(),
        &crate::document::InitialHour {
            branch: Some(fixture.input.branch),
            hour: Some(fixture.input.hour),
            hour_fraction: 0.,
        },
    )?;
    assert!(document.playback.holds_source());
    let Current::Frame(_) = document.playback.current() else {
        panic!("the fixture's hour is recorded");
    };
    let request = fixture.request([256, 144]);
    let mut viewer = Viewer::create(
        Config::default(),
        Options::default(),
        None,
        Arc::new(Mutex::new(None)),
        None,
        Some((fixture.state.device.clone(), fixture.state.queue.clone())),
    );
    viewer.dashboard_map = true;
    viewer.diagnostics = false;
    viewer.graphics.samples = 1;
    viewer.camera = Some(Placement::map(request.camera.center, request.camera.height));
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    frame.wgpu_render_state = Some(fixture.state.clone());
    let target = target(&fixture);
    viewer.install_map(document, fixture.scene()?, "fixture".into());
    assert!(viewer.playhead_map.is_none());
    until(&mut viewer, &ctx, &frame, &target, |v| {
        v.playhead_map.is_some() || v.error.is_some() || v.map_error.is_some()
    });
    assert!(
        viewer.error.is_none() && viewer.map_error.is_none(),
        "{:?}",
        viewer.map_state(&target)
    );
    let document = viewer.document.as_ref().expect("installed");
    assert!(
        !document.playback.holds_source(),
        "the load-time source block outlived the opening of the map's reader"
    );
    until(&mut viewer, &ctx, &frame, &target, |v| {
        (v.rendered.is_some() && v.shown_whole) || v.error.is_some() || v.map_error.is_some()
    });
    assert!(
        viewer.error.is_none() && viewer.map_error.is_none(),
        "{:?}",
        viewer.map_state(&target)
    );
    let document = viewer.document.as_ref().expect("installed");
    assert!(matches!(document.playback.current(), Current::Published));
    Ok(())
}
