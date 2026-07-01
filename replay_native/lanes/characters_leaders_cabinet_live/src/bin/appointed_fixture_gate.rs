use characters_leaders_cabinet_live_lane::domain47_characters_leaders_cabinet_live as compact;
use characters_leaders_cabinet_live_lane::extractor::{
    appointed_advisors_from_active_ideas, encode_frame_payload, encode_frame_record,
    validate_registry_assignment, AdvisorRecord, LaneFrame,
};

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    validate_registry_assignment(include_str!("../../../../domain_registry.json"))
        .map_err(|err| err.to_string())?;

    let roster = eng_fixture_roster();
    let before = appointed_advisors_from_active_ideas(&roster, &[]);
    let after_add = appointed_advisors_from_active_ideas(
        &roster,
        &[
            "ENG_leslie_hore_belisha",
            "ENG_maxwell_aitken",
            "ENG_philip_kerr",
        ],
    );
    let after_remove = appointed_advisors_from_active_ideas(&roster, &["ENG_maxwell_aitken"]);

    let mut frame = LaneFrame::new(0);
    frame.advisor_roster = roster;
    frame.appointed_advisors = after_add;
    let payload = encode_frame_payload(&frame);
    let record = encode_frame_record(1, 1946051909, &frame);
    let summary = compact::decode_payload(0, &payload);
    if !summary.parse_errors.is_empty() {
        return Err(format!(
            "domain47 appointed fixture parse errors: {:?}",
            summary.parse_errors
        ));
    }

    println!(
        "{{\"domain\":\"{}\",\"domain_id\":{},\"result\":\"validated\",\"transition_counts\":{{\"before\":{},\"after_add\":{},\"after_remove\":{}}},\"payload_bytes\":{},\"record_bytes\":{},\"advisor_roster_rows\":{},\"appointed_advisor_rows\":{},\"schema_flags_or\":{},\"parse_errors\":[]}}",
        compact::DOMAIN_NAME,
        compact::DOMAIN_ID,
        before.len(),
        summary.appointed_advisor_rows,
        after_remove.len(),
        payload.len(),
        record.len(),
        summary.advisor_roster_rows,
        summary.appointed_advisor_rows,
        summary.schema_flags_or,
    );
    Ok(())
}

fn eng_fixture_roster() -> Vec<AdvisorRecord> {
    [
        (1459, "ENG_leslie_hore_belisha"),
        (1458, "ENG_maxwell_aitken"),
        (1456, "ENG_philip_kerr"),
    ]
    .into_iter()
    .map(|(char_id, token)| AdvisorRecord {
        country_tag_db_id: 2,
        char_id,
        slot: Some("political_advisor".to_string()),
        ledger: Some("political_advisor".to_string()),
        idea_token: token.to_string(),
    })
    .collect()
}
