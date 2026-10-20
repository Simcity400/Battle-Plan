//! The registered faction route, its cold gate and immutable supplier cache.
use super::*;
use crate::products::faction_state::prepared::Prepared;
use dashboard_base::test_fixtures;
use replay_data::rows::*;
use replay_frame::frame::{DecodedFrame, TableView};
use serde_json::json;
fn frame(hour: i32) -> DecodedFrame {
    let mut frame = test_fixtures::source(1, hour, Some(0));
    *frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow {
            id: 101,
            canonical_slot: 7,
            tag: b"mod".to_vec(),
            ..Default::default()
        }]));
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
            id: 101,
            canonical_slot: 7,
        }]));
    *frame.tables[TableId::Factions.index()] =
        TableView::Complete(TableRows::Factions(vec![FactionRow {
            id: 9,
            ..Default::default()
        }]));
    *frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![FactionMemberRow {
            faction: 9,
            ordinal: 0,
            country: 101,
        }]));
    *frame.tables[TableId::FactionAccounts.index()] =
        TableView::Complete(TableRows::FactionAccounts(vec![FactionAccountRow {
            account: 3,
            country: 101,
            influence_share_raw: 123,
            ..Default::default()
        }]));
    frame
}
#[test]
fn faction_state_hub_cold_cache_and_clear_follow_their_published_supplier() {
    let frames = vec![frame(100), frame(101)];
    let (source, root) = test_fixtures::file_source(test_fixtures::replay(frames.clone()));
    let _root = crate::catalogue::tests::Folder(root);
    let (store, key) = super::super::files(&source, None).unwrap();
    let flag = AtomicBool::new(false);
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: "faction-state".into(),
        params: json!({"view":"influence","branchId":"1","faction":"9"}),
    };
    let ask = || -> Value {
        serde_json::from_str(
            &crate::prepared::pipeline::with_test_hourly(|| {
                super::query(&source, &request, None, &flag)
            })
            .unwrap(),
        )
        .unwrap()
    };
    let cold = ask();
    assert_eq!(cold["data"]["coverage"]["lastHour"], 99);
    assert_eq!(cold["data"]["pending"][0]["firstHour"], 100);
    assert_eq!(cold["gate"], json!(["walk"]));
    let mut state = Prepared::default();
    for frame in &frames {
        state
            .observe(
                &source,
                &crate::products::LoadContext { content: None },
                frame,
                &crate::prepared::frame_changes::FrameChanges::alone(
                    frame,
                    source.replay_ref().format,
                ),
                &flag,
            )
            .unwrap();
    }
    let mut writer = store.replace(key.clone()).unwrap();
    state.flush(&mut writer, 1, &flag).unwrap();
    crate::prepared::coverage::publish_coverage(&mut writer, 1, 100, 101, true).unwrap();
    writer.commit().unwrap();
    let invalidate = || {
        crate::prepared::invalidate(
            &mut source.state::<crate::prepared::Progress>().lock().unwrap(),
        )
    };
    invalidate();
    let ready = ask();
    assert_eq!(ready["data"]["accounts"][0]["runs"][0]["shareRaw"], "123");
    assert_eq!(ready["data"]["coverage"]["complete"], true);
    let computations =
        || dashboard_base::prepared::query::observations::usage(&source, "queries:faction-state").2;
    let previous = computations();
    let (again, decoded) = dashboard_base::prepared::query::progressive::without_block_decodes(ask);
    assert!(!decoded);
    assert_eq!(again, ready);
    assert_eq!(computations(), previous);
    let mut writer = store.extend(&key).unwrap();
    writer
        .write_at("unrelated-faction-test", 0, b"unrelated".as_slice())
        .unwrap();
    writer.commit().unwrap();
    invalidate();
    assert_eq!(ask(), ready);
    assert_eq!(computations(), previous);
    store.clear().unwrap();
    invalidate();
    assert_eq!(ask()["data"]["accounts"], json!([]));
    assert!(computations() > previous);
    assert!(
        crate::products::PRODUCTS
            .iter()
            .any(|p| p.key == "faction-state" && p.content_dependent)
    );
}
