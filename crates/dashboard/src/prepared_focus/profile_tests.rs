//! Read-only recording comparison, using the same frame loop as preparation.
use super::*;
use replay::container::ReplayStream;
use std::{io::Write, path::PathBuf, time::Instant};

#[test]
#[ignore = "requires BATTLE_PLAN_GAME_DIR, BATTLE_PLAN_USER_DIR, BATTLE_PLAN_FOCUS_PROFILE_RECORDING and a private BATTLE_PLAN_FOCUS_PROFILE_DIR"]
fn real_recording_focus_frame_counts_timings_and_exact_output() {
    let required =
        |name: &str| PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("Set {name}")));
    let root = required("BATTLE_PLAN_FOCUS_PROFILE_DIR");
    assert!(root.is_absolute());
    assert!(
        !root
            .components()
            .any(|part| part == std::path::Component::ParentDir)
    );
    assert_eq!(required("LOCALAPPDATA"), root.join("localappdata"));
    assert_eq!(required("APPDATA"), root.join("appdata"));
    std::fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    assert!(
        !root.starts_with(repository),
        "profile output must stay outside Git"
    );
    for path in [required("LOCALAPPDATA"), required("APPDATA")] {
        std::fs::create_dir_all(&path).unwrap();
        assert!(path.canonicalize().unwrap().starts_with(&root));
    }
    engine::executable_tables::initialize_for_tests().unwrap();
    let recording = required("BATTLE_PLAN_FOCUS_PROFILE_RECORDING");
    let cancelled = AtomicBool::new(false);
    let source = Source::open(
        crate::SourceKey {
            source_id: "focus-frame-profile".into(),
            revision: "1".into(),
        },
        recording.clone(),
        &cancelled,
    )
    .unwrap();
    let content = ContentOptions {
        options: installed_content::ResolveOptions {
            game_dir: required("BATTLE_PLAN_GAME_DIR"),
            user_dir: required("BATTLE_PLAN_USER_DIR"),
            cache_dir: root.join("content-cache"),
        },
        allow_unverified: false,
    };
    // Binding and content loading are not hourly presentation work. Both paths
    // start with the same verified catalogue and existing interpreter content.
    let catalogue = crate::catalogue::catalogue(&source, &content).unwrap();
    catalogue.scripted_content().unwrap();
    let mut evaluators = [
        FocusEvaluator {
            legacy: true,
            ..Default::default()
        },
        FocusEvaluator::default(),
    ];
    for evaluator in &mut evaluators {
        evaluator.configure(&source, &content).unwrap();
    }
    let key = store::SourceKey::capture(&recording, &[]).unwrap();
    let stores = [
        store::Store::local(uuid::Uuid::new_v4()).unwrap(),
        store::Store::local(uuid::Uuid::new_v4()).unwrap(),
    ];
    let mut writers = [
        stores[0].replace(key.clone()).unwrap(),
        stores[1].replace(key).unwrap(),
    ];
    let mut accumulators: [BTreeMap<u64, FocusAccumulator>; 2] = Default::default();
    let mut indexes: [BTreeMap<u64, FocusIndex>; 2] = Default::default();
    let mut total_ns = [0u128; 2];
    let mut hours = 0usize;
    let output = root.join("focus-frame-profile.jsonl");
    let mut log = io::BufWriter::new(std::fs::File::create(&output).unwrap());
    let mut stream = ReplayStream::new(&TableId::ALL).unwrap();
    while stream.next_block(source.replay_ref(), |frame| {
        let mut rows = Vec::new();
        let mut results = Vec::new();
        let mut elapsed_ns = [0u128; 2];
        // Alternate which path runs first so warmed image/content caches do
        // not systematically favour either measurement.
        for side in if hours.is_multiple_of(2) { [0, 1] } else { [1, 0] } {
            let evaluator = &mut evaluators[side];
            evaluator.counts = FocusCounts::default();
            let calls = evaluator.interpreter_calls;
            let inputs = evaluator.scoped_input_builds;
            let start = Instant::now();
            let result = evaluator.write_test_frame(&mut writers[side], &source, Some(&content), frame,
                accumulators[side].entry(frame.branch).or_default(), indexes[side].entry(frame.branch).or_default(),
                None, &cancelled)?;
            let elapsed = start.elapsed().as_nanos();
            elapsed_ns[side] = elapsed;
            total_ns[side] += elapsed;
            rows.push(serde_json::json!({
                "path": if side == 0 { "before" } else { "after" },
                "nanoseconds": elapsed.to_string(),
                "interpreterCalls": evaluator.interpreter_calls - calls,
                "scopedInputBuilds": evaluator.scoped_input_builds - inputs,
                "counts": &evaluator.counts,
            }));
            results.push(result);
        }
        assert_eq!(results[0], results[1], "refusals at {}:{}", frame.branch, frame.hour);
        // Compare canonical endpoints after both paths' timed frame work. A2
        // materialises carried values at publication rather than each hour.
        for index in &mut indexes {
            index.get_mut(&frame.branch).unwrap().series.seal();
        }
        assert_eq!(serde_json::to_value(&indexes[0][&frame.branch])?,
            serde_json::to_value(&indexes[1][&frame.branch])?, "exact focus state at {}:{}", frame.branch, frame.hour);
        serde_json::to_writer(&mut log, &serde_json::json!({"branch": frame.branch.to_string(), "hour": frame.hour, "paths": rows}))?;
        log.write_all(b"\n")?;
        eprintln!("focus frame {}:{} before={:.3}ms after={:.3}ms", frame.branch, frame.hour,
            elapsed_ns[0] as f64 / 1_000_000.0, elapsed_ns[1] as f64 / 1_000_000.0);
        hours += 1;
        Ok(())
    }).unwrap() {}
    assert!(hours > 0, "recording supplied no accepted frames");
    for side in 0..2 {
        for index in indexes[side].values_mut() {
            index.flush(&mut writers[side], &cancelled).unwrap();
        }
    }
    for writer in writers {
        writer.commit().unwrap();
    }
    serde_json::to_writer(&mut log, &serde_json::json!({
        "summary": {"hours": hours, "beforeNs": total_ns[0].to_string(), "afterNs": total_ns[1].to_string(),
            "beforeInterpreterCalls": evaluators[0].interpreter_calls, "afterInterpreterCalls": evaluators[1].interpreter_calls,
            "beforeScopedInputBuilds": evaluators[0].scoped_input_builds, "afterScopedInputBuilds": evaluators[1].scoped_input_builds}
    })).unwrap();
    log.write_all(b"\n").unwrap();
    log.flush().unwrap();
    eprintln!(
        "focus profile: {hours} hours; before={:.3}s after={:.3}s; report={}",
        total_ns[0] as f64 / 1_000_000_000.0,
        total_ns[1] as f64 / 1_000_000_000.0,
        output.display()
    );
    for store in stores {
        store.clear().unwrap();
    }
}
