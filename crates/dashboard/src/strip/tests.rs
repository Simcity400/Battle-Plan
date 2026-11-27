use super::patch::{Operation, apply::apply, encode};
use super::*;
use dashboard_base::{
    SourceKey,
    products::at_hour::interactive_pool_snapshot,
    test_fixtures::{
        replay, source, source_with_content,
        visual_mods::{Machine, RED, png, select},
    },
};
use replay_data::rows::{NukeRow, TableId, TableRows};
use replay_frame::frame::TableView;
use serde_json::json;

fn recording() -> Arc<Source> {
    recording_through(116, false)
}

fn recording_through(last: i32, growing: bool) -> Arc<Source> {
    let frames = (100..=last)
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
    let mut replay = replay(frames).into_timelines(&Default::default()).unwrap();
    if growing {
        replay.cursors.push(replay_recording::records::Cursor {
            source: "strip-test".into(),
            capture: 0,
            session: 1,
            segments: 1,
            finished: false,
        });
    }
    Arc::new(
        Source::from_replay(
            SourceKey {
                source_id: "strip-test".into(),
                revision: "1".into(),
            },
            "strip-test.battleplan".into(),
            Arc::new(replay),
            &false.into(),
        )
        .unwrap(),
    )
}

#[test]
fn settled_strip_groups_reopen_byte_for_byte_and_damaged_files_recompute() {
    let folder = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let load = LoadContext { content: None };
    let params = |product, hours: &[i32], whole| {
        json!({"product":product,"asks":hours.iter().map(|hour| json!({
            "branchId":"1", "hour":hour, "family":{"kind":"country-measure", "measure":"politicalPower"}
        })).collect::<Vec<_>>(), "whole":whole})
    };
    let exercise = |source: &Arc<Source>, requests: &[Value], load: &LoadContext| {
        let library = folder.join("library");
        std::fs::create_dir_all(&library).unwrap();
        let cache = cache::Cache::new(Some(folder.clone()), move || Ok(library.clone()), || false);
        let mut counts = [0; 2];
        let mut replies = Vec::new();
        for params in requests {
            let (reply, pending) = source.in_scope(|| {
                cache.answer(source, params, load, &false.into()).unwrap()
            });
            if let Some(pending) = pending {
                counts[usize::from(!matches!(&pending.action, cache::Action::Save(_)))] += 1;
                assert!(cache.store(pending));
            }
            replies.push(reply.data);
        }
        cache.shutdown();
        (counts, replies)
    };
    let page = [
        json!({"product":"nuclear-arsenal","asks":asks(),"whole":true}),
        params("series-rank", &[100, 104], true),
        params("series-rank", &[104, 116], false),
    ];
    let (computed, original) = exercise(&recording(), &page, &load);
    let (recomputed, reopened) = exercise(&recording(), &page, &load);
    assert_eq!((computed, recomputed), ([3, 0], [0, 3]));
    assert_eq!(reopened, original);

    let files: Vec<_> = std::fs::read_dir(folder.join("external")).unwrap().flatten()
        .filter(|file| file.path().extension().is_some_and(|ext| ext == "bpst"))
        .collect();
    assert_eq!(files.len(), 3);
    let bytes: u64 = files.iter().map(|file| file.metadata().unwrap().len()).sum();
    assert!(bytes > 0);
    std::fs::write(files[0].path(), [0]).unwrap();
    let (recomputed, repaired) = exercise(&recording(), &page, &load);
    assert_eq!(recomputed, [1, 2]);
    assert_eq!(repaired, original);
    assert_eq!(exercise(&recording(), &page, &load).0, [0, 3]);

    let growing = recording_through(116, true);
    let live_page = [
        params("series-rank", &[100, 104], true),
        params("series-rank", &[104, 116], false),
    ];
    let (stored, first) = exercise(&growing, &live_page, &load);
    let (stored_again, second) = exercise(&growing, &live_page, &load);
    assert_eq!((stored, stored_again), ([0, 0], [0, 0]));
    assert_eq!(first, second);
    assert_eq!(exercise(&recording(), &page, &load).0, [0, 3]);
    let finished = recording_through(120, false);
    assert_eq!(exercise(&finished, &live_page, &load).0, [2, 0]);
    assert_eq!(exercise(&finished, &live_page, &load).0, [0, 2]);
    assert!(files.into_iter().all(|file| !file.path().exists()));

    let machine = Machine::new("strip_reopen");
    machine.write_mod("art", "gfx/icon.png", &png(RED));
    let frames = (100..=116).map(|hour| source(1, hour, Some(i64::from(hour)))).collect();
    let (configured, options) = machine.open_frames(frames);
    let configured = Arc::new(configured);
    let mut load = LoadContext { content: Some(options) };
    let art_stack = |load: &LoadContext| {
        configured.in_scope(|| {
            configured.presented_for(load.content.as_ref().unwrap()).unwrap().content.layers.art_stack_key()
        })
    };
    let original_stack = art_stack(&load);
    assert_eq!(exercise(&configured, &page, &load).0, [3, 0]);
    assert_eq!(exercise(&configured, &page, &load).0, [0, 3]);
    select(&configured, load.content.as_mut().unwrap(), &["art"]);
    assert_ne!(art_stack(&load), original_stack);
    assert_eq!(exercise(&configured, &page, &load).0, [3, 0]);
    assert_eq!(exercise(&configured, &page, &load).0, [0, 3]);

    let (textured, load) = source_with_content(
        vec![source(1, 100, Some(100))], &folder.join("texture-setting"),
    );
    let textured = Arc::new(textured);
    let settings = load.content.as_ref().unwrap().options.user_dir.join("settings.txt");
    let page = [params("series-rank", &[100], true)];
    assert_eq!(exercise(&textured, &page, &load).0, [1, 0]);
    assert_eq!(exercise(&textured, &page, &load).0, [0, 1]);
    std::fs::write(settings, "language=l_english force_pow2_textures=yes").unwrap();
    assert_eq!(exercise(&textured, &page, &load).0, [1, 0]);
    assert_eq!(exercise(&textured, &page, &load).0, [0, 1]);
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn strip_writes_yield_to_priority_work() {
    let folder = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let library = folder.join("library");
    let busy = Arc::new(AtomicBool::new(false));
    let cache = cache::Cache::new(Some(folder.clone()), move || Ok(library.clone()), {
        let busy = Arc::clone(&busy);
        move || busy.load(std::sync::atomic::Ordering::Acquire)
    });
    let source = recording();
    let params = json!({"product":"nuclear-arsenal","asks":[{"branchId":"1","hour":100}],"whole":true});
    let load = LoadContext { content: None };
    let dropped = (0..2).filter(|_| {
        busy.store(false, std::sync::atomic::Ordering::Release);
        let (_, pending) = cache.answer(&source, &params, &load, &false.into()).unwrap();
        busy.store(true, std::sync::atomic::Ordering::Release);
        !cache.store(pending.unwrap())
    }).count();
    assert_eq!(dropped, 2);
    for (whole, count) in [(true, 0), (false, 1)] {
        let mut params = params.clone();
        params["whole"] = json!(whole);
        let (reply, pending) = cache.answer(&source, &params, &load, &false.into()).unwrap();
        let bytes = base64::engine::general_purpose::STANDARD.decode(reply.data).unwrap();
        assert_eq!(bytes, [b'B', b'P', b'S', b'T', 1, u8::from(whole), count, 0]);
        assert!(pending.is_none());
    }
    assert!(!folder.exists());
    cache.shutdown();
}

#[test]
fn a_strip_prefix_does_not_offer_a_disk_write() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let folder = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let library = folder.join("library");
    let checks = AtomicUsize::new(0);
    let cache = cache::Cache::new(Some(folder.clone()), move || Ok(library.clone()),
        move || checks.fetch_add(1, Ordering::Relaxed) != 0);
    let source = recording();
    let params = json!({"product":"nuclear-arsenal", "asks":asks(), "whole":true});
    let load = LoadContext { content: None };
    assert!(cache::key(&source, &params, &load).is_some());
    let (reply, pending) = cache.answer(&source, &params, &load, &false.into()).unwrap();
    let bytes = base64::engine::general_purpose::STANDARD.decode(reply.data).unwrap();
    assert!(decode(&bytes, true, 1).0.is_some());
    assert!(pending.is_none());
    assert!(!folder.exists());
    cache.shutdown();
}

#[test]
fn a_damaged_recording_copy_cannot_hit_the_healthy_files_groups() {
    let folder = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir(&folder).unwrap();
    let original = recording();
    let replay = original.replay_ref();
    let healthy = folder.join("healthy.battleplan");
    let damaged = folder.join("damaged.battleplan");
    std::fs::write(&healthy, replay.bytes()).unwrap();
    std::fs::write(&damaged, replay.damaged_table(&replay.index[0], TableId::Nukes).unwrap()).unwrap();
    let modified = healthy.metadata().unwrap().modified().unwrap();
    std::fs::OpenOptions::new().write(true).open(&damaged).unwrap().set_modified(modified).unwrap();
    let open = |path: &std::path::Path| {
        let path = path.canonicalize().unwrap();
        let key = SourceKey { source_id: "strip-test".into(), revision: dashboard_core::protocol::replay_revision(&path).unwrap() };
        Source::open(key, path, &false.into()).unwrap()
    };
    let healthy = open(&healthy);
    let damaged = open(&damaged);
    assert_eq!(healthy.replay_ref().revision(), damaged.replay_ref().revision());
    assert_eq!(healthy.key().revision, damaged.key().revision);
    let params = json!({"product":"nuclear-arsenal","asks":[{"branchId":"1","hour":100}],"whole":true});
    let load = LoadContext { content: None };
    assert_ne!(cache::key(&healthy, &params, &load).unwrap(), cache::key(&damaged, &params, &load).unwrap());
    drop((healthy, damaged, original));
    std::fs::remove_dir_all(folder).unwrap();
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
fn priority_work_stops_a_strip_at_the_next_ask_boundary() {
    let mut started = Vec::new();
    let priority = std::cell::Cell::new(false);
    let (bytes, complete) = build(4, true, &false.into(), || priority.get(), |index| {
        started.push(index);
        priority.set(index == 1);
        Ok(json!({"answer": index}))
    })
    .unwrap();
    assert_eq!(started, [0, 1]);
    assert!(!complete);
    let (base, forward, _) = decode(&bytes, true, 2);
    let base = base.unwrap();
    assert_eq!(base, json!({"answer": 0}));
    assert_eq!(apply(&base, &forward[0]).unwrap(), json!({"answer": 1}));
    let (continued, complete) = build(3, false, &false.into(), || false, |index| {
        Ok(json!({"answer": index + 1}))
    }).unwrap();
    assert!(complete);
    let (uninterrupted, complete) = build(4, true, &false.into(), || false, |index| {
        Ok(json!({"answer": index}))
    }).unwrap();
    assert!(complete);
    let mut stitched = bytes;
    stitched[6] = 4;
    stitched.extend_from_slice(&continued[8..]);
    assert_eq!(stitched, uninterrupted);
}

#[test]
fn cancellation_between_answers_and_the_reply_budget_are_preserved() {
    let cancelled = AtomicBool::new(false);
    let mut calls = 0;
    let error = build(3, true, &cancelled, || false, |_| {
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
            build(states.len(), whole, &false.into(), || false, |index| Ok(states
                [index]
                .clone()))
            .unwrap().0,
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
    assert_eq!(cases.len(), 7);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let states = case["states"].as_array().unwrap();
        let bytes = decode_fixture(&case["whole"]);
        assert_eq!(
            build(2, true, &false.into(), || false, |index| Ok(states[index].clone())).unwrap().0,
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
            if name == "replacement block" {
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
