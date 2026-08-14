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

/// Joint certification, consumer half (lane IN, XMAP00 row #44 / GP-U16).
///
/// The producer half is
/// `extractor/native/lanes/production_render_submit_live/tests/joint_certification_xmap19.rs`,
/// which mints this envelope from a synthetic weather-membership census and
/// pins the exact bytes. Both sides derive the payload digest from their own
/// field lists, so a field added, renamed or retyped on either side changes
/// what this test admits, and every recorded hour would otherwise lose its
/// weather silently.
///
/// What this pair does NOT certify: the producer identities. This module admits
/// them structurally — prefix, ordering, and the clear-weather rule — and the
/// side that re-derives them from the authored `map/weatherpositions.txt` and
/// the seed words is `map_viewer::production_features::xmap19::prepare_weather`,
/// which needs the authored content pipeline and is not driven here. The
/// producer's own numbering is certified on its side of the boundary by
/// `extractor/native/lanes/production_render_submit_live/src/weather_catalog.rs::file_ordinal_tests`, which drives the real parse and
/// the real resolver. Closing that last gap is the named residual.
///
/// The fixture is evidence-dependent, so a missing file PANICS with the
/// regeneration command rather than passing quietly.
///
/// kills: any drift between the recorder's envelope and this module's
/// admission, in either direction.
#[test]
fn lane_in_joint_certification_fixture_is_admitted() {
    // The replay tree carries its OWN copy (the boundary gate forbids a
    // filesystem reach into `extractor/`). The SHA-256 pin is the drift guard
    // that replaces the shared path: regenerating the producer fixture without
    // refreshing this copy and this pin fails HERE instead of certifying stale
    // bytes.
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join("xmap19_weather_visual_lifecycle_joint_certification.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "the lane IN joint-certification fixture {} is missing ({error}); regenerate it with \
             IN_WRITE_XMAP19_FIXTURE=1 cargo test -p production_render_submit_live_lane --test \
             joint_certification_xmap19 and copy it to replay_interface/fixtures/",
            path.display()
        )
    });
    {
        use sha2::{Digest, Sha256};
        // The RAW bytes, deliberately. This pin is what
        // `replay_fixture_gen`'s EXTERNAL_FIXTURES list stands on when it stops
        // owning these bytes, so it has to be a byte pin and not a normalized
        // one: carriage returns are legal JSON whitespace, so a digest taken
        // over CR-stripped text would admit a fixture whose committed bytes had
        // changed. `.gitattributes` marks both copies `text eol=lf`, which is
        // what makes the raw form portable rather than a Windows tripwire.
        assert_eq!(
            format!("{:x}", Sha256::digest(text.as_bytes())),
            "4e98dab4ebf3537bd36ccc9bc101b8675f58c90e6cf567c3339de4c77591f8f2",
            "the replay-side fixture copy has drifted from the producer half's committed bytes; \
             refresh replay_interface/fixtures/ and this pin together"
        );
    }
    let bundle: Value = serde_json::from_str(&text).expect("the fixture is JSON");
    let object = bundle.as_object().expect("the fixture is an object");
    let frame_hour = object["frame_hour"].as_i64().expect("frame_hour");
    let rest = Map::from_iter([(
        "map".to_string(),
        json!({
            "_r2_weather_visual_lifecycle": object["_r2_weather_visual_lifecycle"].clone(),
        }),
    )]);

    let mut warnings = Vec::new();
    let view = build_weather_visual_lifecycle(&rest, frame_hour, &mut warnings);
    assert_eq!(
        view.status,
        ViewStatus::Present,
        "the recorder's envelope must be admitted as it is written: {warnings:?}"
    );
    assert_eq!(view.by_region.len(), 305);
    let region_one = view.by_region.get(&1).expect("region one");
    assert_eq!(region_one.airborne_runtime_code, 2);
    // Global file order, one-based: region 1's instantiated position is the
    // second emitter in the whole authored file (the first belongs to a region
    // the engine has no record for). A per-region ordinal would have produced
    // `p1`, and the producer would have hashed its own mistake — which is why
    // the expected identity is written out here rather than read back.
    assert_eq!(
        region_one.producer_instance_ids,
        vec![
            "xmap19.weather:r1:e2:c2:p2:primary".to_string(),
            "xmap19.weather:r1:e2:c2:p2:fading".to_string(),
        ]
    );
    // Sandstorm regions instantiate a primary with no fading entity.
    let region_five = view.by_region.get(&5).expect("region five");
    assert_eq!(region_five.airborne_runtime_code, 7);
    assert_eq!(
        region_five.producer_instance_ids,
        vec!["xmap19.weather:r5:e1:c7:p6:primary".to_string()]
    );
    assert!(view
        .by_region
        .values()
        .all(|row| row.epoch_game_hour <= frame_hour));

    // The consumer must not pass by agreeing with the producer: one mutated
    // value inside the fixture's own payload has to be refused by THIS module's
    // authentication, not by the producer's.
    let mut mutated = rest.clone();
    mutated
        .get_mut("map")
        .and_then(Value::as_object_mut)
        .and_then(|map| map.get_mut("_r2_weather_visual_lifecycle"))
        .and_then(Value::as_object_mut)
        .and_then(|carrier| carrier.get_mut("epochs"))
        .and_then(Value::as_object_mut)
        .and_then(|epochs| epochs.get_mut("1"))
        .and_then(Value::as_object_mut)
        .expect("region one")
        .insert("epoch_ordinal".into(), json!(3));
    warnings.clear();
    assert_eq!(
        build_weather_visual_lifecycle(&mutated, frame_hour, &mut warnings).status,
        ViewStatus::DomainUnavailable,
        "a mutated epoch must fail the consumer's own digest"
    );
}
