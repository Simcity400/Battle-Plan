use super::*;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[test]
#[ignore = "whole-capture measurement requires XMAP14_MEASUREMENT_REPLAY, XMAP14_MEASUREMENT_OUT, BATTLE_PLAN_GAME_DIR and BATTLE_PLAN_USER_DIR"]
fn whole_capture_prepared_land_measurement() {
    let recording = PathBuf::from(std::env::var_os("XMAP14_MEASUREMENT_REPLAY").unwrap());
    let out = PathBuf::from(std::env::var_os("XMAP14_MEASUREMENT_OUT").unwrap());
    let paths = crate::install_paths::ProductPaths {
        game_install: PathBuf::from(std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).unwrap()),
        user_data: PathBuf::from(std::env::var_os("BATTLE_PLAN_USER_DIR").or_else(|| std::env::var_os("HOI4_USER_DIR")).unwrap()),
    };
    let source = crate::recording_open::open_product_recording(&recording).unwrap();
    let provider = crate::timeline::build_recording_provider(
        None,
        source,
        replay_core::TimelineConfig {
            frame_cache_entries: 1,
            ..Default::default()
        },
        &recording.display().to_string(),
    )
    .unwrap()
    .0;
    let hours = provider.hours();
    assert!(hours.len() >= 3);
    let mut tail = Vec::new();
    let last_control = hours
        .iter()
        .rev()
        .copied()
        .find(|hour| {
            let world = provider.world_frame_at(*hour).unwrap().unwrap();
            let map = world.domain("map");
            let authority = map.and_then(|map| map.get("_r2_xmap05_control_authority"));
            let present = authority.is_some_and(|value| value.is_object());
            tail.push(
                json!({"hour_index": hour, "has_mapc1_control_authority": present,
            "map_domain": if present { None } else { map },
            "provenance": if present { None } else { world.domain("provenance") }}),
            );
            present
        })
        .expect("recording carries MAPC1 control authority");
    let selected = [hours[0], hours[hours.len() / 2], last_control];
    eprintln!("selected hours: {selected:?}; authenticating content");
    let (resolver, admission) =
        crate::content_compatibility::bind_provider_content(provider.as_ref(), &paths).unwrap();
    let manifest = provider.content_manifest().unwrap();
    let defines = map_viewer::LandCombatStrengthDefines::from_admitted_content(&resolver).unwrap();
    let (width, height) = map_viewer::foundation_map_dimensions(&resolver, &admission).unwrap();
    let camera = map_viewer::ViewerMapCameraState::from_admitted_content(
        &resolver,
        &admission,
        width,
        height,
        1920,
        1080,
        map_viewer::DEFAULT_VIEWER_CAMERA_SPEED_SETTING,
    )
    .unwrap();
    let viewer = map_viewer::ViewerOwnedMapState {
        sky_enabled: true,
        unit_models_enabled: true,
        viewport_width: 1920,
        viewport_height: 1080,
        input_revision: camera.input_revision,
        camera: Some(camera),
        observer_country_id: None,
        map_mode_id: map_viewer::ViewerMapModeSelection::default().viewer_token(),
        map_mode_selection_epoch: 0,
        selected_object_ids: Vec::new(),
        animation_time_f32_bits: 0,
        frame_delta_seconds_f32_bits: 0,
    };
    let (terrain, _) =
        crate::install_paths::playback_render_settings(&paths, &manifest.required_settings)
            .unwrap();
    let mut session = ProductMapFoundationSession::start(
        &resolver,
        &admission,
        manifest,
        terrain,
        provider.foundation_content(),
    )
    .unwrap();
    eprintln!("content authenticated; preparing frames");
    let mut phase_state = map_viewer::xmap07_snow_mud_phase::Xmap07SnowMudPhaseState::default();
    let phase_durations = map_viewer::xmap07_snow_mud_phase::Xmap07SnowMudPhaseDurations::load(&resolver).unwrap();
    let phase_clock = std::time::Instant::now();
    let mut rows = Vec::new();
    for hour in selected {
        eprintln!("preparing hour {hour}");
        let world = provider.world_frame_at(hour).unwrap().unwrap();
        let raw = world.xmap14_combat_indicator_carrier();
        let unplaced = raw
            .and_then(|raw| raw.get("unplaced"))
            .and_then(|rows| rows.as_array())
            .map(|rows| {
                rows.iter()
                    .filter(|row| row["identity"]["kind"] == "land")
                    .count()
            });
        let mut row = json!({
            "hour_index": hour, "converted_strength_defines": defines,
            "admitted_content_digest": session.content_receipt.content_authority_sha256,
            "unplaced_land_count": unplaced, "placed_land_count": null,
            "named_land_exclusion_counts": {}, "prepared_indicator_rows": null,
            "carrier": raw,
        });
        let phase = phase_state.sample(&world, phase_clock.elapsed().as_secs_f64(), crate::transport::DEFAULT_SPEED_STEP, &phase_durations).unwrap();
        session.update_unit_motion(&world, map_viewer::Xmap13MotionClock {
            seconds: phase_clock.elapsed().as_secs_f64(),
            speed_step: crate::transport::DEFAULT_SPEED_STEP,
            discontinuity: true,
        }).unwrap();
        match session.adapters.prepare_with_xmap12_inputs(
            &world,
            &viewer,
            manifest,
            &session.content_receipt,
            None,
            None,
            Some(&phase),
        ) {
            Ok(prepared) => {
                let mut causes = BTreeMap::<String, u64>::new();
                for exclusion in &prepared.envelope.element_diagnostics {
                    if exclusion.cause_id.starts_with("land-") {
                        *causes.entry(exclusion.cause_id.clone()).or_default() +=
                            exclusion.element_count;
                    }
                }
                row["named_land_exclusion_counts"] = json!(causes);
                row["family_dispositions"] = json!(prepared.envelope.families);
                let identities = map_viewer::Xmap13CombatIdentityProvider::from_world_frame(
                    &world, &session.content_receipt.deterministic_sha256, prepared.units.as_ref(),
                ).unwrap();
                row["xmap13_provider"] = json!({
                    "hour_index": identities.hour_index(),
                    "division_count": identities.division_count(),
                    "census_sha256": identities.census_sha256(),
                    "admission_sha256": identities.admission_sha256(),
                });
                if let Some(combat) = &prepared.combat {
                    assert_eq!(combat.hour_index, hour);
                    assert_eq!(identities.hour_index(), hour);
                    assert_eq!(combat.xmap13_census_sha256, identities.census_sha256());
                    assert_eq!(combat.xmap13_admission_sha256, identities.admission_sha256());
                    let land: Vec<_> = combat
                        .indicators
                        .iter()
                        .filter(|row| row.kind == map_viewer::Xmap14PreparedKind::Land)
                        .collect();
                    row["placed_land_count"] = json!(land.len());
                    row["prepared_indicator_rows"] = json!(land);
                    row["prepared_sha256"] = json!(combat.deterministic_sha256);
                    row["status"] = json!(if land.is_empty()
                        && unplaced.is_some_and(|count| count > 0)
                    {
                        "finding-zero-placed"
                    } else {
                        "measured"
                    });
                } else {
                    row["status"] = json!("combat-family-unavailable");
                }
            }
            Err(error) => {
                row["status"] = json!("preparation-refused");
                row["error"] = json!(error.to_string());
            }
        }
        rows.push(row);
        let report = json!({
            "schema": "hoi4.prepared_land_measurement.v1", "recording": recording,
            "hour_count": hours.len(), "first_hour": hours[0], "last_hour": hours[hours.len()-1],
            "selection": "first-middle-last-with-MAPC1; no indexed land-combat census is exposed by the product provider",
            "last_mapc1_hour": last_control, "tail_control_authority": tail,
            "command": std::env::var("XMAP14_MEASUREMENT_COMMAND").unwrap(),
            "viewer": viewer, "hours": rows,
        });
        std::fs::write(&out, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    assert_eq!(rows.len(), 3);
    for row in &rows {
        assert!(
            row["placed_land_count"].is_number(),
            "prepared combat measurement refused: {row}"
        );
        assert!(row["prepared_indicator_rows"].is_array());
        if row["unplaced_land_count"].as_u64().is_some_and(|count| count > 0) {
            assert!(row["placed_land_count"].as_u64().is_some_and(|count| count > 0),
                "positive land input produced no prepared indicators: {row}");
            assert!(!row["prepared_indicator_rows"].as_array().unwrap().is_empty());
        }
    }
}
