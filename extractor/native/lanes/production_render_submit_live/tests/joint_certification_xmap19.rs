//! XMAP19 visual-lifecycle joint certification, producer half (lane GP-U16).
//!
//! The consumer admits `map._r2_weather_visual_lifecycle` by re-deriving a
//! digest over its OWN field list and comparing it to the one in the envelope
//! (`replay_core/src/world.rs::weather_visual_lifecycle_digest`). The producer
//! cannot call that function — the replay tree is not reachable from the
//! extractor, and the boundary gate forbids reaching for it — so the two
//! implementations are independent by construction, and independent
//! implementations of one digest drift.
//!
//! This test pins the exact envelope bytes the producer mints into a committed
//! fixture. `replay_core/src/world_xmap19_tests.rs` reads the replay tree's own
//! copy of the same file, SHA-256 pinned against this one, and drives it
//! through the real admission path. Either side moving a field, a byte order or
//! a preimage separator fails a gate here instead of silently costing every
//! recorded hour its weather at replay time.
//!
//! # What keeps this from being vacuous, and what it does not cover
//!
//! A fixture the producer both writes and reads proves nothing on its own. What
//! makes this pair load-bearing is that the CONSUMER authenticates it: the
//! consumer half re-derives the digest from its own struct fields and
//! additionally mutates the fixture and requires its own admission to refuse.
//! The census below is shaped for that: 305 regions so the consumer's
//! completeness check is really exercised, three different airborne codes, a
//! fading-less variant, regions whose authored positions interleave in the
//! file, and epochs at different hours and different ordinals.
//!
//! It does NOT cover where the identity NUMBERS come from. This test hands the
//! census its `file_source_ordinal`s directly, so it is blind to a wrong
//! assignment in the catalog; that assignment is certified separately, against
//! the real parse and the real resolver, by
//! `lanes/production_render_submit_live/src/weather_catalog.rs::file_ordinal_tests`. And the side that re-derives the
//! identities from the authored file and the seed words is
//! `map_viewer::production_features::xmap19::prepare_weather`, which needs the
//! authored content pipeline and is not driven from either half — the named
//! residual of this lane.
//!
//! Regenerate deliberately, never casually:
//!
//! ```text
//! IN_WRITE_XMAP19_FIXTURE=1 cargo test -p production_render_submit_live_lane --test joint_certification_xmap19
//! ```

use std::path::PathBuf;

use portable_content_manifest::{ContentLayerKind, UsedContentKey};
use portable_render_stream::{
    WeatherMembership, WeatherPosition, WeatherPositionClass, WeatherResolution,
    WeatherVisualEpochStamp,
};
use production_render_submit_live_lane::weather_visual_lifecycle::mint_weather_visual_lifecycle;
use serde_json::{json, Value};

/// Vanilla's strategic-region count. It is a FIXTURE choice, not a contract:
/// the producer publishes whatever the engine's region vector holds. It is 305
/// here only because that is the number the consumer pins on this build, and a
/// fixture with any other count could not be admitted at all.
const REGION_ROWS: u32 = 305;

/// A stored engine hour inside the calendar window (1936-01-01 00:00).
const RAW_1936: i32 = compact_contract::calendar::DATE_EPOCH_CONST + 1936 * 365 * 24;

fn frame_hour() -> i64 {
    compact_contract::calendar::hour_index_from_raw(RAW_1936 + 4_000)
        .expect("the fixture frame hour is inside the calendar window")
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("xmap19_weather_visual_lifecycle_joint_certification.json")
}

/// The airborne code a fixture region carries. Codes 2, 4 and 7 are three of
/// the five the engine's flag priority can produce, chosen so the consumer's
/// code-to-weather-key table is exercised on more than one row.
fn code_for(region_id: u32) -> u8 {
    match region_id % 7 {
        1 => 2,
        3 => 4,
        5 => 7,
        _ => 0,
    }
}

/// Authored `map/weatherpositions.txt` places for one region, as
/// (one-based file ordinal, source line) pairs.
///
/// Three properties are deliberate.
///
/// * Region 0 has none. The catalog's parser refuses a region-zero row
///   (`lanes/production_render_submit_live/src/weather_catalog.rs::parse_weather_positions`), so a file that gave
///   engine region 0 an emitter could never be admitted at all.
/// * File ordinal 1 belongs to NO region in this census: it stands for an
///   emitter whose authored region has no engine record, which a modded file
///   can carry. It occupies its place in the file the consumer enumerates while
///   appearing in no membership row, so every real ordinal here is one higher
///   than the census alone could reconstruct. This is the case that separates
///   the ordinal carried from the parse from any ranking of retained rows.
/// * Regions INTERLEAVE: region `r`'s first place comes before region `r + 1`'s
///   and its second comes after all of them, so a per-region ordinal would
///   collide across regions.
fn source_places(region_id: u32) -> Vec<(u32, u32)> {
    if region_id == 0 {
        return Vec::new();
    }
    vec![
        (region_id + 1, region_id * 2 + 1),
        (REGION_ROWS + region_id, region_id * 2 + 2 + REGION_ROWS * 2),
    ]
}

fn content_key() -> UsedContentKey {
    UsedContentKey {
        effective_vfs_manifest_sha256: "1".repeat(64),
        normalized_game_relative_path: "map/weatherpositions.txt".into(),
        winning_layer_ordinal: 0,
        layer_kind: ContentLayerKind::Base,
        layer_id: "base".into(),
        source_sha256: "2".repeat(64),
        payload_sha256: "3".repeat(64),
        payload_bytes: 4_096,
    }
}

fn position(
    ordinal: u16,
    file_source_ordinal: u32,
    source_line: u32,
    instantiated: bool,
    fading: bool,
) -> WeatherPosition {
    WeatherPosition {
        position_ordinal: ordinal,
        file_source_ordinal,
        source_line,
        position_xyz_f32_bits: [0x4248_0000, 0, 0x42c8_0000],
        class: if ordinal == 0 {
            WeatherPositionClass::Big
        } else {
            WeatherPositionClass::Small
        },
        resolution: if instantiated {
            WeatherResolution::Instantiated
        } else {
            WeatherResolution::ExplicitEmpty
        },
        selected_authored_effect_index: instantiated.then_some(0),
        primary_entity_instance_id: instantiated.then(|| (900 + u64::from(ordinal)).to_string()),
        fading_entity_instance_id: fading.then(|| (1_900 + u64::from(ordinal)).to_string()),
        ordered_effect_instance_ids: Vec::new(),
        resolved_variant_token: instantiated.then(|| "variant".to_string()),
    }
}

fn census() -> Vec<WeatherMembership> {
    (0..REGION_ROWS)
        .map(|region_id| {
            let code = code_for(region_id);
            let places = source_places(region_id);
            // Only the first authored position of an active region instantiates
            // here; the second stays explicitly empty, which is the shape the
            // engine produces when a position's candidate vector is empty.
            // Sandstorm regions carry no fading entity, so the consumer sees
            // both identity shapes.
            let positions = places
                .iter()
                .enumerate()
                .map(|(ordinal, (file_source_ordinal, source_line))| {
                    let instantiated = ordinal == 0 && code != 0;
                    position(
                        ordinal as u16,
                        *file_source_ordinal,
                        *source_line,
                        instantiated,
                        instantiated && code != 7,
                    )
                })
                .collect::<Vec<_>>();
            let mut membership = WeatherMembership {
                sequence: u64::from(region_id) + 1_000,
                update_index: 12,
                strategic_region_id: region_id,
                rain_light_active: code == 2,
                rain_heavy_active: false,
                snow_active: code == 4,
                blizzard_active: false,
                sandstorm_active: code == 7,
                arctic_water_active: false,
                region_value_i64_fixed_1e5: i64::from(region_id) * 100,
                transition_expiry_game_hour_i32: RAW_1936 + 4_100 + region_id as i32,
                airborne_runtime_code: code,
                selection_seed_words: (code != 0).then(|| [region_id * 7 + 1, region_id * 13 + 2]),
                visual_epoch: Some(WeatherVisualEpochStamp {
                    // Different ordinals and different hours per region: a
                    // producer that folded either into a constant would still
                    // round-trip, and would still be refused by the consumer's
                    // identity re-derivation.
                    ordinal: u64::from(region_id % 5) + 1,
                    game_hour_raw: RAW_1936 + 3_000 + (region_id as i32 % 11),
                }),
                weather_effect_token: match code {
                    2 => Some("rain_light".to_string()),
                    4 => Some("snow".to_string()),
                    7 => Some("sandstorm".to_string()),
                    _ => None,
                },
                weather_position_source_content: content_key(),
                ordered_variant_content: Vec::new(),
                ordered_positions: positions,
                ordered_weather_instances: Vec::new(),
                visibility_updates: Vec::new(),
                membership_sha256: String::new(),
            };
            membership.membership_sha256 = membership
                .computed_membership_sha256()
                .expect("membership seals");
            membership
        })
        .collect()
}

fn produced_bundle() -> Value {
    let envelope = mint_weather_visual_lifecycle(&census(), frame_hour())
        .expect("the fixture census is complete and every epoch is witnessed");
    json!({
        "_comment": "Lane IN (GP-U16) XMAP19 joint-certification fixture. Producer half: extractor/native/lanes/production_render_submit_live/tests/joint_certification_xmap19.rs. Consumer half: replay_interface/native/replay_core/src/world_xmap19_tests.rs. Regenerate with IN_WRITE_XMAP19_FIXTURE=1.",
        "frame_hour": frame_hour(),
        "_r2_weather_visual_lifecycle": envelope,
    })
}

/// kills: any drift between this producer's envelope and the consumer's
/// admission — a renamed or added root field, a row field the consumer does not
/// know, a digest preimage whose separators or integer widths differ, or a
/// producer identity numbered per region instead of per file.
#[test]
fn the_producer_emits_the_committed_joint_certification_bytes() {
    let rendered = serde_json::to_string_pretty(&produced_bundle()).expect("bundle renders") + "\n";
    let path = fixture_path();
    if std::env::var("IN_WRITE_XMAP19_FIXTURE").as_deref() == Ok("1") {
        std::fs::create_dir_all(path.parent().expect("fixtures dir")).expect("create fixtures dir");
        std::fs::write(&path, &rendered).expect("write fixture");
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "the XMAP19 joint-certification fixture {} is missing ({error}); regenerate it with \
             IN_WRITE_XMAP19_FIXTURE=1",
            path.display()
        )
    });
    // Byte for byte. Both copies are pinned `text eol=lf` in `.gitattributes`,
    // which is what keeps this from failing on line endings under Windows
    // `core.autocrlf`; stripping carriage returns instead would have made this
    // a normalized comparison, and the consumer's SHA-256 a normalized pin,
    // which is not the byte pin `replay_fixture_gen`'s EXTERNAL_FIXTURES list
    // requires of anything it stops owning.
    assert_eq!(
        committed, rendered,
        "the producer's envelope bytes drifted from the committed joint-certification fixture"
    );
}

/// kills: a census whose regions all share one epoch identity, which would let
/// the producer pass the fixture comparison while emitting an envelope no
/// consumer could join to its own re-derived producer census.
#[test]
fn the_fixture_exercises_more_than_one_epoch_identity_and_both_identity_shapes() {
    let bundle = produced_bundle();
    let epochs = bundle["_r2_weather_visual_lifecycle"]["epochs"]
        .as_object()
        .expect("epochs object");
    assert_eq!(epochs.len(), REGION_ROWS as usize);
    let identities = epochs
        .values()
        .flat_map(|row| {
            row["producer_instance_ids"]
                .as_array()
                .expect("identity list")
                .iter()
                .map(|value| value.as_str().expect("identity string").to_string())
        })
        .collect::<Vec<_>>();
    assert!(
        identities.iter().any(|identity| identity.ends_with(":fading")),
        "a variant with a fading entity must be present"
    );
    assert!(
        identities.iter().any(|identity| identity.contains(":c7:")),
        "the fading-less sandstorm shape must be present"
    );
    let ordinals = epochs
        .values()
        .map(|row| row["epoch_ordinal"].as_u64().expect("ordinal"))
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        ordinals.len() > 1,
        "the fixture must carry more than one epoch ordinal"
    );
    // Region 1's instantiated position is the SECOND emitter in the authored
    // file (the first belongs to a region the engine has no record for), so the
    // consumer expects `p2`. A per-region ordinal would have said `p1` for every
    // region, and a ranking of the retained rows would have said `p1` here too.
    assert!(
        identities.contains(&"xmap19.weather:r1:e2:c2:p2:primary".to_string()),
        "producer identities must be numbered by global file order: {:?}",
        &identities[..identities.len().min(4)]
    );
}
