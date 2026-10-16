//! Actual viewer draw/capture failure presentation, using a synthetic Scene fixture.
use super::*;
use crate::map::scene_failure_tests::Fixture;
use viewer_foundation::map::camera::Placement;
#[cfg(test)]
use viewer_foundation::map::test_support::TEST_HANG_GUARD;
use viewer_world::map::scene_faults::Point;

fn target(fixture: &Fixture) -> dashboard::protocol::DashboardMapView {
    let bounds = dashboard::protocol::ViewBounds {
        x: 0,
        y: 0,
        width: 256,
        height: 144,
    };
    dashboard::protocol::DashboardMapView {
        options: Default::default(),
        id: "failure-fixture".into(),
        source_id: "verified-fixture".into(),
        path: "fixture".into(),
        revision: "fixture".into(),
        branch_id: fixture.input.branch.to_string(),
        playing: false,
        hour: fixture.input.hour,
        fraction: 0.,
        sequence: 1,
        seek_sequence: Some(1),
        bounds,
        clip: bounds,
        visible: true,
    }
}

fn draw(
    viewer: &mut Viewer,
    ctx: &egui::Context,
    frame: &eframe::Frame,
    target: &dashboard::protocol::DashboardMapView,
) {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(256., 144.),
            )),
            ..Default::default()
        },
        |ui| viewer.map_ui(ui, frame, target),
    );
    output.textures_delta.clear();
}

fn until(
    viewer: &mut Viewer,
    ctx: &egui::Context,
    frame: &eframe::Frame,
    target: &dashboard::protocol::DashboardMapView,
    done: impl Fn(&Viewer) -> bool,
) {
    let deadline = Instant::now() + TEST_HANG_GUARD;
    loop {
        draw(viewer, ctx, frame, target);
        if done(viewer) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "viewer did not finish: {:?}",
            viewer.map_state(target)
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn arm(viewer: &mut Viewer, path: PathBuf, camera: Camera) {
    viewer.capture = Some(Capture {
        path,
        remaining: 0,
        pinned: Some((
            [257, 145],
            Some(Placement {
                center: camera.center,
                height: camera.height,
            }),
        )),
        exit: false,
        time: Some(0.),
        step: 0.,
        frames: 1,
        index: 0,
    });
}

#[test]
#[ignore = "verified replay/content and GPU; requires BATTLE_PLAN_SCENE_FAILURE_INPUT"]
fn failed_redraw_and_failed_hour_never_export_or_acknowledge_stale_pixels() -> std::io::Result<()> {
    let fixture = Fixture::load("viewer")?;
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
    });
    viewer.install_map(document, fixture.scene()?, "fixture".into());
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    frame.wgpu_render_state = Some(fixture.state.clone());
    let mut target = target(&fixture);
    until(&mut viewer, &ctx, &frame, &target, |v| {
        v.rendered.is_some() || v.error.is_some() || v.map_error.is_some()
    });
    assert!(
        viewer.error.is_none() && viewer.map_error.is_none(),
        "{:?}",
        viewer.map_state(&target)
    );
    let registered = viewer.generation;
    let texture = viewer.texture;
    let failed_path = fixture.output.join("failed-redraw.png");
    arm(&mut viewer, failed_path.clone(), request.camera);
    viewer.scene.as_mut().unwrap().test_faults.fatal = Some(Point::Post);
    let submissions = viewer.scene.as_ref().unwrap().test_faults.submissions.get();
    target.sequence += 1;
    until(&mut viewer, &ctx, &frame, &target, |v| v.error.is_some());
    let state = viewer.map_state(&target);
    assert!(state.error.as_deref().unwrap().contains("test Map Post"));
    assert!(!state.loading);
    assert!(state.hour.is_none() && state.branch_id.is_none());
    assert_ne!(state.sequence, state.requested_sequence);
    assert!(!failed_path.exists());
    assert!(
        viewer
            .capture_note
            .as_deref()
            .is_none_or(|note| !note.starts_with("Saved "))
    );
    assert_eq!(
        viewer.scene.as_ref().unwrap().test_faults.submissions.get(),
        submissions
    );
    assert_eq!(
        viewer.generation, registered,
        "failed target registered with egui"
    );
    assert_eq!(viewer.texture, texture);
    assert!(viewer.scene.as_ref().unwrap().generation() > registered);

    let recovered_path = fixture.output.join("recovered.png");
    viewer.scene.as_mut().unwrap().test_faults.fatal = None;
    viewer.capture.as_mut().unwrap().path = recovered_path.clone();
    viewer.retry_map();
    until(&mut viewer, &ctx, &frame, &target, |v| {
        v.capture.is_none() || v.error.is_some()
    });
    assert!(recovered_path.exists(), "{:?}", viewer.capture_note);
    assert_eq!(
        viewer.generation,
        viewer.scene.as_ref().unwrap().generation()
    );
    let state = viewer.map_state(&target);
    assert!(state.error.is_none() && !state.loading);
    assert_eq!(state.hour, Some(target.hour));
    assert_eq!(state.sequence, state.requested_sequence);

    // Real consumer path for a failed hour builder and a decoded gap. Synthetic
    // failure operands avoid inventing a later healthy hour in the OA recording.
    for (index, gap) in [false, true].into_iter().enumerate() {
        let reason = if gap {
            "synthetic recorded allocation gap"
        } else {
            "synthetic unavailable hour operands"
        };
        target.hour -= 1;
        target.sequence += 1;
        target.seek_sequence = Some(target.sequence);
        let current = if gap {
            crate::playback::Current::Gap {
                branch: fixture.input.branch,
                hour: target.hour,
                reason: reason.into(),
            }
        } else {
            let mut playback =
                crate::playback::Playback::shared(fixture.document.playback.replay_handle())?;
            playback.seek(fixture.input.branch, target.hour)?;
            playback.current().clone()
        };
        viewer.ready.insert(
            (fixture.input.branch, target.hour),
            hours::Built {
                epoch: viewer.epoch,
                branch: fixture.input.branch,
                hour: target.hour,
                view: hours::View {
                    camera: request.camera,
                    viewport: [257, 145],
                    ui_scale: 1.,
                },
                current,
                layers: Err(reason.into()),
                laps: Vec::new(),
                wall: 0.,
            },
        );
        let failed_path = fixture.output.join(format!("failed-hour-{index}.png"));
        arm(&mut viewer, failed_path.clone(), request.camera);
        let submissions = viewer.scene.as_ref().unwrap().test_faults.submissions.get();
        until(&mut viewer, &ctx, &frame, &target, |v| {
            v.map_error.is_some()
        });
        let state = viewer.map_state(&target);
        assert_eq!(state.error.as_deref(), Some(reason));
        assert!(state.hour.is_none() && state.branch_id.is_none());
        assert_ne!(state.sequence, state.requested_sequence);
        assert!(!failed_path.exists());
        assert_eq!(
            viewer.scene.as_ref().unwrap().test_faults.submissions.get(),
            submissions
        );
    }
    // Recover to the real decided hour, then refuse an unwritable output in the
    // existing capture path. A rendered frame is not an export success.
    target.hour = fixture.input.hour;
    target.sequence += 1;
    target.seek_sequence = Some(target.sequence);
    arm(&mut viewer, fixture.output.clone(), request.camera);
    until(&mut viewer, &ctx, &frame, &target, |v| {
        v.capture.is_none() || v.error.is_some()
    });
    assert!(
        viewer.error.is_none() && viewer.map_error.is_none(),
        "{:?}",
        viewer.map_state(&target)
    );
    let note = viewer
        .capture_note
        .as_deref()
        .expect("output failure reported");
    assert!(
        !note.starts_with("Saved ") && note.contains(&fixture.output.display().to_string()),
        "{note}"
    );
    let state = viewer.map_state(&target);
    assert_eq!(state.hour, Some(fixture.input.hour));
    assert_eq!(state.sequence, state.requested_sequence);
    Ok(())
}
