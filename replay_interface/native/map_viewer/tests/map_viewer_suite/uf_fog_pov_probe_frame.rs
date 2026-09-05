//! The frame half of the fog POV probe, which lane TU named as untested.
//!
//! A recording carries ONE fog map. Asked about the country it carries, the
//! probe must report the `pov` operand present; asked about any other country it
//! must report it missing, so that country's refusal names the operand it lacks.
//! Only the identity half had a test before this file.

use map_viewer::fog_derivation_binding::FOG_DERIVATION_CONTRACT;
use map_viewer::fog_derivation_law::fog_binding_refusals;
use replay_core::{RawFrame, ReplaySource, SourceError, ViewStatus, WorldFrame, WorldTimeline};
use serde_json::{json, Map, Value};

const RECORDED_POV: &str = "D04";
const OTHER_POV: &str = "GER";
const RECORDED_POV_ID: u32 = 4;
const PROVINCE_SLOTS: usize = 8;
const HOUR_INDEX: i64 = 17_031_100;

struct InMemorySource {
    hours: Vec<i64>,
    rest: Map<String, Value>,
}

impl ReplaySource for InMemorySource {
    fn game_id(&self) -> &str {
        "uf-fog-pov-probe-frame"
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
            date: None,
            save_version: None,
            schema_version: None,
            rest: self.rest.clone().into(),
        })
    }
}

/// The exact FOW1 envelope `build_exact_fow1_visibility` admits.
fn fow1_authority() -> Value {
    json!({
        "schema": "FOW1",
        "schema_version": 1,
        "producer_rva": "0x01606ea0",
        "executable_sha256":
            "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
        "evidence_receipt_sha256":
            "77e4a1c964f868a7109cead30bc7df64860f2ff5097ff4849fede855b5d9e261",
        "complete_vector": true,
        "element_type": "u8",
        "index": "province_database_id",
        "default_level": 0,
        "unit_spotting_available": false,
        "capture_sequence": 91,
        "complete_pov_roster": false,
    })
}

fn frame_recording_one_pov() -> std::sync::Arc<WorldFrame> {
    let levels_hex = "ff".repeat(PROVINCE_SLOTS);
    let rest = json!({
        "countries": {
            RECORDED_POV: {"_r2_tag_db_id": RECORDED_POV_ID},
            OTHER_POV: {"_r2_tag_db_id": 7},
        },
        // The per-id country account is MANDATORY for a frame to name a
        // country: without it every identity is withheld and the visibility
        // view this fixture exists to carry never becomes Present.
        "tag_db_id_index": {"country_composition_version": 2},
        "visibility": {
            "authority": fow1_authority(),
            "povs": {
                RECORDED_POV: {
                    "pov_country_id": RECORDED_POV_ID,
                    "province_slot_count": PROVINCE_SLOTS,
                    "province_levels_encoding": "hex_u8",
                    "province_levels_u8_hex": levels_hex,
                    "spotted_units": null,
                },
            },
        },
    });
    let rest = rest.as_object().expect("snapshot object").clone();
    let timeline = WorldTimeline::open(Box::new(InMemorySource {
        hours: vec![HOUR_INDEX],
        rest,
    }))
    .expect("timeline");
    timeline.frame(HOUR_INDEX).expect("frame")
}

/// The recorded operands the gate reports missing for `requested`.
fn missing_recorded_fields(frame: &WorldFrame, requested: Option<&str>) -> Vec<String> {
    fog_binding_refusals(frame, requested)
        .missing_recorded()
        .iter()
        .map(|field| (*field).to_string())
        .collect()
}

/// Kills: answering the POV probe with "does this recording carry ANY usable fog
/// map". A recording carrying country A, asked about country B, would then
/// report B's `pov` operand present and leave it out of B's refusal.
#[test]
fn the_pov_operand_is_present_only_for_the_country_the_recording_carries() {
    let frame = frame_recording_one_pov();
    assert_eq!(
        frame.visibility_for(RECORDED_POV).status,
        ViewStatus::Present,
        "the fixture must actually carry a POV map or this test proves nothing"
    );

    let for_recorded = missing_recorded_fields(&frame, Some(RECORDED_POV));
    assert!(
        !for_recorded.iter().any(|field| field == "pov"),
        "the recorded POV carries its own map: {for_recorded:?}"
    );

    let for_other = missing_recorded_fields(&frame, Some(OTHER_POV));
    assert!(
        for_other.iter().any(|field| field == "pov"),
        "a country this recording carries no map for must have `pov` in its \
         refusal: {for_other:?}"
    );
}

/// Kills: collapsing the render's question into the recording's. With no
/// requested POV the question is "does this recording carry a usable map at
/// all", which this frame answers yes; the same frame must still answer no for a
/// country it carries no map for.
#[test]
fn the_recording_level_question_is_not_the_render_level_one() {
    let frame = frame_recording_one_pov();
    let recording_level = missing_recorded_fields(&frame, None);
    assert!(
        !recording_level.iter().any(|field| field == "pov"),
        "the recording carries one usable map: {recording_level:?}"
    );
    let for_other = missing_recorded_fields(&frame, Some(OTHER_POV));
    assert!(
        for_other.iter().any(|field| field == "pov"),
        "and it is not this country's: {for_other:?}"
    );
    assert_ne!(
        recording_level, for_other,
        "the two questions must be able to disagree"
    );
}

/// Kills: a probe list that drifts from the contract. Every field the probes
/// report has to be a contract row, or a refusal could name an operand the
/// binding contract does not describe.
#[test]
fn every_reported_field_is_a_contract_row() {
    let frame = frame_recording_one_pov();
    for requested in [Some(RECORDED_POV), Some(OTHER_POV), None] {
        for field in missing_recorded_fields(&frame, requested) {
            assert!(
                FOG_DERIVATION_CONTRACT
                    .iter()
                    .any(|binding| binding.field == field),
                "{field:?} is not a contract row"
            );
        }
    }
}
