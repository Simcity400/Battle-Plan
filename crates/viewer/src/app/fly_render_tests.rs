use super::render_failure_tests::{target, until};
use super::*;
use crate::map::scene_failure_tests::Fixture;
use viewer_runtime::errors::invalid;
use viewer_gpu::camera::Placement;
use viewer_runtime::test_support::TEST_HANG_GUARD;

#[test]
#[ignore = "verified replay/content and GPU; requires BATTLEPLAN_SCENE_FAILURE_INPUT"]
fn the_chosen_aircraft_is_drawn_while_the_frame_holds_the_document() -> std::io::Result<()> {
    let fixture = Fixture::load("fly-aircraft")?;
    let request = fixture.request([256, 144]);
    let document = Document::from_shared_content(
        fixture.document.content.clone(),
        fixture.document.playback.replay_handle(),
        &InitialHour {
            branch: Some(fixture.input.branch),
            hour: Some(fixture.input.hour),
            hour_fraction: 0.,
        },
    )?;
    let assets = Arc::clone(&document.aircraft);
    let name = assets
        .roster()
        .descriptors
        .entities
        .keys()
        .find(|name| assets.player(name).is_ok())
        .cloned()
        .ok_or_else(|| invalid("the content registers no entity that can be flown"))?;
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
    viewer.camera_manual = true;
    viewer.camera = Some(Placement {
        center: request.camera.center,
        height: request.camera.height,
        pose: viewer_gpu::camera::Pose::Map,
    });
    viewer.camera_mode = CameraMode::Fly;
    viewer.install_map(document, fixture.scene()?, "fixture".into());
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    frame.wgpu_render_state = Some(fixture.state.clone());
    let mut target = target(&fixture);
    let failed = |viewer: &Viewer| viewer.error.is_some() || viewer.map_error.is_some();
    until(&mut viewer, &ctx, &frame, &target, |v| {
        v.rendered.is_some() || failed(v)
    });
    assert!(!failed(&viewer), "{:?}", viewer.map_state(&target));
    assert_eq!(viewer.scene.as_ref().unwrap().player_drawn(), 0);

    viewer.fly_aircraft.choose(Some(name.clone()));
    let deadline = std::time::Instant::now() + TEST_HANG_GUARD;
    loop {
        let status = viewer.fly_aircraft.poll(Some(&assets), || {});
        if viewer.fly_aircraft.ready(&assets).is_some() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{name} never baked: {status:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    target.sequence += 1;
    until(&mut viewer, &ctx, &frame, &target, |v| {
        v.scene
            .as_ref()
            .is_some_and(|scene| scene.player_drawn() >= 1)
            || failed(v)
    });
    assert!(!failed(&viewer), "{:?}", viewer.map_state(&target));
    assert!(viewer.document.is_some(), "the frame returns the document");
    assert!(viewer.scene.as_ref().unwrap().player_drawn() >= 1);
    Ok(())
}
