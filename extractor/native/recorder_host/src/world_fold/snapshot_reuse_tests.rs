use super::*;
use snapshot::canon_value::{key_cmp_decoded, CanonValue};
use std::sync::Arc;

fn node_at<'a>(mut value: &'a CanonValue, path: &[&str]) -> &'a CanonValue {
    for key in path {
        let CanonValue::Object(node) = value else {
            panic!("object expected at {key}")
        };
        let index = node
            .binary_search_by(|(stored, _)| key_cmp_decoded(stored, key))
            .unwrap();
        value = &node[index].1;
    }
    value
}

fn same_object(a: &CanonValue, b: &CanonValue) -> bool {
    match (a, b) {
        (CanonValue::Object(a), CanonValue::Object(b)) => Arc::ptr_eq(a, b),
        _ => panic!("object roots expected"),
    }
}

fn assert_reused_build(
    fold: &mut WorldFold,
    hour: i64,
    previous: Option<&CanonValue>,
) -> CanonValue {
    let reference = fold.build_snapshot(hour, "1936.1.1");
    let (skeleton, roots) = fold.build_reused_snapshot(hour, "1936.1.1");
    let mut actual = snapshot::canon_convert::from_value_shared(&skeleton, previous);
    snapshot_reuse::restore_research_roots(&mut actual, roots);
    let expected = snapshot::UnhashedWorldHour::from_shared_value(&reference, None).hashed();
    let prepared = snapshot::UnhashedWorldHour::from_canonical_value(actual.clone()).hashed();
    assert_eq!(
        prepared.value().to_json_bytes(),
        expected.value().to_json_bytes()
    );
    assert_eq!(prepared.world_digest(), expected.world_digest());
    assert_eq!(prepared.value_heap_bytes(), prepared.value().heap_bytes());
    actual
}

#[test]
fn retained_research_matches_full_hours_across_wire_changes_gaps_and_witnesses() {
    let whole = xui09_whole_tail();
    let changed = [(0, 4), (1, 4), (2, 4)];
    let renamed = [(0, "infantry_weapons"), (1, "basic_tank"), (2, "radio_2")];
    let fresh = xui09_hour(&[(1, &XUI09_GER), (2, &XUI09_FRA)], &whole);
    let new_vector = xui09_hour(
        &[(1, &changed), (2, &XUI09_FRA)],
        &xui09_tail(true, &[], &[1, 2], &[], &[(1, &changed)]),
    );
    let new_dictionary = xui09_hour(
        &[(1, &changed), (2, &XUI09_FRA)],
        &xui09_tail(
            false,
            &renamed,
            &[1, 2],
            &[],
            &[(1, &changed), (2, &XUI09_FRA)],
        ),
    );
    let refused = vec![
        (18, country_identity_payload(&[(1, "GER"), (2, "FRA")])),
        (13, xui09_d13_payload(&[(1, 3, 3), (2, 1, 3)], &whole)),
    ];
    let hours = [
        fresh.clone(),
        vec![],
        vec![],
        new_vector,
        new_dictionary,
        refused,
        fresh.clone(),
        vec![(18, country_identity_payload(&[(1, "GER")]))],
        fresh,
    ];
    let mut stream = Vec::new();
    let mut seq = 1;
    for (offset, records) in hours.iter().enumerate() {
        let raw = raw_for(16_959_360 + offset as i64);
        for (domain, payload) in records {
            push_record(
                &mut stream,
                seq,
                raw,
                *domain,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                payload,
            );
            seq += 1;
        }
        if offset == 2 {
            push_record(
                &mut stream,
                seq,
                raw,
                13,
                COMPACT_RECORD_TYPE_DOMAIN_GAP,
                &gq_domain_gap_payload(raw),
            );
            seq += 1;
        }
        push_record(
            &mut stream,
            seq,
            raw,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload(),
        );
        seq += 1;
    }
    let mut reference = WorldFold::new("retained_research");
    reference.ingest_stream(&stream);
    let reference = reference.finish();
    let mut fold = WorldFold::new("retained_research");
    fold.set_emit_canonical(true);
    fold.ingest_stream(&stream);
    let actual = fold.take_finished_canonical();
    assert_eq!(actual.len(), hours.len());
    assert_eq!(actual.len(), reference.len());
    let mut decoded = Vec::new();
    for ((hour, date, tree), (ref_hour, ref_date, bytes)) in actual.iter().zip(&reference) {
        assert_eq!((hour, date), (ref_hour, ref_date));
        let expected: Value = serde_json::from_slice(bytes).unwrap();
        let expected = snapshot::UnhashedWorldHour::from_shared_value(&expected, None).hashed();
        let hashed = tree.clone().hashed();
        assert_eq!(
            hashed.value().to_json_bytes(),
            expected.value().to_json_bytes(),
            "hour {hour}"
        );
        assert_eq!(hashed.world_digest(), expected.world_digest());
        assert_eq!(hashed.value_heap_bytes(), hashed.value().heap_bytes());
        decoded.push(parse(&tree.value().to_json_bytes()));
    }
    let research =
        |index: usize| node_at(actual[index].2.value(), &["countries", "GER", "research"]);
    assert!(
        same_object(research(0), research(1)),
        "zero-domain hour must retain the root"
    );
    assert!(
        same_object(research(1), research(2)),
        "gap changes provenance, not the retained observation"
    );
    assert!(
        !same_object(research(2), research(3)),
        "a changed row must replace the root"
    );
    assert_eq!(decoded[0]["countries"]["GER"]["research"]["tech_count"], 2);
    assert_eq!(decoded[3]["countries"]["GER"]["research"]["tech_count"], 3);
    assert!(decoded[0]["countries"]["GER"]["research"]
        .get("technology_states")
        .is_some());
    assert!(decoded[5]["countries"]["GER"]["research"]
        .get("technology_states")
        .is_none());
    assert!(decoded[6]["countries"]["GER"]["research"]
        .get("technology_states")
        .is_some());
    assert!(
        decoded[7]["countries"]
            .as_object()
            .unwrap()
            .values()
            .all(|country| country.get("research").is_none()
                || country["research"]["tech_count"] == 2)
    );
    assert_eq!(decoded[8]["countries"]["FRA"]["research"]["tech_count"], 1);
    assert!(decoded[2]["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["domain_id"] == 13 && p["problem"] == "domain_gap"));
    assert!(!decoded[3]["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["problem"] == "domain_gap"));
    assert_ne!(
        decoded[0]["countries"]["GER"].get("name_identity"),
        decoded[1]["countries"]["GER"].get("name_identity")
    );
}

#[test]
fn retained_research_preserves_truth_states_and_withheld_identity_paths() {
    let mut fold = WorldFold::new("research_identity");
    for (id, research) in [
        (
            1,
            json!({"unknown": null, "empty": [], "zero": 0, "negative_zero": -0.0,
        "maximum": u64::MAX, "minimum": i64::MIN, "quote\"": "\\\n"}),
        ),
        (2, json!({})),
        (3, Value::Null),
    ] {
        fold.countries.insert(
            id.to_string(),
            json!({"research": research}).as_object().unwrap().clone(),
        );
    }
    fold.countries.insert("4".into(), Map::new());
    let first = assert_reused_build(&mut fold, 0, None);
    let first_bytes = first.to_json_bytes();
    let decoded = parse(&first_bytes);
    assert!(decoded["countries"]["1"]["research"]["unknown"].is_null());
    assert_eq!(decoded["countries"]["1"]["research"]["empty"], json!([]));
    assert_eq!(decoded["countries"]["2"]["research"], json!({}));
    assert!(decoded["countries"]["3"].get("research").unwrap().is_null());
    assert!(decoded["countries"]["4"].get("research").is_none());
    assert_eq!(fold.canonical_research.len(), 2);
    fold.country_identity_known_this_hour = true;
    fold.tag_by_db_id.insert(1, "GER".into());
    fold.tag_by_db_id.insert(2, "GER".into());
    let withheld = assert_reused_build(&mut fold, 1, Some(&first));
    for id in ["1", "2"] {
        assert!(same_object(
            node_at(&first, &["countries", id, "research"]),
            node_at(
                &withheld,
                &[
                    "tag_db_id_index",
                    "withheld_countries",
                    "by_tag_db_id",
                    id,
                    "section",
                    "research"
                ]
            )
        ));
    }
    fold.tag_by_db_id.insert(2, "FRA".into());
    let named = assert_reused_build(&mut fold, 2, Some(&withheld));
    assert!(same_object(
        node_at(&first, &["countries", "1", "research"]),
        node_at(&named, &["countries", "GER", "research"])
    ));
    fold.tag_by_db_id.remove(&1);
    let missing = assert_reused_build(&mut fold, 3, Some(&named));
    assert!(same_object(
        node_at(&first, &["countries", "1", "research"]),
        node_at(
            &missing,
            &[
                "tag_db_id_index",
                "withheld_countries",
                "by_tag_db_id",
                "1",
                "section",
                "research"
            ]
        )
    ));
    assert_eq!(
        first.to_json_bytes(),
        first_bytes,
        "later assembly cannot mutate a published hour"
    );
    fold.countries.get_mut("1").unwrap().remove("research");
    assert_reused_build(&mut fold, 4, Some(&missing));
    assert!(!fold.canonical_research.contains_key("1"));
    fold.set_emit_canonical(false);
    assert!(fold.canonical_research.is_empty());
}

#[test]
fn retained_research_keeps_d45_overlay_out_of_the_source_root() {
    let mut fold = WorldFold::new("research_overlay");
    fold.countries.insert(
        "1".into(),
        json!({"research": {"tech_count": 0,
        "technology_states": [], "_r2_depth_in_progress_live": ["source"]}})
        .as_object()
        .unwrap()
        .clone(),
    );
    let first = assert_reused_build(&mut fold, 0, None);
    let key = FrdResearchKey {
        tag_db_id: 1,
        slot_index: 0,
    };
    fold.frd_research_live.insert(
        key,
        FrdResearchLiveState {
            tag_db_id: 1,
            slot_index: 0,
            flags: 0,
            hour_delta: 1,
            previous_handle: 0,
            current_handle: 1,
            previous_def: 0,
            current_def: 2,
            previous_progress_raw: 0,
            current_progress_raw: 5,
            delta_raw: 5,
            speed_raw: 5,
            base_raw: None,
            eta_hours: Some(0),
        },
    );
    let overlay = assert_reused_build(&mut fold, 1, Some(&first));
    let decoded = parse(&overlay.to_json_bytes());
    assert_eq!(
        decoded["countries"]["1"]["research"]["_r2_depth_in_progress_live"][0]
            ["current_progress_raw"],
        5
    );
    assert!(
        decoded["countries"]["1"]["research"]["_r2_depth_in_progress_live"][0]["base_raw"]
            .is_null()
    );
    assert_eq!(
        decoded["countries"]["1"]["research"]["_r2_depth_in_progress_live"][0]["eta_hours"],
        0
    );
    assert!(same_object(
        &fold.canonical_research["1"],
        node_at(&first, &["countries", "1", "research"])
    ));
    fold.frd_research_live.clear();
    let restored = assert_reused_build(&mut fold, 2, Some(&overlay));
    assert!(same_object(
        node_at(&first, &["countries", "1", "research"]),
        node_at(&restored, &["countries", "1", "research"])
    ));
}
