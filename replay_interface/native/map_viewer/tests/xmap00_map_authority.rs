use std::collections::{BTreeMap, BTreeSet};

use map_viewer::{
    admit_map_authority_json, admit_map_authority_projection, map_authority_observed_status_label,
    map_authority_owner_proven_admission, map_authority_projection_from_world_frame,
    map_authority_requirement_metadata, refuse_missing_xmap00_authority,
    refuse_product_map_until_xmap01, require_promoted_xmap01_renderer, MapAuthorityProjection,
    MapAuthorityRequirement, MapAuthorityRequirementDisposition, MAP_AUTHORITY_INPUT_SCHEMA,
    MAP_AUTHORITY_OWNER_PROVEN_INPUTS, MAP_AUTHORITY_REQUIRED_INPUTS,
};
use replay_core::{
    ContentManifest, RawFrame, ReplaySource, SourceError, SourceGameIdentity,
    SourceProfileIdentity, ViewStatus, WorldFrame, WorldTimeline,
    PORTABLE_CONTENT_MANIFEST_SCHEMA, WEATHER_PROVINCE_SNOW_SCALE_RAW,
};
use serde_json::{json, Map, Value};

const XMAP00_LEDGER_TSV: &str = include_str!(
    "../../../re_evidence/rendering_laws/XMAP00/non_ui_visual_input_ledger_20260730.tsv"
);

fn xmap00_ledger_rows() -> Vec<BTreeMap<String, String>> {
    let mut lines = XMAP00_LEDGER_TSV.lines();
    let header: Vec<&str> = lines.next().expect("ledger header").split('\t').collect();
    lines
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            assert_eq!(fields.len(), header.len(), "ragged ledger row: {line}");
            header
                .iter()
                .zip(fields)
                .map(|(key, value)| ((*key).to_string(), value.to_string()))
                .collect()
        })
        .collect()
}

fn unresolved_requirement(input_id: &str) -> MapAuthorityRequirement {
    let metadata = map_authority_requirement_metadata(input_id).unwrap();
    assert_eq!(metadata.input_id, input_id);
    assert_eq!(metadata.payload_schema, None);
    assert_eq!(metadata.cadence, None);
    assert_eq!(metadata.identity_order, None);
    let blocker = if map_authority_owner_proven_admission(input_id).is_some() {
        format!(
            "{}; WorldFrame {} is DomainUnavailable",
            metadata.blocker,
            map_authority_observed_status_label(input_id)
        )
    } else {
        metadata.blocker.to_string()
    };
    MapAuthorityRequirement {
        input_id: input_id.to_string(),
        payload_schema: None,
        cadence: None,
        identity_order: None,
        disposition: MapAuthorityRequirementDisposition::Unavailable,
        blocker: Some(blocker),
    }
}


struct InMemorySource {
    hours: Vec<i64>,
    rest: Map<String, Value>,
    date: Option<String>,
}

impl ReplaySource for InMemorySource {
    fn game_id(&self) -> &str {
        "xmap00-authority-seam"
    }
    fn hours(&self) -> &[i64] {
        &self.hours
    }
    fn warnings(&self) -> &[String] {
        &[]
    }
    fn frame(&self, hour_index: i64) -> Result<RawFrame, SourceError> {
        Ok(RawFrame {
            hour_index,
            date: self.date.clone(),
            save_version: None,
            schema_version: None,
            rest: self.rest.clone(),
        })
    }
}

fn world_frame_with(rest: Value) -> std::sync::Arc<WorldFrame> {
    world_frame_with_date(rest, Some(MEASURED_CALENDAR_STAMP))
}

const MEASURED_CALENDAR_STAMP: &str = "1936.1.1.12";

const ENGINE_MONTH_LENGTHS: [i64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

fn schema_hour_index(date: &str) -> Option<i64> {
    let fields: Option<Vec<i64>> = date
        .split('.')
        .map(|field| field.parse::<i64>().ok())
        .collect();
    let [year, month, day, hour] = fields?[..] else {
        return None;
    };
    if !(1..=12).contains(&month) || day < 1 || !(0..=23).contains(&hour) {
        return None;
    }
    let month_index = usize::try_from(month - 1).ok()?;
    if day > ENGINE_MONTH_LENGTHS[month_index] {
        return None;
    }
    let day_of_year_index: i64 =
        ENGINE_MONTH_LENGTHS[..month_index].iter().sum::<i64>() + (day - 1);
    Some((year * 365 + day_of_year_index) * 24 + hour)
}

fn world_frame_with_date(rest: Value, date: Option<&str>) -> std::sync::Arc<WorldFrame> {
    let hour_index = date.and_then(schema_hour_index).unwrap_or(7);
    world_frame_with_date_at(rest, date, hour_index)
}

fn world_frame_with_date_at(
    rest: Value,
    date: Option<&str>,
    hour_index: i64,
) -> std::sync::Arc<WorldFrame> {
    let rest = rest.as_object().expect("snapshot object").clone();
    let timeline = WorldTimeline::open(Box::new(InMemorySource {
        hours: vec![hour_index],
        rest,
        date: date.map(str::to_string),
    }))
    .expect("timeline");
    timeline.frame(hour_index).expect("frame")
}

fn fixture_manifest() -> ContentManifest {
    let mut manifest = ContentManifest {
        schema: PORTABLE_CONTENT_MANIFEST_SCHEMA.into(),
        product_modset_sha256: "11".repeat(32),
        source_game: SourceGameIdentity {
            version: "1.19.2".into(),
            build: "a729".into(),
            executable_sha256: "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
                .into(),
            build_profile: "xmap00-authority-seam".into(),
        },
        source_profile: SourceProfileIdentity {
            profile_id: "xmap00-authority-seam".into(),
            playset_id: "vanilla".into(),
            selection_sha256: "00".repeat(32),
        },
        effective_vfs_manifest_sha256: "22".repeat(32),
        ordered_layers: Vec::new(),
        dlc_selection: Vec::new(),
        localisation_language: "l_english".into(),
        required_settings: BTreeMap::new(),
        recorder_local_settings: BTreeMap::new(),
        required_content: Vec::new(),
        manifest_digest_sha256: None,
    };
    manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256().unwrap();
    manifest.with_computed_digest().unwrap()
}

fn requirement<'a>(
    projection: &'a MapAuthorityProjection,
    input_id: &str,
) -> &'a MapAuthorityRequirement {
    projection
        .requirements
        .iter()
        .find(|row| row.input_id == input_id)
        .unwrap_or_else(|| panic!("projection omitted {input_id}"))
}


struct NamedSnapshotFixture {
    name: &'static str,
    rest: Value,
}

fn province_snow_fixture() -> Value {
    json!({
        "map": {
            "_r2_weather_live": {
                "source": "weather_live",
                "scale_raw": WEATHER_PROVINCE_SNOW_SCALE_RAW,
                "observed_province_rows": 2,
                "provinces": {
                    "0": {"snow_raw": 12_345},
                    "1": {"snow_raw": 6_789}
                }
            }
        }
    })
}

/// A complete victory-point census for the fixture hour (XMAP00 row #20).
///
/// `update_index` is derived from the same calendar stamp every fixture frame is
/// built at rather than written as a literal: `MapFurnitureFrame::validate`
/// refuses a carrier whose `update_index` is not the displayed hour, so a
/// hard-coded number would make this fixture silently stop admitting the moment
/// the shared stamp moved -- and a fixture that no longer admits proves the
/// opposite of what it is here for.
fn map_furniture_victory_point_fixture() -> Value {
    json!({
        "map_furniture": {
            "schema": "hoi4.map-furniture-state.a729.v4",
            "executable_sha256":
                "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
            "update_index": schema_hour_index(MEASURED_CALENDAR_STAMP)
                .expect("the shared calendar stamp is a valid hour"),
            "complete": true,
            "victory_points": [
                {"province_id": 42, "current_value": 5},
                {"province_id": 1337, "current_value": 10}
            ]
        }
    })
}

fn occupation_census_fixture() -> Value {
    json!({
        "_r2_occupation_census_live": {
            "source": "occupation_resistance_compliance_live",
            "observed_state_rows": 2,
            "emitted_state_rows": 2,
            "invalid_state_rows": 0,
            "published_state_rows": 2,
            "published_state_digest": occupation_state_digest(&[5, 6]).to_string(),
            "baseline": true,
            "census_complete": true
        },
        "states": {
            "5": {"occupation": {"resistance": 22.3, "compliance": 65.0}},
            "6": {"occupation": {"resistance": 1.5, "compliance": 4.0}}
        }
    })
}

fn division_census_fixture() -> Value {
    json!({
        "divisions": [
            {"div_id": 5, "div_type": 29, "owner": "GER", "location_province": 42},
            {"div_id": 6, "div_type": 29, "owner": "GER", "location_province": 43}
        ]
    })
}

fn taskforce_census_fixture() -> Value {
    json!({
        "order_of_battle": {
            "fleets": [{
                "id": 3,
                "owner": "ENG",
                "task_forces": [{"id": 71, "location_province": 118}]
            }]
        }
    })
}

/// The shape `captures/oz_tip` actually carries: one task force the engine holds
/// no province for, alongside ordinary located ones.
///
/// CHI task force 601 (fleet 2377, one destroyer, naval mission 8
/// `RESERVE_FLEET`) has no `location_province` on all 535 recorded hours. The
/// recording proves this is the engine's own absence marker rather than a failed
/// read: the run manifest's `bound_trips` ledger is empty over the whole capture,
/// so no map-province bound trip ever refused a value, and d56 reports
/// `validated_location_rows` 173 of `task_force_count` 174 -- exactly one row, on
/// a task force whose every other field decoded from the same memory window.
fn taskforce_census_fixture_with_engine_absent_location(status: &str) -> Value {
    json!({
        "order_of_battle": {
            "fleets": [{
                "id": 3,
                "owner": "ENG",
                "task_forces": [
                    {"id": 71, "location_province": 118, "location_province_status": "exact"},
                    {"id": 601, "location_province": null, "location_province_status": status}
                ]
            }]
        }
    })
}

fn air_wing_census_fixture() -> Value {
    json!({
        "order_of_battle": {
            "air_wings": [
                {"id": 12, "owner": "SOV", "air_base_province": 907, "state_id": 219}
            ]
        }
    })
}

/// kills: treating the engine's own "this task force is not deployed to a
/// province" as a gap, which is what refused `units.taskforces_fleets` on every
/// hour of every sealed recording over a single row.
#[test]
fn taskforces_fleets_admits_an_engine_absent_location_and_still_refuses_an_unreadable_one() {
    let admitted_frame =
        world_frame_with(taskforce_census_fixture_with_engine_absent_location("none"));
    assert_eq!(admitted_frame.fleets_status(), ViewStatus::Present);
    let manifest = fixture_manifest();
    let projection =
        map_authority_projection_from_world_frame(&admitted_frame, &manifest).unwrap();
    assert_eq!(
        requirement(&projection, "units.taskforces_fleets").disposition,
        MapAuthorityRequirementDisposition::Admitted,
        "a task force the engine holds no province for must not blank the family"
    );
    assert!(admit_map_authority_projection(&projection)
        .unwrap()
        .is_admitted("units.taskforces_fleets"));

    // The read that actually failed is still a gap, and still costs the census.
    let refused_frame =
        world_frame_with(taskforce_census_fixture_with_engine_absent_location("unknown"));
    assert_eq!(refused_frame.fleets_status(), ViewStatus::Present);
    assert_owner_proven_unavailable(&refused_frame, "units.taskforces_fleets");

    // And a producer that says nothing at all is not claiming absence either.
    let silent = json!({
        "order_of_battle": {
            "fleets": [{
                "id": 3,
                "owner": "ENG",
                "task_forces": [{"id": 601, "location_province": null}]
            }]
        }
    });
    let silent_frame = world_frame_with(silent);
    assert_eq!(silent_frame.fleets_status(), ViewStatus::Present);
    assert_owner_proven_unavailable(&silent_frame, "units.taskforces_fleets");
}

/// A complete all-country theatre-selector census for the fixture hour
/// (`orders.theatre_army_hierarchy`). The evidence array is replay_core's own
/// 31-manifest pin rather than a copied literal, so a producer-evidence drift
/// breaks this fixture loudly instead of admitting against a stale contract.
///
/// The census EXACTLY EQUALS the frame's complete D18 roster identities
/// (measured_countries: GER=1, ENG=2), which is the same equality the row's
/// predicate and the production renderer demand; the countries domain carried
/// here is byte-identical to the D18 fixture's so merging every owner-proven
/// fixture into one frame cannot drift the roster out from under this census.
fn theatre_hierarchy_census_fixture() -> Value {
    use replay_core::theatre_selector::{
        EXACT_A729_EXECUTABLE_SHA256, REQUIRED_PRODUCER_EVIDENCE_SHA256,
        THEATRE_SELECTOR_AUTHORITY, THEATRE_SELECTOR_SCHEMA_VERSION,
    };
    let hour = schema_hour_index(MEASURED_CALENDAR_STAMP).unwrap();
    json!({
        "countries": measured_countries(),
        "_r2_theatre_selector_live": {
            "authority": THEATRE_SELECTOR_AUTHORITY,
            "schema_version": THEATRE_SELECTOR_SCHEMA_VERSION,
            "executable_sha256": EXACT_A729_EXECUTABLE_SHA256,
            "producer_evidence_sha256": REQUIRED_PRODUCER_EVIDENCE_SHA256,
            "content_identity": {
                "language": "l_english",
                "localization_content_sha256": "a".repeat(64),
                "ordered_layers": [
                    {
                        "ordinal": 0,
                        "layer_kind": "base",
                        "layer_id": "base",
                        "content_sha256": "b".repeat(64)
                    }
                ]
            },
            "frame_hour": hour,
            "source_sequence": hour + 1,
            "producer_complete": true,
            "census": [1, 2],
            "negative_controller_dispositions": [
                "theater_group_item.combat_log_ownership_and_eligibility",
                "air_theatre_group_item.current_priority_ordinal",
                "navy_theater_group_item.combat_log_ownership_and_eligibility",
                "reserve_navy_theater_group_item.combat_log_ownership_and_eligibility",
            ],
            "sections": [
                {
                    "owner_tag_db_id": 1,
                    "admission": "admitted",
                    "theatres": [{
                        "kind": "land_theatre",
                        "selector_included": true,
                        "object_id": 4101,
                        "owner_tag_db_id": 1,
                        "engine_order": 0,
                        "display_name": "GHQ",
                        "name_provenance": {"source": "localization_key", "key": "theatre_ghq"},
                        "controller": {
                            "kind": "land",
                            "priority_ordinal": 2,
                            "delete_eligible": false
                        },
                        "land_alert_categories": [],
                        "members": []
                    }],
                    "free_air_wings": [],
                    "free_air_priority": {
                        "selected_priority_ordinal": null,
                        "priority_census_wing_ids": []
                    },
                    "strategic_regions": []
                },
                {
                    "owner_tag_db_id": 2,
                    "admission": "admitted",
                    "theatres": [],
                    "free_air_wings": [],
                    "free_air_priority": {
                        "selected_priority_ordinal": null,
                        "priority_census_wing_ids": []
                    },
                    "strategic_regions": []
                }
            ]
        }
    })
}

/// kills: an `orders.theatre_army_hierarchy` join that could never admit --
/// either by reporting Admitted on a frame whose selector payload the typed
/// decoder refused, or by refusing to seal a frame that carries an admitted
/// all-country schema-6 section set.
#[test]
fn theatre_army_hierarchy_admits_a_complete_schema6_section_set() {
    let frame = world_frame_with(theatre_hierarchy_census_fixture());
    assert_eq!(frame.theatre_selector().status, ViewStatus::Present);
    let manifest = fixture_manifest();
    let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
    assert_eq!(
        requirement(&projection, "orders.theatre_army_hierarchy").disposition,
        MapAuthorityRequirementDisposition::Admitted,
        "a complete all-country schema-6 census must admit"
    );
    assert_eq!(
        requirement(&projection, "orders.theatre_army_hierarchy")
            .payload_schema
            .as_deref(),
        Some("WorldFrame._r2_theatre_selector_live")
    );
    assert!(admit_map_authority_projection(&projection)
        .unwrap()
        .is_admitted("orders.theatre_army_hierarchy"));
}

/// Hours before the producer's first observation carry nothing for this
/// domain; that is absence, not refusal.
#[test]
fn theatre_army_hierarchy_is_absent_where_the_fold_carries_nothing() {
    let frame = world_frame_with(json!({}));
    assert_eq!(frame.theatre_selector().status, ViewStatus::DomainAbsent);
    let manifest = fixture_manifest();
    let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
    let row = requirement(&projection, "orders.theatre_army_hierarchy");
    assert_eq!(row.disposition, MapAuthorityRequirementDisposition::Absent);
    assert!(row.blocker.as_deref().unwrap_or_default().contains(
        "the recording carries no admitted all-country theatre-selector section set"
    ));
}

/// kills: admitting on a stale carrier. A payload labelled for another hour
/// must never vouch for the displayed one even if it decodes clean.
#[test]
fn theatre_army_hierarchy_refuses_a_payload_labelled_for_a_different_hour() {
    let mut rest = theatre_hierarchy_census_fixture();
    rest["_r2_theatre_selector_live"]["frame_hour"] =
        json!(schema_hour_index(MEASURED_CALENDAR_STAMP).unwrap() + 1);
    let frame = world_frame_with(rest);
    assert_eq!(frame.theatre_selector().status, ViewStatus::Present);
    assert_owner_proven_unavailable(&frame, "orders.theatre_army_hierarchy");
}

/// kills: treating a partial census as the all-country hierarchy. A country
/// whose section names a reason keeps the whole row a gap -- named per country,
/// sealed as unavailable.
#[test]
fn theatre_army_hierarchy_refuses_an_incomplete_section_set() {
    let mut rest = theatre_hierarchy_census_fixture();
    let domain = rest["_r2_theatre_selector_live"].as_object_mut().unwrap();
    domain.insert("producer_complete".into(), json!(false));
    domain["sections"] = json!([
        {
            "owner_tag_db_id": 1,
            "admission": "unavailable",
            "reason": "theatre_selector_land_combat_log_leaf_has_no_all_country_law"
        },
        {
            "owner_tag_db_id": 2,
            "admission": "unavailable",
            "reason": "theatre_selector_section_refused_by_its_producer"
        }
    ]);
    let frame = world_frame_with(rest);
    assert_eq!(frame.theatre_selector().status, ViewStatus::Present);
    assert!(!frame.theatre_selector().producer_complete);
    assert_owner_proven_unavailable(&frame, "orders.theatre_army_hierarchy");
}

/// kills: dropping the source-sequence conjunct. Only the sequence is
/// corrupted here -- the payload hour stays valid -- so removing that one
/// check alone turns this test red.
#[test]
fn theatre_army_hierarchy_refuses_a_sequence_that_is_not_hour_plus_one() {
    let mut rest = theatre_hierarchy_census_fixture();
    rest["_r2_theatre_selector_live"]["source_sequence"] =
        json!(schema_hour_index(MEASURED_CALENDAR_STAMP).unwrap() + 2);
    let frame = world_frame_with(rest);
    assert_eq!(frame.theatre_selector().status, ViewStatus::Present);
    assert_owner_proven_unavailable(&frame, "orders.theatre_army_hierarchy");
}

/// kills: any relaxation of the strict u64 hour conversion. The payload here
/// is labelled for hour zero with a lawful sequence, so a reintroduced
/// `.max(0)` would derive displayed hour zero from -1 and ADMIT this frame --
/// only the strict conversion refuses it.
#[test]
fn theatre_army_hierarchy_refuses_a_negative_displayed_hour() {
    let mut rest = theatre_hierarchy_census_fixture();
    rest["_r2_theatre_selector_live"]["frame_hour"] = json!(0);
    rest["_r2_theatre_selector_live"]["source_sequence"] = json!(1);
    let frame = world_frame_with_date_at(rest, None, -1);
    assert_owner_proven_unavailable(&frame, "orders.theatre_army_hierarchy");
}

/// The owner ruling (option A) retires the land combat-log leaf from schema 6;
/// a payload still carrying it must refuse with the decoder's named error
/// instead of silently ignoring the key.
#[test]
fn theatre_army_hierarchy_refuses_a_payload_still_carrying_the_retired_leaf() {
    let mut rest = theatre_hierarchy_census_fixture();
    rest["_r2_theatre_selector_live"]["sections"][0]["theatres"][0]["controller"]
        ["combat_log_owned"] = json!(false);
    let frame = world_frame_with(rest);
    assert_eq!(
        frame.theatre_army_hierarchy_status(),
        ViewStatus::DomainUnavailable
    );
    let manifest = fixture_manifest();
    let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
    assert_eq!(
        requirement(&projection, "orders.theatre_army_hierarchy").disposition,
        MapAuthorityRequirementDisposition::Unavailable
    );
}

/// kills: an icon derivation that reads the retired leaf back from the decoded
/// view -- the field no longer exists, so this only compiles while admission
/// and derivation stay on authoritative state (the session's measured player
/// set) rather than on a carried leaf.
#[test]
fn theatre_army_hierarchy_admits_without_any_combat_log_leaf_on_land_rows() {
    let frame = world_frame_with(theatre_hierarchy_census_fixture());
    assert_eq!(frame.theatre_army_hierarchy_status(), ViewStatus::Present);
    for theatre in frame.theatre_selector().sections() {
        for theatre in &theatre.theatres {
            assert!(matches!(
                theatre.controller,
                replay_core::TheatreControllerState::Land { .. }
                    | replay_core::TheatreControllerState::Navy { .. }
            ));
        }
    }
}

/// The carrier contract is schema 6 now; the producer's sealed output still
/// refuses every hour at the evidence-set size (4 of 31 manifests). This test
/// pins the LEGACY schema-4 shape so a pre-bump recording refuses at the
/// decoder's own named mismatch instead of crashing the projection or
/// admitting. Each corruption is isolated so removing the OTHER check cannot
/// keep this green.
#[test]
fn theatre_army_hierarchy_names_the_schema_version_mismatch_for_a_legacy_carrier() {
    let mut rest = theatre_hierarchy_census_fixture();
    rest["_r2_theatre_selector_live"]["schema_version"] = json!(4);
    let frame = world_frame_with(rest);
    assert_eq!(
        frame.theatre_selector().status,
        ViewStatus::DomainUnavailable
    );
    assert_eq!(
        frame.theatre_selector().diagnostic.as_deref(),
        Some("theatre selector schema version mismatch")
    );
    assert_owner_proven_unavailable(&frame, "orders.theatre_army_hierarchy");
}

#[test]
fn theatre_army_hierarchy_names_the_evidence_identity_mismatch_for_a_short_manifest_set() {
    let mut rest = theatre_hierarchy_census_fixture();
    rest["_r2_theatre_selector_live"]["producer_evidence_sha256"] =
        json!(Vec::<String>::new());
    let frame = world_frame_with(rest);
    assert_eq!(
        frame.theatre_selector().status,
        ViewStatus::DomainUnavailable
    );
    assert_eq!(
        frame.theatre_selector().diagnostic.as_deref(),
        Some("producer evidence identity mismatch")
    );
    assert_owner_proven_unavailable(&frame, "orders.theatre_army_hierarchy");
}

fn occupation_state_digest(ids: &[u32]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    for id in ids {
        for byte in id.to_le_bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(PRIME);
        }
    }
    hash
}

fn rest_raw_province_count(frame: &WorldFrame) -> usize {
    frame
        .domain("map")
        .and_then(Value::as_object)
        .and_then(|map| map.get("_r2_weather_live"))
        .and_then(Value::as_object)
        .and_then(|weather| weather.get("provinces"))
        .and_then(Value::as_object)
        .map(Map::len)
        .expect("fixture carries a raw province object")
}

fn assert_owner_proven_unavailable(frame: &WorldFrame, input_id: &str) {
    let manifest = fixture_manifest();
    let projection = map_authority_projection_from_world_frame(frame, &manifest).unwrap();
    let row = requirement(&projection, input_id);
    assert_eq!(
        row.disposition,
        MapAuthorityRequirementDisposition::Unavailable,
        "{input_id} admitted incomplete data"
    );
    let blocker = map_authority_requirement_metadata(input_id)
        .unwrap()
        .blocker;
    let label = map_authority_observed_status_label(input_id);
    assert_eq!(
        row.blocker.as_deref(),
        Some(format!("{blocker}; WorldFrame {label} is DomainUnavailable").as_str())
    );
    assert_eq!(row.payload_schema, None);
    assert!(!admit_map_authority_projection(&projection)
        .unwrap()
        .is_admitted(input_id));
}

fn d48_building_census_fixture(source_marker: &str) -> Value {
    json!({
        "states": {
            "1": {
                "construction_buildings_state_damage_live": {
                    "source": source_marker,
                    "state_buildings": {
                        "type_0x0000000b": {
                            "level": 3,
                            "healthy_level": 3,
                            "damaged": false,
                            "token": "arms_factory"
                        }
                    }
                }
            }
        }
    })
}

/// The exact D57 human player-country set (XMAP00 row #36).
///
/// One human per TAG, in the producer's non-decreasing
/// `(country_tag, tag_db_id, human_name)` order. ENG's name is `not_sourced`,
/// which is the engine naming nobody for a human-held slot — measured absence,
/// and admitted as such.
///
/// Co-op (two humans on one TAG) is a real engine state the producer emits and
/// this row deliberately REFUSES, because XMAP18's Players mode errors on a
/// repeated TAG. The negative case below pins that.
fn player_country_set_fixture() -> Value {
    let row = |tag: &str, tag_db_id: u32, human_name: Value| {
        json!({
            "country_tag": tag, "tag_db_id": tag_db_id,
            "is_human_controlled": true, "human_marker": true,
            "human_name": human_name,
            "player_key_hash": {"state": "not_sourced"},
            "session_slot": {"state": "not_sourced"},
            "host": {"state": "not_sourced"},
            "connected": {"state": "not_sourced"},
            "valid_from_hour_raw": 0
        })
    };
    json!({
        "session": {"localisation_language": "l_english"},
        "_r2_session_multiplayer_metadata_live": {
            "players_by_country": {
                "state": "known",
                "rows": [
                    row("ENG", 2, json!({"state": "not_sourced"})),
                    row("GER", 1, json!({"state": "known", "value": "Simcity400"}))
                ]
            }
        }
    })
}

/// kills: refusing a lawful co-op player set.
///
/// Two humans on one country, and two the engine named nobody for, are both
/// ordinary multiplayer states: the D57 producer sorts on
/// `(country_tag, tag_db_id, human_name)` and that key is explicitly not unique,
/// so `(GER, 1, None)` twice compares EQUAL and is emitted as-is. XMAP18's
/// Players mode paints set membership — whether a country has any human — so it
/// folds the repeat. A uniqueness rule at this door on the TAG, or on
/// `(TAG, name)`, would refuse a real capture; a display name is not an
/// identity, and the engine's player key is not projected into this view.
#[test]
fn a_co_op_player_set_admits_however_its_humans_are_named() {
    let manifest = fixture_manifest();
    let co_op_row = |human_name: Value| {
        json!({
            "country_tag": "GER", "tag_db_id": 1,
            "is_human_controlled": true, "human_marker": true,
            "human_name": human_name,
            "player_key_hash": {"state": "not_sourced"},
            "session_slot": {"state": "not_sourced"},
            "host": {"state": "not_sourced"},
            "connected": {"state": "not_sourced"},
            "valid_from_hour_raw": 0
        })
    };
    for (label, extra) in [
        // A second, differently named human on GER.
        ("a second named human", co_op_row(json!({"state": "known", "value": "Zeta"}))),
        // The same display name twice: two people, one chosen name.
        ("a shared display name", co_op_row(json!({"state": "known", "value": "Simcity400"}))),
    ] {
        let mut snapshot = fully_measured_snapshot();
        snapshot["_r2_session_multiplayer_metadata_live"]["players_by_country"]["rows"]
            .as_array_mut()
            .expect("the measured fixture carries a D57 row array")
            .push(extra);
        let frame = world_frame_with(snapshot);
        let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
        assert_eq!(
            requirement(&projection, "session.player_country_set").disposition,
            MapAuthorityRequirementDisposition::Admitted,
            "{label} is a lawful co-op set and must not refuse this row",
        );
    }

    // Two unnamed humans on one country: the producer's sort key repeats
    // EXACTLY, which its own stable sort permits, so the decoder's ordering
    // check must not reject it either.
    let mut snapshot = fully_measured_snapshot();
    let rows = snapshot["_r2_session_multiplayer_metadata_live"]["players_by_country"]["rows"]
        .as_array_mut()
        .expect("row array");
    rows.insert(0, co_op_row(json!({"state": "not_sourced"})));
    rows.insert(0, co_op_row(json!({"state": "not_sourced"})));
    // `(ENG, ...)` must still sort before the two `(GER, 1, None)` rows.
    rows.sort_by_key(|row| {
        (
            row["country_tag"].as_str().unwrap_or_default().to_string(),
            row["tag_db_id"].as_u64().unwrap_or_default(),
        )
    });
    let frame = world_frame_with(snapshot);
    let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
    assert_eq!(
        requirement(&projection, "session.player_country_set").disposition,
        MapAuthorityRequirementDisposition::Admitted,
        "two unnamed co-op humans key equal under the producer's own sort",
    );
}

/// kills: admitting a player set whose name is a FAILED READ rather than the
/// engine's own absence.
///
/// `{"state": "not_sourced"}` is the engine naming nobody and must admit;
/// a `known` state with no usable value is a read that did not complete and
/// must not.
#[test]
fn a_player_row_whose_name_failed_to_read_is_not_a_complete_set() {
    let manifest = fixture_manifest();
    let mut snapshot = fully_measured_snapshot();
    snapshot["_r2_session_multiplayer_metadata_live"]["players_by_country"]["rows"][1]
        ["human_name"] = json!({"state": "known"});
    let frame = world_frame_with(snapshot);
    let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
    assert_eq!(
        requirement(&projection, "session.player_country_set").disposition,
        MapAuthorityRequirementDisposition::Unavailable,
    );
}

/// kills: admitting a raid census that carries no `missing_tag_dummy`.
///
/// The producer refuses `dummy_count != 1` on a non-empty parent census
/// (`extractor/native/compact_contract/src/raid_instances.rs`), but
/// `parse_complete_raid_instances` only checks the dummy's shape IF one is
/// present. A country-first census therefore replays structurally, and without
/// this the authority row would admit a population the producer cannot emit.
#[test]
fn a_raid_census_without_the_engines_untagged_parent_is_not_complete() {
    let manifest = fixture_manifest();
    let mut snapshot = fully_measured_snapshot();
    snapshot["raids"]["country_statuses"] = json!([{
        "status_index": 0, "tag": "GER", "tag_db_id": 1,
        "classification": "country", "priority": 1,
        "first_child_ordinal": 0, "child_count": 1
    }]);
    snapshot["raids"]["instances"][0]["status_index"] = json!(0);
    let frame = world_frame_with(snapshot);
    assert_eq!(
        frame.raids_status(),
        ViewStatus::Present,
        "the census still replays structurally, which is why this test exists"
    );
    let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
    assert_eq!(
        requirement(&projection, "mapmode.raids").disposition,
        MapAuthorityRequirementDisposition::Unavailable,
    );
}

/// kills: admitting a raid census whose parent tag is an unresolved numeric id.
///
/// `world_fold` renders an unresolved country id as its decimal string, and
/// `"123".len() == 3`, so the old shape-only check admitted it. Every non-dummy
/// identity must join the resolved D18 roster.
#[test]
fn a_raid_parent_with_an_unresolved_numeric_tag_is_not_complete() {
    let manifest = fixture_manifest();
    let mut snapshot = fully_measured_snapshot();
    snapshot["raids"]["country_statuses"][1]["tag"] = json!("123");
    let frame = world_frame_with(snapshot);
    assert_eq!(
        frame.raids_status(),
        ViewStatus::Present,
        "the census still replays structurally, which is why this test exists"
    );
    let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
    assert_eq!(
        requirement(&projection, "mapmode.raids").disposition,
        MapAuthorityRequirementDisposition::Unavailable,
    );
}

/// kills: refusing the producer's valid raid-free hour.
///
/// The producer's both-empty population returns Ok before its dummy refusal
/// ever runs (`extractor/native/compact_contract/src/raid_instances.rs`), so an
/// hour with no raids and `instances_complete: true` is a MEASURED empty
/// census, not a failed claim, and it must admit. The producer derives the
/// priority list from the same parent vector, so the empty hour carries an
/// empty `countries` too.
#[test]
fn a_measured_empty_raid_census_admits() {
    let manifest = fixture_manifest();
    let mut snapshot = fully_measured_snapshot();
    snapshot["raids"]["countries"] = json!([]);
    snapshot["raids"]["country_statuses"] = json!([]);
    snapshot["raids"]["instances"] = json!([]);
    let frame = world_frame_with(snapshot);
    let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
    assert_eq!(
        requirement(&projection, "mapmode.raids").disposition,
        MapAuthorityRequirementDisposition::Admitted,
    );
}

/// kills: certifying a contradictory empty census. `world_fold` derives the
/// priority list and the parent census from the same engine vector, so empty
/// parents with a NON-empty priority list is a shape the producer cannot emit.
#[test]
fn an_empty_raid_census_with_a_nonempty_priority_list_is_not_complete() {
    let manifest = fixture_manifest();
    let mut snapshot = fully_measured_snapshot();
    snapshot["raids"]["country_statuses"] = json!([]);
    snapshot["raids"]["instances"] = json!([]);
    // `countries` keeps the fixture's GER priority row — the contradiction.
    let frame = world_frame_with(snapshot);
    let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
    assert_eq!(
        requirement(&projection, "mapmode.raids").disposition,
        MapAuthorityRequirementDisposition::Unavailable,
    );
}

/// kills: reading a MISSING or null `raids.countries` as a measured empty
/// priority list. The view's `priorities` map is a lossy projection — the
/// producer always emits the array, so its absence is an unmeasured shape,
/// not an empty measurement.
#[test]
fn an_empty_raid_census_with_a_null_priority_list_is_not_complete() {
    let manifest = fixture_manifest();
    let mut snapshot = fully_measured_snapshot();
    snapshot["raids"]["countries"] = Value::Null;
    snapshot["raids"]["country_statuses"] = json!([]);
    snapshot["raids"]["instances"] = json!([]);
    let frame = world_frame_with(snapshot);
    let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
    assert_eq!(
        requirement(&projection, "mapmode.raids").disposition,
        MapAuthorityRequirementDisposition::Unavailable,
    );
}

/// kills: admitting a non-empty parent census whose priority list disagrees.
/// Both are derived from the same engine vector, so real parents with an
/// empty `countries` is a shape the producer cannot emit.
#[test]
fn a_raid_census_whose_priority_list_disagrees_with_its_parents_is_not_complete() {
    let manifest = fixture_manifest();
    let mut snapshot = fully_measured_snapshot();
    snapshot["raids"]["countries"] = json!([]);
    let frame = world_frame_with(snapshot);
    let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
    assert_eq!(
        requirement(&projection, "mapmode.raids").disposition,
        MapAuthorityRequirementDisposition::Unavailable,
    );
}

/// kills: comparing the priority list to its parents by TAG SET alone. Both
/// come from the same producer rows, so a right-name wrong-value entry (or a
/// collapsed duplicate) is producer-impossible and must refuse.
#[test]
fn a_raid_priority_list_with_the_wrong_value_is_not_complete() {
    let manifest = fixture_manifest();
    let mut snapshot = fully_measured_snapshot();
    snapshot["raids"]["countries"] = json!([{"tag": "GER", "priority": 999}]);
    let frame = world_frame_with(snapshot);
    let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
    assert_eq!(
        requirement(&projection, "mapmode.raids").disposition,
        MapAuthorityRequirementDisposition::Unavailable,
    );
}

/// kills: letting duplicate priority rows collapse silently in the view map.
#[test]
fn a_raid_priority_list_with_duplicate_rows_is_not_complete() {
    let manifest = fixture_manifest();
    let mut snapshot = fully_measured_snapshot();
    snapshot["raids"]["countries"] =
        json!([{"tag": "GER", "priority": 1}, {"tag": "GER", "priority": 1}]);
    let frame = world_frame_with(snapshot);
    let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
    assert_eq!(
        requirement(&projection, "mapmode.raids").disposition,
        MapAuthorityRequirementDisposition::Unavailable,
    );
}

/// kills: leaving the victim and raid-source identity pairs outside the
/// roster join — `world_fold` renders unresolved ids the same way there.
#[test]
fn a_raid_victim_with_an_unresolved_identity_is_not_complete() {
    let manifest = fixture_manifest();
    let mut snapshot = fully_measured_snapshot();
    snapshot["raids"]["instances"][0]["victim_country"] = json!("123");
    snapshot["raids"]["instances"][0]["victim_country_tag_db_id"] = json!(123);
    let frame = world_frame_with(snapshot);
    let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
    assert_eq!(
        requirement(&projection, "mapmode.raids").disposition,
        MapAuthorityRequirementDisposition::Unavailable,
    );
}

/// kills: authenticating a raid identity by roster MEMBERSHIP alone. A real
/// roster tag paired with another country's db id would resolve to the wrong
/// country downstream; the join must hold on the exact `(tag, tag_db_id)`
/// pair.
#[test]
fn a_raid_parent_with_anothers_tag_db_id_is_not_complete() {
    let manifest = fixture_manifest();
    let mut snapshot = fully_measured_snapshot();
    // Parent and owner AGREE on the wrong id, so the census replays
    // structurally and only the roster identity join can refuse it.
    snapshot["raids"]["country_statuses"][1]["tag_db_id"] = json!(2);
    snapshot["raids"]["instances"][0]["owner_tag_db_id"] = json!(2);
    let frame = world_frame_with(snapshot);
    assert_eq!(
        frame.raids_status(),
        ViewStatus::Present,
        "the census still replays structurally, which is why this test exists"
    );
    let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
    assert_eq!(
        requirement(&projection, "mapmode.raids").disposition,
        MapAuthorityRequirementDisposition::Unavailable,
    );
}

/// kills: letting the LEGACY player-country keys establish this admission.
///
/// `build_session_metadata` still falls back to `session.player_countries` and
/// `world.scenario.players`, neither of which this recorder has ever emitted and
/// neither of which carries a `tag_db_id`, a human-controlled marker, or a
/// producer order. A frame using the legacy shape decodes a perfectly measured
/// leaf — and must NOT be able to certify a row whose `identity_order`
/// advertises the whole D57 contract.
#[test]
fn the_legacy_player_country_keys_cannot_certify_the_exact_d57_row() {
    let manifest = fixture_manifest();
    let mut snapshot = fully_measured_snapshot();
    snapshot
        .as_object_mut()
        .expect("snapshot object")
        .remove("_r2_session_multiplayer_metadata_live");
    snapshot["session"]["player_countries"] = json!(["GER"]);
    let frame = world_frame_with(snapshot);

    assert!(
        matches!(
            &frame.session_metadata().player_countries,
            replay_core::Leaf::Measured(rows) if rows.len() == 1
        ),
        "the legacy fallback still decodes, which is exactly why this test exists"
    );
    assert_eq!(
        map_viewer::player_country_set_leaf_status(&frame),
        ViewStatus::DomainAbsent,
        "no D57 carrier means this row has nothing of its own to certify"
    );
    let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
    assert_eq!(
        requirement(&projection, "session.player_country_set").disposition,
        MapAuthorityRequirementDisposition::Absent,
    );
}

/// kills: accepting a D57 row set that is not in the producer's order, or whose
/// rows drop the engine identity / human-controlled marker the ledger promises.
#[test]
fn a_d57_player_set_that_breaks_its_producer_contract_does_not_decode() {
    let base = fully_measured_snapshot();
    let mutate = |edit: &dyn Fn(&mut Value)| {
        let mut snapshot = base.clone();
        edit(&mut snapshot["_r2_session_multiplayer_metadata_live"]["players_by_country"]["rows"]);
        world_frame_with(snapshot)
    };
    // Producer order is (country_tag, tag_db_id, human_name) non-decreasing.
    let reordered = mutate(&|rows| {
        let rows = rows.as_array_mut().expect("row array");
        rows.reverse();
    });
    // The engine country identity is part of the promised order key.
    let no_identity = mutate(&|rows| {
        rows[0].as_object_mut().expect("row").remove("tag_db_id");
    });
    // Every row the producer emits is human-controlled.
    let not_human = mutate(&|rows| {
        rows[0]["is_human_controlled"] = json!(false);
    });
    for (label, frame) in [
        ("reordered", reordered),
        ("no tag_db_id", no_identity),
        ("not human-controlled", not_human),
    ] {
        assert_eq!(
            map_viewer::player_country_set_leaf_status(&frame),
            ViewStatus::DomainUnavailable,
            "{label} decoded as a usable player set"
        );
    }
}

/// The raid instance/status census (XMAP00 row #27).
///
/// Row 0 is the engine's own `missing_tag_dummy` — tag null, tag_db_id 0, no
/// children — which `captures/oz_tip` carries on every hour. It is measured
/// absence, not a defect, and the fixture pins that it admits.
///
/// The fixture also carries the D18 country roster: raid identity joins the
/// resolved roster (an unresolved numeric id renders as a 3-char decimal
/// string, so shape alone cannot authenticate a tag), and a row's own
/// admitting fixture must carry every measured view its predicate consumes.
fn raid_census_fixture() -> Value {
    json!({
        "countries": measured_countries(),
        "raids": {
            "instances_complete": true,
            "ordering": "engine_parent_then_child_producer_order",
            "counters": Value::Null,
            "targets": Value::Null,
            "countries": [{"tag": "GER", "priority": 1}],
            "country_statuses": [
                {
                    "status_index": 0, "tag": Value::Null, "tag_db_id": 0,
                    "classification": "missing_tag_dummy", "priority": 0,
                    "first_child_ordinal": 0, "child_count": 0
                },
                {
                    "status_index": 1, "tag": "GER", "tag_db_id": 1,
                    "classification": "country", "priority": 1,
                    "first_child_ordinal": 0, "child_count": 1
                }
            ],
            "instances": [{
                "producer_ordinal": 0, "status_index": 1, "raid_index": 0,
                "id": 509, "id_type": 84, "prep_time": 2160,
                "phase": "ENDED", "phase_raw": 5,
                "outcome": "FAILURE", "outcome_raw": 1,
                "end_date_present": true,
                "owner_tag": "GER", "owner_tag_db_id": 1,
                "victim_country": "ENG", "victim_country_tag_db_id": 2,
                "raid_source": {
                    "type_present": true, "type_raw": 0, "province": 6570,
                    "tag": "GER", "tag_db_id": 1, "location_raw": [0, 0]
                }
            }]
        }
    })
}

fn owner_proven_fixture_table() -> BTreeMap<&'static str, NamedSnapshotFixture> {
    BTreeMap::from([
        (
            "combat.land_active_epoch",
            NamedSnapshotFixture {
                name: "measured active-land combat census",
                rest: json!({
                    "combats": [{
                        "combat_id": 11,
                        "generation": 1,
                        "kind": "land",
                        "location_province": 42
                    }]
                }),
            },
        ),
        (
            "combat.land_terminal_detail",
            NamedSnapshotFixture {
                name: "measured clean terminal-land combat census",
                rest: json!({
                    "ended_combats": [{
                        "combat_id": 9,
                        "generation": 2,
                        "lifecycle": "ended"
                    }]
                }),
            },
        ),
        (
            "combat.naval_air_epoch",
            NamedSnapshotFixture {
                name: "measured empty naval-combat census",
                rest: json!({"naval_combats": []}),
            },
        ),
        (
            "mapmode.supply_state",
            NamedSnapshotFixture {
                name: "measured D62 hub-motorization census",
                rest: json!({
                    "supply": {"_r2_hub_motorization_live": [{
                        "country": "GER",
                        "owner_tag_db_id": 3,
                        "motorization_country": "GER",
                        "actor_tag_db_id": 3,
                        "province_id": 6354,
                        "state_id": 42,
                        "record_index": 0,
                        "map_entry_index": 0,
                        "motorization_level": 2,
                        "truck_fraction_raw": 100000,
                        "node_motorized": true,
                        "scope": "country_level"
                    }]}
                }),
            },
        ),
        (
            "weather.province_snow_accumulator",
            NamedSnapshotFixture {
                name: "authenticated D24 province-snow row",
                rest: province_snow_fixture(),
            },
        ),
        (
            "units.divisions",
            NamedSnapshotFixture {
                name: "complete division census with identified owners and anchors",
                rest: division_census_fixture(),
            },
        ),
        (
            "units.taskforces_fleets",
            NamedSnapshotFixture {
                name: "complete task-force census under its fleet owner",
                rest: taskforce_census_fixture(),
            },
        ),
        (
            "units.wings",
            NamedSnapshotFixture {
                name: "complete air-wing census with unique identified wings",
                rest: air_wing_census_fixture(),
            },
        ),
        (
            "units.air_base_location",
            NamedSnapshotFixture {
                name: "complete air-wing census whose every wing resolves its air base",
                rest: air_wing_census_fixture(),
            },
        ),
        (
            "orders.theatre_army_hierarchy",
            NamedSnapshotFixture {
                name: "admitted all-country theatre-selector census equal to the complete D18 roster",
                rest: theatre_hierarchy_census_fixture(),
            },
        ),
        (
            "mapmode.occupation_resistance_compliance",
            NamedSnapshotFixture {
                name: "complete D52 per-state occupation census under its own witness",
                rest: occupation_census_fixture(),
            },
        ),
        (
            "country.d18_complete_roster",
            NamedSnapshotFixture {
                name: "complete D18 country roster",
                rest: json!({"countries": measured_countries()}),
            },
        ),
        (
            "objects.building_state_damage",
            NamedSnapshotFixture {
                name: "exact D48 per-state building census under its own recorder marker",
                rest: d48_building_census_fixture("C"),
            },
        ),
        (
            "diplomacy.faction_roster",
            NamedSnapshotFixture {
                name: "producer-witnessed measured-empty faction roster",
                rest: json!({
                    "diplomacy_world": {
                        "domain53_extra": {
                            "faction_roster_status": "measured_empty",
                            "faction_rows": [],
                        },
                    },
                }),
            },
        ),
        (
            "world.calendar_time",
            NamedSnapshotFixture {
                name: "measured engine calendar stamp",
                rest: json!({}),
            },
        ),
        (
            "map_furniture.current_victory_points",
            NamedSnapshotFixture {
                name: "complete ascending victory-point value census for this hour",
                rest: map_furniture_victory_point_fixture(),
            },
        ),
        (
            "mapmode.raids",
            NamedSnapshotFixture {
                name: "complete raid instance/status census under its own ordering claim",
                rest: raid_census_fixture(),
            },
        ),
        (
            "session.player_country_set",
            NamedSnapshotFixture {
                name: "exact D57 human player-country set, co-op rows included",
                rest: player_country_set_fixture(),
            },
        ),
    ])
}

fn measured_countries() -> Value {
    json!({
        "ENG": {"_r2_tag_db_id": 2},
        "GER": {"_r2_tag_db_id": 1},
    })
}




fn merge_snapshot(into: &mut Value, from: &Value) {
    match (into, from) {
        (Value::Object(into), Value::Object(from)) => {
            for (key, value) in from {
                match into.get_mut(key) {
                    Some(existing) => merge_snapshot(existing, value),
                    None => {
                        into.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (into, from) => *into = from.clone(),
    }
}

fn all_owner_proven_fixtures_merged(
    fixtures: &BTreeMap<&'static str, NamedSnapshotFixture>,
) -> Value {
    let mut merged = json!({});
    for fixture in fixtures.values() {
        merge_snapshot(&mut merged, &fixture.rest);
    }
    merged
}

fn fully_measured_snapshot() -> Value {
    all_owner_proven_fixtures_merged(&owner_proven_fixture_table())
}

struct UnmeasuredFixture {
    snapshot: Value,
    date: Option<&'static str>,
    ledger_accessor_stays_present: bool,
}

fn unmeasured_fixture(input_id: &str) -> UnmeasuredFixture {
    let mut snapshot = fully_measured_snapshot();
    let object = snapshot.as_object_mut().unwrap();
    let ledger_accessor_stays_present = match input_id {
        "combat.land_active_epoch" => {
            object.insert("combats".into(), Value::Null);
            false
        }
        "combat.land_terminal_detail" => {
            object.insert(
                "ended_combats".into(),
                json!([{"combat_id": 0, "generation": 2, "lifecycle": "ended"}]),
            );
            true
        }
        "combat.naval_air_epoch" => {
            object.insert("naval_combats".into(), Value::Null);
            // MERGE, never replace: `naval_air` also carries the exact D31 and
            // D56 map-mode carriers that `mapmode.air_superiority` and
            // `mapmode.naval_supremacy` are proven on, and overwriting the whole
            // section would move three rows instead of one.
            object
                .entry("naval_air")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .expect("naval_air is an object in the measured fixture")
                .insert("_r2_air_combat_losses_live".into(), json!([]));
            true
        }
        "country.d18_complete_roster" => {
            // Withdraw ENG's identity IN PLACE. Replacing the whole `countries`
            // map also dropped every country's D48 construction sidecar, which
            // took `mapmode.resource_deposits` down with it once that row was
            // proven — the D48 projection is all-or-nothing across the snapshot
            // (lane PL, 2026-08-21).
            object
                .get_mut("countries")
                .and_then(Value::as_object_mut)
                .and_then(|countries| countries.get_mut("ENG"))
                .and_then(Value::as_object_mut)
                .expect("the measured fixture carries an ENG country entry")
                .remove("_r2_tag_db_id");
            false
        }
        "diplomacy.faction_roster" => {
            object.remove("diplomacy_world");
            false
        }
        "mapmode.occupation_resistance_compliance" => {
            object
                .get_mut("_r2_occupation_census_live")
                .and_then(Value::as_object_mut)
                .expect("the measured fixture carries the D52 census witness")
                .insert("census_complete".into(), json!(false));
            true
        }
        "mapmode.supply_state" => {
            // The fold's fail-closed shape: a domain the collector could not
            // vouch for stores Null, which the strict decoder reports as
            // DomainUnavailable rather than an empty Present census.
            object
                .entry("supply")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .expect("supply is an object in the measured fixture")
                .insert("_r2_hub_motorization_live".into(), Value::Null);
            false
        }
        "weather.province_snow_accumulator" => {
            let provinces = object
                .get_mut("map")
                .and_then(|map| map.get_mut("_r2_weather_live"))
                .and_then(|weather| weather.get_mut("provinces"))
                .and_then(Value::as_object_mut)
                .expect("the measured fixture carries a raw province object");
            provinces.remove("1");
            provinces.insert(
                "not-a-province".into(),
                json!({"snow_raw": 6_789}),
            );
            true
        }
        // Each unit case leaves its own root accessor Present on purpose:
        // `build_division_units`, `build_fleets` and `build_wings` all return a
        // hardcoded `ViewStatus::Present` and only push a coverage warning when
        // they drop a source row, so a bare-accessor admission would promise a
        // complete all-country census that the frame does not carry.
        "units.divisions" => {
            // A source row the decoder could not parse: dropped with a
            // `divisions[1]: ... row skipped` warning, census silently short.
            object.insert(
                "divisions".into(),
                json!([
                    {"div_id": 5, "div_type": 29, "owner": "GER", "location_province": 42},
                    7
                ]),
            );
            true
        }
        "units.taskforces_fleets" => {
            // The task force decodes (its id is a usable uid string) and no row
            // is dropped, so the dropped-row guard does NOT fire; only the
            // raw-identity requirement does. A non-numeric id leaves
            // raw_task_force_id unobtained.
            object.insert(
                "order_of_battle".into(),
                json!({
                    "fleets": [{
                        "id": 3,
                        "owner": "ENG",
                        "task_forces": [{"id": "opaque", "location_province": 118}]
                    }],
                    "air_wings": air_wing_census_fixture()["order_of_battle"]["air_wings"]
                }),
            );
            true
        }
        "units.wings" => {
            // Two wings sharing one engine id: both rows decode, so nothing is
            // dropped and the air-base join still holds — but wing identity is
            // no longer unique, which is what `units.wings` alone asserts.
            object
                .get_mut("order_of_battle")
                .and_then(Value::as_object_mut)
                .expect("the measured fixture carries an order of battle")
                .insert(
                    "air_wings".into(),
                    json!([
                        {"id": 12, "owner": "SOV", "air_base_province": 907, "state_id": 219},
                        {"id": 12, "owner": "SOV", "air_base_province": 908, "state_id": 220}
                    ]),
                );
            true
        }
        "units.air_base_location" => {
            // The wing census stays complete and uniquely identified, and the
            // base province resolves; only the state half of the join is
            // invalid, so this kills the `state_id != 0` requirement on its own.
            object
                .get_mut("order_of_battle")
                .and_then(Value::as_object_mut)
                .expect("the measured fixture carries an order of battle")
                .insert(
                    "air_wings".into(),
                    json!([
                        {"id": 12, "owner": "SOV", "air_base_province": 907, "state_id": 0}
                    ]),
                );
            true
        }
        "objects.building_state_damage" => {
            // The building half of the D48 sidecar, and only that half.
            //
            // This used to corrupt the sidecar's `source` marker instead. That
            // still refuses this row — but the SAME marker gates
            // `mapmode.resource_deposits`, which shares the sidecar, so the
            // marker break moved two rows at once and the orthogonality this
            // fixture set exists to prove would have been lost the moment the
            // resource row was proven (lane PL, 2026-08-21).
            //
            // The PROVINCE-level D48 carrier is the seam that belongs to this
            // row alone. `fold_d48_province_buildings` folds its outcome into
            // `d48_status` with `combine_complete_status` (unavailable wins) but
            // into `buildings_status` with `combine_status` (present wins), and
            // it never touches a state row — so a province sidecar spelled
            // `null` refuses exactly this row, leaves the per-state
            // infrastructure levels `mapmode.infrastructure_state` reads
            // untouched, and never goes near the resource map.
            object.insert(
                "provinces".into(),
                json!({"77": {"construction_buildings_state_damage_live": Value::Null}}),
            );
            false
        }
        "map_furniture.current_victory_points" => {
            // The census is intact and ascending; only its `update_index` names
            // a different hour. That is the hour-misattribution shape this
            // project treats as a stop condition, and it is the one an
            // otherwise-perfect carrier can still have.
            let furniture = object
                .get_mut("map_furniture")
                .and_then(Value::as_object_mut)
                .expect("the measured fixture carries a victory-point census");
            let stamped = furniture
                .get("update_index")
                .and_then(Value::as_i64)
                .expect("the fixture stamps an hour");
            furniture.insert("update_index".into(), json!(stamped + 1));
            false
        }
        "mapmode.raids" => {
            // `build_raids` hardcodes Present whenever `raids` is an object, so
            // withdrawing the completeness claim leaves the ledger accessor
            // Present while both collections go unobtained.
            object["raids"]["instances_complete"] = json!(false);
            true
        }
        "session.player_country_set" => {
            // The producer's own "I could not read the player vector" spelling.
            //
            // This row's ledger accessor IS the player leaf, so this break moves
            // the accessor with it — there is no composite to cover here. The
            // composite direction (leaf still Measured, row still refused) is
            // pinned by `a_co_op_player_set_is_refused_because_the_players_map_mode_rejects_it`.
            object["_r2_session_multiplayer_metadata_live"]["players_by_country"]["state"] =
                json!("unavailable");
            false
        }
        "world.calendar_time" => false,
        "orders.theatre_army_hierarchy" => {
            // The theatre fold's own unusable-read spelling. The ledger
            // accessor IS the decoded view here, so it moves with the payload.
            object.insert("_r2_theatre_selector_live".into(), Value::Null);
            false
        }
        other => panic!("no unmeasured fixture for {other}"),
    };
    UnmeasuredFixture {
        snapshot,
        date: if input_id == "world.calendar_time" {
            None
        } else {
            Some(MEASURED_CALENDAR_STAMP)
        },
        ledger_accessor_stays_present,
    }
}

fn unmeasured_frame(input_id: &str) -> std::sync::Arc<WorldFrame> {
    let fixture = unmeasured_fixture(input_id);
    // The hour is pinned to the shared stamp for every case, including the one
    // that drops the stamp. `world_frame_with_date` falls back to hour 7 when
    // there is no date, which would MOVE the frame's hour as well as remove the
    // date — and a carrier that binds its payload to the displayed hour (row
    // #20) then refuses for the moved hour instead of for the unmeasured input
    // this fixture is about. One change per fixture is the whole point of the
    // sibling test below.
    world_frame_with_date_at(
        fixture.snapshot,
        fixture.date,
        schema_hour_index(MEASURED_CALENDAR_STAMP).expect("the shared stamp is a valid hour"),
    )
}

fn ledger_accessor_status(frame: &WorldFrame, input_id: &str) -> ViewStatus {
    match input_id {
        "combat.land_active_epoch" => frame.combats_status(),
        "combat.land_terminal_detail" => frame.ended_combats_status(),
        "combat.naval_air_epoch" => frame.naval_air_combat_status(),
        "country.d18_complete_roster" => frame.d18_country_roster().complete_status(),
        "diplomacy.faction_roster" => match frame.diplomacy().faction_roster_status() {
            replay_core::world::FactionRosterStatus::Unavailable => ViewStatus::DomainUnavailable,
            replay_core::world::FactionRosterStatus::MeasuredEmpty
            | replay_core::world::FactionRosterStatus::MeasuredNonEmpty => ViewStatus::Present,
        },
        "mapmode.occupation_resistance_compliance" => frame.occupation_status(),
        "units.divisions" => frame.division_units_status(),
        "units.taskforces_fleets" => frame.fleets_status(),
        "units.wings" | "units.air_base_location" => frame.wings_status(),
        "objects.building_state_damage" => frame.d48_buildings_status(),
        "weather.province_snow_accumulator" => frame.weather_province_snow_status(),
        "map_furniture.current_victory_points" => frame.map_furniture_status(),
        "mapmode.raids" => frame.raids_status(),
        "mapmode.supply_state" => frame.hub_motorization_status(),
        "session.player_country_set" => map_viewer::player_country_set_leaf_status(frame),
        "world.calendar_time" => match frame.date() {
            Some(_) => ViewStatus::Present,
            None => ViewStatus::DomainUnavailable,
        },
        "orders.theatre_army_hierarchy" => frame.theatre_army_hierarchy_status(),
        other => panic!("no ledger accessor for {other}"),
    }
}

fn unresolved_projection() -> MapAuthorityProjection {
    MapAuthorityProjection::from_untrusted_parts(
        MAP_AUTHORITY_INPUT_SCHEMA.to_string(),
        42,
        "1".repeat(64),
        "2".repeat(64),
        "3".repeat(64),
        MAP_AUTHORITY_REQUIRED_INPUTS
            .iter()
            .map(|input_id| unresolved_requirement(input_id))
            .collect(),
    )
}

#[test]
fn every_stable_family_has_only_explicit_unresolved_metadata_and_blocker() {
    for input_id in MAP_AUTHORITY_REQUIRED_INPUTS {
        let metadata = map_authority_requirement_metadata(input_id).unwrap();
        assert_eq!(metadata.input_id, *input_id);
        assert_eq!(metadata.payload_schema, None, "{input_id}");
        assert_eq!(metadata.cadence, None, "{input_id}");
        assert_eq!(metadata.identity_order, None, "{input_id}");
        assert!(!metadata.blocker.is_empty(), "{input_id}");
    }
    assert!(map_authority_requirement_metadata("recorder.selected_present").is_none());
}

#[test]
fn xmap00_can_only_return_a_complete_typed_gap_report() {
    let admission = admit_map_authority_projection(&unresolved_projection()).unwrap();
    assert_eq!(admission.gaps().len(), MAP_AUTHORITY_REQUIRED_INPUTS.len());
    assert_eq!(
        admission
            .gaps()
            .iter()
            .map(|gap| gap.input_id.as_str())
            .collect::<Vec<_>>(),
        MAP_AUTHORITY_REQUIRED_INPUTS
    );

    let refusal = refuse_product_map_until_xmap01(&admission);
    assert_eq!(refusal.code, "XMAP00_AUTHORITY_UNAVAILABLE");
    assert!(!refusal.authority_ready);
    assert_eq!(refusal.gap_ids.len(), MAP_AUTHORITY_REQUIRED_INPUTS.len());
    assert!(require_promoted_xmap01_renderer(&admission).is_err());

    let missing_frame = refuse_missing_xmap00_authority("world_frame");
    assert_eq!(missing_frame.code, "XMAP00_AUTHORITY_UNAVAILABLE");
    assert!(!missing_frame.authority_ready);
    assert_eq!(missing_frame.gap_ids, vec!["world_frame"]);
}

#[test]
fn caller_hashes_legacy_ready_rows_and_measured_empty_witnesses_cannot_admit() {
    let projection = unresolved_projection();
    for forbidden_field in [
        "payload_sha256",
        "ordering_sha256",
        "completeness_sha256",
        "provenance_receipt_sha256",
        "measured_empty_witness",
    ] {
        let mut json = serde_json::to_value(&projection).unwrap();
        json["requirements"][0][forbidden_field] = serde_json::json!("1".repeat(64));
        assert!(
            admit_map_authority_json(&serde_json::to_vec(&json).unwrap()).is_err(),
            "{forbidden_field} was silently trusted"
        );
    }

    for forged_disposition in ["present", "measured_empty"] {
        let mut json = serde_json::to_value(&projection).unwrap();
        json["requirements"][0]["disposition"] = serde_json::json!(forged_disposition);
        assert!(
            admit_map_authority_json(&serde_json::to_vec(&json).unwrap()).is_err(),
            "{forged_disposition} was admitted"
        );
    }

    let authority_source = include_str!("../src/map_authority.rs");
    assert!(!authority_source.contains("MapAuthorityReceipt"));
    assert!(!authority_source.contains("MapAuthorityMeasuredEmptyWitness"));
    assert!(!authority_source.contains("Ready {"));
    assert!(!authority_source.contains("fn is_ready"));
}

#[test]
fn recorder_presentation_fields_are_rejected_not_ignored() {
    for forbidden in [
        "selected_present",
        "camera",
        "map_mode",
        "selection",
        "gui_state",
        "draw_geometry",
    ] {
        let mut json = serde_json::to_value(unresolved_projection()).unwrap();
        json.as_object_mut()
            .unwrap()
            .insert(forbidden.into(), serde_json::json!({}));
        assert!(
            admit_map_authority_json(&serde_json::to_vec(&json).unwrap()).is_err(),
            "{forbidden} was silently accepted"
        );
    }
}

#[test]
fn family_identity_order_and_unresolved_metadata_are_fail_closed() {
    let mut missing = unresolved_projection();
    missing.requirements.pop();
    assert!(admit_map_authority_projection(&missing).is_err());

    let mut duplicate = unresolved_projection();
    duplicate.requirements[1].input_id = duplicate.requirements[0].input_id.clone();
    assert!(admit_map_authority_projection(&duplicate).is_err());

    let mut unsorted = unresolved_projection();
    unsorted.requirements.swap(0, 1);
    assert!(admit_map_authority_projection(&unsorted).is_err());

    for field in ["payload_schema", "cadence", "identity_order"] {
        let mut json = serde_json::to_value(unresolved_projection()).unwrap();
        json["requirements"][0][field] = serde_json::json!("caller-invented");
        let error = admit_map_authority_json(&serde_json::to_vec(&json).unwrap()).unwrap_err();
        assert!(
            error.contains("unresolved map authority metadata"),
            "{error}"
        );
    }

    let mut changed_blocker = unresolved_projection();
    changed_blocker.requirements[0].blocker = Some("caller says complete".into());
    assert!(admit_map_authority_projection(&changed_blocker)
        .unwrap_err()
        .contains("canonical unresolved blocker"));
}

#[test]
fn absent_and_unavailable_require_distinct_canonical_blocker_forms() {
    let mut forged_absent = unresolved_projection();
    forged_absent.requirements[0].disposition = MapAuthorityRequirementDisposition::Absent;
    assert!(
        admit_map_authority_json(&serde_json::to_vec(&forged_absent).unwrap())
            .unwrap_err()
            .contains("canonical unresolved blocker")
    );

    let metadata =
        map_authority_requirement_metadata(&forged_absent.requirements[0].input_id).unwrap();
    forged_absent.requirements[0].blocker = Some(format!(
        "{}; WorldFrame {} is DomainAbsent",
        metadata.blocker,
        map_authority_observed_status_label(&forged_absent.requirements[0].input_id)
    ));
    let admitted = admit_map_authority_json(&serde_json::to_vec(&forged_absent).unwrap()).unwrap();
    assert_eq!(
        admitted.gaps()[0].disposition,
        MapAuthorityRequirementDisposition::Absent
    );
    assert_eq!(
        Some(&admitted.gaps()[0].reason),
        forged_absent.requirements[0].blocker.as_ref()
    );

    let mut forged_unavailable = unresolved_projection();
    let metadata =
        map_authority_requirement_metadata(&forged_unavailable.requirements[0].input_id).unwrap();
    forged_unavailable.requirements[0].blocker = Some(format!(
        "{}; WorldFrame {} is DomainAbsent",
        metadata.blocker,
        map_authority_observed_status_label(&forged_unavailable.requirements[0].input_id)
    ));
    assert!(
        admit_map_authority_json(&serde_json::to_vec(&forged_unavailable).unwrap())
            .unwrap_err()
            .contains("canonical unresolved blocker")
    );
}


#[test]
fn the_owner_proven_admission_table_is_exactly_the_ledgers_disposition_column() {
    let rows = xmap00_ledger_rows();
    let semantic: Vec<&BTreeMap<String, String>> = rows
        .iter()
        .filter(|row| row["class"] == "semantic")
        .collect();
    assert_eq!(semantic.len(), 44, "XMAP00 semantic census drifted");

    let mut ledger_ids: Vec<&str> = semantic.iter().map(|row| row["id"].as_str()).collect();
    ledger_ids.sort_unstable();
    assert_eq!(
        ledger_ids, MAP_AUTHORITY_REQUIRED_INPUTS,
        "required-input list drifted from the ledger's semantic rows"
    );

    let ledger_proven: BTreeSet<&str> = semantic
        .iter()
        .filter(|row| row["xmap00_disposition"] == "owner_proven_production_consumer")
        .map(|row| row["id"].as_str())
        .collect();
    let code_proven: BTreeSet<&str> = MAP_AUTHORITY_OWNER_PROVEN_INPUTS
        .iter()
        .map(|admission| admission.input_id)
        .collect();
    assert_eq!(
        ledger_proven, code_proven,
        "admission table is not the ledger's owner-proven set"
    );
    assert_eq!(
        code_proven.len(),
        MAP_AUTHORITY_OWNER_PROVEN_INPUTS.len(),
        "owner-proven admission table contains duplicate input ids"
    );
    assert!(
        !code_proven.is_empty(),
        "owner-proven set unexpectedly empty"
    );
    assert!(
        code_proven.len() < MAP_AUTHORITY_REQUIRED_INPUTS.len(),
        "XMAP00 unexpectedly claims every semantic input is owner-proven"
    );

    for row in &semantic {
        let id = row["id"].as_str();
        let disposition = row["xmap00_disposition"].as_str();
        match map_authority_owner_proven_admission(id) {
            Some(admission) => {
                assert_eq!(disposition, "owner_proven_production_consumer", "{id}");
                assert_eq!(admission.owner_packet, row["owner_packet"], "{id}");
                assert_eq!(admission.payload_schema, row["portable_path"], "{id}");
                assert_eq!(admission.status_accessor, row["status_accessor"], "{id}");
                assert!(!admission.cadence.is_empty(), "{id}");
                assert!(!admission.identity_order.is_empty(), "{id}");
            }
            None => {
                assert_eq!(disposition, "unavailable_until_owner_proves", "{id}");
                let metadata = map_authority_requirement_metadata(id).unwrap();
                assert!(!metadata.blocker.is_empty(), "{id}");
                assert_eq!(metadata.payload_schema, None, "{id}");
            }
        }
    }
}

#[test]
fn owner_proven_inputs_admit_from_the_frames_own_measured_views() {
    let manifest = fixture_manifest();
    let fixtures = owner_proven_fixture_table();
    let proven_ids = MAP_AUTHORITY_OWNER_PROVEN_INPUTS
        .iter()
        .map(|proven| proven.input_id)
        .collect::<BTreeSet<_>>();
    let fixture_ids = fixtures.keys().copied().collect::<BTreeSet<_>>();
    assert_eq!(
        fixture_ids, proven_ids,
        "every owner-proven row must have exactly one named admitting fixture"
    );

    for proven in MAP_AUTHORITY_OWNER_PROVEN_INPUTS {
        let fixture = fixtures
            .get(proven.input_id)
            .unwrap_or_else(|| panic!("{} has no admitting fixture", proven.input_id));
        let frame = world_frame_with(fixture.rest.clone());
        let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
        let row = requirement(&projection, proven.input_id);
        assert_eq!(
            row.disposition,
            MapAuthorityRequirementDisposition::Admitted,
            "{} was not admitted by fixture {:?}",
            proven.input_id,
            fixture.name
        );
        assert_eq!(row.payload_schema.as_deref(), Some(proven.payload_schema));
        assert_eq!(row.cadence.as_deref(), Some(proven.cadence));
        assert_eq!(row.identity_order.as_deref(), Some(proven.identity_order));
        assert_eq!(row.blocker, None);

        let admission = admit_map_authority_projection(&projection).unwrap();
        let admitted = admission
            .admitted()
            .iter()
            .find(|input| input.input_id == proven.input_id)
            .unwrap_or_else(|| panic!("{} disappeared at the admission door", proven.input_id));
        assert_eq!(admitted.owner_packet, proven.owner_packet);
        assert_eq!(admitted.payload_schema, proven.payload_schema);
        assert_eq!(admitted.cadence, proven.cadence);
        assert_eq!(admitted.identity_order, proven.identity_order);
    }

    let frame = world_frame_with(all_owner_proven_fixtures_merged(&fixtures));
    let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
    let proven_count = MAP_AUTHORITY_OWNER_PROVEN_INPUTS.len();
    let admission = admit_map_authority_projection(&projection).unwrap();
    assert_eq!(admission.admitted().len(), proven_count);
    assert_eq!(
        admission.gaps().len(),
        MAP_AUTHORITY_REQUIRED_INPUTS.len() - proven_count
    );
    for proven in MAP_AUTHORITY_OWNER_PROVEN_INPUTS {
        assert!(admission.is_admitted(proven.input_id));
        assert!(
            !admission
                .gaps()
                .iter()
                .any(|gap| gap.input_id == proven.input_id),
            "{} is both admitted and a gap",
            proven.input_id
        );
        let admitted = admission
            .admitted()
            .iter()
            .find(|input| input.input_id == proven.input_id)
            .unwrap();
        assert_eq!(admitted.owner_packet, proven.owner_packet);
        assert_eq!(admitted.payload_schema, proven.payload_schema);
        assert_eq!(admitted.cadence, proven.cadence);
        assert_eq!(admitted.identity_order, proven.identity_order);
    }

    for gap in admission.gaps() {
        assert!(map_authority_owner_proven_admission(&gap.input_id).is_none());
        assert!(!gap.reason.is_empty(), "{} lost its blocker", gap.input_id);
    }
}

#[test]
fn owner_proven_inputs_refuse_with_their_named_blocker_when_unmeasured() {
    let manifest = fixture_manifest();

    let absent = map_authority_projection_from_world_frame(
        &world_frame_with_date(json!({}), None),
        &manifest,
    )
    .unwrap();
    let unobtained = map_authority_projection_from_world_frame(
        &world_frame_with_date(
            json!({
            "combats": Value::Null,
            "ended_combats": Value::Null,
            "naval_combats": Value::Null,
            "map": {"_r2_weather_live": Value::Null},
            "countries": Value::Null,
            "diplomacy_live": Value::Null,
            "states": Value::Null,
            "divisions": Value::Null,
            "order_of_battle": Value::Null,
            "map_furniture": Value::Null,
            // The theatre fold spells an unusable read as null
            // (`world_fold.rs::fold_theatre_selector`), same as every domain here.
            "_r2_theatre_selector_live": Value::Null,
            // XMAP18's map-mode carriers, each spelled the way its own producer
            // spells an unusable read (lane PL, 2026-08-21).
            "raids": Value::Null,
            "world": Value::Null,
            "session": Value::Null,
            "_r2_session_multiplayer_metadata_live": Value::Null,
            "naval_air": Value::Null,
            }),
            None,
        ),
        &manifest,
    )
    .unwrap();

    for proven in MAP_AUTHORITY_OWNER_PROVEN_INPUTS {
        if proven.input_id == "world.calendar_time" {
            continue;
        }
        let blocker = map_authority_requirement_metadata(proven.input_id)
            .unwrap()
            .blocker;
        let label = map_authority_observed_status_label(proven.input_id);
        let empty_frame_disposition = if proven.input_id == "diplomacy.faction_roster" {
            MapAuthorityRequirementDisposition::Unavailable
        } else {
            MapAuthorityRequirementDisposition::Absent
        };
        let empty_frame_status = if proven.input_id == "diplomacy.faction_roster" {
            "DomainUnavailable"
        } else {
            "DomainAbsent"
        };
        let absent_row = requirement(&absent, proven.input_id);
        assert_eq!(
            absent_row.disposition, empty_frame_disposition,
            "{}",
            proven.input_id
        );
        assert_eq!(
            absent_row.blocker.as_deref(),
            Some(format!("{blocker}; WorldFrame {label} is {empty_frame_status}").as_str()),
            "{}",
            proven.input_id
        );
        assert_eq!(absent_row.payload_schema, None);

        let unobtained_row = requirement(&unobtained, proven.input_id);
        assert_eq!(
            unobtained_row.disposition,
            MapAuthorityRequirementDisposition::Unavailable,
            "{}",
            proven.input_id
        );
        assert_eq!(
            unobtained_row.blocker.as_deref(),
            Some(format!("{blocker}; WorldFrame {label} is DomainUnavailable").as_str()),
            "{}",
            proven.input_id
        );
        assert_eq!(unobtained_row.payload_schema, None);
    }

    for projection in [&absent, &unobtained] {
        let admission = admit_map_authority_projection(projection).unwrap();
        assert!(admission.admitted().is_empty());
        assert_eq!(admission.gaps().len(), MAP_AUTHORITY_REQUIRED_INPUTS.len());
        assert_eq!(
            refuse_product_map_until_xmap01(&admission).gap_ids.len(),
            MAP_AUTHORITY_REQUIRED_INPUTS.len()
        );
    }
}

#[test]
fn an_input_without_owner_proven_evidence_can_never_be_admitted() {
    let unproven = "control.controller_by_province";
    assert!(map_authority_owner_proven_admission(unproven).is_none());
    let mut forged = unresolved_projection();
    let index = forged
        .requirements
        .iter()
        .position(|row| row.input_id == unproven)
        .unwrap();
    forged.requirements[index].disposition = MapAuthorityRequirementDisposition::Admitted;
    forged.requirements[index].payload_schema = Some("WorldFrame.map.control_by_province".into());
    forged.requirements[index].cadence =
        Some("one decoded view per canonical recorded hour".into());
    forged.requirements[index].identity_order = Some("province id ascending".into());
    forged.requirements[index].blocker = None;
    assert!(admit_map_authority_projection(&forged)
        .unwrap_err()
        .contains("has no owner-proven production admission"));

    for (field, value) in [
        ("payload_schema", "WorldFrame.selected_present"),
        ("cadence", "whenever the viewer asks"),
        ("identity_order", "caller order"),
    ] {
        let mut drifted = unresolved_projection();
        drifted.requirements[0].disposition = MapAuthorityRequirementDisposition::Admitted;
        let proven = map_authority_owner_proven_admission(&drifted.requirements[0].input_id)
            .expect("requirements[0] is an owner-proven row");
        drifted.requirements[0].payload_schema = Some(proven.payload_schema.into());
        drifted.requirements[0].cadence = Some(proven.cadence.into());
        drifted.requirements[0].identity_order = Some(proven.identity_order.into());
        drifted.requirements[0].blocker = None;
        match field {
            "payload_schema" => drifted.requirements[0].payload_schema = Some(value.into()),
            "cadence" => drifted.requirements[0].cadence = Some(value.into()),
            _ => drifted.requirements[0].identity_order = Some(value.into()),
        }
        assert!(
            admit_map_authority_projection(&drifted)
                .unwrap_err()
                .contains("owner-proven payload/cadence/identity-order contract"),
            "{field} drift was accepted"
        );
    }
}

#[test]
fn only_a_frame_derived_projection_can_carry_an_admission() {
    let manifest = fixture_manifest();
    let frame = world_frame_with(all_owner_proven_fixtures_merged(
        &owner_proven_fixture_table(),
    ));
    let derived = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
    assert!(derived.is_frame_derived());
    assert_eq!(
        admit_map_authority_projection(&derived)
            .unwrap()
            .admitted()
            .len(),
        MAP_AUTHORITY_OWNER_PROVEN_INPUTS.len()
    );

    let copied = MapAuthorityProjection::from_untrusted_parts(
        derived.schema.clone(),
        derived.hour_index,
        derived.content_manifest_digest_sha256.clone(),
        derived.product_modset_sha256.clone(),
        derived.effective_vfs_manifest_sha256.clone(),
        derived.requirements.clone(),
    );
    assert!(!copied.is_frame_derived());
    assert!(admit_map_authority_projection(&copied)
        .unwrap_err()
        .contains("not derived from an authenticated WorldFrame"));

    let mut edited_gap = derived.clone();
    let gap_index = edited_gap
        .requirements
        .iter()
        .position(|row| row.disposition != MapAuthorityRequirementDisposition::Admitted)
        .unwrap();
    edited_gap.requirements[gap_index].blocker = Some("caller says complete".into());
    assert!(!edited_gap.is_frame_derived());
    assert!(admit_map_authority_projection(&edited_gap).is_err());

    let mut edited_hour = derived.clone();
    edited_hour.hour_index += 1;
    assert!(!edited_hour.is_frame_derived());
    assert!(admit_map_authority_projection(&edited_hour)
        .unwrap_err()
        .contains("not derived from an authenticated WorldFrame"));

    assert!(admit_map_authority_projection(&unresolved_projection()).is_ok());
}

#[test]
fn the_naval_air_epoch_needs_its_own_partition_measured_not_a_measured_sibling() {
    let manifest = fixture_manifest();
    let sibling_only = world_frame_with(json!({
        "naval_combats": Value::Null,
        "naval_air": {"_r2_air_combat_losses_live": []},
    }));
    assert_eq!(
        sibling_only.naval_air_combat_status(),
        replay_core::ViewStatus::Present
    );
    assert_eq!(
        sibling_only.naval_air_combat().naval_combats_status,
        replay_core::ViewStatus::DomainUnavailable
    );

    let projection = map_authority_projection_from_world_frame(&sibling_only, &manifest).unwrap();
    let row = requirement(&projection, "combat.naval_air_epoch");
    assert_eq!(
        row.disposition,
        MapAuthorityRequirementDisposition::Unavailable,
        "an unread naval census was admitted behind a measured sibling"
    );
    let blocker = map_authority_requirement_metadata("combat.naval_air_epoch")
        .unwrap()
        .blocker;
    assert_eq!(
        row.blocker.as_deref(),
        Some(format!("{blocker}; WorldFrame naval_combats_status is DomainUnavailable").as_str()),
        "the refusal must name the census it actually read, not the root it did not"
    );
    let admission = admit_map_authority_projection(&projection).unwrap();
    assert!(!admission.is_admitted("combat.naval_air_epoch"));

    let measured = world_frame_with(json!({"naval_combats": []}));
    let projection = map_authority_projection_from_world_frame(&measured, &manifest).unwrap();
    assert_eq!(
        requirement(&projection, "combat.naval_air_epoch").disposition,
        MapAuthorityRequirementDisposition::Admitted
    );
}

#[test]
fn a_terminal_census_that_dropped_rows_is_not_a_measured_one() {
    let manifest = fixture_manifest();
    let lossy = world_frame_with(json!({"ended_combats": [7]}));
    assert_eq!(
        lossy.ended_combats_status(),
        replay_core::ViewStatus::Present
    );
    assert!(lossy.ended_combats().is_empty());
    assert!(
        lossy.coverage_warnings().iter().any(|warning| {
            warning.starts_with("ended_combats[") && warning.contains("row skipped")
        }),
        "decoder drop vocabulary moved; the completeness check no longer sees it: {:?}",
        lossy.coverage_warnings()
    );

    let projection = map_authority_projection_from_world_frame(&lossy, &manifest).unwrap();
    let row = requirement(&projection, "combat.land_terminal_detail");
    assert_eq!(
        row.disposition,
        MapAuthorityRequirementDisposition::Unavailable
    );
    let blocker = map_authority_requirement_metadata("combat.land_terminal_detail")
        .unwrap()
        .blocker;
    assert_eq!(
        row.blocker.as_deref(),
        Some(
            format!("{blocker}; WorldFrame certified ended_combats_status is DomainUnavailable")
                .as_str()
        )
    );
    assert!(!admit_map_authority_projection(&projection)
        .unwrap()
        .is_admitted("combat.land_terminal_detail"));

    let clean = world_frame_with(json!({
        "ended_combats": [{"combat_id": 9, "generation": 2, "lifecycle": "ended"}],
    }));
    assert!(!clean.coverage_warnings().iter().any(|warning| {
        warning.starts_with("ended_combats[") && warning.contains("row skipped")
    }));
    let projection = map_authority_projection_from_world_frame(&clean, &manifest).unwrap();
    assert_eq!(
        requirement(&projection, "combat.land_terminal_detail").disposition,
        MapAuthorityRequirementDisposition::Admitted
    );
}

#[test]
fn a_province_snow_row_without_a_measurement_is_not_admitted() {
    let mut rest = province_snow_fixture();
    rest["map"]["_r2_weather_live"]["provinces"]["1"] = json!({"mud_present": true});
    let frame = world_frame_with(rest);
    assert_eq!(
        frame.weather_province_snow_status(),
        replay_core::ViewStatus::Present
    );
    assert_eq!(
        frame.weather_province_snow().by_province.get(&1),
        Some(&replay_core::Leaf::Absent)
    );
    assert_owner_proven_unavailable(&frame, "weather.province_snow_accumulator");
}

#[test]
fn a_province_snow_member_outside_the_declared_range_is_not_admitted() {
    let mut rest = province_snow_fixture();
    let provinces = rest["map"]["_r2_weather_live"]["provinces"]
        .as_object_mut()
        .unwrap();
    provinces.remove("1");
    provinces.insert("2".into(), json!({"snow_raw": 6_789}));
    let frame = world_frame_with(rest);
    assert_eq!(
        frame.weather_province_snow_status(),
        replay_core::ViewStatus::Present
    );
    assert_eq!(frame.weather_province_snow().by_province.len(), 2);
    assert!(frame
        .weather_province_snow()
        .by_province
        .values()
        .all(|snow| matches!(snow, replay_core::Leaf::Measured(_))));
    assert!(!frame.coverage_warnings().iter().any(|warning| {
        warning.starts_with("map._r2_weather_live.provinces.") && warning.contains("row ignored")
    }));
    assert_owner_proven_unavailable(&frame, "weather.province_snow_accumulator");
}

#[test]
fn an_occupation_census_the_producer_calls_incomplete_is_not_admitted() {
    let mut rest = occupation_census_fixture();
    let census = rest["_r2_occupation_census_live"].as_object_mut().unwrap();
    census.insert("census_complete".into(), json!(false));
    census.insert("observed_state_rows".into(), json!(3));
    census.insert("invalid_state_rows".into(), json!(1));
    let frame = world_frame_with(rest);
    assert_eq!(frame.occupation_status(), replay_core::ViewStatus::Present);
    assert_eq!(frame.occupation_by_state().len(), 2);
    assert_owner_proven_unavailable(&frame, "mapmode.occupation_resistance_compliance");
}

#[test]
fn an_occupation_census_without_its_witness_is_not_admitted() {
    let mut rest = occupation_census_fixture();
    rest.as_object_mut()
        .unwrap()
        .remove("_r2_occupation_census_live");
    let absent = world_frame_with(rest);
    assert_eq!(absent.occupation_status(), replay_core::ViewStatus::Present);
    assert_owner_proven_unavailable(&absent, "mapmode.occupation_resistance_compliance");

    let mut rest = occupation_census_fixture();
    rest["_r2_occupation_census_live"] = Value::Null;
    let refused = world_frame_with(rest);
    assert_eq!(refused.occupation_status(), replay_core::ViewStatus::Present);
    assert_owner_proven_unavailable(&refused, "mapmode.occupation_resistance_compliance");
}

#[test]
fn an_occupation_census_the_decoder_silently_shortened_is_not_admitted() {
    let mut rest = occupation_census_fixture();
    rest["states"]["7"] = json!({"occupation": "not-an-object"});
    let census = rest["_r2_occupation_census_live"].as_object_mut().unwrap();
    census.insert("observed_state_rows".into(), json!(3));
    census.insert("emitted_state_rows".into(), json!(3));
    census.insert("published_state_rows".into(), json!(3));
    census.insert(
        "published_state_digest".into(),
        json!(occupation_state_digest(&[5, 6, 7]).to_string()),
    );
    let frame = world_frame_with(rest);
    assert_eq!(frame.occupation_status(), replay_core::ViewStatus::Present);
    assert_eq!(frame.occupation_by_state().len(), 2);
    assert_owner_proven_unavailable(&frame, "mapmode.occupation_resistance_compliance");
}

#[test]
fn an_occupation_census_that_lost_a_member_after_the_witness_is_not_admitted() {
    let mut rest = occupation_census_fixture();
    rest["states"].as_object_mut().unwrap().remove("6");
    let frame = world_frame_with(rest);
    assert_eq!(frame.occupation_status(), replay_core::ViewStatus::Present);
    assert_eq!(frame.occupation_by_state().len(), 1);
    assert_owner_proven_unavailable(&frame, "mapmode.occupation_resistance_compliance");
}

#[test]
fn an_occupation_census_witness_without_a_published_count_is_not_admitted() {
    let mut rest = occupation_census_fixture();
    rest["_r2_occupation_census_live"]
        .as_object_mut()
        .unwrap()
        .remove("published_state_rows");
    let frame = world_frame_with(rest);
    assert_eq!(frame.occupation_status(), replay_core::ViewStatus::Present);
    assert_owner_proven_unavailable(&frame, "mapmode.occupation_resistance_compliance");
}

#[test]
fn an_occupation_census_that_substituted_one_state_for_another_is_not_admitted() {
    let mut rest = occupation_census_fixture();
    let states = rest["states"].as_object_mut().unwrap();
    let row = states.remove("6").expect("the fixture carries state 6");
    states.insert("7".into(), row);
    let frame = world_frame_with(rest);
    assert_eq!(frame.occupation_status(), replay_core::ViewStatus::Present);
    assert_eq!(frame.occupation_by_state().len(), 2);
    assert_owner_proven_unavailable(&frame, "mapmode.occupation_resistance_compliance");
}

#[test]
fn an_occupation_census_witness_without_a_membership_digest_is_not_admitted() {
    let mut rest = occupation_census_fixture();
    rest["_r2_occupation_census_live"]
        .as_object_mut()
        .unwrap()
        .remove("published_state_digest");
    let absent = world_frame_with(rest);
    assert_owner_proven_unavailable(&absent, "mapmode.occupation_resistance_compliance");

    let mut rest = occupation_census_fixture();
    rest["_r2_occupation_census_live"]["published_state_digest"] =
        json!(occupation_state_digest(&[5, 6]).wrapping_add(1).to_string());
    let wrong = world_frame_with(rest);
    assert_owner_proven_unavailable(&wrong, "mapmode.occupation_resistance_compliance");
}

#[test]
fn an_occupation_row_whose_value_was_not_measured_is_not_admitted() {
    let mut rest = occupation_census_fixture();
    rest["states"]["6"]["occupation"]["compliance"] = Value::Null;
    let frame = world_frame_with(rest);
    assert_eq!(frame.occupation_status(), replay_core::ViewStatus::Present);
    assert_eq!(frame.occupation_by_state().len(), 2);
    assert_owner_proven_unavailable(&frame, "mapmode.occupation_resistance_compliance");
}

#[test]
fn a_province_snow_census_shorter_than_its_observed_row_count_is_not_admitted() {
    let mut rest = province_snow_fixture();
    rest["map"]["_r2_weather_live"]["provinces"]
        .as_object_mut()
        .unwrap()
        .remove("1");
    let frame = world_frame_with(rest);
    assert_eq!(
        frame.weather_province_snow_status(),
        replay_core::ViewStatus::Present
    );
    assert_eq!(frame.weather_province_snow().by_province.len(), 1);
    assert!(frame
        .weather_province_snow()
        .by_province
        .values()
        .all(|snow| matches!(snow, replay_core::Leaf::Measured(_))));
    assert!(!frame.coverage_warnings().iter().any(|warning| {
        warning.starts_with("map._r2_weather_live.provinces.")
            && warning.contains("row ignored")
    }));
    assert_owner_proven_unavailable(&frame, "weather.province_snow_accumulator");
}

#[test]
fn a_zero_row_province_snow_census_is_a_failed_read_not_an_empty_one() {
    let mut rest = province_snow_fixture();
    rest["map"]["_r2_weather_live"]["observed_province_rows"] = json!(0);
    rest["map"]["_r2_weather_live"]["provinces"] = json!({});
    let frame = world_frame_with(rest);
    assert_eq!(
        frame.weather_province_snow_status(),
        replay_core::ViewStatus::Present
    );
    assert!(frame.weather_province_snow().by_province.is_empty());
    assert!(!frame.coverage_warnings().iter().any(|warning| {
        warning.starts_with("map._r2_weather_live.provinces.") && warning.contains("row ignored")
    }));
    assert_owner_proven_unavailable(&frame, "weather.province_snow_accumulator");
}

#[test]
fn province_snow_without_an_observed_row_count_is_not_admitted() {
    let mut rest = province_snow_fixture();
    rest["map"]["_r2_weather_live"]
        .as_object_mut()
        .unwrap()
        .remove("observed_province_rows");
    let frame = world_frame_with(rest);
    assert_eq!(
        frame.weather_province_snow_status(),
        replay_core::ViewStatus::Present
    );
    assert_eq!(frame.weather_province_snow().by_province.len(), 2);
    assert!(frame
        .weather_province_snow()
        .by_province
        .values()
        .all(|snow| matches!(snow, replay_core::Leaf::Measured(_))));
    assert!(!frame.coverage_warnings().iter().any(|warning| {
        warning.starts_with("map._r2_weather_live.provinces.")
            && warning.contains("row ignored")
    }));
    assert_owner_proven_unavailable(&frame, "weather.province_snow_accumulator");
}

#[test]
fn an_unmeasured_calendar_stamp_is_always_unavailable_never_absent() {
    let manifest = fixture_manifest();
    let projection =
        map_authority_projection_from_world_frame(&world_frame_with_date(json!({}), None), &manifest)
            .unwrap();
    let row = requirement(&projection, "world.calendar_time");
    assert_eq!(
        row.disposition,
        MapAuthorityRequirementDisposition::Unavailable
    );
    assert!(row
        .blocker
        .as_deref()
        .is_some_and(|blocker| blocker.ends_with("is DomainUnavailable")));
}

#[test]
fn a_missing_calendar_stamp_moves_no_other_row() {
    let manifest = fixture_manifest();
    let snapshot = all_owner_proven_fixtures_merged(&owner_proven_fixture_table());
    // ONLY the stamp is dropped. `world_frame_with_date(.., None)` also falls
    // back to hour 7, which is a second change and not the one this test is
    // about: a carrier that binds its payload to the displayed hour (row #20)
    // would refuse for the moved hour rather than for the missing date, and the
    // test would then be proving the wrong thing.
    let hour_index = schema_hour_index(MEASURED_CALENDAR_STAMP).expect("the shared stamp is valid");
    let projection = map_authority_projection_from_world_frame(
        &world_frame_with_date_at(snapshot, None, hour_index),
        &manifest,
    )
    .unwrap();
    for proven in MAP_AUTHORITY_OWNER_PROVEN_INPUTS {
        let expected = if proven.input_id == "world.calendar_time" {
            MapAuthorityRequirementDisposition::Unavailable
        } else {
            MapAuthorityRequirementDisposition::Admitted
        };
        assert_eq!(
            requirement(&projection, proven.input_id).disposition,
            expected,
            "{} moved when only the calendar stamp was dropped",
            proven.input_id
        );
    }
}

#[test]
fn a_gregorian_leap_day_is_not_an_engine_date() {
    for gregorian_only in ["1936.2.29.0", "2000.2.29.23", "1960.2.29.12"] {
        assert_stamp_refused_on_its_own_terms(gregorian_only);
    }
}

#[test]
fn a_calendar_stamp_naming_a_day_its_month_lacks_is_not_admitted() {
    for impossible in [
        "1936.4.31.12",
        "1936.6.31.12",
        "1936.9.31.12",
        "1936.11.31.12",
        "1936.1.0.12",
    ] {
        assert_stamp_refused_on_its_own_terms(impossible);
    }
}

#[test]
fn a_calendar_stamp_outside_the_engine_grammar_is_not_admitted() {
    for malformed in [
        "1936.13.1.12",
        "1936.0.1.12",
        "1936.1.1.24",
        "1936.1.1",
        "1936.1.1.12.0",
        "1936.1.1.x",
        "",
    ] {
        assert_stamp_refused_on_its_own_terms(malformed);
    }
}

#[test]
fn a_calendar_stamp_outside_the_producer_window_is_not_admitted() {
    for outside in [
        "1849.12.31.23",
        "2200.1.1.1",
        "2200.12.31.23",
        "2201.1.1.0",
    ] {
        assert_stamp_refused_on_its_own_terms(outside);
    }
}

#[test]
fn a_noncanonically_spelled_calendar_stamp_is_not_admitted() {
    for noncanonical in [
        "01936.1.1.12",
        "1936.01.1.12",
        "1936.1.01.12",
        "1936.1.1.012",
        "+1936.1.1.12",
        "1936.+1.1.12",
        "1936.1.+1.12",
        "1936.1.1.+12",
    ] {
        assert_stamp_refused_on_its_own_terms(noncanonical);
    }
}

fn assert_stamp_refused_on_its_own_terms(stamp: &str) {
    let lenient = lenient_hour_index(stamp).unwrap_or(7);
    let frame = world_frame_with_date_at(json!({}), Some(stamp), lenient);
    assert_eq!(frame.date(), Some(stamp));
    assert_owner_proven_unavailable(&frame, "world.calendar_time");
}

fn lenient_hour_index(date: &str) -> Option<i64> {
    let fields: Option<Vec<i64>> = date
        .split('.')
        .take(4)
        .map(|field| field.parse::<i64>().ok())
        .collect();
    let [year, month, day, hour] = fields?[..] else {
        return None;
    };
    let month_index = usize::try_from(month.clamp(1, 12) - 1).ok()?;
    let day_of_year_index: i64 =
        ENGINE_MONTH_LENGTHS[..month_index].iter().sum::<i64>() + (day - 1);
    Some((year * 365 + day_of_year_index) * 24 + hour)
}

#[test]
fn a_calendar_stamp_disagreeing_with_its_hour_index_is_not_admitted() {
    let honest = schema_hour_index(MEASURED_CALENDAR_STAMP).expect("fixture stamp is well-formed");
    let frame = world_frame_with_date_at(json!({}), Some(MEASURED_CALENDAR_STAMP), honest + 1);
    assert_eq!(frame.date(), Some(MEASURED_CALENDAR_STAMP));
    assert_owner_proven_unavailable(&frame, "world.calendar_time");
}

#[test]
fn engine_emitted_calendar_stamps_are_admitted() {
    let manifest = fixture_manifest();
    for stamp in [
        "1936.1.1.0",
        "1936.1.1.12",
        "1940.9.2.21",
        "1936.2.28.23",
        "1936.12.31.23",
        "1850.1.1.0",
        "2200.1.1.0",
    ] {
        let snapshot = all_owner_proven_fixtures_merged(&owner_proven_fixture_table());
        let frame = world_frame_with_date(snapshot, Some(stamp));
        let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
        assert_eq!(
            requirement(&projection, "world.calendar_time").disposition,
            MapAuthorityRequirementDisposition::Admitted,
            "{stamp} is a date the engine emits and must admit"
        );
    }
}

#[test]
fn an_hour_with_no_calendar_stamp_is_not_admitted() {
    let frame = world_frame_with_date(json!({}), None);
    assert!(frame.date().is_none());
    assert_owner_proven_unavailable(&frame, "world.calendar_time");
}

#[test]
fn a_province_snow_member_the_decoder_discarded_is_not_admitted() {
    let mut rest = province_snow_fixture();
    let provinces = rest["map"]["_r2_weather_live"]["provinces"]
        .as_object_mut()
        .unwrap();
    provinces.remove("1");
    provinces.insert(
        "not-a-province".into(),
        json!({"snow_raw": 6_789}),
    );
    let frame = world_frame_with(rest);
    assert_eq!(
        frame.weather_province_snow_status(),
        replay_core::ViewStatus::Present
    );
    assert_eq!(rest_raw_province_count(&frame), 2);
    assert_eq!(frame.weather_province_snow().by_province.len(), 1);
    assert!(frame
        .weather_province_snow()
        .by_province
        .values()
        .all(|snow| matches!(snow, replay_core::Leaf::Measured(_))));
    assert_owner_proven_unavailable(&frame, "weather.province_snow_accumulator");
}

/// kills: dropping the contributor guard from any of the four unit predicates.
///
/// D18 `country_identity` resolves the owner tag on every unit row and retains
/// its previous map when current-hour identity is unavailable; D32
/// `air_wing_loadouts_live` and D34 `air_base_status_live` back the air-base
/// join. None owns a unit census, so none can be mapped in
/// `DOMAIN_PROBLEM_VIEW_MAP` without invalidating census claims that are still
/// sound — the reconciliation pass leaves them under a warning that says the
/// views "are NOT downgraded and may still present retained state as measured".
/// A predicate that only reads the retained value cannot tell, so it has to
/// read the warning.
#[test]
fn a_problem_on_an_unmapped_contributor_domain_refuses_the_rows_it_feeds() {
    let with_problem = |domain: i64| {
        let mut snapshot = fully_measured_snapshot();
        snapshot.as_object_mut().unwrap().insert(
            "provenance".into(),
            json!({"collector": {"domain_problems": [
                {"domain_id": domain, "problem": "sweep_incomplete"}
            ]}}),
        );
        world_frame_with(snapshot)
    };

    let admitted = |frame: &WorldFrame, input_id: &str| {
        let projection =
            map_authority_projection_from_world_frame(frame, &fixture_manifest()).unwrap();
        requirement(&projection, input_id).disposition
            == MapAuthorityRequirementDisposition::Admitted
    };

    // D18 feeds the owner tag the three enumeration rows claim — but NOT
    // units.air_base_location, whose ledger row promises the base province and
    // state, not the owner. Over-guarding it would refuse a fully proved join.
    let owner = with_problem(18);
    assert_eq!(owner.division_units_status(), ViewStatus::Present);
    assert_eq!(owner.fleets_status(), ViewStatus::Present);
    assert_eq!(owner.wings_status(), ViewStatus::Present);
    for input_id in ["units.divisions", "units.taskforces_fleets", "units.wings"] {
        assert_owner_proven_unavailable(&owner, input_id);
    }
    assert!(
        admitted(&owner, "units.air_base_location"),
        "the air-base join does not claim the owner tag"
    );

    // D56 overlays the raw task-force and fleet identities that
    // units.taskforces_fleets claims, and nothing else these rows read.
    let naval = with_problem(56);
    assert_eq!(naval.fleets_status(), ViewStatus::Present);
    assert_owner_proven_unavailable(&naval, "units.taskforces_fleets");
    for still_admitted in [
        "units.divisions",
        "units.wings",
        "units.air_base_location",
    ] {
        assert!(
            admitted(&naval, still_admitted),
            "D56 must not reach {still_admitted}"
        );
    }

    // D32 and D34 feed only the air-base join, so the enumeration rows stand.
    for domain in [32, 34] {
        let frame = with_problem(domain);
        assert_eq!(frame.wings_status(), ViewStatus::Present, "domain {domain}");
        assert_owner_proven_unavailable(&frame, "units.air_base_location");
        for still_admitted in ["units.divisions", "units.taskforces_fleets", "units.wings"] {
            assert!(
                admitted(&frame, still_admitted),
                "domain {domain} must not reach {still_admitted}"
            );
        }
    }

    // A problem on a domain none of these rows read changes nothing.
    let unrelated = with_problem(86);
    for input_id in [
        "units.divisions",
        "units.taskforces_fleets",
        "units.wings",
        "units.air_base_location",
    ] {
        assert!(
            admitted(&unrelated, input_id),
            "{input_id} must not read an unrelated domain's problem"
        );
    }
}

/// One negative fixture per row can only kill one requirement, so the rest of
/// each unit predicate is killed here. Every case leaves the row's raw
/// `ViewStatus` accessor `Present`, which is exactly what makes the certified
/// layer above it load-bearing.
#[test]
fn each_unit_requirement_refuses_on_its_own() {
    let with = |patch: Value| {
        let mut snapshot = fully_measured_snapshot();
        let object = snapshot.as_object_mut().unwrap();
        for (key, value) in patch.as_object().unwrap() {
            object.insert(key.clone(), value.clone());
        }
        world_frame_with(snapshot)
    };
    let oob = |fleets: Value, wings: Value| {
        json!({"order_of_battle": {"fleets": fleets, "air_wings": wings}})
    };
    let one_fleet = json!([{
        "id": 3, "owner": "ENG",
        "task_forces": [{"id": 71, "location_province": 118}]
    }]);
    let one_wing = json!([{"id": 12, "owner": "SOV", "air_base_province": 907, "state_id": 219}]);

    // units.divisions: a dropped source row, and each identity/join field.
    for divisions in [
        json!([{"div_id": 5, "div_type": 29, "owner": "GER", "location_province": 42}, 7]),
        // Duplicate raw_division_id with DISTINCT div_uids ("5:29" vs "5:31"),
        // so this isolates the raw-id uniqueness guard. The uid-uniqueness
        // guard beside it cannot be isolated by any input, because
        // `division_uid_from_obj` derives the uid from div_id and div_type: a
        // duplicate uid implies a duplicate div_id. It is kept as
        // defence-in-depth against a future uid source, not claimed as killed.
        json!([{"div_id": 5, "div_type": 29, "owner": "GER", "location_province": 42},
               {"div_id": 5, "div_type": 31, "owner": "GER", "location_province": 43}]),
        json!([{"div_id": 5, "div_type": 29, "owner": null, "location_province": 42}]),
        json!([{"div_id": 5, "div_type": 29, "owner": "GER", "location_province": null}]),
        json!([{"div_id": 5, "div_type": null, "owner": "GER", "location_province": 42}]),
    ] {
        let frame = with(json!({"divisions": divisions.clone()}));
        assert_eq!(frame.division_units_status(), ViewStatus::Present, "{divisions}");
        assert_owner_proven_unavailable(&frame, "units.divisions");
    }

    // units.taskforces_fleets: a dropped row, a duplicate raw id, an absent
    // fleet id, and each join field.
    for fleets in [
        json!([{"id": 3, "owner": "ENG",
                "task_forces": [{"id": 71, "location_province": 118}, 9]}]),
        // Duplicate task-force id. Both the uid and raw_task_force_id derive
        // from the same `id` field, so this case cannot separate the two
        // uniqueness guards; it proves the pair refuses a duplicated task force,
        // which is the claim the ledger row actually makes.
        json!([{"id": 3, "owner": "ENG",
                "task_forces": [{"id": 71, "location_province": 118},
                                {"id": 71, "location_province": 119}]}]),
        json!([{"owner": "ENG", "task_forces": [{"id": 71, "location_province": 118}]}]),
        json!([{"id": 3, "task_forces": [{"id": 71, "location_province": 118}]}]),
        json!([{"id": 3, "owner": "ENG", "task_forces": [{"id": 71}]}]),
    ] {
        let frame = with(oob(fleets.clone(), one_wing.clone()));
        assert_eq!(frame.fleets_status(), ViewStatus::Present, "{fleets}");
        assert_owner_proven_unavailable(&frame, "units.taskforces_fleets");
    }

    // units.wings: an unreadable row, a duplicate wing id, an absent owner.
    for wings in [
        json!([{"id": 12, "owner": "SOV", "air_base_province": 907, "state_id": 219}, 4]),
        json!([{"id": 12, "owner": "SOV", "air_base_province": 907, "state_id": 219},
               {"id": 12, "owner": "SOV", "air_base_province": 908, "state_id": 220}]),
        json!([{"id": 12, "air_base_province": 907, "state_id": 219}]),
    ] {
        let frame = with(oob(one_fleet.clone(), wings.clone()));
        assert_eq!(frame.wings_status(), ViewStatus::Present, "{wings}");
        assert_owner_proven_unavailable(&frame, "units.wings");
    }

    // units.air_base_location: the province half of the join, in both of its
    // invalid spellings.
    for wings in [
        json!([{"id": 12, "owner": "SOV", "air_base_province": null, "state_id": 219}]),
        json!([{"id": 12, "owner": "SOV", "air_base_province": 0, "state_id": 219}]),
    ] {
        let frame = with(oob(one_fleet.clone(), wings.clone()));
        assert_eq!(frame.wings_status(), ViewStatus::Present, "{wings}");
        assert_owner_proven_unavailable(&frame, "units.air_base_location");
    }
}

#[test]
fn no_proven_predicate_is_more_permissive_than_its_ledger_accessor() {
    let manifest = fixture_manifest();
    let mut frames = vec![
        world_frame_with(fully_measured_snapshot()),
        world_frame_with(json!({})),
    ];
    for proven in MAP_AUTHORITY_OWNER_PROVEN_INPUTS {
        frames.push(unmeasured_frame(proven.input_id));
    }
    for frame in &frames {
        let projection = map_authority_projection_from_world_frame(frame, &manifest).unwrap();
        for proven in MAP_AUTHORITY_OWNER_PROVEN_INPUTS {
            if requirement(&projection, proven.input_id).disposition
                == MapAuthorityRequirementDisposition::Admitted
            {
                assert_eq!(
                    ledger_accessor_status(frame, proven.input_id),
                    ViewStatus::Present,
                    "{} admitted while its ledger accessor {} refused",
                    proven.input_id,
                    proven.status_accessor
                );
            }
        }
    }
}

#[test]
fn a_proven_input_refuses_when_its_own_payload_is_unmeasured() {
    let manifest = fixture_manifest();
    let mut composites_covered = 0;
    for proven in MAP_AUTHORITY_OWNER_PROVEN_INPUTS {
        let fixture = unmeasured_fixture(proven.input_id);
        let date = fixture.date;
        let frame = world_frame_with_date(fixture.snapshot, date);
        if fixture.ledger_accessor_stays_present {
            composites_covered += 1;
            assert_eq!(
                ledger_accessor_status(&frame, proven.input_id),
                ViewStatus::Present,
                "{}'s fixture no longer leaves its ledger accessor Present, so it proves nothing",
                proven.input_id
            );
        }
        let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
        let row = requirement(&projection, proven.input_id);
        assert_eq!(
            row.disposition,
            MapAuthorityRequirementDisposition::Unavailable,
            "{} admitted on an unmeasured payload",
            proven.input_id
        );
        assert_eq!(row.payload_schema, None, "{}", proven.input_id);
        let blocker = map_authority_requirement_metadata(proven.input_id)
            .unwrap()
            .blocker;
        let label = map_authority_observed_status_label(proven.input_id);
        assert_eq!(
            row.blocker.as_deref(),
            Some(format!("{blocker}; WorldFrame {label} is DomainUnavailable").as_str()),
            "{}",
            proven.input_id
        );
        assert!(!admit_map_authority_projection(&projection)
            .unwrap()
            .is_admitted(proven.input_id));
    }
    assert_eq!(
        composites_covered, 9,
        "the composite-accessor cases are the ones this test exists for"
    );
}

#[test]
fn unmeasuring_one_proven_input_leaves_its_siblings_admitted() {
    let manifest = fixture_manifest();
    for proven in MAP_AUTHORITY_OWNER_PROVEN_INPUTS {
        let projection = map_authority_projection_from_world_frame(
            &unmeasured_frame(proven.input_id),
            &manifest,
        )
        .unwrap();
        for other in MAP_AUTHORITY_OWNER_PROVEN_INPUTS {
            if other.input_id == proven.input_id {
                continue;
            }
            // Documented DATA dependencies, not plumbing wobble: these rows'
            // predicates consume the unmeasured input's own measured view, so
            // fail-closed law REQUIRES them to move. `mapmode.raids` joins
            // every raid identity against the resolved D18 roster (an
            // unresolved numeric id renders as a 3-char decimal string, so
            // shape alone cannot authenticate a tag). The theatre hierarchy's
            // all-country census must EQUAL that same complete roster.
            let consumes_unmeasured_input = matches!(
                (proven.input_id, other.input_id),
                ("country.d18_complete_roster", "mapmode.raids")
                    | ("country.d18_complete_roster", "orders.theatre_army_hierarchy")
            );
            if consumes_unmeasured_input {
                assert_eq!(
                    requirement(&projection, other.input_id).disposition,
                    MapAuthorityRequirementDisposition::Unavailable,
                    "{} consumes {} and must refuse without it",
                    other.input_id,
                    proven.input_id
                );
                continue;
            }
            assert_eq!(
                requirement(&projection, other.input_id).disposition,
                MapAuthorityRequirementDisposition::Admitted,
                "unmeasuring {} moved {}",
                proven.input_id,
                other.input_id
            );
        }
    }
}

#[test]
fn a_proven_row_cannot_be_refused_with_an_observation_it_cannot_make() {
    let mut projection = unresolved_projection();
    let index = projection
        .requirements
        .iter()
        .position(|row| row.input_id == "diplomacy.faction_roster")
        .unwrap();
    let blocker = map_authority_requirement_metadata("diplomacy.faction_roster")
        .unwrap()
        .blocker;
    let label = map_authority_observed_status_label("diplomacy.faction_roster");
    projection.requirements[index].disposition = MapAuthorityRequirementDisposition::Absent;
    projection.requirements[index].blocker =
        Some(format!("{blocker}; WorldFrame {label} is DomainAbsent"));
    let error = admit_map_authority_projection(&projection).unwrap_err();
    assert!(
        error.contains("cannot observe DomainAbsent"),
        "{error}"
    );

    let mut projection = unresolved_projection();
    let index = projection
        .requirements
        .iter()
        .position(|row| row.input_id == "combat.land_active_epoch")
        .unwrap();
    let blocker = map_authority_requirement_metadata("combat.land_active_epoch")
        .unwrap()
        .blocker;
    let label = map_authority_observed_status_label("combat.land_active_epoch");
    projection.requirements[index].disposition = MapAuthorityRequirementDisposition::Absent;
    projection.requirements[index].blocker =
        Some(format!("{blocker}; WorldFrame {label} is DomainAbsent"));
    assert!(admit_map_authority_projection(&projection).is_ok());
}

#[test]
fn the_d18_roster_refuses_every_partial_identity_census() {
    let manifest = fixture_manifest();
    let cases: &[(&str, Value)] = &[
        (
            "missing identity",
            json!({"ENG": {}, "GER": {"_r2_tag_db_id": 1}}),
        ),
        (
            "zero identity",
            json!({"ENG": {"_r2_tag_db_id": 0}, "GER": {"_r2_tag_db_id": 1}}),
        ),
        (
            "duplicate identity",
            json!({"ENG": {"_r2_tag_db_id": 1}, "GER": {"_r2_tag_db_id": 1}}),
        ),
        ("empty census", json!({})),
    ];
    for (name, countries) in cases {
        let frame = world_frame_with(json!({"countries": countries}));
        assert_eq!(
            frame.countries_status(),
            ViewStatus::Present,
            "{name}: fixture no longer keeps the weak accessor Present"
        );
        let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
        assert_ne!(
            requirement(&projection, "country.d18_complete_roster").disposition,
            MapAuthorityRequirementDisposition::Admitted,
            "{name} was admitted as a complete roster"
        );
    }
}

#[test]
fn only_a_witnessed_faction_roster_admits() {
    let manifest = fixture_manifest();
    let admits = |snapshot: Value| {
        let frame = world_frame_with(snapshot);
        assert_eq!(frame.diplomacy().status, ViewStatus::Present);
        let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
        requirement(&projection, "diplomacy.faction_roster").disposition
            == MapAuthorityRequirementDisposition::Admitted
    };
    assert!(
        admits(json!({
            "countries": measured_countries(),
            "diplomacy_world": {
                "domain53_extra": {
                    "faction_roster_status": "measured_nonempty",
                    "faction_rows": [{
                        "faction_id": 1,
                        "members": ["GER", "ENG"],
                        "member_count": 2,
                        "leader": "GER",
                    }],
                },
            },
        })),
        "a witnessed nonempty roster must admit"
    );
    assert!(
        admits(json!({
            "countries": measured_countries(),
            "diplomacy_world": {
                "domain53_extra": {
                    "faction_roster_status": "measured_empty",
                    "faction_rows": [],
                },
            },
        })),
        "a witnessed measured-empty roster is a measurement and must admit"
    );
    assert!(
        !admits(json!({
            "countries": measured_countries(),
            "diplomacy_live": {"faction_members": {"1": ["GER", "ENG"]}},
        })),
        "an unwitnessed legacy roster was admitted as complete"
    );
    assert!(
        !admits(json!({
            "countries": measured_countries(),
            "diplomacy_world": {
                "domain53_extra": {"faction_roster_status": "unavailable"},
            },
        })),
        "an explicitly unavailable modern roster was admitted"
    );
    assert!(
        !admits(json!({
            "countries": measured_countries(),
            "diplomacy_live": {"faction_members": Value::Null},
        })),
        "an unobtained legacy faction carrier was admitted"
    );
    assert!(
        !admits(json!({"countries": measured_countries()})),
        "an absent faction carrier was admitted"
    );
}

#[test]
fn a_faction_member_outside_the_d18_roster_is_not_a_measured_identity() {
    let manifest = fixture_manifest();
    let witnessed = |members: Value, member_count: u64, leader: &str| {
        json!({
            "countries": measured_countries(),
            "diplomacy_world": {
                "domain53_extra": {
                    "faction_roster_status": "measured_nonempty",
                    "faction_rows": [{
                        "faction_id": 1,
                        "members": members,
                        "member_count": member_count,
                        "leader": leader,
                    }],
                },
            },
        })
    };
    let disposition = |snapshot: Value| {
        let frame = world_frame_with(snapshot);
        assert!(
            matches!(
                frame.diplomacy().faction_roster_status(),
                replay_core::world::FactionRosterStatus::MeasuredNonEmpty
            ),
            "fixture no longer decodes as a measured nonempty roster"
        );
        let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
        requirement(&projection, "diplomacy.faction_roster").disposition
    };
    assert_eq!(
        disposition(witnessed(json!(["GER", "999"]), 2, "GER")),
        MapAuthorityRequirementDisposition::Unavailable,
        "an unresolved faction member was admitted as measured identity"
    );
    assert_eq!(
        disposition(witnessed(json!(["GER", "ENG"]), 2, "GER")),
        MapAuthorityRequirementDisposition::Admitted,
        "a fully resolved witnessed roster must still admit"
    );
}

#[test]
fn a_faction_roster_without_a_country_census_is_a_gap_not_a_rejected_projection() {
    let manifest = fixture_manifest();
    let frame = world_frame_with(json!({
        "diplomacy_world": {
            "domain53_extra": {
                "faction_roster_status": "measured_nonempty",
                "faction_rows": [{
                    "faction_id": 1,
                    "members": ["GER", "ENG"],
                    "member_count": 2,
                    "leader": "GER",
                }],
            },
        },
    }));
    assert!(
        matches!(
            frame.diplomacy().faction_roster_status(),
            replay_core::world::FactionRosterStatus::MeasuredNonEmpty
        ),
        "fixture no longer decodes as a measured nonempty roster"
    );
    assert_eq!(
        frame.d18_country_roster().complete_status(),
        ViewStatus::DomainAbsent,
        "fixture must have no country census for this test to mean anything"
    );
    let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
    let row = requirement(&projection, "diplomacy.faction_roster");
    assert_eq!(
        row.disposition,
        MapAuthorityRequirementDisposition::Unavailable,
        "the roster's own DomainAbsent leaked out of the faction predicate"
    );
    let admission = admit_map_authority_projection(&projection).unwrap();
    assert!(!admission.is_admitted("diplomacy.faction_roster"));
}

#[test]
fn lane_hw_rows_with_an_open_producer_defect_stay_unproven() {
    for input_id in [
        "country.current_capital_state",
        "fog.province_visibility_by_pov",
        "fog.spotted_land_by_pov",
        "fog.spotted_taskforces_by_pov",
        "country.presentation_identity",
        "logistics.rail_hub_frames",
        "logistics.rail_topology_state",
        // `mapmode.supply_state` LEFT this list on 2026-08-22 (lane RD): the
        // D62 engine-law dossier landed
        // (extractor/native/runs/rd_d62_supply_law/DOSSIER.md) and
        // MAP_AUTHORITY_OWNER_PROVEN_INPUTS now consumes
        // WorldFrame.hub_motorization for it.
        "mapmode.intelligence",
        // `session.player_country_set` LEFT this list on 2026-08-21 (lane PL).
        //
        // Lane HW's blocker for it was not a producer defect at all — its
        // evidence row is `"_any_capture_carries_it": false` with
        // `session_multiplayer_metadata_live` marked `scheduled: false` in all
        // five captures HW had (t46–t50):
        // extractor/native/runs/hw_admission_a/producer_capture_evidence.json.
        //
        // `captures/oz_tip` schedules the domain and carries it: the D57
        // sidecar publishes `players_by_country` with `state: "known"` and a
        // named human-held row on 13 of 14 sampled hours, the fourteenth being
        // the one injured hour that loses every domain. The premise, not the
        // producer, was what was missing.
        // Witness: extractor/native/runs/pl_xmap18/witness_oz_tip_all.json.
    ] {
        assert!(
            map_authority_owner_proven_admission(input_id).is_none(),
            "{input_id} was promoted while its producer-side blocker is open"
        );
        let metadata = map_authority_requirement_metadata(input_id).unwrap();
        assert!(!metadata.blocker.is_empty(), "{input_id}");
    }
}

#[test]
fn caller_supplied_bytes_cannot_establish_an_admission() {
    let manifest = fixture_manifest();
    let frame = world_frame_with(fully_measured_snapshot());
    let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
    assert_eq!(
        admit_map_authority_projection(&projection)
            .unwrap()
            .admitted()
            .len(),
        MAP_AUTHORITY_OWNER_PROVEN_INPUTS.len()
    );
    let error = admit_map_authority_json(&serde_json::to_vec(&projection).unwrap()).unwrap_err();
    assert!(
        error.contains("derived from the authenticated WorldFrame")
            && error.contains("combat.land_active_epoch"),
        "{error}"
    );
}

#[test]
fn admitted_rows_carry_no_blocker_and_refused_rows_must_keep_theirs() {
    let mut admitted_with_blocker = unresolved_projection();
    let proven =
        map_authority_owner_proven_admission(&admitted_with_blocker.requirements[0].input_id)
            .expect("requirements[0] is an owner-proven row");
    admitted_with_blocker.requirements[0].disposition =
        MapAuthorityRequirementDisposition::Admitted;
    admitted_with_blocker.requirements[0].payload_schema = Some(proven.payload_schema.into());
    admitted_with_blocker.requirements[0].cadence = Some(proven.cadence.into());
    admitted_with_blocker.requirements[0].identity_order = Some(proven.identity_order.into());
    assert!(admit_map_authority_projection(&admitted_with_blocker)
        .unwrap_err()
        .contains("is admitted and carries a blocker"));

    let mut refused_without_blocker = unresolved_projection();
    refused_without_blocker.requirements[0].blocker = None;
    assert!(admit_map_authority_projection(&refused_without_blocker)
        .unwrap_err()
        .contains("is refused without a blocker"));

    let mut contradictory = unresolved_projection();
    let blocker = map_authority_requirement_metadata(&contradictory.requirements[0].input_id)
        .unwrap()
        .blocker;
    contradictory.requirements[0].blocker = Some(format!(
        "{blocker}; WorldFrame {} is Present",
        map_authority_observed_status_label(&contradictory.requirements[0].input_id)
    ));
    assert!(admit_map_authority_projection(&contradictory)
        .unwrap_err()
        .contains("canonical unresolved blocker"));
    let mut unproven_present = unresolved_projection();
    let index = unproven_present
        .requirements
        .iter()
        .position(|row| map_authority_owner_proven_admission(&row.input_id).is_none())
        .unwrap();
    let blocker =
        map_authority_requirement_metadata(&unproven_present.requirements[index].input_id)
            .unwrap()
            .blocker;
    unproven_present.requirements[index].blocker = Some(format!(
        "{blocker}; WorldFrame {} is Present",
        map_authority_observed_status_label(&unproven_present.requirements[index].input_id)
    ));
    assert!(admit_map_authority_projection(&unproven_present).is_ok());
}

fn shipping_performance_run(source: &str) -> &str {
    let start = source
        .find("pub(crate) fn run(")
        .expect("shipping performance run");
    let end = source[start..]
        .find("pub(crate) fn run_stress(")
        .expect("stress benchmark boundary");
    let bounded = &source[start..start + end];
    let function_end = bounded.rfind("\n}").expect("shipping performance end") + 2;
    &bounded[..function_end]
}

fn shipping_app_run(source: &str) -> &str {
    let start = source.find("pub fn run(").expect("shipping app run");
    let end = source[start..]
        .find("#[cfg(test)]")
        .expect("app test-module boundary");
    &source[start..start + end]
}

fn shipping_headless_dump(source: &str) -> &str {
    let start = source
        .find("fn dump_production_recording_frame(")
        .expect("shipping headless dump");
    let end = source[start..]
        .find("fn reject_production_dump_debug_inputs(")
        .expect("next headless function boundary");
    let bounded = &source[start..start + end];
    let function_end = bounded.rfind("\n}").expect("shipping headless end") + 2;
    &bounded[..function_end]
}

fn shipping_composition_start(source: &str) -> &str {
    let start = source
        .find("impl ProductMapCompositionSession {")
        .expect("XMAP22 composition implementation");
    let end = source[start..]
        .find("pub(crate) fn adapter_identity(")
        .expect("XMAP22 composition start boundary");
    &source[start..start + end]
}

fn assert_gate_sequence(name: &str, source: &str) -> usize {
    source
        .find("crate::validate::prepare_product_map_foundation(")
        .or_else(|| source.find("ProductMapCompositionSession::start_for_headless("))
        .or_else(|| source.find("ProductMapCompositionSession::start_for_benchmark("))
        .or_else(|| source.find("ProductMapCompositionSession::start_for_window_target("))
        .or_else(|| source.find("ProductMapFoundationSession::start_with_assigned_stage_schedule("))
        .or_else(|| source.find("ProductMapFoundationSession::start("))
        .unwrap_or_else(|| panic!("{name} does not enter the shared product foundation"))
}

fn assert_shared_foundation_sequence() {
    let source = include_str!("../../replay_app/src/validate.rs");
    for required_sequence in [
        &[
            "product_foundation_assigned_stage_plan()",
            "derive_bundle_content_authority(",
            "execute_foundation_content_preflight(",
            "product_feature_adapters_with_assigned_stage_schedule(",
        ][..],
        &[
            "map_authority_projection_from_world_frame(",
            "admit_map_authority_projection(",
            "prepare_with_xmap12_inputs(",
            "require_fresh_foundation_frame(",
        ][..],
    ] {
        let mut cursor = 0;
        for required in required_sequence {
            let offset = source[cursor..]
                .find(required)
                .unwrap_or_else(|| panic!("shared foundation omits {required}"));
            cursor += offset + required.len();
        }
    }
}

fn compact_rust_source(source: &str) -> String {
    source
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect()
}

#[test]
fn shipping_product_gates_precede_all_presentation_and_device_prerequisites() {
    assert_shared_foundation_sequence();
    let public_api = compact_rust_source(include_str!("../src/lib.rs"));
    assert!(
        !public_api.contains("product_feature_adapters,"),
        "public map_viewer API must not expose the incomplete unscheduled adapter core"
    );
    let app = shipping_app_run(include_str!("../../replay_app/src/app.rs"));
    let headless = shipping_headless_dump(include_str!("../../replay_app/src/headless.rs"));
    let performance =
        shipping_performance_run(include_str!("../../replay_app/src/performance_bench.rs"));

    let app_gate = assert_gate_sequence("app", app);
    let headless_gate = assert_gate_sequence("headless", headless);
    let performance_gate = assert_gate_sequence("performance", performance);

    let app_prefix = &app[..app_gate];
    let headless_prefix = &headless[..headless_gate];
    let performance_prefix = &performance[..performance_gate];
    for (name, prefix) in [
        ("app", app_prefix),
        ("headless", headless_prefix),
        ("performance", performance_prefix),
    ] {
        for forbidden in [
            "WorkloadContract",
            "validate_contract(",
            "production_render_sha256",
            ".production_render(",
            ".production_render_status(",
            "selected_present_sha256",
            ".selected_present",
            "launch_epoch",
            "capture_receipt",
            "open_product_recording_segment_with_authority",
            "authority_sha256",
            "expected_map_operations",
            "expected_selected_present_operations",
            "map_operation_count",
            "selected_present_ui_operation_count",
            ".submit(",
            "ProductObserverUiRuntime::from_playback_content(",
            "Camera::fit(",
            "Window::create(",
            "Gfx::create(",
            "Gfx::create_offscreen(",
            "read_target(",
            "write_bmp(",
        ] {
            assert!(
                !prefix.contains(forbidden),
                "{name} consumes {forbidden} before XMAP refusal"
            );
        }
    }

    let app_condition = app[..app_gate]
        .rfind("if production_recording {")
        .expect("app production-only gate condition");
    assert!(app_condition < app_gate);
    assert!(app[app_gate..].contains("if observer_render_admitted && !production_recording"));

    for (name, source, gate) in [
        ("headless", headless, headless_gate),
        ("performance", performance, performance_gate),
    ] {
        let tail = &source[gate..];
        assert!(
            tail.contains("composition.execute("),
            "{name} does not execute the shared XMAP22 composition"
        );
        for forbidden in [
            "production_render",
            "selected_present",
            ".submit(",
            "ProductObserverUiRuntime",
        ] {
            assert!(
                !tail.contains(forbidden),
                "{name} retains success tail token {forbidden}"
            );
        }
    }
}

#[test]
fn headless_and_recording_benchmark_execute_shared_composition() {
    let headless = shipping_headless_dump(include_str!("../../replay_app/src/headless.rs"));
    let performance =
        shipping_performance_run(include_str!("../../replay_app/src/performance_bench.rs"));
    for (name, source) in [("headless", headless), ("performance", performance)] {
        let gate = assert_gate_sequence(name, source);
        let execution = source[gate..]
            .find("composition.execute(")
            .unwrap_or_else(|| panic!("{name} does not execute the XMAP22 composition"));
        assert!(
            execution > 0,
            "{name} executes before starting the XMAP22 composition"
        );
    }

    let composition = shipping_composition_start(include_str!("../../replay_app/src/app.rs"));
    let foundation = composition
        .find("ProductMapFoundationSession::start_with_assigned_stage_schedule(")
        .expect("XMAP22 composition does not enter the shared product foundation");
    let device = composition
        .find("Gfx::create_offscreen(")
        .expect("XMAP22 composition does not create its offscreen target");
    assert!(
        foundation < device,
        "XMAP22 composition creates a device before content/foundation admission"
    );
}

#[test]
fn shipping_help_and_source_truth_are_bound_to_all_three_foundation_gates() {
    let main = include_str!("../../replay_app/src/main.rs");
    let usage_start = main.find("const USAGE: &str").expect("shipping usage");
    let usage_end = main[usage_start..]
        .find("const SENSITIVE_GAME_FILE_PATHS")
        .expect("usage boundary");
    let usage = &main[usage_start..usage_start + usage_end];
    for required in [
        "executes and measures the complete packet-native map composition with GPU completion, and writes no benchmark receipt unless exhaustive map-family admission and native execution succeed",
        "executes the complete packet-native map composition, and emits no map pixels, BMP, or success receipt unless exhaustive map-family admission and native execution succeed",
    ] {
        assert!(compact_rust_source(usage).contains(&compact_rust_source(required)));
    }

    let headless_source = include_str!("../../replay_app/src/headless.rs");
    let headless_prelude = &headless_source[..headless_source
        .find("use std::io::Write;")
        .expect("headless module prelude boundary")];
    assert!(
        compact_rust_source(headless_prelude).contains(&compact_rust_source(
            "same XMAP22 packet-native composition used by windowed playback",
        ))
    );

    let project = include_str!("../../../../PROJECT.md");
    for required in [
        "Windowed playback",
        "headless frame export",
        "recording benchmark",
        "ProductMapCompositionSession::execute_on_target",
    ] {
        assert!(
            compact_rust_source(project).contains(&compact_rust_source(required)),
            "PROJECT.md omits required product-route token {required}"
        );
    }

    let app = shipping_app_run(include_str!("../../replay_app/src/app.rs"));
    let headless = shipping_headless_dump(headless_source);
    let performance =
        shipping_performance_run(include_str!("../../replay_app/src/performance_bench.rs"));
    assert_gate_sequence("app", app);
    assert_gate_sequence("headless", headless);
    assert_gate_sequence("performance", performance);
}

#[test]
fn content_manifest_absence_is_a_pre_admission_envelope_error() {
    for (name, source, message) in [
        (
            "app",
            shipping_app_run(include_str!("../../replay_app/src/app.rs")),
            "portable playback has no admitted content manifest after binding",
        ),
        (
            "headless",
            shipping_headless_dump(include_str!("../../replay_app/src/headless.rs")),
            "portable playback has no admitted content manifest after binding",
        ),
        (
            "performance",
            shipping_performance_run(include_str!("../../replay_app/src/performance_bench.rs")),
            "performance benchmark recording has no admitted content manifest",
        ),
    ] {
        let structural_error = source
            .find(message)
            .unwrap_or_else(|| panic!("{name} has no stable content-envelope error"));
        let projection = assert_gate_sequence(name, source);
        assert!(
            structural_error < projection,
            "{name} mislabeled a pre-admission content-envelope failure as a family gap"
        );
    }

    let shared_foundation = include_str!("../../replay_app/src/validate.rs");
    assert!(
        shared_foundation.contains("map_authority_projection_from_world_frame("),
        "shared product foundation has no XMAP00 projection"
    );
}
