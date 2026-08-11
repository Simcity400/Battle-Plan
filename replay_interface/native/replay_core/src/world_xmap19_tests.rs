use super::*;
use serde_json::json;

fn lifecycle_rest(frame_hour: i64, corrupt_digest: bool) -> Map<String, Value> {
    let mut rows = BTreeMap::new();
    let mut epochs = Map::new();
    for region_id in 0..305u32 {
        let (code, seed, producer_instance_ids) = if region_id == 7 {
            (
                3u8,
                Some([0x1122_3344, 0xaabb_ccdd]),
                vec![
                    "xmap19.weather:r7:e9:c3:p41:primary".to_string(),
                    "xmap19.weather:r7:e9:c3:p41:fading".to_string(),
                ],
            )
        } else {
            (0u8, None, Vec::new())
        };
        rows.insert(
            region_id,
            WeatherVisualEpoch {
                strategic_region_id: region_id,
                airborne_runtime_code: code,
                transition_expiry_game_hour: 900,
                epoch_game_hour: frame_hour - 2,
                epoch_ordinal: 9,
                selection_seed_words: seed,
                producer_instance_ids: producer_instance_ids.clone(),
            },
        );
        epochs.insert(
            region_id.to_string(),
            json!({
                "strategic_region_id": region_id,
                "airborne_runtime_code": code,
                "transition_expiry_game_hour": 900,
                "epoch_game_hour": frame_hour - 2,
                "epoch_ordinal": 9,
                "selection_seed_words": seed,
                "producer_instance_ids": producer_instance_ids,
            }),
        );
    }
    let source_sequence = 0;
    let digest = if corrupt_digest {
        "00".repeat(32)
    } else {
        weather_visual_lifecycle_digest(frame_hour, source_sequence, &rows)
    };
    json!({
        "map": {
            "_r2_weather_visual_lifecycle": {
                "schema": WEATHER_VISUAL_LIFECYCLE_SCHEMA,
                "schema_version": 1,
                "executable_sha256": WEATHER_VISUAL_LIFECYCLE_EXECUTABLE_SHA256,
                "producer_rva": WEATHER_VISUAL_LIFECYCLE_PRODUCER_RVA,
                "cadence": WEATHER_VISUAL_LIFECYCLE_CADENCE,
                "presentation_independent": true,
                "selected_present_authority": false,
                "complete_region_count": 305,
                "frame_hour": frame_hour,
                "source_sequence": source_sequence,
                "payload_sha256": digest,
                "epochs": epochs,
            }
        }
    })
    .as_object()
    .expect("lifecycle root object")
    .clone()
}

#[test]
fn admits_complete_cold_seek_snapshot() {
    let mut warnings = Vec::new();
    let view = build_weather_visual_lifecycle(&lifecycle_rest(100, false), 100, &mut warnings);
    assert_eq!(view.status, ViewStatus::Present, "{warnings:?}");
    assert_eq!(view.source_sequence, Some(0));
    assert_eq!(view.by_region.len(), 305);
    assert_eq!(
        view.by_region
            .get(&7)
            .and_then(|row| row.selection_seed_words),
        Some([0x1122_3344, 0xaabb_ccdd])
    );
    assert_eq!(
        view.by_region
            .get(&7)
            .expect("region seven")
            .producer_instance_ids,
        vec![
            "xmap19.weather:r7:e9:c3:p41:primary".to_string(),
            "xmap19.weather:r7:e9:c3:p41:fading".to_string(),
        ]
    );
}

#[test]
fn rejects_digest_drift() {
    let mut warnings = Vec::new();
    let view = build_weather_visual_lifecycle(&lifecycle_rest(100, true), 100, &mut warnings);
    assert_eq!(view.status, ViewStatus::DomainUnavailable);
    assert!(view.by_region.is_empty());
    assert!(warnings
        .iter()
        .any(|warning| warning.contains("payload digest mismatch")));
}

#[test]
fn rejects_unknown_schema_fields_and_noncanonical_producer_ids() {
    let mut with_unknown = lifecycle_rest(100, false);
    with_unknown
        .get_mut("map")
        .and_then(Value::as_object_mut)
        .and_then(|map| map.get_mut("_r2_weather_visual_lifecycle"))
        .and_then(Value::as_object_mut)
        .expect("carrier")
        .insert("selected_draw_bytes".into(), json!("forbidden"));
    let mut warnings = Vec::new();
    let view = build_weather_visual_lifecycle(&with_unknown, 100, &mut warnings);
    assert_eq!(view.status, ViewStatus::DomainUnavailable);
    assert!(warnings
        .iter()
        .any(|warning| warning.contains("exact schema")));

    let mut malformed = lifecycle_rest(100, false);
    malformed
        .get_mut("map")
        .and_then(Value::as_object_mut)
        .and_then(|map| map.get_mut("_r2_weather_visual_lifecycle"))
        .and_then(Value::as_object_mut)
        .and_then(|carrier| carrier.get_mut("epochs"))
        .and_then(Value::as_object_mut)
        .and_then(|epochs| epochs.get_mut("7"))
        .and_then(Value::as_object_mut)
        .expect("region seven")
        .insert(
            "producer_instance_ids".into(),
            json!(["xmap19.weather:r7:e9:c3:p041:primary"]),
        );
    warnings.clear();
    let view = build_weather_visual_lifecycle(&malformed, 100, &mut warnings);
    assert_eq!(view.status, ViewStatus::DomainUnavailable);
    assert!(warnings
        .iter()
        .any(|warning| warning.contains("source ordinal invalid")));
}
