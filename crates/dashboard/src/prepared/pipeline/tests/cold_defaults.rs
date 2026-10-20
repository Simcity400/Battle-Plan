//! Default first views before any prepared page has been published.
use super::*;
use serde_json::{Value, json};

#[test]
fn cold_default_requests_publish_empty_extents_and_pending_hours() {
    independent::with_test_hourly(|| {
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![scale_frame(1, 10, 2), scale_frame(1, 11, 2)],
            )],
        )
        .unwrap();
        let mut fixture = Fixture::new(&replay);
        let source = fixture.source("cold-defaults");
        let flag = AtomicBool::new(false);
        let tag = String::from_utf8(crate::test_fixtures::scale_tag(0)).unwrap();
        let requests = [
            (
                "diplomacy-state",
                json!({"view":"pacts","branchId":"1","tags":null}),
            ),
            (
                "diplomacy-state",
                json!({"view":"proposals","branchId":"1","tags":null}),
            ),
            (
                "diplomacy-state",
                json!({"view":"pair","branchId":"1","first":tag,"second":tag,"hour":0}),
            ),
            ("country-groups", json!({"branchId":"1"})),
            ("subjects", json!({"branchId":"1"})),
            ("wars", json!({"branchId":"1","viewpointTag":tag})),
            (
                "wars",
                json!({"branchId":"1","viewpointTag":tag,"reads":[{"hour":11}]}),
            ),
            (
                "event-feed",
                json!({"branchId":"1","firstHour":null,"lastHour":11,
                "categories":["warsPeace","diplomacy","politics","milestones"],"cursor":null,"pageSize":256}),
            ),
            (
                "politics-ledger",
                json!({"branchId":"1","tag":tag,"view":"spending","hour":10,"totals":false}),
            ),
            (
                "series-state",
                json!({"branchId":"1","tags":[],"family":{"kind":"war-casualties","top":{"count":10}},"reads":[{"tag":"","hour":11}]}),
            ),
            (
                "casualty-pairs",
                json!({"branchId":"1","tags":[],"between":false,"window":null}),
            ),
            (
                "casualty-pairs",
                json!({"branchId":"1","tags":[],"between":false,"window":0}),
            ),
            (
                "casualty-breakdown",
                json!({"branchId":"1","tag":tag,"window":null}),
            ),
            (
                "casualty-breakdown",
                json!({"branchId":"1","tag":tag,"window":0}),
            ),
            (
                "events-page",
                json!({"branchId":"1","family":{"kind":"peace-deals"}}),
            ),
            ("construction-log-manifest", json!({"branchId":"1"})),
            (
                "construction-spending",
                json!({"branchId":"1","tag":tag,"hours":[11]}),
            ),
            (
                "production-select",
                json!({"hour":11,"selection":{"branchId":"1","tags":[],"top":10,
                "measure":"day","count":"units","grouping":"type","included":[],"refits":false},"offset":"0","limit":10,"ranked":true}),
            ),
            (
                "production-spending",
                json!({"branchId":"1","tag":tag,"refits":false,"from":null,"hour":11,"end":11}),
            ),
            ("land-battles-manifest", json!({"branchId":"1"})),
            ("land-battles", json!({"branchId":"1","window":0})),
            ("naval-battles-manifest", json!({"branchId":"1"})),
            ("naval-battles", json!({"branchId":"1","window":0})),
            (
                "air-superiority",
                json!({"branchId":"1","view":"moment","firstHour":10,"lastHour":11,"mode":"sides"}),
            ),
        ];
        for (product, params) in requests {
            let request = crate::transport::ProductRequest {
                source_id: source.key().source_id.clone(),
                revision: source.key().revision.clone(),
                product: product.into(),
                params,
            };
            let envelope: Value = serde_json::from_str(
                &crate::prepared::query::query(&source, &request, None, &flag)
                    .unwrap_or_else(|error| panic!("{product}: {error}")),
            )
            .unwrap();
            let data = &envelope["data"];
            assert!(data.is_object(), "{product}: {data}");
            assert_eq!(data["coverage"]["firstHour"], 10, "{product}");
            assert_eq!(data["coverage"]["lastHour"], 9, "{product}");
            assert_eq!(data["coverage"]["complete"], false, "{product}");
            assert!(data["pending"].is_array(), "{product}: {data}");
            if product == "series-state" {
                assert!(
                    data["unknownLabel"]
                        .as_str()
                        .is_some_and(|label| !label.is_empty())
                );
            }
            assert!(envelope["gate"].is_array(), "{product}: {envelope}");
            // Sampling may answer geometry, but it never publishes a value as
            // an accepted prepared hour before the first publication.
            assert!(
                data["pending"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|span| span["firstHour"].as_i64().unwrap() >= 10)
            );
        }
    });
}

#[test]
fn cold_cumulative_values_are_absent_instead_of_zero() {
    independent::with_test_hourly(|| {
        let replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![scale_frame(1, 10, 2)])])
                .unwrap();
        let mut fixture = Fixture::new(&replay);
        let source = fixture.source("cold-totals");
        let flag = AtomicBool::new(false);
        let tag = String::from_utf8(crate::test_fixtures::scale_tag(0)).unwrap();
        let data = crate::products::production_spending::prepared::query(
            &source,
            json!({"branchId":"1","tag":tag,"refits":false,"from":null,"hour":10,"end":10}),
            None,
            &flag,
        )
        .unwrap()
        .unwrap();
        assert!(data["leaves"].as_array().unwrap().is_empty());
        for field in ["total", "rocketTotal"] {
            let runs = data[field].as_array().unwrap();
            assert_eq!(runs.len(), 1);
            assert!(runs[0]["totalRaw"].is_null());
            assert_eq!(runs[0]["reason"], "preparing");
        }
    });
}

#[test]
fn stockpile_cold_sample_is_reused_when_preparation_moves() {
    independent::with_test_hourly(|| {
        use dashboard_base::prepared::query::{observations, progressive};
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![scale_frame(1, 10, 2), scale_frame(1, 11, 2)],
            )],
        )
        .unwrap();
        let mut fixture = Fixture::new(&replay);
        let source = fixture.source("stockpile-cold-cache");
        let flag = AtomicBool::new(false);
        let request = serde_json::json!({"branchId":"1","tag":String::from_utf8(crate::test_fixtures::scale_tag(0)).unwrap(),
            "from":11,"to":11,"width":1,"included":[]});
        let query = || {
            crate::products::equipment_stockpile::prepared::query(
                &source,
                &request,
                &crate::products::LoadContext { content: None },
                &flag,
            )
            .unwrap()
            .unwrap()
        };
        let cold = query();
        assert_eq!(cold["coverage"]["lastHour"], 9);
        let computations = observations::usage(&source, "stockpile:at-hour").2;
        assert_eq!(computations, 1);
        let (store, key) = crate::prepared::files(&source, None).unwrap();
        let mut writer = store.replace(key).unwrap();
        json(
            &mut writer,
            &coverage_section(1),
            &Coverage {
                schema: SCHEMA,
                first_hour: 10,
                last_hour: 10,
                complete: false,
            },
        )
        .unwrap();
        writer.commit().unwrap();
        let (mut advanced, refused) = progressive::without_block_decodes(query);
        assert!(!refused, "a re-ask must not repeat the stockpile look-back");
        assert_eq!(advanced["coverage"]["lastHour"], 10);
        advanced["coverage"] = cold["coverage"].clone();
        assert_eq!(advanced, cold, "only preparation metadata changes");
        assert_eq!(
            observations::usage(&source, "stockpile:at-hour").2,
            computations
        );
    });
}

#[test]
fn battle_prefix_pages_preserve_the_complete_pages_observed_facts() {
    independent::with_test_hourly(|| {
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![scale_frame(1, 10, 2), scale_frame(1, 11, 2)],
            )],
        )
        .unwrap();
        let mut fixture = Fixture::new(&replay);
        let source = fixture.source("battle-prefix-pages");
        let flag = AtomicBool::new(false);
        let (store, key) = crate::prepared::files(&source, None).unwrap();
        let mut writer = store.replace(key.clone()).unwrap();
        for family in ["land", "naval"] {
            crate::prepared::series::write_reply(
                &mut writer,
                &format!("{family}-battle-availability-1"),
                0,
                &true,
            )
            .unwrap();
            crate::prepared::series::write_reply(&mut writer, &format!("{family}-battles-1"), 0,
                &json!({"window":0,"interval":{"firstHour":10,"lastHour":11},"rows":[],"observed":[[10,11]],
                    "reasons":[],"warnings":[],"contentWarnings":[]})).unwrap();
        }
        crate::prepared::series::write_reply(
            &mut writer,
            "land-battle-warnings-1",
            0,
            &Vec::<String>::new(),
        )
        .unwrap();
        json(
            &mut writer,
            &coverage_section(1),
            &Coverage {
                schema: SCHEMA,
                first_hour: 10,
                last_hour: 11,
                complete: false,
            },
        )
        .unwrap();
        writer.commit().unwrap();
        let query = |product: &str| {
            let request = crate::transport::ProductRequest {
                source_id: source.key().source_id.clone(),
                revision: source.key().revision.clone(),
                product: product.into(),
                params: json!({"branchId":"1","window":0}),
            };
            serde_json::from_str::<Value>(
                &crate::prepared::query::query(&source, &request, None, &flag).unwrap(),
            )
            .unwrap()["data"]
                .clone()
        };
        let partial = [query("land-battles"), query("naval-battles")];
        let mut writer = store.extend(&key).unwrap();
        json(
            &mut writer,
            &coverage_section(1),
            &Coverage {
                schema: SCHEMA,
                first_hour: 10,
                last_hour: 11,
                complete: true,
            },
        )
        .unwrap();
        writer.commit().unwrap();
        for (product, mut partial) in ["land-battles", "naval-battles"].into_iter().zip(partial) {
            assert_eq!(
                partial["approximate"],
                json!([{"firstHour":10,"lastHour":11}])
            );
            let fields = partial.as_object_mut().unwrap();
            for field in ["coverage", "pending", "approximate", "warningsComplete"] {
                fields.remove(field);
            }
            assert_eq!(partial, query(product));
        }
    });
}

#[test]
fn losses_witness_the_selected_origin_without_waiting_for_complete_identity_history() {
    independent::with_test_hourly(|| {
        let replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![scale_frame(1, 10, 2)])])
                .unwrap();
        let mut fixture = Fixture::new(&replay);
        let source = fixture.source("cold-loss-origin");
        let flag = AtomicBool::new(false);
        let tag = String::from_utf8(crate::test_fixtures::scale_tag(0)).unwrap();
        let data = crate::products::equipment_losses::prepared::query(&source,
            &json!({"selection":{"branchId":"1","tag":tag,"landSource":"destruction", "country":{"slot":0,"countryId":100000,"originHour":10}},
                "from":10,"to":10,"width":1,"reads":[10],"headHour":10,"detailHour":10}),
            &crate::products::LoadContext { content: None }, &flag,
        ).unwrap();
        assert_eq!(data["coverage"]["lastHour"], 9);
        assert_eq!(data["pending"], json!([{"firstHour":10,"lastHour":10}]));
        assert!(data["designs"].as_array().unwrap().is_empty());
    });
}

#[test]
fn sinking_prefixes_are_revisable_until_preparation_finishes() {
    independent::with_test_hourly(|| {
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![scale_frame(1, 10, 2), scale_frame(1, 11, 2)],
            )],
        )
        .unwrap();
        let mut fixture = Fixture::new(&replay);
        let source = fixture.source("revisable-sinking-prefix");
        let flag = AtomicBool::new(false);
        let (store, key) = crate::prepared::files(&source, None).unwrap();
        let mut writer = store.replace(key).unwrap();
        publish_coverage(&mut writer, 1, 10, 10, false).unwrap();
        writer.commit().unwrap();
        for (product, params) in [
            ("ships-sunk", json!({"branchId":"1","hour":11})),
            (
                "naval-losses",
                json!({"branchId":"1","hour":11,"tags":[],"top":null,"kind":"both","attribution":"lost","period":"running"}),
            ),
        ] {
            let request = crate::transport::ProductRequest {
                source_id: source.key().source_id.clone(),
                revision: source.key().revision.clone(),
                product: product.into(),
                params,
            };
            let reply: Value = serde_json::from_str(
                &crate::prepared::query::query(&source, &request, None, &flag).unwrap(),
            )
            .unwrap();
            assert_eq!(
                reply["data"]["approximate"],
                json!([{"firstHour":10,"lastHour":10}]),
                "{product}"
            );
            assert_eq!(
                reply["data"]["pending"],
                json!([{"firstHour":11,"lastHour":11}]),
                "{product}"
            );
        }
    });
}

#[test]
fn ranged_feed_beyond_the_prefix_is_an_empty_pending_page() {
    independent::with_test_hourly(|| {
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![
                    scale_frame(1, 10, 2),
                    scale_frame(1, 11, 2),
                    scale_frame(1, 12, 2),
                ],
            )],
        )
        .unwrap();
        let mut fixture = Fixture::new(&replay);
        let source = fixture.source("ranged-feed-prefix");
        let flag = AtomicBool::new(false);
        let (store, key) = crate::prepared::files(&source, None).unwrap();
        let mut writer = store.replace(key).unwrap();
        publish_coverage(&mut writer, 1, 10, 10, false).unwrap();
        writer.commit().unwrap();
        let request = crate::transport::ProductRequest {
            source_id: source.key().source_id.clone(),
            revision: source.key().revision.clone(),
            product: "event-feed".into(),
            params: json!({"branchId":"1","firstHour":12,"lastHour":12,"pageSize":256,"categories":["milestones"]}),
        };
        let reply: Value = serde_json::from_str(
            &crate::prepared::query::query(&source, &request, None, &flag).unwrap(),
        )
        .unwrap();
        assert!(
            reply["data"]["reply"]["rows"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            reply["data"]["pending"],
            json!([{"firstHour":12,"lastHour":12}])
        );
        assert_eq!(reply["data"]["reply"]["firstHour"], 12);
        assert_eq!(reply["data"]["reply"]["lastHour"], 12);
    });
}

#[test]
fn settled_peace_event_prefixes_are_revisable() {
    independent::with_test_hourly(|| {
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![scale_frame(1, 10, 2), scale_frame(1, 11, 2)],
            )],
        )
        .unwrap();
        let mut fixture = Fixture::new(&replay);
        let source = fixture.source("revisable-peace-prefix");
        let flag = AtomicBool::new(false);
        let (store, key) = crate::prepared::files(&source, None).unwrap();
        let mut writer = store.replace(key).unwrap();
        publish_coverage(&mut writer, 1, 10, 10, false).unwrap();
        writer.commit().unwrap();
        for product in ["events-page", "events-clusters"] {
            let request = crate::transport::ProductRequest {
                source_id: source.key().source_id.clone(),
                revision: source.key().revision.clone(),
                product: product.into(),
                params: json!({"branchId":"1","family":{"kind":"peace-deals"},"from":10,"to":11,"width":2}),
            };
            let reply: Value = serde_json::from_str(
                &crate::prepared::query::query(&source, &request, None, &flag).unwrap(),
            )
            .unwrap();
            assert_eq!(
                reply["data"]["approximate"],
                json!([{"firstHour":10,"lastHour":10}]),
                "{product}"
            );
        }
    });
}

#[test]
fn diplomacy_state_is_registered_and_prepared_on_its_own() {
    use crate::products::Product;
    use crate::products::diplomacy_state::{DiplomacyParams, DiplomacyState};
    assert!(crate::products::find(DiplomacyState::KEY).is_some());
    let params: DiplomacyParams =
        serde_json::from_value(json!({"view":"opinions","branchId":"1","hour":10,"tags":[]}))
            .unwrap();
    DiplomacyState::validate(&params).unwrap();
    assert_eq!(gate("diplomacy-state"), vec!["diplomacy_state"]);
    assert!(
        FAMILIES
            .iter()
            .any(|family| family.name == "diplomacy_state")
    );
    independent::with_test_families(
        &[&independent::diplomacy::DIPLOMACY, &independent::IDENTITIES],
        || {
            let mut f = scale_frame(1, 10, 2);
            for table in [
                TableId::RelationOpinions,
                TableId::RelationTuples,
                TableId::SliceReads,
                TableId::LexicalTokens,
                TableId::IncomingDiplomaticActions,
            ] {
                *f.tables[table.index()] = replay_frame::frame::TableView::Complete(
                    replay_data::rows::TableRows::empty(table),
                );
            }
            let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![f])]).unwrap();
            let mut fixture = Fixture::new(&replay);
            let source = fixture.source("diplomacy-independent");
            run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
            let flag = AtomicBool::new(false);
            assert!(complete(&source, None, 1, "diplomacy-state", &flag).unwrap());
            let request = crate::transport::ProductRequest {
                source_id: source.key().source_id.clone(),
                revision: source.key().revision.clone(),
                product: "diplomacy-state".into(),
                params: json!({"view":"pacts","branchId":"1","tags":null}),
            };
            let reply: Value = serde_json::from_str(
                &crate::prepared::query::query(&source, &request, None, &flag).unwrap(),
            )
            .unwrap();
            assert_eq!(reply["data"]["coverage"]["complete"], true);
            assert_eq!(reply["data"]["pacts"], json!([]));
            super::super::super::files(&source, None)
                .unwrap()
                .0
                .clear()
                .unwrap();
        },
    );
}
