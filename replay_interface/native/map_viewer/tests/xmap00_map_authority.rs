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

/// The authoritative XMAP00 semantic ledger. The admission table in
/// `map_authority.rs` is a transcription of this file's `xmap00_disposition`
/// column and must not be able to drift from it.
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
    // An owner-proven family's refusal always names the status the frame
    // reported, because its availability is a fact about the recording rather
    // than a pending proof; an unproven family may be refused with no status
    // at all (the projection has no accessor to consult for it yet).
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

// --------------------------------------------------------------------------
// A WorldFrame built from an in-memory snapshot, so the derived seam can be
// driven at every status a real recording can produce.
// --------------------------------------------------------------------------

struct InMemorySource {
    hours: Vec<i64>,
    rest: Map<String, Value>,
    /// The authenticated calendar stamp `world.calendar_time` is derived from.
    /// It lives on the source frame rather than in `rest`, so it has to be
    /// settable here for that row to be driven at all.
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

/// The engine's `year.month.day.hour` stamp. Every fixture carries one so that
/// `world.calendar_time` is admitted alongside the row under test; the calendar
/// row's own negatives override it.
const MEASURED_CALENDAR_STAMP: &str = "1936.1.1.12";

/// The engine's month lengths: 365-day, **no leap years**
/// Mirrored here so the harness builds frames whose
/// `hour_index` really is the one its `date` encodes — the guard requires the
/// pair to agree, and a fixture violating the schema would pass or fail for the
/// wrong reason.
const ENGINE_MONTH_LENGTHS: [i64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// `((year * 365 + day_of_year_index) * 24 + hour)` for a well-formed stamp.
/// `None` for anything the engine could not emit, so a malformed-stamp fixture
/// can still be built with an arbitrary hour index.
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
    // A frame's hour index is the one its stamp encodes whenever the stamp is
    // well-formed; malformed-stamp fixtures keep an arbitrary hour so they
    // exercise the stamp check rather than the pair check.
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

// --------------------------------------------------------------------------
// Owner-proven fixtures.
//
// Every proven row has exactly one NAMED measured fixture, and the frame in
// which they are all measured at once is their merge — so a measured shape is
// never written twice. Every proven row also has one unmeasured fixture. For
// the rows whose ledger `status_accessor` is a COMPOSITE weaker than the
// admitted payload, the unmeasured fixture deliberately leaves that composite
// `Present` — that is the frame in which reverting the row to its ledger
// accessor would wrongly admit, and `ledger_accessor_stays_present` marks it so
// the test can assert the fixture really is that frame instead of assuming it.
// --------------------------------------------------------------------------

struct NamedSnapshotFixture {
    name: &'static str,
    rest: Value,
}

/// A complete D24 province-snow census.
///
/// `observed_province_rows` is the engine's province-array size and the
/// collector authenticates every row against it, so the measured set is exactly
/// the dense range `0..observed_province_rows` — a census declaring one row can
/// only carry province 0. An earlier version of this fixture declared one row
/// and used province 42, which no recorder can produce; it is kept dense here so
/// the admitting case is a shape that can actually occur.
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

/// Two occupied states with both certified leaves measured, under a D52
/// state-root census that says every root the engine reported produced a row.
///
/// `census_complete` is the producer's verdict, not a count this fixture
/// re-derives: the two counts beside it are the numbers it was computed from and
/// are carried so a reader can see the arithmetic.
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

/// The recorder's membership digest, recomputed in the test rather than pasted
/// as a literal.
///
/// A hard-coded hex constant here would pass whatever the two implementations
/// happened to agree on, including agreeing on nothing: if the producer and the
/// admission predicate ever drifted apart, a literal would keep matching one of
/// them and the fixture would still be "green". Recomputing from the ids means
/// the fixture asserts the same thing the recorder asserts.
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

/// The number of members in the snapshot's raw province object, which is what
/// the guard compares the D24 witness against. Read from the frame rather than
/// the fixture so the test sees exactly what the derivation sees.
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
            "weather.province_snow_accumulator",
            NamedSnapshotFixture {
                name: "authenticated D24 province-snow row",
                rest: province_snow_fixture(),
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
            "diplomacy.faction_roster",
            NamedSnapshotFixture {
                name: "producer-witnessed measured-empty faction roster",
                // Measured-empty asserts no membership identity, so it joins
                // the D18 roster vacuously and admits on its own carrier.
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
                // The stamp lives on the source frame, which
                // `world_frame_with` supplies for every fixture; this row adds
                // nothing to the snapshot body.
                rest: json!({}),
            },
        ),
    ])
}

/// Two countries with a complete, unique, nonzero D18 identity each.
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

/// A snapshot in which every owner-proven input is measured: the merge of the
/// one named fixture each row owns, so there is no second place for a measured
/// shape to be written and go stale.
fn fully_measured_snapshot() -> Value {
    all_owner_proven_fixtures_merged(&owner_proven_fixture_table())
}

/// One proven row's unmeasured fixture.
struct UnmeasuredFixture {
    snapshot: Value,
    /// The source-frame calendar stamp this fixture is built with.
    ///
    /// `world.calendar_time`'s payload is not in the snapshot body at all — it
    /// is the source frame's own `Option<String>` — so its unmeasured fixture
    /// is an hour with no stamp. Every other row keeps the measured stamp, so
    /// unmeasuring it moves nothing else.
    date: Option<&'static str>,
    /// Whether this fixture leaves the row's ledger `status_accessor` reporting
    /// `Present`. True only for rows whose ledger accessor is a composite the
    /// certified predicate strengthens; for a row whose ledger accessor already
    /// IS its payload's fail-closed verdict there is no such frame, and
    /// claiming one would be a fiction.
    ledger_accessor_stays_present: bool,
}

/// The snapshot in which `input_id`'s own payload is not measured.
fn unmeasured_fixture(input_id: &str) -> UnmeasuredFixture {
    let mut snapshot = fully_measured_snapshot();
    let object = snapshot.as_object_mut().unwrap();
    let ledger_accessor_stays_present = match input_id {
        // `combats_status` IS this row's payload verdict: `build_combats`
        // downgrades to `DomainUnavailable` on a malformed row, so there is no
        // frame where the accessor is Present and the census is not measured.
        "combat.land_active_epoch" => {
            object.insert("combats".into(), Value::Null);
            false
        }
        // `ended_combats_status` stays `Present` after the decoder drops a
        // malformed terminal row; only the certified reading sees the loss.
        "combat.land_terminal_detail" => {
            object.insert(
                "ended_combats".into(),
                json!([{"combat_id": 0, "generation": 2, "lifecycle": "ended"}]),
            );
            true
        }
        // The merged naval/air root is `Present` because the AIR partition is
        // (`build_naval_air_combat` combines the naval, sunk-convoy and air
        // statuses, and any `Present` dominates); the naval census itself was
        // explicitly unobtained.
        "combat.naval_air_epoch" => {
            object.insert("naval_combats".into(), Value::Null);
            object.insert(
                "naval_air".into(),
                json!({"_r2_air_combat_losses_live": []}),
            );
            true
        }
        // `d18_country_roster.status` IS this row's payload verdict. ENG has no
        // identity, so the roster is unavailable — and so is the accessor.
        "country.d18_complete_roster" => {
            object.insert("countries".into(), json!({"ENG": {}, "GER": {"_r2_tag_db_id": 1}}));
            false
        }
        // `faction_roster_status` IS this row's payload verdict, over a single
        // atomic leaf. Removing the witness unavailables both.
        "diplomacy.faction_roster" => {
            object.remove("diplomacy_world");
            false
        }
        // `occupation_status` stays `Present` when the producer's own census
        // says a state root was dropped: `build_occupation` sees a perfectly
        // well-formed `states` object and has no way to know the world was
        // larger than the one it was handed. Only the D52 witness sees it.
        "mapmode.occupation_resistance_compliance" => {
            object
                .get_mut("_r2_occupation_census_live")
                .and_then(Value::as_object_mut)
                .expect("the measured fixture carries the D52 census witness")
                .insert("census_complete".into(), json!(false));
            true
        }
        // `weather_province_snow_status` stays `Present` after the decoder
        // discards a non-numeric province key: declared 2 == raw 2, but only one
        // member survived decoding, and only the certified reading sees it.
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
        // The stamp is a source-frame value, so this row's snapshot body is
        // untouched and the `date` field below carries the mutation. `date` is
        // then `None`, which is what its ledger accessor reads too, so there is
        // no frame in which that accessor stays `Present` HERE — the stamp that
        // reads present and still refuses is
        // `a_calendar_stamp_disagreeing_with_its_hour_index_is_not_admitted`.
        "world.calendar_time" => false,
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

/// The frame an unmeasured fixture describes.
fn unmeasured_frame(input_id: &str) -> std::sync::Arc<WorldFrame> {
    let fixture = unmeasured_fixture(input_id);
    world_frame_with_date(fixture.snapshot, fixture.date)
}

/// The accessor the XMAP00 ledger's `status_accessor` column names for each
/// proven row, read off the frame directly. This is deliberately the LEDGER's
/// reading and not the admission table's predicate: the two agreeing or
/// disagreeing is the thing under test.
fn ledger_accessor_status(frame: &WorldFrame, input_id: &str) -> ViewStatus {
    match input_id {
        "combat.land_active_epoch" => frame.combats_status(),
        "combat.land_terminal_detail" => frame.ended_combats_status(),
        "combat.naval_air_epoch" => frame.naval_air_combat_status(),
        "country.d18_complete_roster" => frame.d18_country_roster().status,
        "diplomacy.faction_roster" => match frame.diplomacy().faction_roster_status() {
            replay_core::world::FactionRosterStatus::Unavailable => ViewStatus::DomainUnavailable,
            replay_core::world::FactionRosterStatus::MeasuredEmpty
            | replay_core::world::FactionRosterStatus::MeasuredNonEmpty => ViewStatus::Present,
        },
        "mapmode.occupation_resistance_compliance" => frame.occupation_status(),
        "weather.province_snow_accumulator" => frame.weather_province_snow_status(),
        // The ledger names `date` for this row, and `WorldFrame::date` is an
        // `Option<&str>` rather than a `ViewStatus`: the only reading it
        // supports is whether the hour carries a stamp at all. That WEAK
        // reading is the point — a stamp that is malformed, or that disagrees
        // with `hour_index`, still reads `Present` here, and the property below
        // is that the certified predicate never admits where this refuses.
        "world.calendar_time" => match frame.date() {
            Some(_) => ViewStatus::Present,
            None => ViewStatus::DomainUnavailable,
        },
        other => panic!("no ledger accessor for {other}"),
    }
}

fn unresolved_projection() -> MapAuthorityProjection {
    // Nothing a caller assembles carries a frame-derivation witness, which is
    // exactly the point: this is the untrusted shape.
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

// --------------------------------------------------------------------------
// The derived seam: the ledger's disposition column decides which inputs may
// admit; the authenticated frame decides whether they do.
// --------------------------------------------------------------------------

/// kills: adding, dropping or renaming an entry in
/// `MAP_AUTHORITY_OWNER_PROVEN_INPUTS` (or changing one of its transcribed
/// ledger columns) without the authoritative XMAP00 TSV's own
/// `xmap00_disposition`/`owner_packet`/`portable_path`/`status_accessor`
/// columns agreeing.
#[test]
fn the_owner_proven_admission_table_is_exactly_the_ledgers_disposition_column() {
    let rows = xmap00_ledger_rows();
    let semantic: Vec<&BTreeMap<String, String>> = rows
        .iter()
        .filter(|row| row["class"] == "semantic")
        .collect();
    assert_eq!(semantic.len(), 45, "XMAP00 semantic census drifted");

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
    // Every other semantic row is still refused. The count is deliberately not
    // restated as a literal here: the equality above already pins the set to
    // `non_ui_visual_input_ledger_20260730.tsv`, and a number in this test would
    // only be a second place for one admission to have to be edited.
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
                // An unproven family keeps its named blocker and nothing else.
                let metadata = map_authority_requirement_metadata(id).unwrap();
                assert!(!metadata.blocker.is_empty(), "{id}");
                assert_eq!(metadata.payload_schema, None, "{id}");
            }
        }
    }
}

/// kills: reverting `map_authority_projection_from_world_frame` to build every
/// row through `unavailable_requirement`, or admitting a proven input from
/// anything other than its own view's `Present` status.
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

    // The merged snapshot is the frame in which every proven row is measured at
    // once, so it is also `fully_measured_snapshot`'s only definition — nothing
    // restates a measured fixture in two places.
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

    // Nothing else moved: every still-unproven row keeps its named blocker.
    for gap in admission.gaps() {
        assert!(map_authority_owner_proven_admission(&gap.input_id).is_none());
        assert!(!gap.reason.is_empty(), "{} lost its blocker", gap.input_id);
    }
}

/// kills: admitting a proven input whose view the recording did not measure —
/// i.e. treating an absent or explicitly unobtained collection as data.
#[test]
fn owner_proven_inputs_refuse_with_their_named_blocker_when_unmeasured() {
    let manifest = fixture_manifest();

    // Built without a calendar stamp too: this test asserts that an unmeasured
    // hour admits *nothing*, so the source frame must be as bare as the
    // snapshot body.
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
            // Explicitly unobtained rather than missing: `build_occupation`
            // distinguishes the two, and this projection is the unobtained one.
            "states": Value::Null,
            }),
            None,
        ),
        &manifest,
    )
    .unwrap();

    for proven in MAP_AUTHORITY_OWNER_PROVEN_INPUTS {
        // `world.calendar_time` has no snapshot domain to be absent: its whole
        // payload is a source-frame `Option<String>`, which has no third state
        // between "no key" and "explicit null". Both readings are the same
        // fact — the hour carries no measured stamp — and the guard maps them
        // both to `DomainUnavailable` on purpose. That contract is asserted in
        // `an_unmeasured_calendar_stamp_is_always_unavailable_never_absent`.
        if proven.input_id == "world.calendar_time" {
            continue;
        }
        let blocker = map_authority_requirement_metadata(proven.input_id)
            .unwrap()
            .blocker;
        let label = map_authority_observed_status_label(proven.input_id);
        // `diplomacy.faction_roster`'s carrier is a single `Leaf`, and
        // `faction_roster_status` maps BOTH of its refusal states to
        // `DomainUnavailable` on purpose: "no faction carrier at all" is not
        // positive knowledge of an empty roster, so it fails closed rather than
        // reporting the softer `DomainAbsent`. Every other proven row can tell
        // a missing key from an unobtained one and reports `DomainAbsent`.
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

/// kills: letting a caller mark an unproven family admitted, or letting an
/// admitted row carry a contract other than the one the owner proved.
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

    // A proven row still cannot invent its own contract.
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

/// kills: dropping the frame-derivation witness (or checking it only at the
/// JSON door). Without it a caller can copy an admitted projection's rows into
/// a hand-assembled projection, or edit a derived one, and have the in-process
/// door accept the admission.
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

    // Byte-identical rows, assembled by a caller rather than derived.
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

    // Editing a derived projection breaks the witness too — including edits to
    // rows that are not themselves admitted.
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

    // A projection with no admissions at all needs no witness: the untrusted
    // gap report stays a supported shape.
    assert!(admit_map_authority_projection(&unresolved_projection()).is_ok());
}

/// kills: admitting `combat.naval_air_epoch` on the merged root status alone.
/// That status lets any measured sibling dominate, so an unread naval census
/// would be presented as a measured-empty one.
#[test]
fn the_naval_air_epoch_needs_its_own_partition_measured_not_a_measured_sibling() {
    let manifest = fixture_manifest();
    let sibling_only = world_frame_with(json!({
        "naval_combats": Value::Null,
        "naval_air": {"_r2_air_combat_losses_live": []},
    }));
    // Precondition: the merged root status this input's ledger accessor names
    // really does read Present here, while the naval census does not.
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

    // The naval census on its own is what admits it.
    let measured = world_frame_with(json!({"naval_combats": []}));
    let projection = map_authority_projection_from_world_frame(&measured, &manifest).unwrap();
    assert_eq!(
        requirement(&projection, "combat.naval_air_epoch").disposition,
        MapAuthorityRequirementDisposition::Admitted
    );
}

/// kills: admitting `combat.land_terminal_detail` straight off
/// `ended_combats_status`. That status stays `Present` after the decoder skips
/// malformed terminal rows, so the raw reading turns a census that lost rows
/// into a measured one.
#[test]
fn a_terminal_census_that_dropped_rows_is_not_a_measured_one() {
    let manifest = fixture_manifest();
    let lossy = world_frame_with(json!({"ended_combats": [7]}));
    // Precondition: the raw status really does still read Present, and the
    // decoder really does record the drop in this exact vocabulary.
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

    // A clean terminal census still admits, so the check is a completeness
    // gate and not a blanket refusal.
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

/// A row with NO MEASUREMENT is still a hole, and still costs the owner.
///
/// This used to mutate a row's `province_id` so it contradicted its object key.
/// That shape no longer exists: the producer stopped repeating the key inside
/// the row, because it authenticated nothing -- both were written from the same
/// field -- and cost a copy of every province number 13,414 times per game
/// hour. Left as it was, this test would have gone on asserting a refusal
/// production deliberately removed (Codex review round 8).
///
/// kills: admitting `weather.province_snow_accumulator` when a declared row
/// carries no readable `snow_raw`.
#[test]
fn a_province_snow_row_without_a_measurement_is_not_admitted() {
    let mut rest = province_snow_fixture();
    rest["map"]["_r2_weather_live"]["provinces"]["1"] = json!({"mud_present": true});
    let frame = world_frame_with(rest);
    assert_eq!(
        frame.weather_province_snow_status(),
        replay_core::ViewStatus::Present
    );
    // ABSENT, not Unobtained: the member is not there at all, which is a
    // different fact from "there and unreadable" -- and both are holes, which
    // is what the owner below is proven unavailable by.
    assert_eq!(
        frame.weather_province_snow().by_province.get(&1),
        Some(&replay_core::Leaf::Absent)
    );
    assert_owner_proven_unavailable(&frame, "weather.province_snow_accumulator");
}

/// kills: dropping the in-range check from `province_snow_accumulator_status`.
/// `observed_province_rows` is the engine's array size, so the measured set is
/// the dense range `0..declared`. Substituting an out-of-range province for a
/// lost one keeps the declared, raw and decoded counts all equal, produces no
/// warning, and leaves every retained leaf measured — counting alone cannot see
/// it.
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
    // Declared 2, raw 2, decoded 2 — every count agrees, and province 1 is gone.
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

/// kills: dropping the `census_complete` conjunct from
/// `occupation_resistance_compliance_status`. A state root the collector could
/// not read leaves NO other trace on the frame: the snapshot is a complete,
/// well-formed description of a SMALLER world, `build_occupation` reports
/// `Present`, every leaf is `Measured`, the raw and decoded counts agree with
/// each other, and no warning is raised. The recorder's own verdict is the only
/// thing that contradicts it.
///
/// The fixture is a producer-representative dropped root: three roots observed,
/// two rows emitted, one counted unreadable at header `+52`, and the recorder
/// therefore calling its own census incomplete.
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

/// kills: treating a missing or null witness as "nothing was missing". An
/// absent count is not a statement of completeness, and an explicit null is the
/// recorder saying its own d52 record was refused this hour -- the loudest
/// possible reason not to admit the census that record was supposed to produce.
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

/// kills: comparing the decoded membership against itself, or against the raw
/// `states` object without filtering to the members that carry an `occupation`
/// object. `build_occupation` drops a non-numeric key with a bare `continue`
/// and no warning, and the producer's census was fixed before that drop
/// happened, so `census_complete` stays true and cannot see it.
///
/// The fixture is built so that ONLY that comparison can refuse it: the added
/// state's key is numeric and its value is an object, so the structural walk is
/// content; and the witness names all three ids, so the digest and the published
/// count both agree with the raw object. What the decoder will not accept is the
/// wrong-typed `occupation` member, which it skips with no warning at all.
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
    // Raw carries three occupation members; only two survived decoding.
    assert_eq!(frame.occupation_by_state().len(), 2);
    assert_owner_proven_unavailable(&frame, "mapmode.occupation_resistance_compliance");
}

/// kills: trusting the frame's own self-consistency. A whole well-formed member
/// is gone here, so the raw object is structurally intact, raw and decoded agree
/// at the smaller number, every surviving leaf is measured, and
/// `census_complete` is still true because the producer computed it before the
/// loss. Only the recorder's own published witness contradicts the frame.
///
/// A lost member moves the published count AND the digest together, so this test
/// deliberately claims NEITHER conjunct individually -- removing either one alone
/// leaves the other refusing. What it pins is that the recorder's witness is
/// consulted at all. The two conjuncts are separated elsewhere:
/// `an_occupation_census_that_substituted_one_state_for_another_is_not_admitted`
/// isolates the digest, and the two witness-field tests isolate each field's
/// absence.
#[test]
fn an_occupation_census_that_lost_a_member_after_the_witness_is_not_admitted() {
    let mut rest = occupation_census_fixture();
    rest["states"].as_object_mut().unwrap().remove("6");
    let frame = world_frame_with(rest);
    assert_eq!(frame.occupation_status(), replay_core::ViewStatus::Present);
    // Every self-consistent check passes: one raw member, one decoded row,
    // measured, structurally intact, and the producer said the census was whole.
    assert_eq!(frame.occupation_by_state().len(), 1);
    assert_owner_proven_unavailable(&frame, "mapmode.occupation_resistance_compliance");
}

/// kills: treating a witness with no `published_state_rows` as satisfying the
/// size check. An older recorder's witness omits the field entirely, and
/// defaulting it to the observed count -- or skipping the comparison when it is
/// missing -- would admit exactly the losses the field was added to catch.
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

/// kills: checking cardinality and calling it membership. This frame swaps
/// state 6 for state 7: the structural walk passes, raw and decoded counts are
/// both two, `published_state_rows` is two, every leaf is `Measured`, and
/// `census_complete` is true — every count-based conjunct agrees, and the census
/// has still lost a state and invented another. Only the digest over the
/// ascending ids sees it. Found by review round 2, not by foresight.
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

/// kills: treating a witness with no `published_state_digest` as satisfying the
/// membership check, or accepting a digest the frame does not reproduce.
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

/// kills: dropping the per-row measured check. `build_occupation` retains a row
/// whose value failed its own read as an `Unobtained` leaf and still reports
/// `Present`, and this input IS the per-state resistance/compliance pair, so a
/// membership-only guard would admit a census whose values are missing.
#[test]
fn an_occupation_row_whose_value_was_not_measured_is_not_admitted() {
    let mut rest = occupation_census_fixture();
    rest["states"]["6"]["occupation"]["compliance"] = Value::Null;
    let frame = world_frame_with(rest);
    assert_eq!(frame.occupation_status(), replay_core::ViewStatus::Present);
    assert_eq!(frame.occupation_by_state().len(), 2);
    assert_owner_proven_unavailable(&frame, "mapmode.occupation_resistance_compliance");
}

/// kills: admitting `weather.province_snow_accumulator` when the D24 row-count witness proves a member was lost before the snapshot.
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

/// kills: dropping the measured-range check from
/// `province_snow_accumulator_status`. A declared zero is a failed read, not a
/// measured-empty census — the collector emits no rows and flags the domain
/// `SOURCE_UNAVAILABLE` — but every count equality holds vacuously at
/// `0 == 0 == 0`, so counting alone would admit the failure as complete data.
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
    // Nothing to see: no rows, no unmeasured leaves, no warnings, and every
    // count agrees with every other.
    assert!(frame.weather_province_snow().by_province.is_empty());
    assert!(!frame.coverage_warnings().iter().any(|warning| {
        warning.starts_with("map._r2_weather_live.provinces.") && warning.contains("row ignored")
    }));
    assert_owner_proven_unavailable(&frame, "weather.province_snow_accumulator");
}

/// kills: treating an absent D24 `observed_province_rows` witness as proof that no province-snow member is missing.
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
    // The census itself is intact — two measured rows, nothing dropped. The
    // only thing missing is the witness that would prove nothing was lost.
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

/// kills: giving `world.calendar_time` an absent/unobtained distinction it has
/// no evidence for. `RawFrame::date` is an `Option<String>`: a missing stamp
/// and an explicitly unrecorded one are the same value, so the guard must
/// report the same refusal for both rather than invent a difference. Reporting
/// `DomainAbsent` for either would claim the recorder said something it did not.
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

/// kills: wiring the calendar derivation to something in the snapshot body.
/// Dropping the source-frame stamp must refuse that row and leave every other
/// proven row admitted — the same independence property
/// `each_proven_input_tracks_only_its_own_view` asserts for the rows whose
/// mutation lives in the snapshot.
#[test]
fn a_missing_calendar_stamp_moves_no_other_row() {
    let manifest = fixture_manifest();
    let snapshot = all_owner_proven_fixtures_merged(&owner_proven_fixture_table());
    let projection =
        map_authority_projection_from_world_frame(&world_frame_with_date(snapshot, None), &manifest)
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

/// kills: validating the calendar with Gregorian leap years. the game's calendar is
/// 365-day with **no** leap years (`compact_contract/src/calendar.rs`), so February always has 28 days and
/// `1936.2.29` — a real Gregorian leap day — is a date the engine cannot emit.
///
/// Each frame carries the hour index a **lenient** guard would compute for its
/// stamp, so the pair check agrees and only the month-length rule can refuse.
/// Supplying an unrelated hour would let the pair check pass this test even
/// with the month table removed.
#[test]
fn a_gregorian_leap_day_is_not_an_engine_date() {
    for gregorian_only in ["1936.2.29.0", "2000.2.29.23", "1960.2.29.12"] {
        assert_stamp_refused_on_its_own_terms(gregorian_only);
    }
}

/// kills: checking the calendar day against a month-independent `1..=31`.
#[test]
fn a_calendar_stamp_naming_a_day_its_month_lacks_is_not_admitted() {
    for impossible in [
        "1936.4.31.12",  // April has 30 days
        "1936.6.31.12",  // June has 30
        "1936.9.31.12",  // September has 30
        "1936.11.31.12", // November has 30
        "1936.1.0.12",   // day zero
    ] {
        assert_stamp_refused_on_its_own_terms(impossible);
    }
}

/// kills: dropping the month, hour or field-count checks. Every stamp here is
/// paired with the hour a lenient guard would derive from it, so the pair check
/// cannot be what refuses.
#[test]
fn a_calendar_stamp_outside_the_engine_grammar_is_not_admitted() {
    for malformed in [
        "1936.13.1.12", // month past December
        "1936.0.1.12",  // month zero
        "1936.1.1.24",  // hour past 23
        "1936.1.1",     // three fields
        "1936.1.1.12.0", // five fields
        "1936.1.1.x",   // non-numeric
        "",
    ] {
        assert_stamp_refused_on_its_own_terms(malformed);
    }
}

/// kills: admitting a stamp outside the producer's fail-closed hour window.
/// `raw_date_in_range` is inclusive from the first hour of 1850 to the **first
/// hour of 2200** — not all of 2200 — so `2200.1.1.1` is already out.
#[test]
fn a_calendar_stamp_outside_the_producer_window_is_not_admitted() {
    for outside in [
        "1849.12.31.23", // one hour before the window opens
        "2200.1.1.1",    // one hour after it closes
        "2200.12.31.23", // deep inside year 2200, far past the bound
        "2201.1.1.0",
    ] {
        assert_stamp_refused_on_its_own_terms(outside);
    }
}

/// kills: accepting a noncanonical spelling of a real date. The schema says the
/// engine does not zero-pad, and `date_from_raw` formats unsigned fields, so
/// neither padding nor a leading `+` is a string any recorder writes — even
/// though Rust's unsigned parser accepts both.
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

/// Refuse a stamp while giving it every other chance to admit: the frame's
/// `hour_index` is the one a guard that had *skipped the rule under test* would
/// have derived, so the pair check agrees and the refusal can only come from
/// the stamp rule itself.
fn assert_stamp_refused_on_its_own_terms(stamp: &str) {
    let lenient = lenient_hour_index(stamp).unwrap_or(7);
    let frame = world_frame_with_date_at(json!({}), Some(stamp), lenient);
    assert_eq!(frame.date(), Some(stamp));
    assert_owner_proven_unavailable(&frame, "world.calendar_time");
}

/// `((year * 365 + day_of_year_index) * 24 + hour)` computed **without** the
/// month-length, grammar or window rules — what a guard missing the rule under
/// test would produce. Deliberately permissive: it accepts a signed or padded
/// field, a 31st of April and a hour of 24, so the negatives above can hand the
/// frame a self-consistent pair.
fn lenient_hour_index(date: &str) -> Option<i64> {
    // Takes the FIRST four fields and ignores any extra, which is what a guard
    // missing the field-count rejection would do. Anything less than four is
    // genuinely unparseable.
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

/// kills: validating `date` without checking it against `hour_index`. The
/// admitted payload is the **pair**, `hour_index` is derived from `date`
/// and this stamp is perfectly well-formed — only
/// the sort key disagrees with it.
#[test]
fn a_calendar_stamp_disagreeing_with_its_hour_index_is_not_admitted() {
    let honest = schema_hour_index(MEASURED_CALENDAR_STAMP).expect("fixture stamp is well-formed");
    let frame = world_frame_with_date_at(json!({}), Some(MEASURED_CALENDAR_STAMP), honest + 1);
    assert_eq!(frame.date(), Some(MEASURED_CALENDAR_STAMP));
    assert_owner_proven_unavailable(&frame, "world.calendar_time");
}

/// kills: a stamp check so strict it refuses real recordings. These are dates
/// the engine does emit, each with the hour index the schema derives from it,
/// including both ends of the producer's inclusive window.
#[test]
fn engine_emitted_calendar_stamps_are_admitted() {
    let manifest = fixture_manifest();
    for stamp in [
        "1936.1.1.0",
        "1936.1.1.12",
        "1940.9.2.21",
        "1936.2.28.23",
        "1936.12.31.23",
        "1850.1.1.0", // first hour of the window
        "2200.1.1.0", // last hour of the window, inclusive
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

/// kills: admitting `world.calendar_time` for an hour that carries no stamp.
/// This is the case the old derivation could not distinguish from a measured
/// one, because it passed `None` for every hour alike.
#[test]
fn an_hour_with_no_calendar_stamp_is_not_admitted() {
    let frame = world_frame_with_date(json!({}), None);
    assert!(frame.date().is_none());
    assert_owner_proven_unavailable(&frame, "world.calendar_time");
}

/// kills: dropping the `raw == decoded` conjunct from
/// `province_snow_accumulator_status`.
///
/// A non-numeric province key is discarded by the decoder, so the raw object
/// keeps **2** members while `by_province` decodes only **1**. The witness stays
/// at the fixture's 2, which makes `declared == raw` hold (2 == 2) and every
/// other conjunct pass — every retained leaf is measured and in range, and the
/// declared count is inside the producer's range. **Only `raw == decoded`
/// refuses**, so removing it turns this test red and nothing else does.
///
/// The same conjunct is what catches a duplicate key collapsing two members
/// onto one id (`"1"` and `"01"`); that shape is not tested separately because
/// it exercises no additional conjunct.
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
    // Declared 2 == raw 2, but only one member survived decoding.
    assert_eq!(rest_raw_province_count(&frame), 2);
    assert_eq!(frame.weather_province_snow().by_province.len(), 1);
    assert!(frame
        .weather_province_snow()
        .by_province
        .values()
        .all(|snow| matches!(snow, replay_core::Leaf::Measured(_))));
    assert_owner_proven_unavailable(&frame, "weather.province_snow_accumulator");
}

/// kills: pointing any proven row's `status` predicate at a view that reports a
/// measurement its own payload did not get. The property is one-directional and
/// holds for every proven row regardless of how strong its ledger accessor is:
/// the certified predicate may be STRICTER than the ledger's
/// `status_accessor`, never more permissive. A row wired to a neighbouring view
/// — the defect the old combinatorial test tried and failed to express — breaks
/// it on the frames below, because each unmeasures exactly one carrier.
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

/// kills: weakening any proven row's predicate so that its own unmeasured
/// payload still admits. For the three rows whose ledger accessor is a composite
/// (`ended_combats_status`, `naval_air_combat_status`,
/// `weather_province_snow_status`), the fixture ALSO leaves that composite
/// `Present` — asserted, not assumed — so reverting the row's `status` field to
/// its ledger accessor makes exactly that row admit.
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
        composites_covered, 4,
        "the composite-accessor cases are the ones this test exists for"
    );
}

/// kills: unmeasuring one proven input silently moving another. Each fixture
/// mutates one carrier; every proven row that does not read that carrier must
/// still admit.
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

/// kills: accepting a refusal that claims an observation the row's predicate
/// cannot make. `faction_roster_status` never reports `DomainAbsent` —
/// `build_diplomacy` forces a missing carrier to `Leaf::Unobtained` — and a
/// gap-only projection carries no frame witness, so without the
/// `can_report_absent` check a caller could hand back a fabricated absence and
/// have it validated.
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

    // A row whose predicate CAN observe an absence keeps the same form legal.
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

/// kills: admitting the D18 roster from a partial identity census. Each row
/// below keeps `countries_status` `Present` and breaks the roster in one way
/// `exact_d18_country_roster` is documented to refuse.
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

/// kills: admitting the faction roster on the diplomacy root, collapsing a
/// measured-empty roster into an unavailable one, or — the case Codex's
/// round-2 review found — admitting the LEGACY `diplomacy_live.faction_members`
/// object. Every frame below keeps `diplomacy.status` `Present` through the
/// `countries` sibling, so the root alone cannot distinguish any of them.
///
/// The legacy reader accepts any well-formed nonempty object as measured: it
/// has no expected faction count and no completeness witness, so a recording
/// that captured one faction and missed another is indistinguishable from a
/// complete one. Only the modern D53 route carries the producer's own coverage
/// claim, and only that may admit. A measured-EMPTY roster under that witness
/// is a measurement and must admit.
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
                    // The modern row schema `parse_domain53_faction_rows`
                    // enforces: numeric nonzero `faction_id`, a nonempty
                    // duplicate-free TAG array, a `member_count` that agrees
                    // with it, and a `leader` drawn from the members.
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

/// kills: dropping the member-identity join from `faction_roster_status`.
///
/// When the recorder cannot map a member's `tag_db_id` to a TAG it emits the
/// numeric id rather than fabricating one, and `parse_domain53_faction_rows`
/// accepts any nonempty string as a member. A witnessed row naming an
/// unresolved member therefore decodes as a measured nonempty roster, and
/// without the join it would be admitted as a measured membership census while
/// that member is explicitly unidentified. Found by Codex's round-4 review.
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
        // The precondition the whole test rests on: replay_core really does
        // decode the unresolved member as a MEASURED roster, so the refusal
        // below can only come from the admission predicate.
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
    // "999" is what the fold emits for a member whose tag_db_id it could not
    // resolve; the D18 roster is GER/ENG only.
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

/// kills: propagating the D18 roster's own status out of `faction_roster_status`.
///
/// The faction carrier is independent of the `countries` key, so a witnessed
/// nonempty roster can be measured on a frame that carries no country census at
/// all. `complete_d18_roster` reports `DomainAbsent` for that frame; returning
/// it verbatim would describe a different view's absence, and because this row
/// declares `can_report_absent: false` the derived projection would then be
/// REJECTED WHOLESALE by `validate_requirement` instead of recording a gap —
/// one row's missing identity would cost the entire admission report.
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
        frame.d18_country_roster().status,
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
    // The whole projection must still admit as a gap report.
    let admission = admit_map_authority_projection(&projection).unwrap();
    assert!(!admission.is_admitted("diplomacy.faction_roster"));
}

/// Promoting any id below without closing its producer-side defect would put a
/// row into the admission table that no capture can satisfy. The structured
/// `non_ui_visual_input_ledger_20260730.tsv` classifies each row as unavailable.
#[test]
fn lane_hw_rows_with_an_open_producer_defect_stay_unproven() {
    for input_id in [
        // `capital_state` is minted only by the collector's direct-JSON country
        // renderer; `git grep -c capital_state HEAD -- extractor/native/recorder_host/`
        // returns no hits, so the fold never writes it into the snapshot.
        "country.current_capital_state",
        // XMAP05 production needs the all-country `complete_pov_roster` FOW1
        // census; `complete_pov_roster` appears nowhere in `extractor/`.
        "fog.province_visibility_by_pov",
        "fog.spotted_land_by_pov",
        "fog.spotted_taskforces_by_pov",
        // The producer spells a legitimately-absent cosmetic tag as JSON null,
        // which arrives at the typed seam as `Unobtained`.
        "country.presentation_identity",
        // `build_rail_replay` admits only `exact_a729_crailwaygfx_carrier`; the
        // recorder emits `exact_a729_supply_simulation_tables`.
        "logistics.rail_hub_frames",
        "logistics.rail_topology_state",
        "mapmode.supply_state",
        // The ledger's own evidence anchor for this row is `map_authority.rs`
        // itself and its engine object reads "intelligence projection
        // unresolved"; promoting it would be circular.
        "mapmode.intelligence",
        // D57 is scheduled in none of the committed captures.
        "session.player_country_set",
    ] {
        assert!(
            map_authority_owner_proven_admission(input_id).is_none(),
            "{input_id} was promoted while its producer-side blocker is open"
        );
        let metadata = map_authority_requirement_metadata(input_id).unwrap();
        assert!(!metadata.blocker.is_empty(), "{input_id}");
    }
}

/// kills: dropping the JSON door's admission refusal, which would let bytes
/// that never saw a WorldFrame assert that a proven input was measured.
#[test]
fn caller_supplied_bytes_cannot_establish_an_admission() {
    let manifest = fixture_manifest();
    let frame = world_frame_with(fully_measured_snapshot());
    let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
    // The same value admits in process, where it was derived from the frame.
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

/// kills: letting an admitted row keep a blocker, or a refused row drop one —
/// either would make the gap report's reason strings unreliable.
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

    // A proven input cannot be refused while its own view reported Present:
    // the derivation turns that status into an admission, so the pairing is a
    // contradiction rather than a conservative refusal.
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
    // The same pairing stays legal for a family whose owner still owes a proof.
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
