use super::*;
use focus_research_decisions_depth_live::xui09_research_contract::{
    classify_country_research, classify_snapshot, WitnessShape, RAW_DATE_EPOCH,
};
use serde_json::{json, Value};
use std::path::PathBuf;

const FIXTURE_RAW_HOUR: i32 = 43_799_999 + 16_959_360;
const FIXTURE_HOUR_INDEX: i64 = FIXTURE_RAW_HOUR as i64 - RAW_DATE_EPOCH;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../replay_interface/fixtures/xui09_research_joint_certification.json")
}

fn witness_row(tag_db_id: u32, states: &[(u32, u8)]) -> CountryObjectRow {
    let states: Vec<(u32, u8)> = states.to_vec();
    let completed: Vec<u32> = states
        .iter()
        .filter(|(_, state)| *state == 4)
        .map(|(def_index, _)| *def_index)
        .collect();
    let mut row = super::domain13_tech_delta_tests::bare_country_row(tag_db_id);
    row.research = Some(CountryResearchRow {
        completed_tech_count: Some(completed.len() as u32),
        research_slot_count: Some(2),
        active_research: None,
        technology_witness: Ok(Arc::new(CountryTechnologyWitness { states, completed })),
    });
    row
}

fn fixture_rows() -> Vec<CountryObjectRow> {
    vec![
        witness_row(7, &[(0, 4), (1, 3), (2, 6), (3, 4), (4, 1)]),
        witness_row(11, &[(0, 1), (1, 4), (2, 2), (3, 6), (4, 3)]),
    ]
}

fn fixture_dictionary() -> TechnologyIdDictionary {
    TechnologyIdDictionary {
        by_definition_index: [
            (0u32, "infantry_weapons"),
            (1, "fighter1"),
            (2, "basic_machine_tools"),
            (3, "electronic_mechanical_engineering"),
            (4, "construction1"),
            (7, "improved_medium_battery"),
            (11, "marines"),
            (40, "night_vision"),
        ]
        .into_iter()
        .map(|(index, id)| (index, (0usize, id.to_string())))
        .collect(),
    }
}

fn country_identity_payload(rows: &[(u32, &str)]) -> Vec<u8> {
    let mut payload = vec![0u8; 20];
    payload[8..12].copy_from_slice(&(rows.len() as u32).to_le_bytes());
    for (tag_db_id, tag) in rows {
        payload.extend_from_slice(&tag_db_id.to_le_bytes());
        payload.extend_from_slice(&(tag.len() as u16).to_le_bytes());
        payload.extend_from_slice(tag.as_bytes());
        payload.push(0);
    }
    payload
}

fn frame_end_payload() -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&0u64.to_le_bytes());
    payload.extend_from_slice(&0u64.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload
}

fn folded_snapshot(rows: &[CountryObjectRow], identities: &[(u32, &str)]) -> Value {
    let mut tracker = CompactTechCountryDeltaTracker::default();
    let tech = compact_tech_data_payload_delta(rows, 0, 0, &mut tracker, &fixture_dictionary());

    let mut stream = Vec::new();
    append_compact_record(
        &mut stream,
        1,
        FIXTURE_RAW_HOUR,
        COMPACT_DOMAIN_COUNTRY_IDENTITY,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &country_identity_payload(identities),
    );
    append_compact_record(
        &mut stream,
        2,
        FIXTURE_RAW_HOUR,
        COMPACT_DOMAIN_TECH,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &tech,
    );
    append_compact_record(
        &mut stream,
        3,
        FIXTURE_RAW_HOUR,
        compact_contract::COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        0,
        &frame_end_payload(),
    );

    let mut fold = recorder_host::world_fold::WorldFold::new("jo_xui09_joint");
    fold.ingest_stream(&stream);
    let snapshots = fold.finish();
    assert_eq!(
        snapshots.len(),
        1,
        "one FRAME_END must fold to exactly one snapshot"
    );
    serde_json::from_slice(&snapshots[0].2).expect("the fold emits JSON")
}

#[test]
fn xui09_joint_the_folded_witness_is_admissible_under_the_consumers_own_law() {
    let snapshot = folded_snapshot(&fixture_rows(), &[(7, "GER"), (11, "FRA")]);

    let shapes = classify_snapshot(&snapshot, Some(FIXTURE_RAW_HOUR as i64))
        .expect("the folded snapshot satisfies the XUI09 wire law");
    assert_eq!(
        shapes,
        vec![
            ("FRA".to_string(), WitnessShape::Complete),
            ("GER".to_string(), WitnessShape::Complete),
        ],
        "every country of the D18 roster carries a complete witness for this hour"
    );

    let germany = &snapshot["countries"]["GER"]["research"];
    assert_eq!(germany["technology_states_count"], json!(5));
    assert_eq!(germany["completed_technologies_count"], json!(2));
    assert_eq!(germany["tech_count"], json!(2));
    assert_eq!(
        germany["technology_states_game_hour_raw"],
        json!(FIXTURE_RAW_HOUR as i64)
    );
    assert_eq!(
        germany["technology_states"][0]["technology_id"],
        json!("infantry_weapons")
    );
    assert_eq!(germany["technology_states"][0]["state_enum_raw"], json!(4));
    assert_eq!(
        germany["completed_technologies"][1]["tech_def_index_raw"],
        json!(3)
    );
    let france = &snapshot["countries"]["FRA"]["research"];
    assert_eq!(france["technology_states_count"], json!(5));
    assert_eq!(france["completed_technologies_count"], json!(1));
    assert_eq!(
        france["completed_technologies"][0]["technology_id"],
        json!("fighter1"),
        "the two countries carry the same technologies in different states"
    );
}

#[test]
fn xui09_joint_the_committed_fixture_is_exactly_what_the_producer_emits() {
    let snapshot = folded_snapshot(&fixture_rows(), &[(7, "GER"), (11, "FRA")]);
    let fixture = serde_json::json!({
        "_comment": "Lane JO joint-certification fixture (gap-map row G4-X9S1 / XUI09). \
                     Producer half: collector/src/attach_log/xui09_joint_certification_tests.rs. \
                     Consumer half: replay_interface/native/game_ui/tests/xui09_joint_certification.rs. \
                     Regenerate with JO_WRITE_XUI09_FIXTURE=1.",
        "hour_index": FIXTURE_HOUR_INDEX,
        "game_hour_raw": FIXTURE_RAW_HOUR,
        "snapshot": snapshot,
    });
    let rendered = format!("{}\n", serde_json::to_string_pretty(&fixture).unwrap());
    let path = fixture_path();
    if std::env::var_os("JO_WRITE_XUI09_FIXTURE").is_some() {
        std::fs::write(&path, rendered.as_bytes()).expect("the fixture directory exists");
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "the joint-certification fixture at {} is unreadable ({error}); regenerate it with \
             JO_WRITE_XUI09_FIXTURE=1",
            path.display()
        )
    });
    assert_eq!(
        committed.replace("\r\n", "\n"),
        rendered,
        "the committed fixture is not what the producer emits; regenerate it with \
         JO_WRITE_XUI09_FIXTURE=1 and re-run the replay-side consumer gate"
    );
}

#[test]
fn xui09_joint_a_country_the_frame_did_not_measure_loses_its_witness() {
    let mut tracker = CompactTechCountryDeltaTracker::default();
    let dictionary = fixture_dictionary();
    let identities = [(7u32, "GER"), (11, "FRA")];

    let first = compact_tech_data_payload_delta(&fixture_rows(), 0, 0, &mut tracker, &dictionary);
    let torn = vec![
        fixture_rows().remove(0),
        super::domain13_tech_delta_tests::bare_country_row(11),
    ];
    let second = compact_tech_data_payload_delta(&torn, 0, 0, &mut tracker, &dictionary);

    let mut stream = Vec::new();
    let mut sequence = 0u64;
    for (hour, tech) in [(FIXTURE_RAW_HOUR, &first), (FIXTURE_RAW_HOUR + 1, &second)] {
        sequence += 1;
        append_compact_record(
            &mut stream,
            sequence,
            hour,
            COMPACT_DOMAIN_COUNTRY_IDENTITY,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &country_identity_payload(&identities),
        );
        sequence += 1;
        append_compact_record(
            &mut stream,
            sequence,
            hour,
            COMPACT_DOMAIN_TECH,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            tech,
        );
        sequence += 1;
        append_compact_record(
            &mut stream,
            sequence,
            hour,
            compact_contract::COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &frame_end_payload(),
        );
    }
    let mut fold = recorder_host::world_fold::WorldFold::new("jo_xui09_unmeasured");
    fold.ingest_stream(&stream);
    let snapshots = fold.finish();
    assert_eq!(snapshots.len(), 2, "two hours were closed");
    let second_hour: Value = serde_json::from_slice(&snapshots[1].2).expect("the fold emits JSON");

    let france = &second_hour["countries"]["FRA"]["research"];
    assert!(
        france.get("technology_states").is_none(),
        "an unmeasured country must not carry last hour's classifier vector: {france}"
    );
    assert_eq!(
        france["technology_states_unmeasured_reason"],
        json!(XUI09_REFUSAL_RESEARCH_ROW_ABSENT),
        "and the drop must name its cause"
    );
    let germany = &second_hour["countries"]["GER"]["research"];
    assert_eq!(
        germany["technology_states_game_hour_raw"],
        json!(FIXTURE_RAW_HOUR as i64 + 1),
        "a measured-and-unchanged country is re-sealed for the hour it was measured in"
    );
    assert_eq!(
        classify_country_research(germany, Some(FIXTURE_RAW_HOUR as i64 + 1)),
        Ok(WitnessShape::Complete)
    );
    assert!(
        classify_snapshot(&second_hour, Some(FIXTURE_RAW_HOUR as i64 + 1))
            .expect("the roster law still holds")
            .iter()
            .any(|(_, shape)| *shape == WitnessShape::Absent),
        "the hour is injured and says so, rather than looking complete"
    );
}

#[test]
fn xui09_joint_a_sparse_classifier_vector_keeps_its_engine_indices() {
    let rows = [witness_row(7, &[(2, 4), (7, 6), (11, 2), (40, 4)])];
    let snapshot = folded_snapshot(&rows, &[(7, "GER")]);
    let states = &snapshot["countries"]["GER"]["research"]["technology_states"];
    assert_eq!(
        states
            .as_array()
            .expect("states are rows")
            .iter()
            .map(|row| (
                row["tech_def_index_raw"].as_u64().unwrap(),
                row["technology_id"].as_str().unwrap().to_string()
            ))
            .collect::<Vec<_>>(),
        vec![
            (2, "basic_machine_tools".to_string()),
            (7, "improved_medium_battery".to_string()),
            (11, "marines".to_string()),
            (40, "night_vision".to_string()),
        ],
        "a sparse vector keeps the engine's own definition indices and their ids"
    );
    let mut tracker = CompactTechCountryDeltaTracker::default();
    let payload = compact_tech_data_payload_delta(&rows, 0, 0, &mut tracker, &fixture_dictionary());
    let tail = payload
        .windows(4)
        .position(|window| window == COMPACT_TECH_XUI09_TAIL_MAGIC.to_le_bytes())
        .expect("the payload carries an XUI09 tail");
    let vector_flags_at = payload.len() - (16 + 4 * 5 + 4 * 2) + 4;
    assert!(vector_flags_at > tail);
    assert_eq!(
        u32::from_le_bytes(
            payload[vector_flags_at..vector_flags_at + 4]
                .try_into()
                .unwrap()
        ) & COMPACT_TECH_XUI09_COUNTRY_FLAG_DENSE,
        0,
        "a vector whose indices are not 0..n must not claim to be dense"
    );
}

#[test]
fn xui09_the_collector_offsets_are_the_ones_lane_io_pinned() {
    use focus_research_decisions_depth_live::xui09_research_contract as contract;
    assert_eq!(
        TECH_STATUS_ALL_TECHNOLOGIES_DATA,
        contract::TECH_STATUS_ALL_TECHNOLOGIES_DATA
    );
    assert_eq!(
        TECH_STATUS_ALL_TECHNOLOGIES_SIZE,
        contract::TECH_STATUS_ALL_TECHNOLOGIES_SIZE
    );
    assert_eq!(TECHNOLOGY_UI_STATE_ENUM, contract::TECHNOLOGY_UI_STATE_ENUM);
    assert_eq!(
        TECHNOLOGY_STATUS_BACKLINK,
        contract::TECHNOLOGY_STATUS_BACKLINK
    );
    assert_eq!(
        TECHNOLOGY_TEMPLATE_ID_STRING,
        contract::TECHNOLOGY_TEMPLATE_ID_STRING
    );
    assert_eq!(
        TECHNOLOGY_TEMPLATE_ID_STRING_CAPACITY,
        contract::TECHNOLOGY_TEMPLATE_ID_STRING_SIZE,
        "lane IO pinned this word under the name `..._SIZE`; the VALUE is the one \
    );
    assert_eq!(
        u64::from(XUI09_STATE_ENUM_MIN),
        u64::from(contract::X9S1_STATE_ENUM_MIN)
    );
    assert_eq!(
        u64::from(XUI09_STATE_ENUM_MAX),
        u64::from(contract::X9S1_STATE_ENUM_MAX)
    );
}

#[test]
fn technology_template_id_string_matches_the_shared_msvc_reader_layout() {
    const MSVC_STRING_CAPACITY_DISPLACEMENT: usize = 0x18;
    assert_eq!(
        TECHNOLOGY_TEMPLATE_ID_STRING + MSVC_STRING_CAPACITY_DISPLACEMENT,
        TECHNOLOGY_TEMPLATE_ID_STRING_CAPACITY
    );
    assert_eq!(MSVC_STRING_SSO_MAX, 15);
}

#[test]
fn xui09_the_technology_id_map_is_sent_once_and_not_again() {
    let mut tracker = CompactTechCountryDeltaTracker::default();
    let dictionary = fixture_dictionary();
    let rows = fixture_rows();

    let baseline = compact_tech_data_payload_delta(&rows, 0, 0, &mut tracker, &dictionary);
    assert_eq!(
        xui09_dictionary_count(&baseline),
        5,
        "the first payload carries every id its rows use"
    );

    let mut moved = rows.clone();
    moved[0].research.as_mut().unwrap().technology_witness =
        Ok(Arc::new(CountryTechnologyWitness {
            states: vec![(0, 4), (1, 3), (2, 6), (3, 4), (4, 3)],
            completed: vec![0, 3],
        }));
    let patch = compact_tech_data_payload_delta(&moved, 0, 0, &mut tracker, &dictionary);
    assert_eq!(
        xui09_dictionary_count(&patch),
        0,
        "a patch does not restate ids the fold already has"
    );
    assert_eq!(
        xui09_vector_count(&patch),
        1,
        "only the country whose witness changed is carried"
    );

    let quiet = compact_tech_data_payload_delta(&moved, 0, 0, &mut tracker, &dictionary);
    assert_eq!(xui09_dictionary_count(&quiet), 0);
    assert_eq!(
        xui09_vector_count(&quiet),
        0,
        "an hour in which no classifier state moved costs the measured set and nothing else"
    );
}

fn xui09_tail_offset(payload: &[u8]) -> usize {
    payload
        .windows(4)
        .position(|window| window == COMPACT_TECH_XUI09_TAIL_MAGIC.to_le_bytes())
        .expect("the payload carries an XUI09 tail")
}

fn xui09_dictionary_count(payload: &[u8]) -> u32 {
    let at = xui09_tail_offset(payload) + 8;
    u32::from_le_bytes(payload[at..at + 4].try_into().unwrap())
}

fn xui09_vector_count(payload: &[u8]) -> u32 {
    let at = xui09_tail_offset(payload) + 20;
    u32::from_le_bytes(payload[at..at + 4].try_into().unwrap())
}

#[test]
fn xui09_the_fold_refuses_a_tail_it_cannot_join() {
    let dictionary = fixture_dictionary();
    let rows = fixture_rows();
    let mut tracker = CompactTechCountryDeltaTracker::default();
    let good = compact_tech_data_payload_delta(&rows, 0, 0, &mut tracker, &dictionary);
    let tail = xui09_tail_offset(&good);

    assert!(
        !fold_refused(&good),
        "the unedited payload must publish a witness, or the refusals below prove nothing"
    );

    let mut unknown_flag = good.clone();
    unknown_flag[tail + 4] |= 0b1000_0000;
    assert!(
        fold_refused(&unknown_flag),
        "an unassigned XUI09 tail flag must refuse the record"
    );

    let mut unknown_country_flag = good.clone();
    let vectors_at = unknown_country_flag.len() - xui09_vector_span(&good);
    unknown_country_flag[vectors_at + 4] |= 0b0100_0000;
    assert!(
        fold_refused(&unknown_country_flag),
        "an unassigned per-country flag must refuse the record"
    );

    let mut no_ids = good.clone();
    no_ids[tail + 8..tail + 12].copy_from_slice(&0u32.to_le_bytes());
    assert!(
        fold_refused(&no_ids),
        "a row with no id in the dictionary must refuse the record rather than publish an \
         unidentified technology"
    );
}

fn xui09_vector_span(payload: &[u8]) -> usize {
    let tail = xui09_tail_offset(payload);
    let measured = u32::from_le_bytes(payload[tail + 12..tail + 16].try_into().unwrap()) as usize;
    let refused = u32::from_le_bytes(payload[tail + 16..tail + 20].try_into().unwrap()) as usize;
    let dictionary_bytes = {
        let mut cursor = tail + 24;
        let count = xui09_dictionary_count(payload) as usize;
        for _ in 0..count {
            cursor += 4;
            let len = u16::from_le_bytes(payload[cursor..cursor + 2].try_into().unwrap()) as usize;
            cursor += 2 + len;
        }
        cursor - (tail + 24)
    };
    payload.len() - (tail + 24 + dictionary_bytes + measured * 4 + refused * 8)
}

fn fold_refused(payload: &[u8]) -> bool {
    let mut fold = recorder_host::world_fold::WorldFold::new("jo_xui09_refusal_probe");
    fold.ingest_record(
        1,
        FIXTURE_RAW_HOUR,
        COMPACT_DOMAIN_TECH,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        payload,
        0,
    );
    fold.ingest_record(
        2,
        FIXTURE_RAW_HOUR,
        compact_contract::COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
        0,
    );
    let snapshots = fold.finish();
    let Some((_, _, bytes)) = snapshots.into_iter().next() else {
        return true;
    };
    let snapshot: Value = serde_json::from_slice(&bytes).expect("the fold emits JSON");
    snapshot["countries"]
        .as_object()
        .map(|countries| {
            countries.values().all(|country| {
                country
                    .get("research")
                    .and_then(|research| research.get("technology_states"))
                    .is_none()
            })
        })
        .unwrap_or(true)
}

fn folded_hours(
    frames: &[(Vec<CountryObjectRow>, u32)],
    identities: &[(u32, &str)],
    delta: bool,
) -> Vec<Value> {
    let dictionary = fixture_dictionary();
    let mut tracker = CompactTechCountryDeltaTracker::default();
    let mut stream = Vec::new();
    let mut sequence = 0u64;
    for (index, (rows, status_flags)) in frames.iter().enumerate() {
        let hour = FIXTURE_RAW_HOUR + index as i32;
        let tech = if delta {
            compact_tech_data_payload_delta(rows, 0, *status_flags, &mut tracker, &dictionary)
        } else {
            compact_tech_data_payload(rows, 0, *status_flags, &dictionary)
        };
        for (domain, payload) in [
            (
                COMPACT_DOMAIN_COUNTRY_IDENTITY,
                country_identity_payload(identities),
            ),
            (COMPACT_DOMAIN_TECH, tech),
        ] {
            sequence += 1;
            append_compact_record(
                &mut stream,
                sequence,
                hour,
                domain,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                0,
                &payload,
            );
        }
        sequence += 1;
        append_compact_record(
            &mut stream,
            sequence,
            hour,
            compact_contract::COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &frame_end_payload(),
        );
    }
    let mut fold = recorder_host::world_fold::WorldFold::new("jo_xui09_hours");
    fold.ingest_stream(&stream);
    fold.finish()
        .into_iter()
        .map(|(_, _, bytes)| serde_json::from_slice(&bytes).expect("the fold emits JSON"))
        .collect()
}

fn xui09_witnesses(snapshot: &Value) -> BTreeMap<String, Value> {
    snapshot["countries"]
        .as_object()
        .expect("countries")
        .iter()
        .map(|(tag, country)| {
            let research = country.get("research").cloned().unwrap_or(Value::Null);
            let witness: serde_json::Map<String, Value> =
                recorder_host::world_fold::XUI09_WITNESS_KEYS
                    .iter()
                    .filter_map(|key| {
                        research
                            .get(*key)
                            .map(|value| (key.to_string(), value.clone()))
                    })
                    .collect();
            (tag.clone(), Value::Object(witness))
        })
        .collect()
}

#[test]
fn xui09_a_patch_stream_folds_to_the_same_witnesses_as_a_whole_stream() {
    let identities = [(7u32, "GER"), (11, "FRA")];
    let steady = fixture_rows();
    let mut moved = fixture_rows();
    moved[0].research.as_mut().unwrap().technology_witness =
        Ok(Arc::new(CountryTechnologyWitness {
            states: vec![(0, 4), (1, 3), (2, 6), (3, 4), (4, 3)],
            completed: vec![0, 3],
        }));
    let torn = vec![
        fixture_rows().remove(0),
        super::domain13_tech_delta_tests::bare_country_row(11),
    ];
    let frames: Vec<(Vec<CountryObjectRow>, u32)> = vec![
        (steady.clone(), 0),
        (steady.clone(), 0),
        (moved.clone(), 0),
        (torn, 0),
        (steady.clone(), 0),
        (moved, 0),
    ];

    let patched = folded_hours(&frames, &identities, true);
    let whole = folded_hours(&frames, &identities, false);
    assert_eq!(patched.len(), frames.len());
    assert_eq!(whole.len(), frames.len());
    for (hour, (patched, whole)) in patched.iter().zip(&whole).enumerate() {
        assert_eq!(
            xui09_witnesses(patched),
            xui09_witnesses(whole),
            "hour {hour}: the patch stream and the whole stream disagree about the witness"
        );
    }
}

#[test]
fn xui09_a_whole_tail_must_carry_every_witness_it_measures() {
    let dictionary = fixture_dictionary();
    let rows = fixture_rows();
    let mut tracker = CompactTechCountryDeltaTracker::default();
    let good = compact_tech_data_payload_delta(&rows, 0, 0, &mut tracker, &dictionary);
    let tail = xui09_tail_offset(&good);
    assert_eq!(
        u32::from_le_bytes(good[tail + 4..tail + 8].try_into().unwrap())
            & COMPACT_TECH_XUI09_FLAG_PATCH,
        0,
        "the seeding payload is whole"
    );

    let mut stripped = good[..good.len() - xui09_vector_span(&good)].to_vec();
    stripped[tail + 20..tail + 24].copy_from_slice(&0u32.to_le_bytes());
    assert!(
        fold_refused(&stripped),
        "a whole tail that measures countries it does not carry must refuse the record"
    );
}

#[test]
fn xui09_a_country_whose_vectors_disagree_is_refused_by_name() {
    let dictionary = fixture_dictionary();
    let identities = [(7u32, "GER")];

    let mut row = witness_row(7, &[(0, 4), (1, 3), (2, 6), (3, 4), (4, 1)]);
    row.research.as_mut().unwrap().technology_witness = Ok(Arc::new(CountryTechnologyWitness {
        states: vec![(0, 4), (1, 3), (2, 6), (3, 4), (4, 1)],
        completed: vec![0, 1, 3],
    }));
    row.research.as_mut().unwrap().completed_tech_count = Some(3);
    assert_eq!(
        xui09_refusal_of(&row, &dictionary),
        Some(Xui09RefusalReason::CrossSealCompleted as u32),
        "a researched technology the classifier calls unavailable must be refused"
    );

    let mut converse = witness_row(7, &[(0, 4), (1, 3), (2, 6), (3, 4), (4, 1)]);
    converse.research.as_mut().unwrap().technology_witness =
        Ok(Arc::new(CountryTechnologyWitness {
            states: vec![(0, 4), (1, 3), (2, 6), (3, 4), (4, 1)],
            completed: vec![0],
        }));
    converse.research.as_mut().unwrap().completed_tech_count = Some(1);
    assert_eq!(
        xui09_refusal_of(&converse, &dictionary),
        Some(Xui09RefusalReason::CrossSealCompleted as u32),
        "a state-4 technology absent from G4 membership must be refused too"
    );

    let mut active = witness_row(7, &[(0, 4), (1, 5), (2, 6), (3, 4), (4, 1)]);
    active.research.as_mut().unwrap().completed_tech_count = Some(2);
    assert_eq!(
        xui09_refusal_of(&active, &dictionary),
        Some(Xui09RefusalReason::CrossSealActive as u32),
        "an unauthenticated active state must be refused"
    );

    let mut orphan_slot = witness_row(7, &[(0, 4), (1, 3), (2, 6), (3, 4), (4, 1)]);
    orphan_slot.research.as_mut().unwrap().active_research =
        Some(vec![active_slot_row(1, "fighter1")]);
    assert_eq!(
        xui09_refusal_of(&orphan_slot, &dictionary),
        Some(Xui09RefusalReason::CrossSealActive as u32),
        "a d13 slot no classifier row consumes must be refused"
    );

    let mut wrong_name = witness_row(7, &[(0, 4), (1, 5), (2, 6), (3, 4), (4, 1)]);
    wrong_name.research.as_mut().unwrap().completed_tech_count = Some(2);
    wrong_name.research.as_mut().unwrap().active_research =
        Some(vec![active_slot_row(1, "not_fighter1")]);
    assert_eq!(
        xui09_refusal_of(&wrong_name, &dictionary),
        Some(Xui09RefusalReason::CrossSealActive as u32),
        "an active slot whose identity disagrees with the classifier's must be refused"
    );

    let mut nameless = witness_row(7, &[(0, 4), (1, 5), (2, 6), (3, 4), (4, 1)]);
    nameless.research.as_mut().unwrap().completed_tech_count = Some(2);
    nameless.research.as_mut().unwrap().active_research = Some(vec![CountryResearchSlotRow {
        tech_name: None,
        ..active_slot_row(1, "fighter1")
    }]);
    assert_eq!(
        xui09_refusal_of(&nameless, &dictionary),
        Some(Xui09RefusalReason::CrossSealActive as u32),
        "an active slot with no identity must be refused"
    );

    let good = witness_row(7, &[(0, 4), (1, 3), (2, 6), (3, 4), (4, 1)]);
    assert_eq!(xui09_refusal_of(&good, &dictionary), None);
    let snapshot = folded_hours(&[(vec![good], 0)], &identities, false).remove(0);
    assert!(
        snapshot["countries"]["GER"]["research"]["technology_states"].is_array(),
        "a consistent country still publishes its witness"
    );
}

fn active_slot_row(tech_def_index_raw: u32, tech_name: &str) -> CountryResearchSlotRow {
    CountryResearchSlotRow {
        slot_index: 0,
        tech_handle_raw: 1,
        tech_def_index_raw,
        progress_raw: 0,
        tech_name: Some(tech_name.to_string()),
        progress_units: 0.0,
        live_progress_raw: None,
        live_progress_units: None,
        base_required_points_raw: None,
        base_required_points_units: None,
        ahead_of_time_years_raw: None,
        ahead_of_time_years: None,
        ahead_of_time_penalty_raw: None,
        ahead_of_time_penalty_pct: None,
        ahead_of_time_reduction_years_raw: None,
        ahead_of_time_reduction_years: None,
        ahead_of_time_reduced_penalty_raw: None,
        ahead_of_time_reduced_penalty_pct: None,
        ahead_of_time_provenance: None,
        ahead_of_time_derived: None,
    }
}

fn xui09_refusal_of(row: &CountryObjectRow, dictionary: &TechnologyIdDictionary) -> Option<u32> {
    let mut tracker = CompactTechCountryDeltaTracker::default();
    let payload =
        compact_tech_data_payload_delta(std::slice::from_ref(row), 0, 0, &mut tracker, dictionary);
    let tail = xui09_tail_offset(&payload);
    let refused_count =
        u32::from_le_bytes(payload[tail + 16..tail + 20].try_into().unwrap()) as usize;
    if refused_count == 0 {
        return None;
    }
    let at = tail
        + 24
        + xui09_dictionary_bytes(&payload)
        + u32::from_le_bytes(payload[tail + 12..tail + 16].try_into().unwrap()) as usize * 4;
    Some(u32::from_le_bytes(
        payload[at + 4..at + 8].try_into().unwrap(),
    ))
}

#[test]
fn xui09_the_pin_recovers_after_a_cross_seal_refusal() {
    let identities = [(7u32, "GER")];
    let states: &[(u32, u8)] = &[(0, 4), (1, 5), (2, 6), (3, 4), (4, 1)];
    let mut refused = witness_row(7, states);
    refused.research.as_mut().unwrap().completed_tech_count = Some(2);
    let mut healthy = witness_row(7, states);
    healthy.research.as_mut().unwrap().completed_tech_count = Some(2);
    healthy.research.as_mut().unwrap().active_research = Some(vec![active_slot_row(1, "fighter1")]);

    let hours = folded_hours(
        &[
            (vec![refused], 0),
            (vec![healthy.clone()], 0),
            (vec![healthy], 0),
        ],
        &identities,
        true,
    );
    assert_eq!(hours.len(), 3, "no record was refused");
    assert!(
        hours[0]["countries"]["GER"]["research"]
            .get("technology_states")
            .is_none(),
        "the disagreeing hour carries no witness"
    );
    for hour in &hours[1..] {
        assert!(
            hour["countries"]["GER"]["research"]["technology_states"].is_array(),
            "the country recovers once its vectors agree, and stays recovered"
        );
    }
}

#[test]
fn xui09_the_fold_refuses_impossible_counts_and_unassigned_reasons() {
    let dictionary = fixture_dictionary();
    let rows = fixture_rows();
    let mut tracker = CompactTechCountryDeltaTracker::default();
    let good = compact_tech_data_payload_delta(&rows, 0, 0, &mut tracker, &dictionary);
    let tail = xui09_tail_offset(&good);

    for (offset, what) in [
        (8usize, "dictionary entries"),
        (12, "measured countries"),
        (16, "refusals"),
        (20, "country vectors"),
    ] {
        let mut oversized = good.clone();
        oversized[tail + offset..tail + offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(
            fold_refused(&oversized),
            "a tail claiming u32::MAX {what} must refuse the record rather than allocate for it"
        );
    }

    let torn = vec![
        fixture_rows().remove(0),
        super::domain13_tech_delta_tests::bare_country_row(11),
    ];
    let mut fresh = CompactTechCountryDeltaTracker::default();
    let with_refusal = compact_tech_data_payload_delta(&torn, 0, 0, &mut fresh, &dictionary);
    let tail = xui09_tail_offset(&with_refusal);
    let refused_at = tail
        + 24
        + xui09_dictionary_bytes(&with_refusal)
        + u32::from_le_bytes(with_refusal[tail + 12..tail + 16].try_into().unwrap()) as usize * 4;
    let mut unassigned = with_refusal.clone();
    unassigned[refused_at + 4..refused_at + 8].copy_from_slice(&99u32.to_le_bytes());
    assert!(
        fold_refused(&unassigned),
        "an unassigned refusal reason must refuse the record"
    );
    assert!(!fold_refused(&with_refusal));
}

fn xui09_dictionary_bytes(payload: &[u8]) -> usize {
    let tail = xui09_tail_offset(payload);
    let count = xui09_dictionary_count(payload) as usize;
    let mut cursor = tail + 24;
    for _ in 0..count {
        cursor += 4;
        let len = u16::from_le_bytes(payload[cursor..cursor + 2].try_into().unwrap()) as usize;
        cursor += 2 + len;
    }
    cursor - (tail + 24)
}

#[test]
fn xui09_a_record_with_trailing_bytes_changes_nothing() {
    let dictionary = fixture_dictionary();
    let identities = [(7u32, "GER"), (11, "FRA")];
    let rows = fixture_rows();

    let mut tracker = CompactTechCountryDeltaTracker::default();
    let seed = compact_tech_data_payload_delta(&rows, 0, 0, &mut tracker, &dictionary);
    let mut moved = fixture_rows();
    moved[0].research.as_mut().unwrap().technology_witness =
        Ok(Arc::new(CountryTechnologyWitness {
            states: vec![(0, 4), (1, 3), (2, 6), (3, 4), (4, 3)],
            completed: vec![0, 3],
        }));
    let patch = compact_tech_data_payload_delta(&moved, 0, 0, &mut tracker, &dictionary);

    let fold_of = |extra: &[u8]| {
        let mut fold = recorder_host::world_fold::WorldFold::new("jo_xui09_trailing");
        let mut stream = Vec::new();
        let mut sequence = 0u64;
        for (hour, payload) in [(FIXTURE_RAW_HOUR, &seed), (FIXTURE_RAW_HOUR + 1, &patch)] {
            let mut payload = payload.clone();
            if hour != FIXTURE_RAW_HOUR {
                payload.extend_from_slice(extra);
            }
            sequence += 1;
            append_compact_record(
                &mut stream,
                sequence,
                hour,
                COMPACT_DOMAIN_COUNTRY_IDENTITY,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                0,
                &country_identity_payload(&identities),
            );
            sequence += 1;
            append_compact_record(
                &mut stream,
                sequence,
                hour,
                COMPACT_DOMAIN_TECH,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                0,
                &payload,
            );
            sequence += 1;
            append_compact_record(
                &mut stream,
                sequence,
                hour,
                compact_contract::COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                0,
                &frame_end_payload(),
            );
        }
        fold.ingest_stream(&stream);
        fold.finish()
            .into_iter()
            .map(|(_, _, bytes)| {
                serde_json::from_slice::<Value>(&bytes).expect("the fold emits JSON")
            })
            .collect::<Vec<_>>()
    };

    let clean = fold_of(&[]);
    let damaged = fold_of(&[0x00]);
    assert_eq!(clean.len(), 2);
    assert_eq!(damaged.len(), 2);
    assert_eq!(xui09_witnesses(&clean[0]), xui09_witnesses(&damaged[0]));
    assert_ne!(
        xui09_witnesses(&clean[1]),
        xui09_witnesses(&damaged[1]),
        "the clean patch really does change the world, or this proves nothing"
    );
    assert_eq!(
        xui09_witnesses(&damaged[1]),
        xui09_witnesses(&damaged[0]),
        "a record refused for trailing bytes must leave the accumulator untouched"
    );
}

#[test]
fn xui09_the_fold_refuses_an_active_slot_whose_identity_disagrees() {
    let identities = [(7u32, "GER")];
    let mut row = witness_row(7, &[(0, 4), (1, 5), (2, 6), (3, 4), (4, 1)]);
    row.research.as_mut().unwrap().completed_tech_count = Some(2);
    row.research.as_mut().unwrap().active_research = Some(vec![active_slot_row(1, "fighter1")]);

    let good = folded_hours(&[(vec![row.clone()], 0)], &identities, false).remove(0);
    assert!(
        good["countries"]["GER"]["research"]["technology_states"].is_array(),
        "the honest payload publishes, or the refusal below proves nothing"
    );

    let mut tracker = CompactTechCountryDeltaTracker::default();
    let payload = compact_tech_data_payload_delta(
        std::slice::from_ref(&row),
        0,
        0,
        &mut tracker,
        &fixture_dictionary(),
    );
    let at = payload
        .windows(8)
        .position(|window| window == b"fighter1")
        .expect("the active slot carries its name on the wire");
    let mut renamed = payload.clone();
    renamed[at..at + 8].copy_from_slice(b"fighterX");

    let mut fold = recorder_host::world_fold::WorldFold::new("jo_xui09_hostile_active");
    let mut stream = Vec::new();
    append_compact_record(
        &mut stream,
        1,
        FIXTURE_RAW_HOUR,
        COMPACT_DOMAIN_COUNTRY_IDENTITY,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &country_identity_payload(&identities),
    );
    append_compact_record(
        &mut stream,
        2,
        FIXTURE_RAW_HOUR,
        COMPACT_DOMAIN_TECH,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &renamed,
    );
    append_compact_record(
        &mut stream,
        3,
        FIXTURE_RAW_HOUR,
        compact_contract::COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        0,
        &frame_end_payload(),
    );
    fold.ingest_stream(&stream);
    let snapshot: Value = serde_json::from_slice(&fold.finish().remove(0).2).expect("JSON");
    let research = &snapshot["countries"]["GER"]["research"];
    assert!(
        research.get("technology_states").is_none(),
        "the fold must not publish a witness whose active slot the consumer cannot join: {research}"
    );
    assert_eq!(
        research["technology_states_unmeasured_reason"],
        json!(recorder_host::world_fold::XUI09_REFUSAL_CROSS_SEAL_ACTIVE),
        "and it must name the cross-seal that failed"
    );
}

#[test]
fn xui09_a_repeated_technology_identity_is_refused_and_recovers() {
    let identities = [(7u32, "GER")];
    let colliding = TechnologyIdDictionary {
        by_definition_index: [
            (0u32, "infantry_weapons"),
            (1, "fighter1"),
            (2, "fighter1"),
            (3, "electronic_mechanical_engineering"),
            (4, "construction1"),
        ]
        .into_iter()
        .map(|(index, id)| (index, (0usize, id.to_string())))
        .collect(),
    };
    let row = witness_row(7, &[(0, 4), (1, 3), (2, 6), (3, 4), (4, 1)]);
    assert_eq!(
        xui09_refusal_of(&row, &colliding),
        Some(Xui09RefusalReason::DuplicateTechnologyId as u32),
        "a country whose vector carries one identity twice must be refused by the producer"
    );

    let mut tracker = CompactTechCountryDeltaTracker::default();
    let first =
        compact_tech_data_payload_delta(std::slice::from_ref(&row), 0, 0, &mut tracker, &colliding);
    let second = compact_tech_data_payload_delta(
        std::slice::from_ref(&row),
        0,
        0,
        &mut tracker,
        &fixture_dictionary(),
    );
    let mut fold = recorder_host::world_fold::WorldFold::new("jo_xui09_duplicate_id");
    let mut stream = Vec::new();
    let mut sequence = 0u64;
    for (hour, payload) in [(FIXTURE_RAW_HOUR, &first), (FIXTURE_RAW_HOUR + 1, &second)] {
        sequence += 1;
        append_compact_record(
            &mut stream,
            sequence,
            hour,
            COMPACT_DOMAIN_COUNTRY_IDENTITY,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &country_identity_payload(&identities),
        );
        sequence += 1;
        append_compact_record(
            &mut stream,
            sequence,
            hour,
            COMPACT_DOMAIN_TECH,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            payload,
        );
        sequence += 1;
        append_compact_record(
            &mut stream,
            sequence,
            hour,
            compact_contract::COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &frame_end_payload(),
        );
    }
    fold.ingest_stream(&stream);
    let hours: Vec<Value> = fold
        .finish()
        .into_iter()
        .map(|(_, _, bytes)| serde_json::from_slice(&bytes).expect("JSON"))
        .collect();
    assert_eq!(hours.len(), 2, "neither record was refused outright");
    assert!(
        hours[0]["countries"]["GER"]["research"]
            .get("technology_states")
            .is_none(),
        "the colliding hour carries no witness"
    );
    assert!(
        hours[1]["countries"]["GER"]["research"]["technology_states"].is_array(),
        "and the country publishes again as soon as its identities are unique"
    );
}

fn derived_active_slot_row() -> CountryResearchSlotRow {
    let rendered = |raw: i64| raw as f64 / 100_000.0;
    CountryResearchSlotRow {
        ahead_of_time_years_raw: Some(300_273),
        ahead_of_time_years: Some(rendered(300_273)),
        ahead_of_time_penalty_raw: Some(300_273),
        ahead_of_time_penalty_pct: Some(rendered(300_273)),
        ahead_of_time_reduction_years_raw: Some(50_000),
        ahead_of_time_reduction_years: Some(rendered(50_000)),
        ahead_of_time_reduced_penalty_raw: Some(250_136),
        ahead_of_time_reduced_penalty_pct: Some(rendered(250_136)),
        ahead_of_time_derived: Some(CountryResearchAheadDerivedAuthority {
            authority: A29_DERIVED_PRODUCER_AUTHORITY,
            producer_game_hour_raw: FIXTURE_RAW_HOUR,
            penalty_factor_raw: 100_000,
        }),
        ..active_slot_row(1, "fighter1")
    }
}

#[test]
fn xui09_a_derived_quartet_reaches_the_snapshot_under_the_words_the_consumer_reads() {
    let identities = [(7u32, "GER")];
    let mut row = witness_row(7, &[(0, 4), (1, 5), (2, 6), (3, 4), (4, 1)]);
    row.research.as_mut().unwrap().completed_tech_count = Some(2);
    row.research.as_mut().unwrap().active_research = Some(vec![derived_active_slot_row()]);

    let snapshot = folded_hours(&[(vec![row], 0)], &identities, false).remove(0);
    let slot = &snapshot["countries"]["GER"]["research"]["in_progress_live"][0];
    assert_eq!(
        slot["tech_name"],
        json!("fighter1"),
        "the slot survived: {slot}"
    );

    assert_eq!(slot["ahead_of_time_authority"], json!("derived"));
    assert_eq!(
        slot["ahead_of_time_derived_producer_game_hour_raw"],
        json!(FIXTURE_RAW_HOUR)
    );
    assert_eq!(
        slot["ahead_of_time_derived_penalty_factor_raw"],
        json!(100_000)
    );
    assert_eq!(slot["ahead_of_time_fixed_scale"], json!(100_000));
    assert_eq!(slot["ahead_of_time_years_raw"], json!(300_273));
    assert_eq!(slot["ahead_of_time_penalty_raw"], json!(300_273));
    assert_eq!(slot["ahead_of_time_reduction_years_raw"], json!(50_000));
    assert_eq!(slot["ahead_of_time_reduced_penalty_raw"], json!(250_136));
    assert_eq!(slot["ahead_of_time_years"], json!(3.00273));
    assert_eq!(slot["ahead_of_time_reduced_penalty_pct"], json!(2.50136));
    assert!(
        slot.get("ahead_of_time_provenance").is_none(),
        "a derived quartet carries no observed receipt"
    );

    let days = super::a29_days_ahead_of_time(1939, FIXTURE_RAW_HOUR, 0).expect("days");
    assert_eq!(days, 1_096);
    assert_eq!(super::a29_years_from_days(days), Some(300_273));
    assert_eq!(super::a29_penalty_from_days(days, 100_000), Some(300_273));
    let reduced = super::a29_days_ahead_of_time(1939, FIXTURE_RAW_HOUR, 50_000).expect("reduced");
    assert_eq!(
        super::a29_penalty_from_days(reduced, 100_000),
        Some(250_136)
    );
}
