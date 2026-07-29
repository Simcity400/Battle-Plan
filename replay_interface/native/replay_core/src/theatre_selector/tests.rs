//! Falsifiable admission tests for the replay-side theatre-selector view.
//!
//! Each test names the production change that turns it red.

use super::*;
use serde_json::json;

const RECORDER_COUNTRY: u32 = 23;
const OBSERVER_COUNTRY: u32 = 77;
const EMPTY_COUNTRY: u32 = 91;
const BROKEN_COUNTRY: u32 = 105;

fn raw(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

fn hash(byte: &str) -> String {
    byte.repeat(64 / byte.len())
}

fn content_identity_json() -> Value {
    json!({
        "language": "l_english",
        "localization_content_sha256": hash("7"),
        "ordered_layers": [
            {"ordinal": 0, "layer_kind": "base", "layer_id": "base", "content_sha256": hash("1")},
            {"ordinal": 1, "layer_kind": "mod", "layer_id": "ugc_1234", "content_sha256": hash("2")}
        ]
    })
}

fn expectation() -> ReplayContentExpectation {
    ReplayContentExpectation {
        language: "l_english".into(),
        localization_content_sha256: hash("7"),
        ordered_layers: vec![
            ContentLayer {
                ordinal: 0,
                layer_kind: "base".into(),
                layer_id: "base".into(),
                content_sha256: hash("1"),
            },
            ContentLayer {
                ordinal: 1,
                layer_kind: "mod".into(),
                layer_id: "ugc_1234".into(),
                content_sha256: hash("2"),
            },
        ],
    }
}

fn land_section(owner: u32) -> Value {
    json!({
        "owner_tag_db_id": owner,
        "admission": "admitted",
        "theatres": [{
            "kind": "land_theatre",
            "object_id": 100,
            "owner_tag_db_id": owner,
            "engine_order": 0,
            "display_name": "Western Front",
            "name_provenance": {"source": "engine_runtime_string"},
            "priority_ordinal": null,
            "alert_categories": [3, 9],
            "members": [
                {"kind": "army_group", "object_id": 101, "parent_kind": "land_theatre",
                 "parent_id": 100, "engine_order": 0, "reserve_navy": false,
                 "display_name": null, "name_provenance": null, "priority_ordinal": null},
                {"kind": "army", "object_id": 102, "parent_kind": "army_group",
                 "parent_id": 101, "engine_order": 0, "reserve_navy": false,
                 "display_name": null, "name_provenance": null, "priority_ordinal": null}
            ]
        }],
        "free_air_wings": []
    })
}

fn air_section(owner: u32) -> Value {
    json!({
        "owner_tag_db_id": owner,
        "admission": "admitted",
        "theatres": [{
            "kind": "air_theatre",
            "object_id": 400,
            "owner_tag_db_id": owner,
            "engine_order": 0,
            "display_name": "Home Air",
            "name_provenance": {"source": "engine_runtime_string"},
            "priority_ordinal": null,
            "alert_categories": [],
            "members": [
                {"kind": "air_group", "object_id": 500, "parent_kind": "air_theatre",
                 "parent_id": 400, "engine_order": 0, "reserve_navy": false,
                 "display_name": "Fighter Group",
                 "name_provenance": {"source": "engine_runtime_string"},
                 "priority_ordinal": null},
                {"kind": "air_wing", "object_id": 600, "parent_kind": "air_group",
                 "parent_id": 500, "engine_order": 0, "reserve_navy": false,
                 "display_name": null, "name_provenance": null, "priority_ordinal": null}
            ]
        }],
        "free_air_wings": [{"wing_id": 601, "engine_order": 0, "priority_ordinal": null}]
    })
}

fn frame_json() -> Value {
    json!({
        THEATRE_SELECTOR_FIELD: {
            "schema_version": THEATRE_SELECTOR_SCHEMA_VERSION,
            "authority": THEATRE_SELECTOR_AUTHORITY,
            "executable_sha256": EXACT_A729_EXECUTABLE_SHA256,
            "elapsed_wall_ms": 7,
            "frame_hour": 900,
            "source_sequence": 901,
            "producer_complete": false,
            "content_identity": content_identity_json(),
            "census": [RECORDER_COUNTRY, OBSERVER_COUNTRY, EMPTY_COUNTRY, BROKEN_COUNTRY],
            "sections": [
                land_section(RECORDER_COUNTRY),
                air_section(OBSERVER_COUNTRY),
                {"owner_tag_db_id": EMPTY_COUNTRY, "admission": "admitted",
                 "theatres": [], "free_air_wings": []},
                {"owner_tag_db_id": BROKEN_COUNTRY, "admission": "unavailable",
                 "reason": "identity_authentication_failed"}
            ],
            "uncaptured_controller_fields": REQUIRED_UNCAPTURED_CONTROLLER_FIELDS
        }
    })
}

fn view() -> TheatreSelectorView {
    parse_theatre_selector(&raw(frame_json()))
}

// ---------------------------------------------------------------------------

/// Red when the view stops serving a country other than the recorder's own --
/// the precise failure that blocked XUI07 on the recorder-POV carrier.
#[test]
fn serves_any_selectable_country_not_just_the_recorder() {
    let view = view();
    assert_eq!(view.status, ViewStatus::Present);
    let observer = view.country(OBSERVER_COUNTRY);
    assert_eq!(observer.status, ViewStatus::Present);
    assert_eq!(observer.theatres.len(), 1);
    assert_eq!(observer.theatres[0].kind, ObjectKind::AirTheatre);
    assert_eq!(observer.free_air_wings.len(), 1);
}

/// Red when a country outside the census falls back to another country's
/// section instead of refusing.
#[test]
fn country_outside_the_census_refuses() {
    let missing = view().country(999);
    assert_eq!(missing.status, ViewStatus::DomainUnavailable);
    assert!(missing.theatres.is_empty());
    assert!(missing.rejection.is_some());
}

/// Red when a measured-empty country is reported as unavailable, or an
/// unavailable one is reported as measured empty.
#[test]
fn measured_empty_and_refused_are_distinguishable() {
    let view = view();
    let empty = view.country(EMPTY_COUNTRY);
    assert_eq!(empty.status, ViewStatus::Present);
    assert!(empty.is_measured_empty());

    let broken = view.country(BROKEN_COUNTRY);
    assert_eq!(broken.status, ViewStatus::DomainUnavailable);
    assert!(!broken.is_measured_empty());
    assert_eq!(
        broken.rejection.as_deref(),
        Some("identity_authentication_failed")
    );
}

/// Red when a refused section is allowed to carry rows, which would let a
/// partial prefix reach the panel under an unavailable label.
#[test]
fn refused_section_carrying_rows_refuses_the_frame() {
    let mut value = frame_json();
    value[THEATRE_SELECTOR_FIELD]["sections"][3]["theatres"] = json!([]);
    let view = parse_theatre_selector(&raw(value));
    assert_eq!(view.status, ViewStatus::DomainUnavailable);
}

/// Red when the census/section length check is removed.
///
/// The extra section is deliberately a *duplicate of an admitted one*: a
/// length-blind `zip` would silently ignore it and every other guard would
/// still pass, so only the length equality can catch it.
#[test]
fn extra_section_beyond_the_census_refuses() {
    let mut long = frame_json();
    let extra = land_section(RECORDER_COUNTRY);
    long[THEATRE_SELECTOR_FIELD]["sections"]
        .as_array_mut()
        .unwrap()
        .push(extra);
    assert_eq!(
        parse_theatre_selector(&raw(long)).status,
        ViewStatus::DomainUnavailable
    );
}

/// Red when the census/section ordering check is removed.
#[test]
fn section_order_must_follow_the_census() {
    let mut swapped = frame_json();
    swapped[THEATRE_SELECTOR_FIELD]["sections"]
        .as_array_mut()
        .unwrap()
        .swap(0, 1);
    assert_eq!(
        parse_theatre_selector(&raw(swapped)).status,
        ViewStatus::DomainUnavailable
    );
}

/// Red when the census ascending/nonzero law is dropped.
///
/// The sections are reordered to match the descending census, so the
/// section-order check cannot mask the missing ordering law.
#[test]
fn census_must_be_strictly_ascending() {
    let mut value = frame_json();
    value[THEATRE_SELECTOR_FIELD]["census"] = json!([OBSERVER_COUNTRY, RECORDER_COUNTRY]);
    value[THEATRE_SELECTOR_FIELD]["sections"] = json!([
        air_section(OBSERVER_COUNTRY),
        land_section(RECORDER_COUNTRY),
    ]);
    value[THEATRE_SELECTOR_FIELD]["producer_complete"] = json!(true);
    assert_eq!(
        parse_theatre_selector(&raw(value)).status,
        ViewStatus::DomainUnavailable
    );

    let mut zeroed = frame_json();
    zeroed[THEATRE_SELECTOR_FIELD]["census"] = json!([0, OBSERVER_COUNTRY]);
    zeroed[THEATRE_SELECTOR_FIELD]["sections"] = json!([
        {"owner_tag_db_id": 0, "admission": "admitted", "theatres": [], "free_air_wings": []},
        air_section(OBSERVER_COUNTRY),
    ]);
    zeroed[THEATRE_SELECTOR_FIELD]["producer_complete"] = json!(true);
    assert_eq!(
        parse_theatre_selector(&raw(zeroed)).status,
        ViewStatus::DomainUnavailable
    );
}

/// Red when a theatre row is allowed to name an owner other than its section's.
#[test]
fn cross_owner_theatre_row_refuses() {
    let mut value = frame_json();
    value[THEATRE_SELECTOR_FIELD]["sections"][0]["theatres"][0]["owner_tag_db_id"] =
        json!(OBSERVER_COUNTRY);
    assert_eq!(
        parse_theatre_selector(&raw(value)).status,
        ViewStatus::DomainUnavailable
    );
}

/// Red when contiguous per-kind engine order stops being enforced, which is how
/// a dropped theatre would arrive as a silent prefix.
#[test]
fn non_contiguous_theatre_order_refuses() {
    let mut value = frame_json();
    value[THEATRE_SELECTOR_FIELD]["sections"][0]["theatres"][0]["engine_order"] = json!(1);
    assert_eq!(
        parse_theatre_selector(&raw(value)).status,
        ViewStatus::DomainUnavailable
    );
}

/// Red when the parent-existence law is dropped and an air wing can arrive
/// without the group it hangs from.
#[test]
fn orphan_member_refuses() {
    let mut value = frame_json();
    value[THEATRE_SELECTOR_FIELD]["sections"][1]["theatres"][0]["members"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    assert_eq!(
        parse_theatre_selector(&raw(value)).status,
        ViewStatus::DomainUnavailable
    );
}

/// Red when free-wing ordering stops being contiguous within a section.
#[test]
fn non_contiguous_free_wing_order_refuses() {
    let mut value = frame_json();
    value[THEATRE_SELECTOR_FIELD]["sections"][1]["free_air_wings"][0]["engine_order"] = json!(3);
    assert_eq!(
        parse_theatre_selector(&raw(value)).status,
        ViewStatus::DomainUnavailable
    );
}

/// Red when alert categories are admitted on a non-land theatre.
#[test]
fn alert_categories_on_a_non_land_theatre_refuse() {
    let mut value = frame_json();
    value[THEATRE_SELECTOR_FIELD]["sections"][1]["theatres"][0]["alert_categories"] = json!([1]);
    assert_eq!(
        parse_theatre_selector(&raw(value)).status,
        ViewStatus::DomainUnavailable
    );
}

/// Red when the producer-complete cross-check is removed, letting a frame claim
/// completeness while carrying a refused section.
#[test]
fn producer_complete_contradiction_refuses() {
    let mut value = frame_json();
    value[THEATRE_SELECTOR_FIELD]["producer_complete"] = json!(true);
    assert_eq!(
        parse_theatre_selector(&raw(value)).status,
        ViewStatus::DomainUnavailable
    );
}

/// Red when the executable, schema, or authority identity checks are removed.
#[test]
fn identity_mismatches_refuse() {
    for (path, replacement) in [
        ("executable_sha256", json!("00".repeat(32))),
        ("schema_version", json!(1)),
        ("authority", json!("exact_a729_theatre_selector")),
    ] {
        let mut value = frame_json();
        value[THEATRE_SELECTOR_FIELD][path] = replacement;
        assert_eq!(
            parse_theatre_selector(&raw(value)).status,
            ViewStatus::DomainUnavailable,
            "{path} must be authenticated"
        );
    }
}

/// Red when a missing domain field is reported as unavailable rather than
/// absent -- a recording that never carried domain 98 is a different fact from
/// one whose domain-98 frame was refused.
#[test]
fn absent_domain_is_absent_not_unavailable() {
    assert_eq!(
        parse_theatre_selector(&raw(json!({}))).status,
        ViewStatus::DomainAbsent
    );
    assert_eq!(
        parse_theatre_selector(&raw(json!({THEATRE_SELECTOR_FIELD: Value::Null}))).status,
        ViewStatus::DomainUnavailable
    );
}

// ---------------------------------------------------------------------------
// Replay-side content admission
// ---------------------------------------------------------------------------

/// Red when `authorize` stops comparing replay's own resolved effective-VFS
/// identity with the producer's, which would let a display name rendered under
/// one mod set be shown under another.
#[test]
fn content_identity_mismatch_refuses() {
    let mut wrong_language = expectation();
    wrong_language.language = "l_german".into();
    assert_eq!(
        view().authorize(&wrong_language, 900, None).status,
        ViewStatus::DomainUnavailable
    );

    let mut wrong_loc = expectation();
    wrong_loc.localization_content_sha256 = hash("9");
    assert_eq!(
        view().authorize(&wrong_loc, 900, None).status,
        ViewStatus::DomainUnavailable
    );

    let mut wrong_mods = expectation();
    wrong_mods.ordered_layers.pop();
    assert_eq!(
        view().authorize(&wrong_mods, 900, None).status,
        ViewStatus::DomainUnavailable
    );

    let mut reordered = expectation();
    reordered.ordered_layers.swap(0, 1);
    assert_eq!(
        view().authorize(&reordered, 900, None).status,
        ViewStatus::DomainUnavailable
    );
}

/// Red when a matching identity stops authorizing, i.e. the gate is not merely
/// a refusal that always fires.
#[test]
fn matching_content_identity_authorizes() {
    let authorized = view().authorize(&expectation(), 900, Some(900));
    assert_eq!(authorized.status, ViewStatus::Present);
    assert_eq!(
        authorized.country(OBSERVER_COUNTRY).status,
        ViewStatus::Present
    );
}

/// Red when the frame-hour binding is removed, letting a stale hour's
/// hierarchy render against the current WorldFrame.
#[test]
fn frame_hour_mismatch_refuses() {
    assert_eq!(
        view().authorize(&expectation(), 901, None).status,
        ViewStatus::DomainUnavailable
    );
}

/// Red when the strictly-increasing source-sequence law is dropped, letting a
/// replayed or reordered frame be admitted.
#[test]
fn source_sequence_must_strictly_increase() {
    assert_eq!(
        view().authorize(&expectation(), 900, Some(901)).status,
        ViewStatus::DomainUnavailable
    );
    assert_eq!(
        view().authorize(&expectation(), 900, Some(9_000)).status,
        ViewStatus::DomainUnavailable
    );
    assert_eq!(
        view().authorize(&expectation(), 900, Some(1)).status,
        ViewStatus::Present
    );
}

/// Red when a refused view keeps the sections it parsed before refusing --
/// the only way a refused frame could start answering country lookups with
/// rows.
#[test]
fn a_refused_frame_refuses_every_country() {
    let refused = view().authorize(&expectation(), 42, None);
    for country in [RECORDER_COUNTRY, OBSERVER_COUNTRY, EMPTY_COUNTRY] {
        let section = refused.country(country);
        assert_eq!(section.status, ViewStatus::DomainUnavailable);
        assert!(section.theatres.is_empty());
    }
}

// ---------------------------------------------------------------------------
// Uncaptured fields
// ---------------------------------------------------------------------------

/// Red when a null priority ordinal is parsed into a defaulted zero, which
/// would silently turn an unmeasured field into a measurement.
#[test]
fn null_priority_ordinal_stays_none() {
    let view = view();
    let section = view.country(RECORDER_COUNTRY);
    assert_eq!(section.theatres[0].priority_ordinal, None);
}

/// Red when a measured priority ordinal stops being distinguishable from an
/// unavailable one.
#[test]
fn measured_priority_ordinal_survives() {
    let mut value = frame_json();
    value[THEATRE_SELECTOR_FIELD]["sections"][0]["theatres"][0]["priority_ordinal"] = json!(0);
    let view = parse_theatre_selector(&raw(value));
    assert_eq!(
        view.country(RECORDER_COUNTRY).theatres[0].priority_ordinal,
        Some(0)
    );
}

/// Red when the declared uncaptured-field list stops reaching the consumer,
/// which is how a panel would render a default for a field nobody measured.
#[test]
fn uncaptured_controller_fields_reach_the_consumer() {
    let declared = view().uncaptured_controller_fields;
    for field in REQUIRED_UNCAPTURED_CONTROLLER_FIELDS {
        assert!(
            declared.contains(&field.to_string()),
            "{field} must reach the consumer"
        );
    }
}


// ---------------------------------------------------------------------------
// Defects found by adversarial review
// ---------------------------------------------------------------------------

/// Red when a required hierarchy list may be absent and default to empty. A
/// theatre whose members vanished from the projection would otherwise render
/// as a legitimately empty theatre -- a silent truncation.
#[test]
fn a_missing_required_list_refuses_rather_than_reading_as_empty() {
    for path in ["members", "alert_categories"] {
        let mut value = frame_json();
        value[THEATRE_SELECTOR_FIELD]["sections"][0]["theatres"][0]
            .as_object_mut()
            .unwrap()
            .remove(path);
        assert_eq!(
            parse_theatre_selector(&raw(value)).status,
            ViewStatus::DomainUnavailable,
            "a missing {path} list must refuse"
        );
    }

    let mut value = frame_json();
    value[THEATRE_SELECTOR_FIELD]["sections"][1]
        .as_object_mut()
        .unwrap()
        .remove("free_air_wings");
    assert_eq!(
        parse_theatre_selector(&raw(value)).status,
        ViewStatus::DomainUnavailable
    );
}

/// Red when member engine order stops being checked for contiguity, which is
/// how a dropped member would arrive as a plausible-looking gap.
#[test]
fn non_contiguous_member_order_refuses() {
    let mut value = frame_json();
    value[THEATRE_SELECTOR_FIELD]["sections"][0]["theatres"][0]["members"][0]["engine_order"] =
        json!(42);
    assert_eq!(
        parse_theatre_selector(&raw(value)).status,
        ViewStatus::DomainUnavailable
    );
}

/// Red when member identity is scoped per theatre instead of per section,
/// which would admit one fleet under two navy theatres.
#[test]
fn one_member_under_two_theatres_refuses() {
    let navy = |theatre_id: u32, order: u32| {
        json!({
            "kind": "navy_theatre",
            "object_id": theatre_id,
            "owner_tag_db_id": RECORDER_COUNTRY,
            "engine_order": order,
            "display_name": "Home Fleet Area",
            "name_provenance": {"source": "engine_runtime_string"},
            "priority_ordinal": null,
            "alert_categories": [],
            "members": [{
                "kind": "fleet", "object_id": 7000, "parent_kind": "navy_theatre",
                "parent_id": theatre_id, "engine_order": 0, "reserve_navy": false,
                "display_name": null, "name_provenance": null, "priority_ordinal": null
            }]
        })
    };
    let mut value = frame_json();
    let theatres = value[THEATRE_SELECTOR_FIELD]["sections"][0]["theatres"]
        .as_array_mut()
        .unwrap();
    theatres.push(navy(800, 0));
    theatres.push(navy(801, 1));
    assert_eq!(
        parse_theatre_selector(&raw(value)).status,
        ViewStatus::DomainUnavailable
    );
}

/// Red when a wing may be both grouped and ungrouped in one section.
#[test]
fn a_wing_that_is_both_grouped_and_free_refuses() {
    let mut value = frame_json();
    value[THEATRE_SELECTOR_FIELD]["sections"][1]["free_air_wings"] = json!([
        {"wing_id": 600, "engine_order": 0, "priority_ordinal": null},
        {"wing_id": 601, "engine_order": 1, "priority_ordinal": null}
    ]);
    assert_eq!(
        parse_theatre_selector(&raw(value)).status,
        ViewStatus::DomainUnavailable
    );
}

/// Red when a *missing* priority key is read as an unmeasured field. An
/// explicit null is the producer declaring a hole; absence means the
/// projection is not the shape this consumer was built against.
#[test]
fn a_missing_priority_declaration_refuses() {
    let mut value = frame_json();
    value[THEATRE_SELECTOR_FIELD]["sections"][0]["theatres"][0]
        .as_object_mut()
        .unwrap()
        .remove("priority_ordinal");
    assert_eq!(
        parse_theatre_selector(&raw(value)).status,
        ViewStatus::DomainUnavailable
    );

    let mut member = frame_json();
    member[THEATRE_SELECTOR_FIELD]["sections"][0]["theatres"][0]["members"][0]
        .as_object_mut()
        .unwrap()
        .remove("priority_ordinal");
    assert_eq!(
        parse_theatre_selector(&raw(member)).status,
        ViewStatus::DomainUnavailable
    );
}

/// Red when the consumer's required list drifts from the producer's declared
/// list.
///
/// The two crates share no dependency -- the recording file is their only
/// interface -- so the list is hand-copied on each side and each side asserts
/// against a copy of the other's. The lane test catches producer-side drift;
/// this one catches consumer-side drift. Without both, an edit to either
/// constant alone would pass every test while silently changing which holes
/// are declared.
#[test]
fn replay_required_list_matches_the_producer_declaration() {
    // Kept in sync by hand with
    // `theatre_selector_live_lane::UNCAPTURED_CONTROLLER_FIELDS`.
    const PRODUCER_DECLARED: &[&str] = &[
        "theater_group_settings_view.current_priority_ordinal",
        "theater_group_settings_view.delete_eligibility",
        "orders_group_item.army_training_state",
        "orders_group_item.exercise_stop_at_max_state",
        "orders_group_item.border_conflict_state",
        "navy_theater_fleet_item.live_spotting_state",
        "navy_theater_fleet_item.exercise_stop_at_max_state",
        "theater_group_item.supply_priority_ordinal",
        "theater_group_item.combat_log_ownership_and_eligibility",
        "navy_theater_group_item.current_priority_ordinal",
        "navy_theater_group_item.combat_log_ownership_and_eligibility",
        "reserve_navy_theater_group_item.combat_log_ownership_and_eligibility",
        "new_theater_group_item.current_eligibility",
        "strategic_region_alert_entry.category_to_atlas_frame_law",
        "strategic_region_alert_entry.runtime_glow_state",
        "strategic_region_item.region_and_parent_identity",
        "strategic_region_item.engine_order",
        "strategic_region_item.alert_identities_and_order",
        "strategic_region_item.naval_access_row_input",
        "fleet_row_item_box.split_off_state_and_count",
        "reserve_fleet_row_item_box.split_off_state_and_count",
        "air_theatre_wing_item.active_visual_frame",
        "air_theatre_item.current_priority_ordinal",
        "air_theatre_group_item.current_priority_ordinal",
        "air_theatre_free_wings_item.current_priority_ordinal",
    ];
    let mut required = REQUIRED_UNCAPTURED_CONTROLLER_FIELDS.to_vec();
    let mut declared = PRODUCER_DECLARED.to_vec();
    required.sort_unstable();
    declared.sort_unstable();
    assert_eq!(required, declared);
}

/// Red when a *truncated* declaration is admitted. A producer that declares
/// fewer holes than replay knows exist is claiming to have measured something
/// it did not, and a panel would render a default for it.
#[test]
fn a_truncated_uncaptured_field_declaration_refuses() {
    let mut short = frame_json();
    let mut fields = REQUIRED_UNCAPTURED_CONTROLLER_FIELDS.to_vec();
    let dropped = fields.pop().expect("the required list is not empty");
    short[THEATRE_SELECTOR_FIELD]["uncaptured_controller_fields"] = json!(fields);
    let view = parse_theatre_selector(&raw(short));
    assert_eq!(view.status, ViewStatus::DomainUnavailable);
    assert!(view.diagnostic.unwrap().contains(dropped));
}

/// Red when a duplicated declaration entry is admitted, which would let a
/// producer pad a truncated list back to the expected length.
#[test]
fn a_duplicated_uncaptured_field_declaration_refuses() {
    let mut duped = frame_json();
    let mut fields = REQUIRED_UNCAPTURED_CONTROLLER_FIELDS.to_vec();
    fields.push(REQUIRED_UNCAPTURED_CONTROLLER_FIELDS[0]);
    duped[THEATRE_SELECTOR_FIELD]["uncaptured_controller_fields"] = json!(fields);
    assert_eq!(
        parse_theatre_selector(&raw(duped)).status,
        ViewStatus::DomainUnavailable
    );
}

/// Red when a missing or malformed uncaptured-field declaration silently reads
/// as "nothing is unresolved" -- the reading that would let a panel render a
/// default for a field nobody measured.
#[test]
fn a_missing_uncaptured_field_declaration_refuses() {
    let mut absent = frame_json();
    absent[THEATRE_SELECTOR_FIELD]
        .as_object_mut()
        .unwrap()
        .remove("uncaptured_controller_fields");
    assert_eq!(
        parse_theatre_selector(&raw(absent)).status,
        ViewStatus::DomainUnavailable
    );

    let mut malformed = frame_json();
    malformed[THEATRE_SELECTOR_FIELD]["uncaptured_controller_fields"] = json!([7]);
    assert_eq!(
        parse_theatre_selector(&raw(malformed)).status,
        ViewStatus::DomainUnavailable
    );
}

/// Red when a runtime string is parsed as a localization key, which would let
/// replay re-resolve a player-typed theatre name through its own localization.
#[test]
fn name_provenance_is_preserved() {
    let view = view();
    assert_eq!(
        view.country(RECORDER_COUNTRY).theatres[0].name_provenance,
        NameProvenance::EngineRuntimeString
    );

    let mut value = frame_json();
    value[THEATRE_SELECTOR_FIELD]["sections"][0]["theatres"][0]["name_provenance"] =
        json!({"source": "localization_key", "key": "THEATRE_DEFAULT"});
    let keyed = parse_theatre_selector(&raw(value));
    assert_eq!(
        keyed.country(RECORDER_COUNTRY).theatres[0].name_provenance,
        NameProvenance::LocalizationKey("THEATRE_DEFAULT".into())
    );
}

/// Red when a named member is admitted without provenance.
#[test]
fn named_member_without_provenance_refuses() {
    let mut value = frame_json();
    value[THEATRE_SELECTOR_FIELD]["sections"][1]["theatres"][0]["members"][0]["name_provenance"] =
        Value::Null;
    assert_eq!(
        parse_theatre_selector(&raw(value)).status,
        ViewStatus::DomainUnavailable
    );
}
