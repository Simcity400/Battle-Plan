use super::*;
use crate::production_features::xmap12::Xmap12CitiesBuildingsMapObjectsAdapter;
use crate::xmap09_light_census::admit_light_census;
use crate::xmap09_light_lifecycle::ViewerLightLifecycle;
use serde_json::json;

struct PackagedCache;
impl snapshot::CertifiedPrefoldSpoolDecoder for PackagedCache {
    fn decoder_id(&self) -> &'static str {
        "light-census.packaged-cache"
    }
    fn decode_certified_spool(
        &self,
        _: snapshot::CertifiedPrefoldSpoolRequest,
    ) -> std::result::Result<snapshot::PrefoldedSpool, snapshot::PrefoldDecodeError> {
        panic!("the whole-recording census requires its certified packaged cache")
    }
}

#[test]
#[ignore = "whole-recording recovered-06 light census; requires authenticated installed content"]
fn recovered_06_whole_recording_viewer_light_census() {
    let recording = std::path::PathBuf::from(std::env::var("LIGHT_CENSUS_RECORDING").unwrap());
    let game = std::path::PathBuf::from(std::env::var("BATTLE_PLAN_GAME_ROOT").or_else(|_| std::env::var("HOI4_GAME_ROOT")).unwrap());
    let profile = std::path::PathBuf::from(std::env::var("LIGHT_CENSUS_PROFILE").unwrap());
    let output = std::path::PathBuf::from(std::env::var("LIGHT_CENSUS_OUTPUT").unwrap());
    let scratch = std::env::temp_dir().join(format!("viewer_light_census_{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let source =
        replay_core::open_recording_with_spooled_decoder(&recording, &scratch, &PackagedCache)
            .unwrap();
    let timeline = replay_core::WorldTimeline::open_with_config(
        source,
        replay_core::TimelineConfig { frame_cache_entries: 1, ..Default::default() },
    )
    .unwrap();
    let hours = timeline.hours().to_vec();
    assert_eq!(hours.len(), 994);
    let manifest = timeline.content_manifest().unwrap();
    let layers = manifest
        .ordered_layers
        .iter()
        .filter(|l| l.required_for_playback && l.layer_kind != replay_core::ContentLayerKind::Base)
        .map(|l| crate::RecordedContentSource {
            ordinal: l.ordinal,
            source_id: l.layer_id.clone(),
        })
        .collect::<Vec<_>>();
    let resolver =
        crate::GameFileResolver::from_recorded_content_layers(game, profile, &layers).unwrap();
    let inventory = resolver
        .playback_inventory(
            manifest,
            crate::PlaybackEnvironment {
                source_game: manifest.source_game.clone(),
                source_profile: manifest.source_profile.clone(),
                dlc_selection: manifest.dlc_selection.clone(),
                localisation_language: manifest.localisation_language.clone(),
                settings: manifest.required_settings.clone(),
            },
        )
        .unwrap();
    let admission = timeline.admit_playback_content(&inventory).unwrap();
    let resolver = resolver.with_content_admission(&admission).unwrap();
    let (width, height) = foundation_map_dimensions(&resolver, &admission).unwrap();
    let camera = crate::ViewerMapCameraState::from_admitted_content(
        &resolver, &admission, width, height, 1920, 1080, 1.0,
    )
    .unwrap();
    let mut viewer = ViewerOwnedMapState {
        viewport_width: 1920,
        viewport_height: 1080,
        camera: Some(camera),
        observer_country_id: None,
        map_mode_id: "political".into(),
        map_mode_selection_epoch: 0,
        selected_object_ids: vec![],
        input_revision: 0,
        animation_time_f32_bits: 0,
        frame_delta_seconds_f32_bits: (1.0f32 / 60.0).to_bits(),
    };
    let camera = viewer.camera.as_ref().unwrap();
    let grid = crate::xmap09_tiled_point_light_grid(camera.current_eye(),
        camera.current_look_at_direction().unwrap(), width as f32, height as f32).unwrap();
    let adapter = Xmap12CitiesBuildingsMapObjectsAdapter::new(
        &resolver,
        &admission,
        crate::TerrainLodCullOptions::default(),
    )
    .unwrap();
    let mut lights = ViewerLightLifecycle::default();
    let mut building_visual = None;
    let mut fixed_runtime = None;
    let mut rows = Vec::new();
    let selection = Default::default();
    let mut progress = std::fs::File::create(output.with_extension("jsonl")).unwrap();
    for (index, &hour) in hours.iter().enumerate() {
        let frame = timeline.frame(hour).unwrap();
        viewer.animation_time_f32_bits = ((index + 1) as f32 / 60.0).to_bits();
        let mut context = FoundationFeatureContext::new_for_owned_accounting_tests(
            &frame,
            &viewer,
            &[],
            &[],
            &selection,
        );
        context.product_modset_sha256 = &manifest.product_modset_sha256;
        context.effective_vfs_manifest_sha256 = &manifest.effective_vfs_manifest_sha256;
        context.viewer_state_sha256 = &viewer.camera.as_ref().unwrap().deterministic_sha256;
        let result = adapter.prepare(&context, building_visual.as_ref(), fixed_runtime.as_ref());
        let census = match result {
            Ok(preparation) => match preparation.present {
                Some(present) => {
                    let census = present
                        .prepared
                        .placed_entities
                        .as_ref()
                        .map_err(|e| {
                            lights.reset_population();
                            e.clone()
                        })
                        .and_then(|placed| {
                            lights.derive_light_instance_rows(&resolver, placed, &viewer)
                        })
                        .and_then(|rows| {
                            rows.with_rows(|rows| {
                                admit_light_census(Ok(rows), grid)
                            })
                        })
                        .map(|census| census.counts)
                        .map_err(|e| e.to_string());
                    building_visual = Some(present.building_visual);
                    fixed_runtime = Some(present.fixed_runtime);
                    census
                }
                None => {
                    lights.reset_population();
                    Err(format!(
                        "XMAP12.placed_entities: {:?}",
                        preparation.output.disposition
                    ))
                }
            },
            Err(error) => {
                lights.reset_population();
                Err(format!("XMAP12.placed_entities: {error}"))
            }
        };
        rows.push(json!({"hour": hour, "census": census}));
        use std::io::Write;
        writeln!(progress, "{}", rows.last().unwrap()).unwrap();
        if index % 10 == 0 {
            println!(
                "light census {}/{} hour={hour}: {}",
                index + 1,
                hours.len(),
                rows.last().unwrap()["census"]
            );
        }
    }
    let admitted_hours = rows
        .iter()
        .filter(|r| r["census"].get("Ok").is_some())
        .count();
    let mut totals = std::collections::BTreeMap::new();
    for field in [
        "placed_entities",
        "authored_light_events",
        "fired_instances",
        "admitted_instances",
        "uploaded_lights",
    ] {
        totals.insert(
            field,
            rows.iter()
                .filter_map(|r| r["census"]["Ok"][field].as_u64())
                .sum::<u64>(),
        );
    }
    let report = json!({"schema":"hoi4.viewer-light-census.v1", "hours":rows.len(), "admitted_hours":admitted_hours,
        "refused_hours":rows.len()-admitted_hours, "presentation_delta_f32_bits":viewer.frame_delta_seconds_f32_bits,
        "presentation_frames_per_hour":1, "content_manifest_sha256":manifest.manifest_digest_sha256,
        "totals_over_presentations":totals, "rows":rows});
    std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    assert_eq!(report["hours"], 994);
    assert!(report["rows"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["census"].get("Ok").is_some()
            || r["census"]["Err"]
                .as_str()
                .is_some_and(|s| s.contains("XMAP12") || s.contains("assets."))));
}
