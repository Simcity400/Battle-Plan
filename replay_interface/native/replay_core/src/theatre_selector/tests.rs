use super::*;
use serde_json::json;

const GER: u32 = 23;
const ENG: u32 = 77;
const EMPTY: u32 = 91;
const BROKEN: u32 = 105;

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

fn empty_body(owner: u32) -> Value {
    json!({
        "owner_tag_db_id": owner,
        "admission": "admitted",
        "theatres": [],
        "free_air_wings": [],
        "free_air_priority": {
            "selected_priority_ordinal": null,
            "priority_census_wing_ids": []
        },
        "strategic_regions": []
    })
}

fn land_section(owner: u32) -> Value {
    json!({
        "owner_tag_db_id": owner,
        "admission": "admitted",
        "theatres": [{
            "kind": "land_theatre",
            "selector_included": true,
            "object_id": 100,
            "owner_tag_db_id": owner,
            "engine_order": 0,
            "display_name": "Western Front",
            "name_provenance": {"source": "engine_runtime_string"},
            "controller": {
                "kind": "land",
                "priority_ordinal": 2,
                "delete_eligible": true
            },
            "land_alert_categories": [3, 9],
            "members": [
                {
                    "kind": "army_group", "object_id": 101,
                    "parent_kind": "land_theatre", "parent_id": 100,
                    "engine_order": 0, "reserve_navy": false,
                    "display_name": null, "name_provenance": null,
                    "controller": {
                        "kind": "army_group", "training_visible": true,
                        "stop_at_max_visible": true, "border_conflict_visible": false
                    }
                },
                {
                    "kind": "army", "object_id": 102,
                    "parent_kind": "army_group", "parent_id": 101,
                    "engine_order": 0, "reserve_navy": false,
                    "display_name": null, "name_provenance": null,
                    "controller": {
                        "kind": "army", "training_visible": false,
                        "stop_at_max_visible": false, "border_conflict_visible": true
                    }
                }
            ]
        }],
        "free_air_wings": [],
        "free_air_priority": {
            "selected_priority_ordinal": null,
            "priority_census_wing_ids": []
        },
        "strategic_regions": [{
            "owner_tag_db_id": owner,
            "engine_region_index": 7,
            "display_name": "Western Germany",
            "engine_order": 0,
            "parent": "selector_root",
            "pinned_frame": 2,
            "background_frame": 3,
            "naval_access": {
                "access_slot": 4, "applicable": true,
                "raw_result": 2, "visible_frame": 3
            },
            "alerts": [{
                "alert_id": 22, "engine_order": 0,
                "atlas_frame": 8, "glow": "red"
            }]
        }]
    })
}

fn air_section(owner: u32) -> Value {
    json!({
        "owner_tag_db_id": owner,
        "admission": "admitted",
        "theatres": [{
            "kind": "air_theatre",
            "selector_included": true,
            "object_id": 400,
            "owner_tag_db_id": owner,
            "engine_order": 0,
            "display_name": "Home Air",
            "name_provenance": {"source": "engine_runtime_string"},
            "controller": {
                "kind": "air",
                "selected_priority_ordinal": 4,
                "priority_census_wing_ids": [600]
            },
            "land_alert_categories": [],
            "members": [
                {
                    "kind": "air_group", "object_id": 500,
                    "parent_kind": "air_theatre", "parent_id": 400,
                    "engine_order": 0, "reserve_navy": false,
                    "display_name": "Fighter Group",
                    "name_provenance": {"source": "engine_runtime_string"},
                    "controller": {"kind": "air_group_negative_no_priority"}
                },
                {
                    "kind": "air_wing", "object_id": 600,
                    "parent_kind": "air_group", "parent_id": 500,
                    "engine_order": 0, "reserve_navy": false,
                    "display_name": null, "name_provenance": null,
                    "controller": {"kind": "air_wing", "active_visual_frame": 5, "bucket_key": 17}
                }
            ]
        }],
        "free_air_wings": [
            {"wing_id": 601, "engine_order": 0, "active_visual_frame": 3, "bucket_key": 19},
            {"wing_id": 602, "engine_order": 1, "active_visual_frame": 4, "bucket_key": 20}
        ],
        "free_air_priority": {
            "selected_priority_ordinal": 1,
            "priority_census_wing_ids": [601]
        },
        "strategic_regions": []
    })
}

fn navy_section(owner: u32) -> Value {
    json!({
        "owner_tag_db_id": owner,
        "admission": "admitted",
        "theatres": [{
            "kind": "navy_theatre",
            "selector_included": true,
            "object_id": 700,
            "owner_tag_db_id": owner,
            "engine_order": 0,
            "display_name": "Home Fleet",
            "name_provenance": {"source": "engine_runtime_string"},
            "controller": {"kind": "navy", "priority_ordinal": 3},
            "land_alert_categories": [],
            "members": [
                {
                    "kind": "fleet", "object_id": 701,
                    "parent_kind": "navy_theatre", "parent_id": 700,
                    "engine_order": 0, "reserve_navy": false,
                    "display_name": null, "name_provenance": null,
                    "controller": {
                        "kind": "fleet",
                        "split_off_count": 2, "split_off_visible": true
                    }
                },
                {
                    "kind": "task_force", "object_id": 702,
                    "parent_kind": "fleet", "parent_id": 701,
                    "engine_order": 0, "reserve_navy": false,
                    "display_name": null, "name_provenance": null,
                    "controller": {
                        "kind": "task_force", "spotting_visible": true,
                        "training_visible": false, "stop_at_max_visible": false
                    }
                },
                {
                    "kind": "reserve_fleet_aggregate", "object_id": 703,
                    "parent_kind": "navy_theatre", "parent_id": 700,
                    "engine_order": 0, "reserve_navy": true,
                    "display_name": null, "name_provenance": null,
                    "controller": {
                        "kind": "reserve_fleet_aggregate",
                        "split_off_count": 0, "split_off_visible": false
                    }
                },
                {
                    "kind": "task_force", "object_id": 704,
                    "parent_kind": "reserve_fleet_aggregate", "parent_id": 703,
                    "engine_order": 0, "reserve_navy": true,
                    "display_name": null, "name_provenance": null,
                    "controller": {
                        "kind": "task_force", "spotting_visible": false,
                        "training_visible": false, "stop_at_max_visible": false
                    }
                }
            ]
        }],
        "free_air_wings": [],
        "free_air_priority": {
            "selected_priority_ordinal": null,
            "priority_census_wing_ids": []
        },
        "strategic_regions": []
    })
}

fn frame_json() -> Value {
    json!({
        THEATRE_SELECTOR_FIELD: {
            "schema_version": THEATRE_SELECTOR_SCHEMA_VERSION,
            "authority": THEATRE_SELECTOR_AUTHORITY,
            "executable_sha256": EXACT_A729_EXECUTABLE_SHA256,
            "producer_evidence_sha256": REQUIRED_PRODUCER_EVIDENCE_SHA256,
            "elapsed_wall_ms": 7,
            "frame_hour": 900,
            "source_sequence": 901,
            "producer_complete": false,
            "content_identity": content_identity_json(),
            "census": [GER, ENG, EMPTY, BROKEN],
            "sections": [
                land_section(GER),
                air_section(ENG),
                empty_body(EMPTY),
                {"owner_tag_db_id": BROKEN, "admission": "unavailable",
                 "reason": "identity_authentication_failed"}
            ],
            "negative_controller_dispositions": REQUIRED_NEGATIVE_CONTROLLER_DISPOSITIONS
        }
    })
}

fn view() -> TheatreSelectorView {
    parse_theatre_selector(&raw(frame_json()))
}

#[test]
fn serves_any_selectable_country_and_keeps_measured_empty_distinct() {
    let view = view();
    assert_eq!(view.status, ViewStatus::Present);
    assert_eq!(view.country(ENG).theatres[0].kind, ObjectKind::AirTheatre);
    assert!(view.country(EMPTY).is_measured_empty());
    assert_eq!(view.country(BROKEN).status, ViewStatus::DomainUnavailable);
    assert_eq!(view.country(999).status, ViewStatus::DomainUnavailable);
}

#[test]
fn land_selector_inclusion_filters_without_renumbering_raw_rows() {
    let mut value = frame_json();
    let mut excluded = land_section(GER)["theatres"][0].clone();
    excluded["selector_included"] = json!(false);
    excluded["object_id"] = json!(99);
    excluded["engine_order"] = json!(0);
    excluded["members"] = json!([]);
    value[THEATRE_SELECTOR_FIELD]["sections"][0]["theatres"][0]["engine_order"] = json!(1);
    value[THEATRE_SELECTOR_FIELD]["sections"][0]["theatres"]
        .as_array_mut()
        .unwrap()
        .insert(0, excluded);
    let view = parse_theatre_selector(&raw(value));
    let section = view.country(GER);
    assert_eq!(section.status, ViewStatus::Present);
    assert_eq!(section.theatres.len(), 2);
    assert_eq!(
        section
            .theatres_of_kind(ObjectKind::LandTheatre)
            .map(|row| (row.object_id, row.engine_order))
            .collect::<Vec<_>>(),
        vec![(100, 1)]
    );

    let mut only_excluded = frame_json();
    only_excluded[THEATRE_SELECTOR_FIELD]["census"] = json!([GER]);
    only_excluded[THEATRE_SELECTOR_FIELD]["sections"] = json!([land_section(GER)]);
    only_excluded[THEATRE_SELECTOR_FIELD]["sections"][0]["theatres"][0]["selector_included"] =
        json!(false);
    only_excluded[THEATRE_SELECTOR_FIELD]["sections"][0]["strategic_regions"] = json!([]);
    only_excluded[THEATRE_SELECTOR_FIELD]["producer_complete"] = json!(true);
    assert!(parse_theatre_selector(&raw(only_excluded))
        .country(GER)
        .is_measured_empty());
}

#[test]
fn selector_inclusion_is_required_and_navy_air_cannot_be_excluded() {
    let mut missing = frame_json();
    missing[THEATRE_SELECTOR_FIELD]["sections"][0]["theatres"][0]
        .as_object_mut()
        .unwrap()
        .remove("selector_included");
    let mut excluded_air = frame_json();
    excluded_air[THEATRE_SELECTOR_FIELD]["sections"][1]["theatres"][0]["selector_included"] =
        json!(false);
    for invalid in [missing, excluded_air] {
        assert_eq!(
            parse_theatre_selector(&raw(invalid)).status,
            ViewStatus::DomainUnavailable
        );
    }
}

#[test]
fn typed_controller_and_strategic_inputs_survive() {
    let section = view().country(GER);
    assert!(matches!(
        section.theatres[0].controller,
        TheatreControllerState::Land {
            priority_ordinal: 2,
            delete_eligible: true
        }
    ));
    assert_eq!(section.strategic_regions[0].engine_region_index, 7);
    assert_eq!(section.strategic_regions[0].alerts[0].atlas_frame, 8);
    assert_eq!(
        section.strategic_regions[0].naval_access.visible_frame,
        Some(3)
    );
}

#[test]
fn task_force_is_a_distinct_fleet_child_and_reserve_subject() {
    let mut value = frame_json();
    value[THEATRE_SELECTOR_FIELD]["census"] = json!([GER]);
    value[THEATRE_SELECTOR_FIELD]["sections"] = json!([navy_section(GER)]);
    value[THEATRE_SELECTOR_FIELD]["producer_complete"] = json!(true);
    let section = parse_theatre_selector(&raw(value)).country(GER);
    assert_eq!(section.status, ViewStatus::Present);
    assert_eq!(section.theatres[0].members[1].kind, ObjectKind::TaskForce);
    assert_eq!(
        section.theatres[0].members[1].parent_kind,
        ObjectKind::Fleet
    );
    assert!(!section.theatres[0].members[1].reserve_navy);
    assert_eq!(section.theatres[0].members[3].kind, ObjectKind::TaskForce);
    assert_eq!(
        section.theatres[0].members[3].parent_kind,
        ObjectKind::ReserveFleetAggregate
    );
    assert!(section.theatres[0].members[3].reserve_navy);
}

#[test]
fn task_force_parent_reserve_and_split_laws_refuse_contradictions() {
    let mut base = frame_json();
    base[THEATRE_SELECTOR_FIELD]["census"] = json!([GER]);
    base[THEATRE_SELECTOR_FIELD]["sections"] = json!([navy_section(GER)]);
    base[THEATRE_SELECTOR_FIELD]["producer_complete"] = json!(true);
    let mut wrong_parent = base.clone();
    wrong_parent[THEATRE_SELECTOR_FIELD]["sections"][0]["theatres"][0]["members"][1]["parent_id"] =
        json!(999);
    let mut reserve_fleet = base.clone();
    reserve_fleet[THEATRE_SELECTOR_FIELD]["sections"][0]["theatres"][0]["members"][0]
        ["reserve_navy"] = json!(true);
    let mut split = base;
    split[THEATRE_SELECTOR_FIELD]["sections"][0]["theatres"][0]["members"][0]["controller"]
        ["split_off_visible"] = json!(false);
    for bad in [wrong_parent, reserve_fleet, split] {
        assert_eq!(
            parse_theatre_selector(&raw(bad)).status,
            ViewStatus::DomainUnavailable
        );
    }
}

#[test]
fn hierarchy_census_and_order_mutations_refuse() {
    let mut wrong_owner = frame_json();
    wrong_owner[THEATRE_SELECTOR_FIELD]["sections"][0]["theatres"][0]["owner_tag_db_id"] =
        json!(ENG);
    let mut orphan = frame_json();
    orphan[THEATRE_SELECTOR_FIELD]["sections"][1]["theatres"][0]["members"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    let mut gap = frame_json();
    gap[THEATRE_SELECTOR_FIELD]["sections"][0]["theatres"][0]["members"][0]["engine_order"] =
        json!(42);
    let mut swapped = frame_json();
    swapped[THEATRE_SELECTOR_FIELD]["sections"]
        .as_array_mut()
        .unwrap()
        .swap(0, 1);
    for bad in [wrong_owner, orphan, gap, swapped] {
        assert_eq!(
            parse_theatre_selector(&raw(bad)).status,
            ViewStatus::DomainUnavailable
        );
    }
}

#[test]
fn free_air_priority_is_an_exact_filtered_subset_of_free_wing_census() {
    let admitted = parse_theatre_selector(&raw(frame_json()));
    assert_eq!(admitted.status, ViewStatus::Present);
    let air = admitted.country(ENG);
    assert_eq!(
        air.free_air_wings
            .iter()
            .map(|wing| wing.wing_id)
            .collect::<Vec<_>>(),
        [601, 602]
    );
    assert_eq!(air.free_air_priority_census_wing_ids, [601]);

    let mut bad = frame_json();
    bad[THEATRE_SELECTOR_FIELD]["sections"][1]["free_air_priority"]["priority_census_wing_ids"] =
        json!([600]);
    assert_eq!(
        parse_theatre_selector(&raw(bad)).status,
        ViewStatus::DomainUnavailable
    );
}

#[test]
fn strategic_frames_alert_law_and_access_law_refuse_tampering() {
    for (path, replacement) in [("pinned_frame", json!(3)), ("background_frame", json!(1))] {
        let mut bad = frame_json();
        bad[THEATRE_SELECTOR_FIELD]["sections"][0]["strategic_regions"][0][path] = replacement;
        assert_eq!(
            parse_theatre_selector(&raw(bad)).status,
            ViewStatus::DomainUnavailable
        );
    }
    let mut atlas = frame_json();
    atlas[THEATRE_SELECTOR_FIELD]["sections"][0]["strategic_regions"][0]["alerts"][0]
        ["atlas_frame"] = json!(9);
    let mut access = frame_json();
    access[THEATRE_SELECTOR_FIELD]["sections"][0]["strategic_regions"][0]["naval_access"]
        ["visible_frame"] = json!(2);
    for bad in [atlas, access] {
        assert_eq!(
            parse_theatre_selector(&raw(bad)).status,
            ViewStatus::DomainUnavailable
        );
    }
}

#[test]
fn evidence_schema_executable_and_negative_dispositions_are_exact() {
    for (path, replacement) in [
        ("executable_sha256", json!("00".repeat(32))),
        ("schema_version", json!(2)),
        ("authority", json!("wrong")),
        ("producer_evidence_sha256", json!([])),
        ("negative_controller_dispositions", json!([])),
    ] {
        let mut bad = frame_json();
        bad[THEATRE_SELECTOR_FIELD][path] = replacement;
        assert_eq!(
            parse_theatre_selector(&raw(bad)).status,
            ViewStatus::DomainUnavailable,
            "{path}"
        );
    }
}
#[test]
fn a_land_controller_carrying_the_retired_combat_log_leaf_is_refused() {
    let mut bad = frame_json();
    bad[THEATRE_SELECTOR_FIELD]["sections"][0]["theatres"][0]["controller"]
        ["combat_log_owned"] = json!(false);
    let view = parse_theatre_selector(&raw(bad));
    assert_eq!(view.status, ViewStatus::DomainUnavailable);
    assert_eq!(
        view.diagnostic.as_deref(),
        Some("land controller carries the retired combat_log_owned leaf")
    );
}

#[test]
fn the_negative_disposition_set_requires_the_land_combat_log_row() {
    let mut bad = frame_json();
    bad[THEATRE_SELECTOR_FIELD]["negative_controller_dispositions"] = json!([
        "navy_theater_group_item.combat_log_ownership_and_eligibility",
        "reserve_navy_theater_group_item.combat_log_ownership_and_eligibility",
        "air_theatre_group_item.current_priority_ordinal"
    ]);
    let view = parse_theatre_selector(&raw(bad));
    assert_eq!(view.status, ViewStatus::DomainUnavailable);
    assert_eq!(
        view.diagnostic.as_deref(),
        Some("negative controller dispositions do not exactly match exact-a729 authority")
    );
}


#[test]
fn every_producer_evidence_identity_is_authenticated() {
    for index in 0..REQUIRED_PRODUCER_EVIDENCE_SHA256.len() {
        let mut bad = frame_json();
        bad[THEATRE_SELECTOR_FIELD]["producer_evidence_sha256"][index] =
            json!("00".repeat(32));
        assert_eq!(
            parse_theatre_selector(&raw(bad)).status,
            ViewStatus::DomainUnavailable,
            "producer evidence identity {index}"
        );
    }
}

#[test]
fn v6_rejects_the_legacy_manifest_only_evidence_alias() {
    let mut legacy = frame_json();
    let evidence = legacy[THEATRE_SELECTOR_FIELD]["producer_evidence_sha256"].clone();
    legacy[THEATRE_SELECTOR_FIELD]["producer_manifest_sha256"] = evidence.clone();
    assert_eq!(
        parse_theatre_selector(&raw(legacy.clone())).status,
        ViewStatus::DomainUnavailable
    );
    legacy[THEATRE_SELECTOR_FIELD]
        .as_object_mut()
        .expect("D98 fixture object")
        .remove("producer_evidence_sha256");
    legacy[THEATRE_SELECTOR_FIELD]["producer_manifest_sha256"] = evidence;
    assert_eq!(
        parse_theatre_selector(&raw(legacy)).status,
        ViewStatus::DomainUnavailable
    );
}

#[test]
fn producer_complete_must_match_every_section() {
    let mut bad = frame_json();
    bad[THEATRE_SELECTOR_FIELD]["producer_complete"] = json!(true);
    assert_eq!(
        parse_theatre_selector(&raw(bad)).status,
        ViewStatus::DomainUnavailable
    );
}

#[test]
fn content_hour_and_sequence_authorization_is_fail_closed() {
    assert_eq!(
        view().authorize(&expectation(), 900, Some(900)).status,
        ViewStatus::Present
    );
    let mut wrong = expectation();
    wrong.language = "l_german".into();
    assert_eq!(
        view().authorize(&wrong, 900, None).status,
        ViewStatus::DomainUnavailable
    );
    assert_eq!(
        view().authorize(&expectation(), 901, None).status,
        ViewStatus::DomainUnavailable
    );
    assert_eq!(
        view().authorize(&expectation(), 900, Some(901)).status,
        ViewStatus::DomainUnavailable
    );
    let mut wrong_sequence = frame_json();
    wrong_sequence[THEATRE_SELECTOR_FIELD]["source_sequence"] = json!(902);
    assert_eq!(
        parse_theatre_selector(&raw(wrong_sequence))
            .authorize(&expectation(), 900, None)
            .status,
        ViewStatus::DomainUnavailable
    );
}

#[test]
fn measured_empty_section_survives_parse_and_exact_authorization() {
    let mut value = frame_json();
    value[THEATRE_SELECTOR_FIELD]["census"] = json!([EMPTY]);
    value[THEATRE_SELECTOR_FIELD]["sections"] = json!([empty_body(EMPTY)]);
    value[THEATRE_SELECTOR_FIELD]["producer_complete"] = json!(true);
    let admitted = parse_theatre_selector(&raw(value)).authorize(
        &expectation(),
        900,
        Some(900),
    );
    assert_eq!(admitted.status, ViewStatus::Present);
    assert!(admitted.producer_complete);
    assert_eq!(admitted.census, [EMPTY]);
    assert!(admitted.country(EMPTY).is_measured_empty());
}

#[test]
fn replay_mirrors_every_producer_schema_bound() {
    let mut countries = frame_json();
    countries[THEATRE_SELECTOR_FIELD]["census"] =
        Value::Array(vec![json!(1); MAX_COUNTRIES + 1]);

    let mut theatres = frame_json();
    theatres[THEATRE_SELECTOR_FIELD]["sections"][0]["theatres"] =
        Value::Array(vec![json!(null); MAX_THEATRES_PER_SECTION + 1]);

    let mut language = frame_json();
    language[THEATRE_SELECTOR_FIELD]["content_identity"]["language"] =
        json!("x".repeat(MAX_TOKEN_BYTES + 1));

    let mut layer_id = frame_json();
    layer_id[THEATRE_SELECTOR_FIELD]["content_identity"]["ordered_layers"][0]["layer_id"] =
        json!("x".repeat(MAX_TOKEN_BYTES + 1));

    let mut layers = frame_json();
    layers[THEATRE_SELECTOR_FIELD]["content_identity"]["ordered_layers"] =
        Value::Array(vec![json!(null); MAX_CONTENT_LAYERS + 1]);

    let mut display_name = frame_json();
    display_name[THEATRE_SELECTOR_FIELD]["sections"][0]["theatres"][0]["display_name"] =
        json!("x".repeat(MAX_DISPLAY_NAME_BYTES + 1));

    let mut alerts = frame_json();
    alerts[THEATRE_SELECTOR_FIELD]["sections"][0]["theatres"][0]["land_alert_categories"] =
        Value::Array(vec![json!(0); MAX_ALERTS_PER_THEATRE + 1]);

    let mut alert_category = frame_json();
    alert_category[THEATRE_SELECTOR_FIELD]["sections"][0]["theatres"][0]
        ["land_alert_categories"] = json!([MAX_ALERT_CATEGORY + 1]);

    let mut free_wings = frame_json();
    free_wings[THEATRE_SELECTOR_FIELD]["sections"][1]["free_air_wings"] =
        Value::Array(vec![json!(null); MAX_FREE_WINGS_PER_SECTION + 1]);

    let mut priority = frame_json();
    priority[THEATRE_SELECTOR_FIELD]["sections"][1]["free_air_priority"]
        ["priority_census_wing_ids"] =
        Value::Array(vec![json!(1); MAX_PRIORITY_CENSUS_PER_SECTION + 1]);

    let mut strategic = frame_json();
    strategic[THEATRE_SELECTOR_FIELD]["sections"][0]["strategic_regions"] =
        Value::Array(vec![json!(null); MAX_STRATEGIC_REGIONS_PER_SECTION + 1]);

    for bad in [
        countries,
        theatres,
        language,
        layer_id,
        layers,
        display_name,
        alerts,
        alert_category,
        free_wings,
        priority,
        strategic,
    ] {
        assert_eq!(
            parse_theatre_selector(&raw(bad)).status,
            ViewStatus::DomainUnavailable
        );
    }

    assert_eq!(
        checked_section_member_total(MAX_MEMBERS_PER_SECTION, 1),
        Err("section member count exceeds bound".into())
    );
    assert_eq!(MAX_ALERT_CATEGORY, 0x51);
}

#[test]
fn missing_required_lists_and_refused_rows_are_not_empty_measurements() {
    for field in [
        "theatres",
        "free_air_wings",
        "free_air_priority",
        "strategic_regions",
    ] {
        let mut bad = frame_json();
        bad[THEATRE_SELECTOR_FIELD]["sections"][0]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert_eq!(
            parse_theatre_selector(&raw(bad)).status,
            ViewStatus::DomainUnavailable,
            "{field}"
        );
    }
    let mut smuggled = frame_json();
    smuggled[THEATRE_SELECTOR_FIELD]["sections"][3]["theatres"] = json!([]);
    assert_eq!(
        parse_theatre_selector(&raw(smuggled)).status,
        ViewStatus::DomainUnavailable
    );
}

#[test]
fn absent_domain_is_distinct_from_refused_domain() {
    assert_eq!(
        parse_theatre_selector(&raw(json!({}))).status,
        ViewStatus::DomainAbsent
    );
    assert_eq!(
        parse_theatre_selector(&raw(json!({THEATRE_SELECTOR_FIELD: Value::Null}))).status,
        ViewStatus::DomainUnavailable
    );
}

#[test]
fn runtime_and_localization_name_provenance_remain_distinct() {
    assert_eq!(
        view().country(GER).theatres[0].name_provenance,
        NameProvenance::EngineRuntimeString
    );
    let mut keyed = frame_json();
    keyed[THEATRE_SELECTOR_FIELD]["sections"][0]["theatres"][0]["name_provenance"] =
        json!({"source": "localization_key", "key": "THEATRE_DEFAULT"});
    assert_eq!(
        parse_theatre_selector(&raw(keyed)).country(GER).theatres[0].name_provenance,
        NameProvenance::LocalizationKey("THEATRE_DEFAULT".into())
    );
}
