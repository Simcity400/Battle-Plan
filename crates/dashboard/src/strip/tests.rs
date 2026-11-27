use super::patch::{Operation, apply::apply, encode};
use super::*;
use dashboard_base::{
    SourceKey,
    products::at_hour::interactive_pool_snapshot,
    test_fixtures::{replay, source},
};
use replay_data::rows::{NukeRow, TableId, TableRows};
use replay_frame::frame::TableView;
use serde_json::json;

fn recording() -> Arc<Source> {
    let frames = (100..=116)
        .filter(|hour| *hour != 108)
        .map(|hour| {
            let mut frame = source(1, hour, Some(i64::from(hour)));
            if hour != 110 {
                *frame.tables[TableId::Nukes.index()] =
                    TableView::Complete(TableRows::Nukes(vec![NukeRow {
                        country: 9001,
                        nuke_type: 4,
                        amount_raw: 713,
                        ready: hour,
                    }]));
            }
            frame
        })
        .collect();
    Arc::new(
        Source::from_replay(
            SourceKey {
                source_id: "strip-test".into(),
                revision: "1".into(),
            },
            "strip-test.battleplan".into(),
            Arc::new(replay(frames).into_timelines(&Default::default()).unwrap()),
            &false.into(),
        )
        .unwrap(),
    )
}

fn asks() -> Vec<Value> {
    (100..=116)
        .map(|hour| json!({"branchId": "1", "hour": hour}))
        .collect()
}

fn direct(source: &Arc<Source>, product: &str, asks: &[Value]) -> Vec<Value> {
    asks.iter()
        .map(|ask| {
            state(crate::answer::answer(
                source,
                product,
                ask,
                &LoadContext { content: None },
                &false.into(),
            ))
            .unwrap()
        })
        .collect()
}

fn audit(
    expected: &[Value],
    mut value: Value,
    forward: &[Vec<Operation>],
    backward: &[Vec<Operation>],
) {
    assert_eq!(value, expected[0]);
    let mut matched = 1;
    for (patch, expected) in forward.iter().zip(&expected[1..]) {
        value = apply(&value, patch).unwrap();
        assert_eq!(&value, expected);
        matched += 1;
    }
    assert_eq!(matched, 17);
    let mut matched = 1;
    for (patch, expected) in backward
        .iter()
        .rev()
        .zip(expected[..expected.len() - 1].iter().rev())
    {
        value = apply(&value, patch).unwrap();
        assert_eq!(&value, expected);
        matched += 1;
    }
    assert_eq!(matched, 17);
}

fn decode(
    bytes: &[u8],
    whole: bool,
    count: usize,
) -> (Option<Value>, Vec<Vec<Operation>>, Vec<Vec<Operation>>) {
    assert_eq!(
        &bytes[..8],
        &[b'B', b'P', b'S', b'T', 1, u8::from(whole), count as u8, 0]
    );
    let mut at = 8;
    let mut next = || {
        let len = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        at += 4;
        let part = &bytes[at..at + len];
        at += len;
        part
    };
    let base = whole.then(|| rmp_serde::from_slice(next()).unwrap());
    let mut forward = Vec::new();
    let mut backward = Vec::new();
    for _ in 1..count {
        forward.push(rmp_serde::from_slice(next()).unwrap());
        backward.push(rmp_serde::from_slice(next()).unwrap());
    }
    assert_eq!(at, bytes.len());
    (base, forward, backward)
}

#[test]
fn a_strip_rebuilds_seventeen_exact_answers_and_keeps_the_interactive_lanes() {
    let source = recording();
    source.in_scope(|| {
        let load = LoadContext { content: None };
        let cancelled = false.into();
        let expected = direct(&source, "nuclear-arsenal", &asks());
        assert_eq!(
            expected[8],
            json!({"refused": "Nuclear arsenal hour is not recorded"})
        );
        assert!(expected[10]["answer"]["missing"].is_string());
        assert_ne!(expected[0], expected[1]);
        for hour in [100, 104, 109, 111] {
            crate::answer::answer(
                &source,
                "nuclear-arsenal",
                &json!({"branchId":"1", "hour":hour}),
                &load,
                &cancelled,
            )
            .unwrap();
        }
        let held = interactive_pool_snapshot(&source).unwrap();
        assert!(!held.is_empty());
        let mut previous = None;
        for whole in [true, false] {
            let reply = crate::answer::answer(
                &source,
                PRODUCT,
                &json!({"product":"nuclear-arsenal", "asks":asks(), "whole":whole}),
                &load,
                &cancelled,
            )
            .unwrap();
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(reply["data"].as_str().unwrap())
                .unwrap();
            let (base, forward, backward) = decode(&bytes, whole, 17);
            assert_eq!(base, whole.then(|| expected[0].clone()));
            audit(
                &expected,
                base.unwrap_or_else(|| expected[0].clone()),
                &forward,
                &backward,
            );
            if whole {
                previous = Some((forward, backward));
            } else {
                assert_eq!(previous, Some((forward, backward)));
            }
            assert_eq!(interactive_pool_snapshot(&source).unwrap(), held);
        }
    });
    assert!(crate::answer::answers(PRODUCT, &Value::Null));
    assert!(crate::products::find(PRODUCT).is_none());
}

#[test]
fn a_strip_uses_the_series_answer_path_and_rejects_commands_and_bad_requests() {
    let source = recording();
    source.in_scope(|| {
        let load = LoadContext { content: None };
        let cancelled = false.into();
        let product = "series-rank";
        let ask = json!({"branchId":"1", "hour":100, "family":{"kind":"country-measure", "measure":"politicalPower"}});
        let expected = direct(&source, product, std::slice::from_ref(&ask));
        let reply = answer(&source, &json!({"product":product,"asks":[ask],"whole":true}), &load, &cancelled).unwrap();
        let bytes = base64::engine::general_purpose::STANDARD.decode(reply.data).unwrap();
        assert_eq!(decode(&bytes, true, 1).0, Some(expected[0].clone()));
        for product in dashboard_follow::registry::PRODUCTS {
            let error = answer(&source, &json!({"product":product,"asks":[{}],"whole":true}), &load, &cancelled).unwrap_err();
            assert_eq!(dashboard_core::refusal(&error), Some(format!("A strip cannot answer the {product} command").as_str()));
        }
        let recursive = json!({"product":PRODUCT,"asks":[{}],"whole":true});
        assert_eq!(dashboard_core::refusal(&answer(&source, &recursive, &load, &cancelled).unwrap_err()), Some("A strip cannot answer strip"));
        for params in [
            json!({"product":"nuclear-arsenal","asks":[],"whole":true}),
            json!({"product":"nuclear-arsenal","asks":vec![json!({});18],"whole":true}),
            json!({"product":"nuclear-arsenal","asks":[null],"whole":true}),
        ] {
            assert!(dashboard_core::refusal(&answer(&source, &params, &load, &cancelled).unwrap_err()).is_some());
        }
        assert!(answer(&source, &json!({"product":"unknown","asks":[{}],"whole":true}), &load, &cancelled).is_err());
        assert!(state(Err(io::Error::other("read failed"))).is_err());
    });
}

#[test]
fn cancellation_between_answers_and_the_reply_budget_are_preserved() {
    let cancelled = AtomicBool::new(false);
    let mut calls = 0;
    let error = build(3, true, &cancelled, |_| {
        calls += 1;
        cancelled.store(true, std::sync::atomic::Ordering::Release);
        Ok(json!({"answer":1}))
    })
    .unwrap_err();
    assert_eq!(calls, 1);
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    let mut bytes = Vec::new();
    let too_large = "x".repeat(dashboard_base::products::MAX_PRODUCT_REPLY_BYTES);
    let error = append(&mut bytes, rmp_serde::to_vec(&too_large).unwrap()).unwrap_err();
    assert!(dashboard_base::products::is_over_budget(&error));
}

#[test]
fn the_shared_fixture_is_the_real_encoders_bytes_in_both_modes() {
    let folder = std::env::var_os("CARGO_MANIFEST_DIR").expect("tests run with the crate's folder");
    let path =
        std::path::Path::new(&folder).join("../../ui/src/dashboard/strip/messages.fixture.json");
    let fixture: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let decode_fixture = |bytes: &Value| {
        bytes
            .as_array()
            .unwrap()
            .iter()
            .map(|byte| u8::try_from(byte.as_u64().unwrap()).unwrap())
            .collect::<Vec<_>>()
    };
    let states = fixture["states"].as_array().unwrap();
    for (whole, field) in [(true, "whole"), (false, "patchesOnly")] {
        let bytes = decode_fixture(&fixture[field]);
        assert_eq!(
            build(states.len(), whole, &false.into(), |index| Ok(states
                [index]
                .clone()))
            .unwrap(),
            bytes
        );
        let (base, forward, backward) = decode(&bytes, whole, states.len());
        assert_eq!(base, whole.then(|| states[0].clone()));
        let mut value = states[0].clone();
        for (patch, expected) in forward.iter().zip(&states[1..]) {
            value = apply(&value, patch).unwrap();
            assert_eq!(&value, expected);
        }
        for (patch, expected) in backward
            .iter()
            .rev()
            .zip(states[..states.len() - 1].iter().rev())
        {
            value = apply(&value, patch).unwrap();
            assert_eq!(&value, expected);
        }
    }

    let cases = fixture["arrayCases"].as_array().unwrap();
    assert_eq!(cases.len(), 6);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let states = case["states"].as_array().unwrap();
        let bytes = decode_fixture(&case["whole"]);
        assert_eq!(
            build(2, true, &false.into(), |index| Ok(states[index].clone())).unwrap(),
            bytes,
            "{name}"
        );
        let (base, forward, backward) = decode(&bytes, true, 2);
        assert_eq!(base.as_ref(), Some(&states[0]), "{name}");
        for (from, to, patch) in [(0, 1, &forward[0]), (1, 0, &backward[0])] {
            assert_eq!(apply(&states[from], patch).unwrap(), states[to], "{name}");
            let target = &states[to]["answer"]["rows"];
            assert!(
                encode(patch).unwrap().len() < rmp_serde::to_vec(target).unwrap().len(),
                "{name}"
            );
            if name == "edit budget exceeded" {
                assert_eq!(patch.len(), 20);
                assert!(
                    patch.iter().all(
                        |edit| matches!(edit, Operation::Replace(_, value) if value.is_object())
                    )
                );
            }
            if name == "scroll and edit" {
                assert!(patch.iter().any(|edit| matches!(edit, Operation::Replace(path, _) if path.last() == Some(&patch::Part::Key("count".into())))));
            }
        }
    }
}
