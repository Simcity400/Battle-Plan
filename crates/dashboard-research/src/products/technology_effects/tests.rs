use super::*;
use dashboard_base::{
    products::at_hour::entry,
    test_fixtures::{CAMPAIGN, records, source},
};
use replay::campaign::Replay;
use serde_json::{Value, json};
use std::sync::atomic::AtomicBool;

fn recording() -> Source {
    let frames = (20..=30).map(|hour| source(1, hour, None)).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    Source::from_replay(dashboard_base::SourceKey { source_id: "technology-effects".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
}

fn asked(source: &Source, request: Value) -> Result<Value, dashboard_core::QueryError> {
    let text = (entry::<TechnologyEffectsAt>().registered.load)(source, "lease", request, &LoadContext { content: None }, &AtomicBool::new(false))?;
    let mut reply: Value = serde_json::from_str(&text).unwrap();
    Ok(reply["data"].take())
}

#[test]
fn effects_without_the_recorded_content_are_withheld_by_name() {
    let reply = asked(&recording(), json!({ "branchId": "1", "country": "GER", "token": "basic_train", "hour": 25 })).unwrap();
    assert_eq!(reply["runs"], json!([]));
    assert!(reply["unavailable"].as_str().unwrap().contains("recorded content"), "{reply}");
}

#[test]
fn a_request_without_a_technology_is_refused() {
    let params = TechnologyEffectsParams { branch_id: "1".into(), country: serde_json::from_value(json!("GER")).unwrap(), token: String::new(), database_index: None };
    assert_eq!(TechnologyEffectsAt::validate(&params), Err("Technology effects need a technology"));
}
