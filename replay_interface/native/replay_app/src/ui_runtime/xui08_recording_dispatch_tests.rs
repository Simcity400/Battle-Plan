//! Conditional current-recording coverage, separate from deterministic fixtures.
//! An ignored result is not evidence that the dispatcher or rendering passed.
use super::*;
use std::path::PathBuf;

fn required_environment(name: &str) -> String {
    let value = std::env::var(name).unwrap_or_else(|error| {
        panic!("current-recording XUI08 regression requires explicit {name}: {error}")
    });
    assert!(!value.trim().is_empty(), "{name} must not be empty");
    value
}

fn recording_sha256(path: &std::path::Path) -> String {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path).expect("open explicit recording for byte custody");
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).expect("read explicit recording for byte custody");
        if count == 0 { break; }
        hash.update(&buffer[..count]);
    }
    format!("{:x}", hash.finalize())
}

#[test]
#[ignore = "requires explicit current vanilla .battleplan, admitted install/profile, active PCS3 hour and current-machine D3D11; never a skip-as-proof"]
fn current_vanilla_recording_occluded_dispatch_preserves_presented_xui08() {
    let recording = PathBuf::from(required_environment("BATTLE_PLAN_ARROW_XUI08_RECORDING"));
    let expected_sha256 = required_environment("BATTLE_PLAN_ARROW_XUI08_RECORDING_SHA256");
    assert_eq!(expected_sha256.len(), 64, "explicit recording SHA256 must have 64 hex digits");
    assert_eq!(recording_sha256(&recording), expected_sha256,
        "recording differs from the parent's current-capture receipt");
    let hour: i64 = required_environment("BATTLE_PLAN_ARROW_XUI08_HOUR")
        .parse().expect("explicit canonical XUI08 hour must be signed i64");
    let paths = crate::install_paths::ProductPaths {
        game_install: PathBuf::from(required_environment("BATTLE_PLAN_ARROW_XUI08_GAME")),
        user_data: PathBuf::from(required_environment("BATTLE_PLAN_ARROW_XUI08_USERDATA")),
    };
    let logical_stream_id = std::env::var("BATTLE_PLAN_ARROW_XUI08_LOGICAL_STREAM").or_else(|_| std::env::var("HOI4_ARROW_XUI08_LOGICAL_STREAM"))
        .ok().map(|value| value.parse::<u64>().expect("logical stream must be unsigned u64"));
    let source = crate::recording_open::open_product_recording_segment(
        &recording, logical_stream_id,
    ).expect("open current recording through product container/prefold admission");
    let (provider, _) = crate::timeline::build_recording_provider(
        None, source,
        replay_core::TimelineConfig {
            frame_cache_entries: 1,
            build_schedule: replay_core::BuildSchedule::Serial,
        },
        &recording.display().to_string(),
    ).expect("open product timeline without a debug bundle");
    let manifest = provider.content_manifest().expect("recording must carry its content manifest");
    assert_eq!(manifest.source_game.executable_sha256,
        replay_core::peace_conference_presentation::XUI08_EXECUTABLE_SHA256);
    assert!(manifest.ordered_layers.iter().all(|layer|
        layer.layer_kind != portable_content_manifest::ContentLayerKind::Mod),
        "this conditional acceptance is vanilla only; mod acceptance is not implied");
    let (resolver, admission) = crate::content_compatibility::bind_provider_content(
        provider.as_ref(), &paths,
    ).expect("authenticate actual effective content against this recording");
    let profile = crate::install_paths::playback_ui_profile_from_paths(
        &paths, &resolver, crate::win32::actual_client_input_variant(),
    ).expect("admit actual current-machine UI profile");
    let focus = Arc::new(replay_core::admit_static_focus_definitions(
        provider.static_artifact(replay_core::FOCUS_DEFS_STATIC_ARTIFACT)
            .expect("recording lacks authenticated focus definitions"),
    ).expect("admit recording's actual focus definitions"));
    let research = Arc::new(game_ui::static_research::admit_static_research_catalogs(
        provider.static_artifact(replay_core::LOCALISATION_JOIN_ARTIFACT),
        provider.static_artifact(replay_core::IDEA_TECH_LOC_XREF_ARTIFACT),
        provider.static_artifact(replay_core::TECHNOLOGY_SHARING_ARTIFACT),
        &focus,
    ).expect("admit recording's actual research/static catalogs"));
    let frame = provider.world_frame_at(hour).expect("decode explicit admitted hour")
        .expect("explicit hour has no WorldFrame");
    let exact = frame.peace_conference_presentation_exact().presentation.as_ref()
        .expect("selected hour requires an authenticated active PCS3 carrier");
    let snapshot = game_ui::readonly_design_peace::peace_conference_snapshot_from_world(&frame)
        .expect("convert actual current PCS3 snapshot");
    let conference = game_ui::readonly_design_peace::admit_peace_conference(&snapshot)
        .expect("admit actual conference");
    assert!(conference.active(), "measured absent conference is not positive XUI08 coverage");
    let record_sha256 = exact.record_sha256.clone();
    let viewport = profile.physical_viewport();
    // Device creation follows recording/content/catalog admission, just as playback does.
    let gfx = Gfx::create_offscreen(viewport.width, viewport.height)
        .expect("create actual current-machine D3D11 offscreen destination");
    let mut runtime = ProductObserverUiRuntime::from_playback_content(
        &gfx, &resolver, &admission, &profile, provider.decision_catalog(), research, focus,
    ).expect("construct the actual product UI runtime and all required admitted providers");
    let mut viewer = game_ui::production_renderer::ObserverViewerUiState::default();

    // Establish the passive conference through execute/Presented before the
    // viewer command can open it. No direct replacement of presented authority.
    runtime.begin_observer_frame_transaction(&viewer).unwrap();
    let closed = runtime.execute(&gfx, &frame, &mut viewer, 0, false, 100_000, Some(game_ui::NativeGuiClock::from_application_elapsed_nanoseconds(100_000 * 1000)),
        ProductionLinesUiSupplementalInputs::from_frame(&frame), None)
        .expect("execute actual closed-panel observer frame");
    runtime.finish_observer_frame_disposition(&mut viewer,
        ObserverUiFrameDisposition::Presented, true, closed.production_lines_applied_scroll_offset)
        .expect("publish the passive actual conference");
    assert_eq!(runtime.xui08_presented.as_ref().unwrap().record_sha256, record_sha256);

    runtime.begin_observer_frame_transaction(&viewer).unwrap();
    runtime.toggle_xui08(&mut viewer).expect("actual viewer command opens Presented conference");
    let opened = runtime.execute(&gfx, &frame, &mut viewer, 0, false, 200_000, Some(game_ui::NativeGuiClock::from_application_elapsed_nanoseconds(200_000 * 1000)),
        ProductionLinesUiSupplementalInputs::from_frame(&frame), None)
        .expect("execute actual open-panel compositor");
    assert!(opened.authored_roots.iter().any(|root|
        root == game_ui::peace_conference_ui::PEACE_CONFERENCE_ROOT),
        "no actual XUI08 root rendered; gaps={:?}", opened.feature_local_gaps);
    assert!(opened.peace_conference_input_region.is_some(),
        "render refusal is not positive XUI08 dispatcher coverage: {:?}", opened.feature_local_gaps);
    runtime.finish_observer_frame_disposition(&mut viewer,
        ObserverUiFrameDisposition::Presented, true, opened.production_lines_applied_scroll_offset)
        .expect("publish actual rendered XUI08 frame");
    let published_viewer = viewer.peace_conference.clone();
    let published = runtime.xui08_presented.as_ref().expect("Presented XUI08 authority").clone();
    assert!(published.input_region.is_some());
    assert!(!published.hit_targets.is_empty(), "positive publication must contain actual hit authority");

    // The real close command is local intent. Occlusion rejects the frame via
    // the product dispatcher, not by manually invoking a rollback helper.
    runtime.begin_observer_frame_transaction(&viewer).unwrap();
    runtime.close_xui08(&mut viewer);
    let candidate = runtime.execute(&gfx, &frame, &mut viewer, 0, false, 300_000, Some(game_ui::NativeGuiClock::from_application_elapsed_nanoseconds(300_000 * 1000)),
        ProductionLinesUiSupplementalInputs::from_frame(&frame), None)
        .expect("execute staged local close before occlusion");
    assert!(!viewer.peace_conference.open);
    assert!(runtime.xui08_pending_staged);
    runtime.finish_observer_frame_disposition(&mut viewer,
        ObserverUiFrameDisposition::Occluded, true, candidate.production_lines_applied_scroll_offset)
        .expect("actual Occluded dispatcher rejects executed frame");
    assert_eq!(viewer.peace_conference, published_viewer);
    let retained = runtime.xui08_presented.as_ref().expect("prior publication survives occlusion");
    assert_eq!(retained.record_sha256, published.record_sha256);
    assert_eq!(retained.input_region, published.input_region);
    assert_eq!(retained.hit_targets, published.hit_targets);
    assert_eq!(retained.tooltip_targets, published.tooltip_targets);
    assert!(runtime.xui08_pending.is_none());
    assert!(!runtime.xui08_pending_staged);
    assert!(runtime.xui08_pending_viewer_checkpoint.is_none());
    assert!(runtime.observer_frame_checkpoint.is_none());
    assert!(!runtime.present_transaction.has_pending_disposition());

    runtime.begin_observer_frame_transaction(&viewer).unwrap();
    let recovery = runtime.execute(&gfx, &frame, &mut viewer, 0, false, 400_000, Some(game_ui::NativeGuiClock::from_application_elapsed_nanoseconds(400_000 * 1000)),
        ProductionLinesUiSupplementalInputs::from_frame(&frame), None)
        .expect("execute the next frame after actual occlusion rollback");
    assert!(recovery.peace_conference_input_region.is_some(),
        "next actual frame did not recover XUI08 rendering: {:?}", recovery.feature_local_gaps);
    runtime.finish_observer_frame_disposition(&mut viewer,
        ObserverUiFrameDisposition::Presented, true, recovery.production_lines_applied_scroll_offset)
        .expect("actual Presented dispatcher accepts recovery");
    assert!(viewer.peace_conference.open, "occluded close must not remain entered");
    assert_eq!(runtime.xui08_presented.as_ref().unwrap().record_sha256, record_sha256);
    assert!(!runtime.present_transaction.has_pending_disposition());
    assert_eq!(recording_sha256(&recording), expected_sha256,
        "recording bytes changed during the current-machine test");
    eprintln!("{}", serde_json::json!({
        "schema": "arrow.xui08.current_recording_dispatch.v1",
        "recording_sha256": expected_sha256, "logical_stream_id": logical_stream_id,
        "hour": hour, "record_sha256": record_sha256,
        "content_manifest_sha256": admission.manifest_digest_sha256(),
        "viewport": [viewport.width, viewport.height],
        "presented_hit_count": published.hit_targets.len(),
        "actual_dispatch": "closed_presented_then_open_presented_then_close_occluded_then_recovery_presented",
        "scope": "current-machine vanilla XUI08 only; no clean-machine or whole-project claim"
    }));
}
