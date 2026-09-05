use serde_json::json;
use std::io::Write;
use std::path::PathBuf;

#[test]
#[ignore = "requires the recovered current recording, authenticated content, and a scheduled D3D functional slot"]
fn current_recording_executes_nonzero_paired_lights() -> crate::Result<()> {
    let recording = PathBuf::from(
        std::env::var_os("PARTICLE_FUNCTIONAL_REPLAY")
            .ok_or("PARTICLE_FUNCTIONAL_REPLAY is required")?,
    );
    let output = PathBuf::from(
        std::env::var_os("PARTICLE_FUNCTIONAL_OUT").ok_or("PARTICLE_FUNCTIONAL_OUT is required")?,
    );
    let selected_hour: i64 = std::env::var("PARTICLE_FUNCTIONAL_HOUR")?.parse()?;
    let mut log = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    writeln!(
        log,
        "{}",
        json!({"schema": "hoi4.paired-light-functional.v1",
        "recording": recording, "recording_bytes": std::fs::metadata(&recording)?.len(),
        "selected_hour": selected_hour, "max_presentations": 360,
        "renderer_delta_f32_bits": (1.0f32 / 60.0).to_bits()})
    )?;
    log.flush()?;
    let result = execute(&recording, selected_hour, &mut log);
    writeln!(
        log,
        "{}",
        json!({"stage": "finished", "passed": result.is_ok(),
        "error": result.as_ref().err().map(ToString::to_string)})
    )?;
    log.flush()?;
    result
}

fn execute(
    recording: &std::path::Path,
    selected_hour: i64,
    log: &mut std::fs::File,
) -> crate::Result<()> {
    let paths = crate::install_paths::ProductPaths {
        game_install: PathBuf::from(
            std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).ok_or("BATTLE_PLAN_GAME_DIR is required")?,
        ),
        user_data: PathBuf::from(
            std::env::var_os("BATTLE_PLAN_USER_DIR").or_else(|| std::env::var_os("HOI4_USER_DIR")).ok_or("BATTLE_PLAN_USER_DIR is required")?,
        ),
    };
    writeln!(log, "{}", json!({"stage": "opening_package"}))?;
    log.flush()?;
    let source = crate::recording_open::open_product_recording(recording)?;
    writeln!(log, "{}", json!({"stage": "package_opened"}))?;
    log.flush()?;
    let provider = crate::timeline::build_recording_provider(
        None,
        source,
        replay_core::TimelineConfig {
            frame_cache_entries: 1,
            ..Default::default()
        },
        &recording.display().to_string(),
    )?
    .0;
    if !provider.hours().contains(&selected_hour) {
        return Err("requested functional hour is absent from the current recording".into());
    }
    writeln!(log, "{}", json!({"stage": "loading_world_frame", "hour": selected_hour,
        "recorded_hours": provider.hours().len()}))?;
    log.flush()?;
    let world = provider
        .world_frame_at(selected_hour)?
        .ok_or("functional world frame is unavailable")?;
    writeln!(
        log,
        "{}",
        json!({"stage": "opened", "recorded_hours": provider.hours().len()})
    )?;
    log.flush()?;
    let light_input = map_viewer::map_authority_viewer_derived_input_status(
        &world, "lighting.point_light_lifecycle",
    );
    if light_input != Some(replay_core::ViewStatus::Present) {
        writeln!(log, "{}", json!({"stage": "light_population_unavailable",
            "hour": selected_hour, "status": format!("{light_input:?}")}))?;
        log.flush()?;
        return Err("selected recording hour cannot admit the paired-light population".into());
    }
    let (resolver, admission) =
        crate::content_compatibility::bind_provider_content(provider.as_ref(), &paths)
            .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
    writeln!(log, "{}", json!({"stage": "content_admitted"}))?;
    log.flush()?;
    let manifest = provider
        .content_manifest()
        .ok_or("current recording content manifest is unavailable")?;
    let (width, height) = map_viewer::foundation_map_dimensions(&resolver, &admission)?;
    let camera = map_viewer::ViewerMapCameraState::from_admitted_content(
        &resolver,
        &admission,
        width,
        height,
        1920,
        1080,
        map_viewer::DEFAULT_VIEWER_CAMERA_SPEED_SETTING,
    )?;
    let mut viewer = map_viewer::ViewerOwnedMapState {
        sky_enabled: true,
        viewport_width: 1920,
        viewport_height: 1080,
        input_revision: camera.input_revision,
        camera: Some(camera),
        observer_country_id: None,
        map_mode_id: map_viewer::ViewerMapModeSelection::default().viewer_token(),
        map_mode_selection_epoch: 0,
        selected_object_ids: Vec::new(),
        animation_time_f32_bits: 0,
        frame_delta_seconds_f32_bits: (1.0f32 / 60.0).to_bits(),
    };
    let (terrain, _) =
        crate::install_paths::playback_render_settings(&paths, &manifest.required_settings)?;
    let ui = crate::install_paths::playback_ui_profile_from_paths(
        &paths,
        &resolver,
        crate::win32::actual_client_input_variant(),
    )?;
    let selection = map_viewer::xmap15_viewer_selection(&world, None);
    let mut composition = crate::app::ProductMapCompositionSession::start_for_headless(
        &resolver,
        &admission,
        manifest,
        terrain,
        1920,
        1080,
        provider.foundation_content(),
    )?;
    writeln!(
        log,
        "{}",
        json!({"stage": "content_and_renderer_ready",
        "content_manifest_sha256": manifest.manifest_digest_sha256,
        "product_modset_sha256": manifest.product_modset_sha256})
    )?;
    log.flush()?;
    let started = std::time::Instant::now();
    let mut nonzero_presentations = 0;
    for presentation in 1..=360 {
        if started.elapsed().as_secs() >= 240 {
            return Err(
                "functional paired-light measurement exceeded its 240-second work bound".into(),
            );
        }
        let delta = f32::from_bits(viewer.frame_delta_seconds_f32_bits);
        viewer.animation_time_f32_bits =
            (f32::from_bits(viewer.animation_time_f32_bits) + delta).to_bits();
        let executed = composition.execute(
            &resolver,
            &admission,
            manifest,
            &world,
            &viewer,
            &selection,
            ui.localisation_language(),
            delta,
            false,
            false,
            ui.key().gui_scale(),
        );
        let receipt = match executed {
            Ok(receipt) => receipt,
            Err(error) => {
                writeln!(
                    log,
                    "{}",
                    json!({"stage": "composition_refused", "presentation": presentation,
                    "viewer": viewer, "error": error.to_string(), "families": composition.last_foundation_envelope(),
                    "prepared_lighting": composition.last_prepared().and_then(|p| p.lighting.as_ref()).map(|l| json!({
                        "census": l.point_light_census,
                        "light_data_sha256": l.light_data_map.payload_sha256,
                        "light_data_bytes": l.light_data_map.bytes.as_ref(),
                        "light_index_sha256": l.light_index_map.payload_sha256
                    }))})
                )?;
                return Err(error);
            }
        };
        let lighting = composition
            .last_prepared()
            .and_then(|p| p.lighting.as_ref())
            .ok_or("executed player frame has no prepared lighting")?;
        lighting.validate()?;
        let counts = lighting
            .point_light_census
            .as_ref()
            .map_err(|e| e.to_string())?;
        writeln!(
            log,
            "{}",
            json!({"stage": "executed", "presentation": presentation,
            "viewer": viewer, "census": counts,
            "light_data_sha256": lighting.light_data_map.payload_sha256,
            "light_data_bytes": lighting.light_data_map.bytes.as_ref(),
            "light_index_sha256": lighting.light_index_map.payload_sha256,
            "composition": receipt})
        )?;
        log.flush()?;
        if !receipt.execution_complete
            || !receipt.foundation_complete
            || !receipt.final_pixels_measured
        {
            return Err("paired-light frame has no completed product execution/readback".into());
        }
        if counts.paired_uploaded_lights > 0 {
            nonzero_presentations += 1;
        }
        if nonzero_presentations == 2 {
            return Ok(());
        }
    }
    Err("current recording produced no two executed presentations with nonzero paired-light uploads".into())
}
