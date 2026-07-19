use compact_contract::{
    calendar::DATE_EPOCH_CONST, COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_MAGIC,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_FRAME_END, COMPACT_RECORD_VERSION,
};
use politics_tooltip_inputs_live_lane as politics73;
use recorder_host::world_fold::WorldFold;
use serde_json::{json, Value};

fn push_record(
    out: &mut Vec<u8>,
    sequence: u64,
    game_hour_raw: i32,
    domain_id: u16,
    record_type: u16,
    payload: &[u8],
) {
    out.extend_from_slice(COMPACT_RECORD_MAGIC);
    out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
    out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&sequence.to_le_bytes());
    out.extend_from_slice(&game_hour_raw.to_le_bytes());
    out.extend_from_slice(&domain_id.to_le_bytes());
    out.extend_from_slice(&record_type.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(payload);
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

fn reason_tree() -> Vec<politics73::ReasonNode> {
    vec![politics73::ReasonNode {
        source_ordinal: 10,
        node_id: "root".into(),
        parent_node_id: None,
        source_identity: "CIdea.allowed_to_remove".into(),
        result: false,
        resolved_reason: "Cannot replace this idea".into(),
    }]
}

fn term(ordinal: u32, source: &str, raw_value: i64) -> politics73::NumericSourceTerm {
    politics73::NumericSourceTerm {
        source_ordinal: ordinal,
        source_identity: source.into(),
        evaluation_context: "country=GER;pov=GER".into(),
        raw_value,
        scale: 1000,
    }
}

fn row(
    frame_hour: u64,
    target: &str,
    line_kind: politics73::PoliticsLineKind,
    typed_value: politics73::PoliticsTypedValue,
) -> politics73::PoliticsTooltipInput {
    politics73::PoliticsTooltipInput {
        frame_hour,
        country_tag: "GER".into(),
        target_kind: "idea".into(),
        target_token_or_slot: target.into(),
        line_kind,
        availability_state: politics73::AvailabilityState::Measured,
        typed_value: Some(typed_value),
        evaluation_context: "country=GER;pov=GER".into(),
    }
}

fn exact_frame(frame_hour: u64) -> politics73::PoliticsTooltipFrame {
    let rows = vec![
        row(
            frame_hour,
            "idea:trigger",
            politics73::PoliticsLineKind::EvaluatedTriggerResultAndReasonTree,
            politics73::PoliticsTypedValue::TriggerEvaluation {
                result: false,
                reason_tree: reason_tree(),
            },
        ),
        row(
            frame_hour,
            "idea:cost",
            politics73::PoliticsLineKind::EffectiveHireOrRemovalCost,
            politics73::PoliticsTypedValue::EffectiveCost {
                action: politics73::CostAction::Remove,
                resource_token: "political_power".into(),
                raw_value: 150_000,
                scale: 1000,
                affordable: true,
            },
        ),
        row(
            frame_hour,
            "idea:aggregate",
            politics73::PoliticsLineKind::AggregatedConditionalModifier,
            politics73::PoliticsTypedValue::ConditionalModifierAggregate {
                modifier_token: "political_power_gain".into(),
                raw_total: 25,
                scale: 1000,
                terms: vec![term(10, "base", 10), term(20, "conditional", 15)],
            },
        ),
        row(
            frame_hour,
            "slot:political_advisor",
            politics73::PoliticsLineKind::AdvisorSlotCapacityVacancyEligibilityAndDisabledReason,
            politics73::PoliticsTypedValue::AdvisorSlotState {
                capacity: 3,
                occupied: 2,
                vacancy: true,
                eligible: false,
                disabled_reason_tree: reason_tree(),
            },
        ),
        row(
            frame_hour,
            "modifier:timed",
            politics73::PoliticsLineKind::TimedDynamicModifierValueSourceAndExpiry,
            politics73::PoliticsTypedValue::TimedDynamicModifier {
                modifier_token: "war_support_modifier".into(),
                raw_value: -50,
                scale: 1000,
                source_token: "event:12".into(),
                applied_hour: Some(frame_hour - 10),
                expiry_hour: Some(frame_hour + 10),
            },
        ),
        row(
            frame_hour,
            "party:fascism",
            politics73::PoliticsLineKind::DisplayedPartyPopularityChangeSource,
            politics73::PoliticsTypedValue::PartyPopularityChange {
                party_token: "fascism".into(),
                raw_change: 30,
                scale: 1000,
                sources: vec![term(10, "drift", 10), term(20, "advisor", 20)],
            },
        ),
    ];
    politics73::PoliticsTooltipFrame {
        elapsed_wall_ms: 7,
        status_flags: 0,
        frame_hour,
        producer_manifest_sha256: [0xA5; 32],
        completeness_record_identity: format!("a729.politics.rows[{frame_hour}]"),
        expected_row_count: rows.len() as u32,
        rows,
    }
}

fn fold_frame(payload_frame_hour: u64, snapshot_hour: u64) -> Value {
    let payload = politics73::encode_payload(&exact_frame(payload_frame_hour)).unwrap();
    let raw = DATE_EPOCH_CONST + snapshot_hour as i32;
    let mut stream = Vec::new();
    push_record(
        &mut stream,
        1,
        raw,
        politics73::DOMAIN_ID,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &payload,
    );
    push_record(
        &mut stream,
        2,
        raw,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    let mut fold = WorldFold::new("a24_d73_integration");
    fold.ingest_stream(&stream);
    let snapshots = fold.finish();
    assert_eq!(snapshots.len(), 1);
    serde_json::from_slice(&snapshots[0].2).unwrap()
}

#[test]
fn all_six_a24_families_fold_with_exact_provenance() {
    let hour = 16_959_360u64;
    let snapshot = fold_frame(hour, hour);
    let trace = &snapshot["_r2_politics_tooltip_inputs_live"];
    assert_eq!(trace["authority"], json!("exact_a729_politics_producer"));
    assert_eq!(trace["status_flags"], json!(0));
    assert_eq!(trace["expected_row_count"], json!(6));
    assert_eq!(trace["rows"].as_array().unwrap().len(), 6);
    assert_eq!(
        trace["rows"][0]["typed_value"]["kind"],
        json!("trigger_evaluation")
    );
    assert_eq!(trace["rows"][1]["typed_value"]["raw_value"], json!(150_000));
    assert_eq!(trace["rows"][2]["typed_value"]["raw_total"], json!(25));
    assert_eq!(trace["rows"][3]["typed_value"]["vacancy"], json!(true));
    assert_eq!(
        trace["rows"][4]["typed_value"]["source_token"],
        json!("event:12")
    );
    assert_eq!(trace["rows"][5]["typed_value"]["raw_change"], json!(30));
    assert_eq!(trace["producer_manifest_sha256"], json!("a5".repeat(32)));
}

#[test]
fn pti1_frame_cannot_attach_to_a_different_snapshot_hour() {
    let hour = 16_959_360u64;
    let snapshot = fold_frame(hour, hour + 1);
    assert_eq!(snapshot["_r2_politics_tooltip_inputs_live"], Value::Null);
}
