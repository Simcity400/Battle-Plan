use super::*;
use crate::map::scene_failure_tests::Fixture;
use viewer_gpu::camera::Placement;
#[cfg(test)]
use viewer_runtime::test_support::TEST_HANG_GUARD;
use viewer_world::scene_faults::Point;

pub(super) fn target(fixture: &Fixture) -> dashboard_core::protocol::DashboardMapView {
    let bounds = dashboard_core::protocol::ViewBounds {
        x: 0,
        y: 0,
        width: 256,
        height: 144,
    };
    dashboard_core::protocol::DashboardMapView {
        options: Default::default(),
        id: "failure-fixture".into(),
        source_id: "verified-fixture".into(),
        path: "fixture".into(),
        revision: "fixture".into(),
        branch_id: fixture.input.branch.to_string(),
        playing: false,
        hours_per_second: 1.0,
        hour: fixture.input.hour,
        fraction: 0.,
        sequence: 1,
        seek_sequence: Some(1),
        composed_at: 0.0,
        clock_id: "page-1".into(),
        bounds,
        clip: bounds,
        visible: true,
    }
}

fn draw(
    viewer: &mut Viewer,
    ctx: &egui::Context,
    frame: &eframe::Frame,
    target: &dashboard_core::protocol::DashboardMapView,
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

pub(super) fn until(
    viewer: &mut Viewer,
    ctx: &egui::Context,
    frame: &eframe::Frame,
    target: &dashboard_core::protocol::DashboardMapView,
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
                pose: viewer_gpu::camera::Pose::Map,
            }),
        )),
        exit: false,
        time: Some(0.),
        step: 0.,
        frames: 1,
        index: 0,
        paced: false,
        export: None,
    });
}

#[test]
#[ignore = "verified replay/content and GPU; requires BATTLEPLAN_SCENE_FAILURE_INPUT"]
fn a_drawn_map_names_an_undeclared_distant_battle_effect() -> std::io::Result<()> {
    viewer_audio::System::silent();
    let mut fixture = Fixture::load("viewer-battle-silence")?;
    let content = viewer_audio::Content::new(Default::default(), None).with_battles(
        Some(content_reader_sound::combat_sounds::BattleDefines {
            effect: "undeclared_battle_effect".into(),
            falloff_default: 50.0,
            min_unit_count: 1,
        }),
        Default::default(),
    );
    std::sync::Arc::get_mut(&mut fixture.document.content)
        .expect("the fixture's freshly loaded content has no other holder")
        .audio = Some(Arc::new(viewer_audio::Source {
        content: Arc::new(content),
        layers: Default::default(),
    }));
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
        pose: viewer_gpu::camera::Pose::Map,
    });
    viewer.install_map(document, fixture.scene()?, "fixture".into());
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    frame.wgpu_render_state = Some(fixture.state.clone());
    let target = target(&fixture);
    until(&mut viewer, &ctx, &frame, &target, |v| {
        v.rendered.is_some() || v.error.is_some() || v.map_error.is_some()
    });
    assert!(
        viewer.error.is_none() && viewer.map_error.is_none(),
        "{:?}",
        viewer.map_state(&target)
    );
    assert!(
        viewer.map_warnings.iter().any(|warning| {
            warning.starts_with(
            "Map battle ambience: the battle sound effect undeclared_battle_effect is not declared"
        )
        }),
        "{:?}",
        viewer.map_warnings
    );
    Ok(())
}

#[test]
#[ignore = "verified replay/content and GPU; requires BATTLEPLAN_SCENE_FAILURE_INPUT"]
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
        Options {
            postfx: true,
            ..Options::default()
        },
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
    let registered = viewer.scene_target.generation;
    let texture = viewer.scene_target.texture;
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
        viewer.scene_target.generation, registered,
        "failed target registered with egui"
    );
    assert_eq!(viewer.scene_target.texture, texture);
    assert!(viewer.scene.as_ref().unwrap().generation() > registered);

    assert!(viewer.capture.is_none(), "the failed frame refused its capture by name");
    let recovered_path = fixture.output.join("recovered.png");
    viewer.scene.as_mut().unwrap().test_faults.fatal = None;
    arm(&mut viewer, recovered_path.clone(), request.camera);
    viewer.retry_map();
    until(&mut viewer, &ctx, &frame, &target, |v| {
        v.capture.is_none() || v.error.is_some()
    });
    assert!(recovered_path.exists(), "{:?}", viewer.capture_note);
    assert_eq!(
        viewer.scene_target.generation,
        viewer.scene.as_ref().unwrap().generation()
    );
    let state = viewer.map_state(&target);
    assert!(state.error.is_none() && !state.loading);
    assert_eq!(state.hour, Some(target.hour));
    assert_eq!(state.sequence, state.requested_sequence);

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

#[test]
#[ignore = "verified replay/content and GPU; requires BATTLEPLAN_SCENE_FAILURE_INPUT"]
fn a_widget_export_writes_the_frame_it_draws_at_the_widgets_own_size() -> std::io::Result<()> {
    let fixture = Fixture::load("viewer-export")?;
    let request = fixture.request([256, 144]);
    let document = Document::from_shared_content(
        fixture.document.content.clone(),
        fixture.document.playback.replay_handle(),
        &InitialHour { branch: Some(fixture.input.branch), hour: Some(fixture.input.hour), hour_fraction: 0. },
    )?;
    let mut viewer = Viewer::create(Config::default(), Options::default(), None, Arc::new(Mutex::new(None)), None, Some((fixture.state.device.clone(), fixture.state.queue.clone())));
    viewer.dashboard_map = true;
    viewer.diagnostics = false;
    viewer.graphics.samples = 1;
    viewer.camera_manual = true;
    viewer.camera = Some(Placement { center: request.camera.center, height: request.camera.height, pose: viewer_gpu::camera::Pose::Map });
    viewer.install_map(document, fixture.scene()?, "fixture".into());
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    frame.wgpu_render_state = Some(fixture.state.clone());
    let target = target(&fixture);
    until(&mut viewer, &ctx, &frame, &target, |v| v.rendered.is_some() || v.error.is_some() || v.map_error.is_some());
    let path = fixture.output.join("widget-export.png");
    let (job, answers) = crate::host::tests::job(&path.display().to_string());
    viewer.host_export(job, &ctx, &mut frame);
    until(&mut viewer, &ctx, &frame, &target, |v| v.capture.is_none() || v.error.is_some());
    let answered = crate::host::tests::taken(&answers);
    assert!(matches!(answered.as_slice(), [Ok(saved)] if saved.starts_with("Saved ")), "{answered:?}");
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&path)?));
    let mut reader = decoder.read_info().map_err(std::io::Error::other)?;
    let mut pixels = vec![0; reader.output_buffer_size().ok_or_else(|| std::io::Error::other("the PNG has no size"))?];
    let info = reader.next_frame(&mut pixels).map_err(std::io::Error::other)?;
    assert_eq!((info.width, info.height), (target.bounds.width, target.bounds.height), "the picture is the widget’s own size");
    let first = &pixels[..4];
    assert!(pixels[..info.buffer_size()].chunks_exact(4).any(|pixel| pixel != first), "the picture holds the drawn map, not one flat colour");
    Ok(())
}

#[test]
#[ignore = "verified replay/content and GPU; requires BATTLEPLAN_SCENE_FAILURE_INPUT"]
fn a_retired_map_shown_again_draws_its_new_target() -> std::io::Result<()> {
    let fixture = Fixture::load("viewer-retired")?;
    let document = || {
        Document::from_shared_content(
            fixture.document.content.clone(),
            fixture.document.playback.replay_handle(),
            &InitialHour {
                branch: Some(fixture.input.branch),
                hour: Some(fixture.input.hour),
                hour_fraction: 0.,
            },
        )
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
    viewer.install_map(document()?, fixture.scene()?, "fixture".into());
    until(&mut viewer, &ctx, &frame, &target, |v| {
        v.rendered.is_some() || v.error.is_some() || v.map_error.is_some()
    });
    assert!(viewer.error.is_none() && viewer.map_error.is_none());
    let shown = viewer.scene_target.generation;
    viewer.retire_map(&frame, ());
    assert!(viewer.scene_target.texture.is_none());
    viewer.install_map(document()?, fixture.scene()?, "fixture".into());
    until(&mut viewer, &ctx, &frame, &target, |v| {
        v.rendered.is_some() || v.error.is_some() || v.map_error.is_some()
    });
    assert!(
        viewer.error.is_none() && viewer.map_error.is_none(),
        "{:?}",
        viewer.map_state(&target)
    );
    assert_eq!(viewer.scene.as_ref().unwrap().generation(), shown);
    assert!(viewer.scene_target.texture.is_some());
    Ok(())
}
