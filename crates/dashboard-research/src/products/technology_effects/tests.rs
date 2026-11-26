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
    let text = (entry::<TechnologyEffectsAt>().load)(source, "lease", request, &LoadContext { content: None }, &AtomicBool::new(false))?;
    let mut reply: Value = serde_json::from_str(&text).unwrap();
    Ok(reply["data"].take())
}

#[test]
fn effects_without_the_recorded_content_are_withheld_by_name() {
    let source = recording();
    let reply = asked(&source, json!({ "branchId": "1", "country": "GER", "token": "basic_train", "hour": 25 })).unwrap();
    assert_eq!(reply["runs"], json!([]));
    assert!(reply["unavailable"].as_str().unwrap().contains("recorded content"), "{reply}");
    let request = json!({"branchId":"1", "country":"GER", "token":"basic_train"});
    let panel = dashboard_base::products::at_hour::exact_panel::<TechnologyEffectsAt>(&source, &LoadContext { content: None }, &request).unwrap();
    let reader = panel.exact().unwrap();
    for step in dashboard_base::test_fixtures::played(source.replay_ref(), reader, 20, 30) {
        let mut materialized = step.value.as_ref().as_ref().unwrap().clone();
        materialized["hour"] = step.hour.into();
        assert_eq!(materialized, (reader.cold)(step.hour, &false.into()).unwrap());
        if step.hour > 20 { assert_eq!(step.product_calls, 0); }
    }
    for hour in [24, 27] {
        let seed = (reader.advance)(hour, true, &false.into(), None);
        assert!(seed.fresh);
        assert_eq!(*seed.value, (reader.cold)(hour, &false.into()));
    }
}

