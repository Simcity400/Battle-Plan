use std::collections::BTreeMap;

use portable_render_stream::{
    WeatherMembership, WeatherPosition, WeatherResolution, A729_EXECUTABLE_SHA256,
};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

pub const WEATHER_VISUAL_LIFECYCLE_SCHEMA: &str = "hoi4.weather-visual-lifecycle.a729.v1";
pub const WEATHER_VISUAL_LIFECYCLE_SCHEMA_VERSION: u64 = 1;
pub const WEATHER_VISUAL_LIFECYCLE_PRODUCER_RVA: &str = "0x0166ec50";
pub const WEATHER_VISUAL_LIFECYCLE_CADENCE: &str = "initial_baseline_and_airborne_code_change";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WeatherVisualLifecycleRefusal {
    UnwitnessedEpoch {
        unwitnessed_regions: usize,
        first_unwitnessed_region: u32,
    },
    Integrity(String),
}

impl std::fmt::Display for WeatherVisualLifecycleRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnwitnessedEpoch {
                unwitnessed_regions,
                first_unwitnessed_region,
            } => write!(
                formatter,
                "{unwitnessed_regions} strategic region(s), lowest {first_unwitnessed_region}, \
                 carry a visual epoch this recorder never witnessed being instantiated; the \
                 epoch hour and ordinal are unknowable for them"
            ),
            Self::Integrity(reason) => write!(formatter, "{reason}"),
        }
    }
}

fn integrity<T>(reason: impl Into<String>) -> Result<T, WeatherVisualLifecycleRefusal> {
    Err(WeatherVisualLifecycleRefusal::Integrity(reason.into()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EpochRow {
    strategic_region_id: u32,
    airborne_runtime_code: u8,
    transition_expiry_game_hour: i32,
    epoch_game_hour: i64,
    epoch_ordinal: u64,
    selection_seed_words: Option<[u32; 2]>,
    producer_instance_ids: Vec<String>,
}

pub fn mint_weather_visual_lifecycle(
    census: &[WeatherMembership],
    frame_hour: i64,
) -> Result<Value, WeatherVisualLifecycleRefusal> {
    if census.is_empty() {
        return integrity("weather visual lifecycle census is empty");
    }
    for (index, membership) in census.iter().enumerate() {
        let expected = u32::try_from(index)
            .map_err(|_| WeatherVisualLifecycleRefusal::Integrity("census exceeds u32".into()))?;
        if membership.strategic_region_id != expected {
            return integrity(format!(
                "weather census region {} is out of order or missing at index {index}",
                membership.strategic_region_id
            ));
        }
    }
    let complete_region_count = census.len() as u64;
    let source_sequence = census
        .iter()
        .map(|membership| membership.sequence)
        .max()
        .unwrap_or(0);
    if source_sequence == 0 {
        return integrity("weather census carries no non-zero source sequence");
    }

    let mut unwitnessed_regions = 0usize;
    let mut first_unwitnessed_region = None;
    let mut rows = BTreeMap::new();
    for membership in census {
        let region_id = membership.strategic_region_id;
        if !matches!(membership.airborne_runtime_code, 0 | 2 | 3 | 4 | 5 | 7) {
            return integrity(format!(
                "weather census region {region_id} carries airborne code {}",
                membership.airborne_runtime_code
            ));
        }
        if (membership.airborne_runtime_code == 0) != membership.selection_seed_words.is_none() {
            return integrity(format!(
                "weather census region {region_id} pairs its seed words with the wrong code"
            ));
        }
        let Some(epoch) = membership.visual_epoch else {
            unwitnessed_regions += 1;
            first_unwitnessed_region.get_or_insert(region_id);
            continue;
        };
        if unwitnessed_regions != 0 {
            continue;
        }
        let Some(epoch_game_hour) =
            compact_contract::calendar::hour_index_from_raw(epoch.game_hour_raw)
        else {
            return integrity(format!(
                "weather census region {region_id} stamps epoch hour {} outside the calendar",
                epoch.game_hour_raw
            ));
        };
        if epoch_game_hour > frame_hour {
            return integrity(format!(
                "weather census region {region_id} dates its epoch at hour {epoch_game_hour}, \
                 after the frame hour {frame_hour}"
            ));
        }
        let producer_instance_ids = producer_instance_ids(
            region_id,
            epoch.ordinal,
            membership.airborne_runtime_code,
            &membership.ordered_positions,
        )?;
        rows.insert(
            region_id,
            EpochRow {
                strategic_region_id: region_id,
                airborne_runtime_code: membership.airborne_runtime_code,
                transition_expiry_game_hour: membership.transition_expiry_game_hour_i32,
                epoch_game_hour,
                epoch_ordinal: epoch.ordinal,
                selection_seed_words: membership.selection_seed_words,
                producer_instance_ids,
            },
        );
    }
    if let Some(first_unwitnessed_region) = first_unwitnessed_region {
        return Err(WeatherVisualLifecycleRefusal::UnwitnessedEpoch {
            unwitnessed_regions,
            first_unwitnessed_region,
        });
    }

    let payload_sha256 = payload_digest(frame_hour, source_sequence, &rows);
    let mut epochs = Map::new();
    for (region_id, row) in &rows {
        epochs.insert(
            region_id.to_string(),
            json!({
                "strategic_region_id": row.strategic_region_id,
                "airborne_runtime_code": row.airborne_runtime_code,
                "transition_expiry_game_hour": row.transition_expiry_game_hour,
                "epoch_game_hour": row.epoch_game_hour,
                "epoch_ordinal": row.epoch_ordinal,
                "selection_seed_words": row.selection_seed_words,
                "producer_instance_ids": row.producer_instance_ids,
            }),
        );
    }
    Ok(json!({
        "schema": WEATHER_VISUAL_LIFECYCLE_SCHEMA,
        "schema_version": WEATHER_VISUAL_LIFECYCLE_SCHEMA_VERSION,
        "executable_sha256": A729_EXECUTABLE_SHA256,
        "producer_rva": WEATHER_VISUAL_LIFECYCLE_PRODUCER_RVA,
        "cadence": WEATHER_VISUAL_LIFECYCLE_CADENCE,
        "presentation_independent": true,
        "selected_present_authority": false,
        "complete_region_count": complete_region_count,
        "frame_hour": frame_hour,
        "source_sequence": source_sequence,
        "payload_sha256": payload_sha256,
        "epochs": Value::Object(epochs),
    }))
}

fn producer_instance_ids(
    region_id: u32,
    epoch_ordinal: u64,
    airborne_runtime_code: u8,
    positions: &[WeatherPosition],
) -> Result<Vec<String>, WeatherVisualLifecycleRefusal> {
    let mut identities = Vec::new();
    let mut prior_position_ordinal = None;
    let mut prior_file_ordinal = None;
    for position in positions {
        if prior_position_ordinal.is_some_and(|prior| position.position_ordinal <= prior) {
            return integrity(format!(
                "region {region_id} authored positions are unordered or repeated"
            ));
        }
        prior_position_ordinal = Some(position.position_ordinal);
        if position.file_source_ordinal == 0
            || prior_file_ordinal.is_some_and(|prior| position.file_source_ordinal <= prior)
        {
            return integrity(format!(
                "region {region_id} authored file ordinals are zero, unordered or repeated"
            ));
        }
        prior_file_ordinal = Some(position.file_source_ordinal);
        let instantiated = position.resolution == WeatherResolution::Instantiated;
        if instantiated != position.primary_entity_instance_id.is_some() {
            return integrity(format!(
                "region {region_id} position {} disagrees with its own resolution",
                position.position_ordinal
            ));
        }
        if !instantiated {
            if position.fading_entity_instance_id.is_some() {
                return integrity(format!(
                    "region {region_id} position {} fades without a primary",
                    position.position_ordinal
                ));
            }
            continue;
        }
        if airborne_runtime_code == 0 {
            return integrity(format!(
                "region {region_id} instantiates a producer under clear weather"
            ));
        }
        identities.push(producer_instance_id(
            region_id,
            epoch_ordinal,
            airborne_runtime_code,
            position.file_source_ordinal,
            "primary",
        ));
        if position.fading_entity_instance_id.is_some() {
            identities.push(producer_instance_id(
                region_id,
                epoch_ordinal,
                airborne_runtime_code,
                position.file_source_ordinal,
                "fading",
            ));
        }
    }
    Ok(identities)
}

fn producer_instance_id(
    region_id: u32,
    epoch_ordinal: u64,
    airborne_runtime_code: u8,
    source_ordinal: u32,
    role: &str,
) -> String {
    format!(
        "xmap19.weather:r{region_id}:e{epoch_ordinal}:c{airborne_runtime_code}:p{source_ordinal}:{role}"
    )
}

fn payload_digest(frame_hour: i64, source_sequence: u64, rows: &BTreeMap<u32, EpochRow>) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(WEATHER_VISUAL_LIFECYCLE_SCHEMA.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(A729_EXECUTABLE_SHA256.as_bytes());
    bytes.extend_from_slice(WEATHER_VISUAL_LIFECYCLE_PRODUCER_RVA.as_bytes());
    bytes.extend_from_slice(WEATHER_VISUAL_LIFECYCLE_CADENCE.as_bytes());
    bytes.extend_from_slice(&frame_hour.to_le_bytes());
    bytes.extend_from_slice(&source_sequence.to_le_bytes());
    bytes.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    for (region_id, row) in rows {
        bytes.extend_from_slice(&region_id.to_le_bytes());
        bytes.extend_from_slice(&row.strategic_region_id.to_le_bytes());
        bytes.push(row.airborne_runtime_code);
        bytes.extend_from_slice(&row.transition_expiry_game_hour.to_le_bytes());
        bytes.extend_from_slice(&row.epoch_game_hour.to_le_bytes());
        bytes.extend_from_slice(&row.epoch_ordinal.to_le_bytes());
        match row.selection_seed_words {
            Some(words) => {
                bytes.push(1);
                bytes.extend_from_slice(&words[0].to_le_bytes());
                bytes.extend_from_slice(&words[1].to_le_bytes());
            }
            None => bytes.push(0),
        }
        bytes.extend_from_slice(&(row.producer_instance_ids.len() as u32).to_le_bytes());
        for identity in &row.producer_instance_ids {
            bytes.extend_from_slice(identity.as_bytes());
            bytes.push(0);
        }
    }
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_render_stream::{UsedContentKey, WeatherPositionClass, WeatherVisualEpochStamp};

    fn census() -> Vec<WeatherMembership> {
        vec![
            membership(0, 0, None, vec![position(0, 4, 1, false, false)]),
            membership(
                1,
                2,
                Some([11, 22]),
                vec![
                    position(0, 7, 2, true, true),
                    position(1, 9, 3, false, false),
                ],
            ),
        ]
    }

    fn membership(
        region: u32,
        code: u8,
        seed: Option<[u32; 2]>,
        positions: Vec<WeatherPosition>,
    ) -> WeatherMembership {
        let mut membership = WeatherMembership {
            sequence: u64::from(region) + 40,
            update_index: 3,
            strategic_region_id: region,
            rain_light_active: code == 2,
            rain_heavy_active: false,
            snow_active: false,
            blizzard_active: false,
            sandstorm_active: false,
            arctic_water_active: false,
            region_value_i64_fixed_1e5: 0,
            transition_expiry_game_hour_i32: 4_600,
            airborne_runtime_code: code,
            selection_seed_words: seed,
            visual_epoch: Some(WeatherVisualEpochStamp {
                ordinal: 1,
                game_hour_raw: RAW_1936 + 100,
            }),
            weather_effect_token: (code == 2).then(|| "rain_light".to_string()),
            weather_position_source_content: content_key(),
            ordered_variant_content: Vec::new(),
            ordered_positions: positions,
            ordered_weather_instances: Vec::new(),
            visibility_updates: Vec::new(),
            membership_sha256: String::new(),
        };
        membership.membership_sha256 = membership.computed_membership_sha256().unwrap();
        membership
    }

    fn content_key() -> UsedContentKey {
        UsedContentKey {
            effective_vfs_manifest_sha256: "a".repeat(64),
            stored_game_relative_path: "map/weatherpositions.txt".into(),
            normalized_game_relative_path: "map/weatherpositions.txt".into(),
            winning_layer_ordinal: 0,
            layer_kind: portable_content_manifest::ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: "b".repeat(64),
            payload_sha256: "c".repeat(64),
            payload_bytes: 12,
        }
    }

    fn position(
        ordinal: u16,
        source_line: u32,
        file_source_ordinal: u32,
        primary: bool,
        fading: bool,
    ) -> WeatherPosition {
        WeatherPosition {
            position_ordinal: ordinal,
            file_source_ordinal,
            source_line,
            position_xyz_f32_bits: [0, 0, 0],
            class: WeatherPositionClass::Small,
            resolution: if primary {
                WeatherResolution::Instantiated
            } else {
                WeatherResolution::ExplicitEmpty
            },
            selected_authored_effect_index: primary.then_some(0),
            primary_entity_instance_id: primary.then(|| "5".to_string()),
            fading_entity_instance_id: fading.then(|| "6".to_string()),
            ordered_effect_instance_ids: Vec::new(),
            resolved_variant_token: primary.then(|| "rain_light_small".to_string()),
        }
    }

    const RAW_1936: i32 = compact_contract::calendar::DATE_EPOCH_CONST + 1936 * 365 * 24;

    fn hour_index(raw: i32) -> i64 {
        compact_contract::calendar::hour_index_from_raw(raw).unwrap()
    }

    #[test]
    fn a_complete_witnessed_census_mints_the_consumers_exact_root() {
        let frame_hour = hour_index(RAW_1936 + 400);
        let value = mint_weather_visual_lifecycle(&census(), frame_hour).unwrap();
        let root = value.as_object().unwrap();
        let mut names = root.keys().cloned().collect::<Vec<_>>();
        names.sort();
        assert_eq!(
            names,
            vec![
                "cadence",
                "complete_region_count",
                "epochs",
                "executable_sha256",
                "frame_hour",
                "payload_sha256",
                "presentation_independent",
                "producer_rva",
                "schema",
                "schema_version",
                "selected_present_authority",
                "source_sequence",
            ],
            "the consumer compares the root field set for equality, not containment"
        );
        assert_eq!(root["complete_region_count"], json!(2));
        assert_eq!(root["frame_hour"], json!(frame_hour));
        assert_eq!(root["source_sequence"], json!(41));
        let epochs = root["epochs"].as_object().unwrap();
        assert_eq!(epochs.len(), 2);
        let mut row_names = epochs["1"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        row_names.sort();
        assert_eq!(
            row_names,
            vec![
                "airborne_runtime_code",
                "epoch_game_hour",
                "epoch_ordinal",
                "producer_instance_ids",
                "selection_seed_words",
                "strategic_region_id",
                "transition_expiry_game_hour",
            ]
        );
        assert_eq!(epochs["0"]["selection_seed_words"], Value::Null);
        assert_eq!(epochs["0"]["producer_instance_ids"], json!([]));
        assert_eq!(
            epochs["1"]["epoch_game_hour"],
            json!(hour_index(RAW_1936 + 100))
        );
        assert_eq!(
            epochs["1"]["producer_instance_ids"],
            json!([
                "xmap19.weather:r1:e1:c2:p2:primary",
                "xmap19.weather:r1:e1:c2:p2:fading",
            ]),
            "producer source ordinals are global file order, one-based"
        );
    }

    #[test]
    fn an_unwitnessed_epoch_refuses_the_whole_hour_and_counts_the_regions() {
        let mut census = census();
        census[0].visual_epoch = None;
        census[1].visual_epoch = None;
        assert_eq!(
            mint_weather_visual_lifecycle(&census, hour_index(RAW_1936 + 400)),
            Err(WeatherVisualLifecycleRefusal::UnwitnessedEpoch {
                unwitnessed_regions: 2,
                first_unwitnessed_region: 0,
            })
        );
    }

    #[test]
    fn an_incomplete_or_reordered_census_is_an_integrity_refusal() {
        let mut short = census();
        short.remove(0);
        assert!(matches!(
            mint_weather_visual_lifecycle(&short, hour_index(RAW_1936 + 400)),
            Err(WeatherVisualLifecycleRefusal::Integrity(_))
        ));
        let mut reordered = census();
        reordered.swap(0, 1);
        assert!(matches!(
            mint_weather_visual_lifecycle(&reordered, hour_index(RAW_1936 + 400)),
            Err(WeatherVisualLifecycleRefusal::Integrity(_))
        ));
        assert!(matches!(
            mint_weather_visual_lifecycle(&[], hour_index(RAW_1936 + 400)),
            Err(WeatherVisualLifecycleRefusal::Integrity(_))
        ));
    }

    #[test]
    fn an_epoch_after_the_frame_hour_or_off_calendar_is_refused() {
        let mut future = census();
        future[1].visual_epoch = Some(WeatherVisualEpochStamp {
            ordinal: 1,
            game_hour_raw: RAW_1936 + 600,
        });
        assert!(matches!(
            mint_weather_visual_lifecycle(&future, hour_index(RAW_1936 + 400)),
            Err(WeatherVisualLifecycleRefusal::Integrity(_))
        ));
        let mut off_calendar = census();
        off_calendar[1].visual_epoch = Some(WeatherVisualEpochStamp {
            ordinal: 1,
            game_hour_raw: 12,
        });
        assert!(matches!(
            mint_weather_visual_lifecycle(&off_calendar, hour_index(RAW_1936 + 400)),
            Err(WeatherVisualLifecycleRefusal::Integrity(_))
        ));
    }

    #[test]
    fn seed_pairing_and_clear_weather_producers_are_refused() {
        let mut seeded_clear = census();
        seeded_clear[0].selection_seed_words = Some([1, 2]);
        assert!(matches!(
            mint_weather_visual_lifecycle(&seeded_clear, hour_index(RAW_1936 + 400)),
            Err(WeatherVisualLifecycleRefusal::Integrity(_))
        ));
        let mut clear_producer = census();
        clear_producer[0].ordered_positions = vec![position(0, 4, 1, true, false)];
        assert!(matches!(
            mint_weather_visual_lifecycle(&clear_producer, hour_index(RAW_1936 + 400)),
            Err(WeatherVisualLifecycleRefusal::Integrity(_))
        ));
    }

    #[test]
    fn a_zero_or_rewound_file_ordinal_is_refused() {
        let mut zeroed = census();
        zeroed[1].ordered_positions[0].file_source_ordinal = 0;
        assert!(matches!(
            mint_weather_visual_lifecycle(&zeroed, hour_index(RAW_1936 + 400)),
            Err(WeatherVisualLifecycleRefusal::Integrity(_))
        ));
        let mut rewound = census();
        rewound[1].ordered_positions[1].file_source_ordinal = 1;
        assert!(matches!(
            mint_weather_visual_lifecycle(&rewound, hour_index(RAW_1936 + 400)),
            Err(WeatherVisualLifecycleRefusal::Integrity(_))
        ));
    }

    #[test]
    fn every_hashed_field_moves_the_payload_digest() {
        let digest = |census: &[WeatherMembership], hour: i64| {
            mint_weather_visual_lifecycle(census, hour).unwrap()["payload_sha256"]
                .as_str()
                .unwrap()
                .to_string()
        };
        let base = digest(&census(), hour_index(RAW_1936 + 400));
        assert_ne!(
            base,
            digest(&census(), hour_index(RAW_1936 + 401)),
            "frame hour"
        );

        let mut sequence = census();
        sequence[1].sequence += 1;
        assert_ne!(
            base,
            digest(&sequence, hour_index(RAW_1936 + 400)),
            "source sequence"
        );

        let mut wider = census();
        wider.push(membership(
            2,
            0,
            None,
            vec![position(0, 11, 4, false, false)],
        ));
        assert_ne!(
            base,
            digest(&wider, hour_index(RAW_1936 + 400)),
            "row count"
        );

        let mut recoded = census();
        recoded[1].airborne_runtime_code = 4;
        recoded[1].rain_light_active = false;
        recoded[1].snow_active = true;
        recoded[1].weather_effect_token = Some("snow".to_string());
        assert_ne!(
            base,
            digest(&recoded, hour_index(RAW_1936 + 400)),
            "airborne code"
        );

        let mut expiry = census();
        expiry[1].transition_expiry_game_hour_i32 += 1;
        assert_ne!(base, digest(&expiry, hour_index(RAW_1936 + 400)), "expiry");

        let mut epoch_hour = census();
        epoch_hour[1].visual_epoch = Some(WeatherVisualEpochStamp {
            ordinal: 1,
            game_hour_raw: RAW_1936 + 99,
        });
        assert_ne!(
            base,
            digest(&epoch_hour, hour_index(RAW_1936 + 400)),
            "epoch hour"
        );

        let mut ordinal = census();
        ordinal[1].visual_epoch = Some(WeatherVisualEpochStamp {
            ordinal: 2,
            game_hour_raw: RAW_1936 + 100,
        });
        assert_ne!(
            base,
            digest(&ordinal, hour_index(RAW_1936 + 400)),
            "epoch ordinal"
        );

        let mut seed = census();
        seed[1].selection_seed_words = Some([11, 23]);
        assert_ne!(
            base,
            digest(&seed, hour_index(RAW_1936 + 400)),
            "seed words"
        );

        let mut identities = census();
        identities[1].ordered_positions[0].file_source_ordinal = 3;
        identities[1].ordered_positions[1].file_source_ordinal = 4;
        assert_ne!(
            base,
            digest(&identities, hour_index(RAW_1936 + 400)),
            "producer identities"
        );

        let mut fewer = census();
        fewer[1].ordered_positions[0].fading_entity_instance_id = None;
        assert_ne!(
            base,
            digest(&fewer, hour_index(RAW_1936 + 400)),
            "identity count"
        );
    }
}
