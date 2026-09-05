use super::*;

fn assert_json_bytes(actual: &Value, expected: &Value) {
    assert_eq!(actual, expected);
    assert_eq!(
        serde_json::to_vec(actual).unwrap(),
        serde_json::to_vec(expected).unwrap()
    );
}

#[test]
fn map_detail_owned_assembly_preserves_truth_states_and_changed_rows() {
    let mut fold = WorldFold::new("fold_speed_map");
    fold.states.insert(
        "10".into(),
        json!({
            "resources": {"oil": 0, "signed": i64::MIN, "unsigned": u64::MAX, "fraction": -0.0},
            "buildings": null, "unrelated": [9]
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    fold.states.insert(
        "2".into(),
        json!({"resources": {}, "buildings": []})
            .as_object()
            .unwrap()
            .clone(),
    );
    fold.states.insert("3".into(), Map::new());
    fold.map_detail_province_buildings.insert(
        "80".into(),
        json!({"factory": 0, "unknown": null})
            .as_object()
            .unwrap()
            .clone(),
    );
    fold.map_detail_province_buildings
        .insert("9".into(), Map::new());
    assert_json_bytes(
        &fold.render_map_detail_layer(),
        &json!({
            "resources_by_state": {
                "10": {"oil": 0, "signed": i64::MIN, "unsigned": u64::MAX, "fraction": -0.0}, "2": {}
            },
            "buildings_by_state": {"10": null, "2": []},
            "province_buildings_by_province": {"80": {"factory": 0, "unknown": null}, "9": {}}
        }),
    );
    fold.states
        .get_mut("10")
        .unwrap()
        .insert("resources".into(), json!({"oil": 7}));
    fold.states.remove("2");
    fold.states
        .get_mut("3")
        .unwrap()
        .insert("buildings".into(), json!({}));
    fold.map_detail_province_buildings.remove("80");
    fold.map_detail_province_buildings
        .get_mut("9")
        .unwrap()
        .insert("factory".into(), json!(2));
    assert_json_bytes(
        &fold.render_map_detail_layer(),
        &json!({
            "resources_by_state": {"10": {"oil": 7}},
            "buildings_by_state": {"10": null, "3": {}},
            "province_buildings_by_province": {"9": {"factory": 2}}
        }),
    );
    assert!(!fold.render_map_detail_layer()["resources_by_state"]
        .as_object()
        .unwrap()
        .contains_key("3"));
}

#[test]
fn map_detail_snapshot_keeps_absent_empty_gap_and_recovery_distinct() {
    let mut fold = WorldFold::new("fold_speed_map_gap");
    let absent = fold.build_snapshot(0, "1936.1.1.0");
    assert!(!absent["map"]
        .as_object()
        .unwrap()
        .contains_key("map_detail_live"));
    fold.map_detail_layer_present = true;
    let empty = fold.build_snapshot(1, "1936.1.1.1");
    assert_json_bytes(
        &empty["map"]["map_detail_live"],
        &json!({
            "resources_by_state": {}, "buildings_by_state": {}, "province_buildings_by_province": {}
        }),
    );
    fold.map_detail_unknown_this_hour = true;
    fold.mark_fold_domain_unknown(
        MAP_DETAIL_DOMAIN_ID,
        "sample_failed",
        "map detail sample failed",
    );
    let gap = fold.build_snapshot(2, "1936.1.1.2");
    assert_eq!(
        gap["map"].as_object().unwrap().get("map_detail_live"),
        Some(&Value::Null)
    );
    assert!(gap["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| { row["domain_id"] == json!(23) && row["problem"] == json!("sample_failed") }));
    fold.reset_hour_state();
    fold.map_detail_unknown_this_hour = false;
    fold.mark_fold_domain_success(MAP_DETAIL_DOMAIN_ID);
    let recovered = fold.build_snapshot(3, "1936.1.1.3");
    assert_json_bytes(
        &recovered["map"]["map_detail_live"],
        &empty["map"]["map_detail_live"],
    );
}

fn project_fixture() -> SpecialProjectsState {
    SpecialProjectsState {
        facilities: BTreeMap::from([(
            9,
            SpecialProjectFacilityState {
                id: 9,
                province: 23,
                active_project_handle: Some(0),
                scientist_char_id: None,
                unread_rewards: 0,
                building: Some("lab_\u{96ea}".into()),
            },
        )]),
        projects: BTreeMap::from([(
            10,
            SpecialProjectState {
                id: 10,
                static_def_handle: u32::MAX,
                iterations: 0,
                worked_at_facility: Some(9),
            },
        )]),
        scientists: BTreeMap::from([(
            11,
            SpecialProjectScientistState {
                char_id: 11,
                trait_count: Some(0),
                skill_values: Some([i32::MIN, 0, 1, i32::MAX]),
                name_token: Some(String::new()),
                display_name: None,
            },
        )]),
        project_states: BTreeMap::from([(
            10,
            SpecialProjectProgressState {
                id: 10,
                static_def_handle: u32::MAX,
                current_state_offset: 16,
                current_state_enum: 2,
                prototype_state_enum: 3,
                simple_state_1_enum: 4,
                simple_state_2_enum: 5,
                simple_state_3_enum: 6,
                prototype_iterations: 7,
                phase_progress_raw: u32::MAX,
                phase_target_raw: 0,
                completed: None,
            },
        )]),
        breakthroughs: BTreeMap::from([(
            (7, 0, 99),
            SpecialProjectBreakthroughState {
                tag_db_id: 7,
                program_status_index: 0,
                specialization_handle: 99,
                specialization: Some("specialization_nuclear"),
                points_raw: u32::MAX,
            },
        )]),
        count: 1,
        source: "measured \"source\"\n\u{96ea}".into(),
        header_extension: 17,
        ..SpecialProjectsState::default()
    }
}

fn expected_projects() -> Value {
    json!({
        "count": 1, "source": "measured \"source\"\n\u{96ea}",
        "facilities": {"9": {"id": 9, "province": 23, "active_project": null,
            "scientist_char_id": null, "unread_rewards": 0, "building": "lab_\u{96ea}", "_r2_active_project_handle": 0}},
        "projects": {"10": {"id": 10, "static_def": null, "iterations": 0, "worked_at_facility": 9, "_r2_static_def_handle": u32::MAX}},
        "scientists": {"11": {"char_id": 11, "trait_count": 0, "name_token": "", "display_name": null,
            "_r2_skill_values": [i32::MIN, 0, 1, i32::MAX]}},
        "_r2_project_states": {"10": {"id": 10, "static_def": null, "current_state": null, "completed": null,
            "phase_progress_raw": u32::MAX, "phase_target_raw": 0, "_r2_static_def_handle": u32::MAX,
            "_r2_current_state_offset": 16, "_r2_current_state_enum": 2, "_r2_prototype_state_enum": 3,
            "_r2_simple_state_1_enum": 4, "_r2_simple_state_2_enum": 5, "_r2_simple_state_3_enum": 6, "_r2_prototype_iterations": 7}},
        "_r2_breakthroughs": [{"country": "GER", "tag_db_id": 7, "program_status_index": 0,
            "specialization": "specialization_nuclear", "points_raw": u32::MAX,
            "_r2_specialization_authority": "exact_a729_serializer_token_enum_join", "_r2_country_authority": "exact_country_identity"}],
        "project_lifecycle": null, "project_progress": null, "reward_history": null,
        "_r2_project_authority": null, "_r2_header_extension": 17
    })
}

#[test]
fn special_projects_owned_assembly_matches_explicit_rows_and_deletions() {
    let mut fold = WorldFold::new("fold_speed_projects");
    fold.tag_by_db_id.insert(7, "GER".into());
    let mut state = project_fixture();
    let mut unresolved = BTreeSet::from([123]);
    let mut expected = expected_projects();
    assert_json_bytes(&state.to_json(&fold, &mut unresolved), &expected);
    assert_eq!(unresolved, BTreeSet::from([123]));
    state.facilities.get_mut(&9).unwrap().province = 24;
    state.projects.remove(&10);
    state.scientists.get_mut(&11).unwrap().display_name = Some("Scientist".into());
    state.project_states.get_mut(&10).unwrap().completed = Some(false);
    state.breakthroughs.clear();
    expected["facilities"]["9"]["province"] = json!(24);
    expected["projects"] = json!({});
    expected["scientists"]["11"]["display_name"] = json!("Scientist");
    expected["_r2_project_states"]["10"]["completed"] = json!(false);
    expected["_r2_breakthroughs"] = json!([]);
    assert_json_bytes(&state.to_json(&fold, &mut unresolved), &expected);
    assert_eq!(unresolved, BTreeSet::from([123]));
}

#[test]
fn special_projects_owned_sidecars_preserve_authority_and_unresolved_countries() {
    let mut fold = WorldFold::new("fold_speed_project_witnesses");
    fold.tag_by_db_id.insert(7, "GER".into());
    let mut state = project_fixture();
    state.lifecycle = Some(ProjectLifecycleFold {
        rows: vec![ProjectLifecycleFoldRow {
            tag_db_id: 8,
            local_id: 12,
            started: true,
            completed: false,
            deleted: false,
        }],
        countries: 2,
        aborted_countries: 1,
    });
    state.owner_progress = Some(ProjectOwnerProgressFold {
        rows: vec![ProjectOwnerProgressFoldRow {
            tag_db_id: 7,
            local_id: 10,
            static_def_handle: u32::MAX,
            current_state_offset: 16,
            current_state_enum: 2,
            prototype_state_enum: 3,
            simple_state_1_enum: 4,
            simple_state_2_enum: 5,
            simple_state_3_enum: 6,
            prototype_iterations: 7,
            phase_progress_raw: u32::MAX,
            phase_target_raw: 0,
            completed: false,
            deleted: false,
        }],
        countries: 2,
        aborted_countries: 1,
    });
    state.reward_history = Some(ProjectRewardHistoryFold {
        rows: vec![ProjectRewardHistoryFoldRow {
            tag_db_id: 8,
            local_id: 12,
            history_index: 0,
            prototype_reward_handle: u32::MAX,
            option_handle: 0,
            province: -1,
            progress_raw: u32::MAX,
            character_ref_type: 2,
            character_ref_id: 11,
            country_ref: 0,
            scientist_ref: 99,
            state_ref: 0,
            date_raw: i32::MIN,
            country_present: true,
            scientist_present: false,
            state_present: true,
            deleted: false,
        }],
        countries: 2,
        aborted_countries: 1,
    });
    state.authority = Some(ProjectAuthorityFold {
        record_sequence: u64::MAX,
        game_hour_raw: i32::MIN,
        status_flags: 5,
        d18_country_count: 2,
        owner_progress_row_count: 1,
        breakthrough_row_count: 1,
        walked_countries: 2,
        aborted_countries: 1,
        authority_flags: D41_AUTHORITY_ALL,
        d18_identity_sha256: "ab".repeat(32),
        carrier_payload_sha256: "cd".repeat(32),
    });
    let mut expected = expected_projects();
    expected["project_lifecycle"] = json!({
        "row_count": 1, "started_count": 1, "completed_count": 0, "walked_countries": 2, "aborted_countries": 1,
        "__r2_deferred_country_rows": {"tag_member": "countries", "overflow_member": "countries_by_tag_db_id",
            "rows": {"8": [{"local_id": 12, "started": true, "completed": false}]}}
    });
    expected["project_progress"] = json!({
        "authority": "exact_a729_owner_joined_project_progress", "row_count": 1, "walked_countries": 2, "aborted_countries": 1,
        "__r2_deferred_country_rows": {"tag_member": "countries", "overflow_member": "countries_by_tag_db_id",
            "rows": {"7": [{"local_id": 10, "static_def_handle": u32::MAX, "current_state_offset": 16,
                "current_state_enum": 2, "prototype_state_enum": 3, "simple_state_1_enum": 4, "simple_state_2_enum": 5,
                "simple_state_3_enum": 6, "prototype_iterations": 7, "phase_progress_raw": u32::MAX, "phase_target_raw": 0, "completed": false}]}}
    });
    expected["reward_history"] = json!({
        "row_count": 1, "walked_countries": 2, "aborted_countries": 1,
        "__r2_deferred_country_rows": {"tag_member": "countries", "overflow_member": "countries_by_tag_db_id",
            "rows": {"8": [{"local_id": 12, "history_index": 0, "prototype_reward_handle": u32::MAX,
                "option_handle": 0, "province": -1, "progress_raw": u32::MAX, "character_ref_type": 2,
                "character_ref_id": 11, "country_ref_raw": 0, "scientist_ref_raw": null, "state_ref_raw": 0, "date_raw": i32::MIN}]}}
    });
    expected["_r2_project_authority"] = json!({
        "schema": "a729_d41_project_authority_v1", "status": "exact", "record_sequence": u64::MAX,
        "game_hour_raw": i32::MIN, "source_status_flags": 5, "executable_sha256": D41_AUTHORITY_EXECUTABLE_SHA256,
        "a20_static_receipt_sha256": D41_A20_STATIC_RECEIPT_SHA256, "x9_producer_manifest_sha256": D41_X9_PRODUCER_MANIFEST_SHA256,
        "d18_identity_sha256": "ab".repeat(32), "d18_country_count": 2, "owner_progress_row_count": 1,
        "breakthrough_row_count": 1, "walked_countries": 2, "aborted_countries": 1,
        "authority_flags": D41_AUTHORITY_ALL, "carrier_payload_sha256": "cd".repeat(32)
    });
    let mut unresolved = BTreeSet::from([123]);
    assert_json_bytes(&state.to_json(&fold, &mut unresolved), &expected);
    assert_eq!(unresolved, BTreeSet::from([8, 123]));
    state.authority.as_mut().unwrap().authority_flags = 0;
    expected["_r2_project_authority"]["status"] = json!("unavailable");
    expected["_r2_project_authority"]["authority_flags"] = json!(0);
    assert_json_bytes(&state.to_json(&fold, &mut unresolved), &expected);
    fold.special_projects = Some(state);
    let snapshot = fold.build_snapshot(1, "1936.1.1.1");
    let sidecars = &snapshot["special_projects"];
    assert_eq!(
        sidecars["project_progress"]["countries"]["GER"][0]["phase_progress_raw"],
        json!(u32::MAX)
    );
    assert!(sidecars["project_lifecycle"]["countries"]
        .as_object()
        .unwrap()
        .is_empty());
    assert_eq!(
        sidecars["project_lifecycle"]["countries_by_tag_db_id"]["8"]["row"][0]["local_id"],
        json!(12)
    );
    assert_eq!(
        sidecars["project_lifecycle"]["countries_by_tag_db_id"]["8"]["cause"],
        json!(COUNTRY_WITHHELD_NO_IDENTITY)
    );
    assert!(!sidecars["project_progress"]
        .as_object()
        .unwrap()
        .contains_key("__r2_deferred_country_rows"));
}

#[test]
fn special_projects_unmeasured_empty_gap_and_recovery_keep_their_witnesses() {
    let mut fold = WorldFold::new("fold_speed_project_gap");
    let mut unresolved = BTreeSet::new();
    assert_eq!(
        fold.build_special_projects_value(&mut unresolved),
        Value::Null
    );
    let mut state = SpecialProjectsState::default();
    let mut expected = json!({
        "count": 0, "source": "", "facilities": {}, "projects": {}, "scientists": {},
        "project_lifecycle": null, "project_progress": null, "reward_history": null,
        "_r2_project_authority": null, "_r2_header_extension": 0, "_r2_project_states": {}, "_r2_breakthroughs": []
    });
    assert_json_bytes(&state.to_json(&fold, &mut unresolved), &expected);
    state.lifecycle = Some(ProjectLifecycleFold {
        rows: vec![],
        countries: 0,
        aborted_countries: 0,
    });
    state.owner_progress = Some(ProjectOwnerProgressFold {
        rows: vec![],
        countries: 0,
        aborted_countries: 0,
    });
    state.reward_history = Some(ProjectRewardHistoryFold {
        rows: vec![],
        countries: 0,
        aborted_countries: 0,
    });
    let parked_empty =
        json!({"tag_member": "countries", "overflow_member": "countries_by_tag_db_id", "rows": {}});
    expected["project_lifecycle"] = json!({"row_count": 0, "started_count": 0, "completed_count": 0,
        "walked_countries": 0, "aborted_countries": 0, "__r2_deferred_country_rows": parked_empty});
    expected["project_progress"] = json!({"authority": "exact_a729_owner_joined_project_progress", "row_count": 0,
        "walked_countries": 0, "aborted_countries": 0, "__r2_deferred_country_rows": parked_empty});
    expected["reward_history"] = json!({"row_count": 0, "walked_countries": 0, "aborted_countries": 0,
        "__r2_deferred_country_rows": parked_empty});
    assert_json_bytes(&state.to_json(&fold, &mut unresolved), &expected);
    fold.special_projects = Some(state);
    fold.mark_fold_domain_unknown(
        SPECIAL_PROJECTS_DOMAIN_ID,
        "sample_failed",
        "special project sample failed",
    );
    assert_eq!(
        fold.build_special_projects_value(&mut unresolved),
        Value::Null
    );
    let gap = fold.build_snapshot(2, "1936.1.1.2");
    assert_eq!(gap["special_projects"], Value::Null);
    assert!(gap["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| { row["domain_id"] == json!(41) && row["problem"] == json!("sample_failed") }));
    fold.reset_hour_state();
    fold.mark_fold_domain_success(SPECIAL_PROJECTS_DOMAIN_ID);
    assert_json_bytes(
        &fold.build_special_projects_value(&mut unresolved),
        &expected,
    );
    assert!(unresolved.is_empty());
}
