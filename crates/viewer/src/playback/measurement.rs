//! Read-only decoder attribution on an explicitly supplied recording.
use super::*;
use std::{
    collections::{BTreeMap, VecDeque},
    hint::black_box,
    path::Path,
    time::Instant,
};

fn timings(samples: &[f64]) -> serde_json::Value {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable_by(f64::total_cmp);
    if sorted.is_empty() {
        return serde_json::json!({"count": 0});
    }
    let percentile = |percent| sorted[(sorted.len() - 1) * percent / 100];
    serde_json::json!({"count": sorted.len(), "sumMs": sorted.iter().sum::<f64>(),
        "p10Ms": percentile(10), "p50Ms": percentile(50), "p90Ms": percentile(90),
        "maxMs": sorted.last().unwrap()})
}

#[derive(Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CopyInputs {
    copies: u64,
    baseline_rows: u64,
    baseline_row_allocation_bytes: u64,
    changed_rows: u64,
    deleted_rows: u64,
    first_hour: Hour,
    first_rows: usize,
    first_row_allocation_bytes: usize,
    first_heap_bytes: usize,
    max_rows: usize,
    max_row_allocation_bytes: usize,
}

// This is attribution-only: ordinary wall trials never inspect the inputs.
// A baseline held by an output must use the shared merge on its next delta.
fn copy_inputs(
    decoder: &FrameDecoder,
    frame: &Frame,
    held: &VecDeque<Current>,
    counts: &mut BTreeMap<TableId, CopyInputs>,
) {
    let Some(previous) = decoder.current() else {
        return;
    };
    for table in TableId::ALL {
        let replay_frame::frame::TableUpdate::Delta {
            changed, deleted, ..
        } = &frame.tables[table.index()]
        else {
            continue;
        };
        let view = &previous.tables[table.index()];
        let TableView::Complete(rows) = &**view else {
            continue;
        };
        if !held.iter().any(|current| {
            matches!(current, Current::Frame(old)
            if old.tables[table.index()].as_ptr() == view.as_ptr())
        }) {
            continue;
        }
        let count = counts.entry(table).or_default();
        if count.copies == 0 {
            count.first_hour = frame.hour;
            count.first_rows = rows.len();
            count.first_row_allocation_bytes = rows.allocated_bytes();
            count.first_heap_bytes = rows.heap_bytes();
        }
        count.copies += 1;
        count.baseline_rows += rows.len() as u64;
        count.baseline_row_allocation_bytes += rows.allocated_bytes() as u64;
        count.changed_rows += changed.len() as u64;
        count.deleted_rows += deleted.len() as u64;
        count.max_rows = count.max_rows.max(rows.len());
        count.max_row_allocation_bytes = count.max_row_allocation_bytes.max(rows.allocated_bytes());
    }
}

#[test]
#[ignore = "CPU benchmark needs BATTLE_PLAN_MAP_BENCH_REPLAY and BATTLE_PLAN_GAME_DIR"]
fn decoder_apply_and_materialize_measurement() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let path = std::env::var_os("BATTLE_PLAN_MAP_BENCH_REPLAY").or_else(|| std::env::var_os("HOI4_MAP_BENCH_REPLAY")).expect("BATTLE_PLAN_MAP_BENCH_REPLAY");
    let replay = Replay::open_path(Path::new(&path)).unwrap();
    let attribute = std::env::var_os("BATTLE_PLAN_MAP_DECODE_ATTRIBUTION").or_else(|| std::env::var_os("HOI4_MAP_DECODE_ATTRIBUTION")).is_some();
    if attribute {
        replay_frame::cpu::enable();
    }
    let workers = std::env::var("BATTLE_PLAN_MAP_DECODE_WORKERS").or_else(|_| std::env::var("HOI4_MAP_DECODE_WORKERS"))
        .map_or(1, |value| value.parse::<usize>().unwrap().max(1));
    let parallel = replay::container::Parallel {
        workers,
        on_worker: &|| {},
    };
    let map_tables = dashboard_base::prepared_map::selected_tables();
    let only_stride = std::env::var("BATTLE_PLAN_MAP_DECODE_STRIDE").or_else(|_| std::env::var("HOI4_MAP_DECODE_STRIDE"))
        .ok()
        .map(|value| value.parse::<usize>().unwrap());
    let mut reports = Vec::new();
    let rounds = std::env::var("BATTLE_PLAN_MAP_DECODE_ROUNDS").or_else(|_| std::env::var("HOI4_MAP_DECODE_ROUNDS"))
        .map_or(if attribute { 1 } else { 3 }, |value| {
            value.parse::<usize>().unwrap()
        });
    let selections = match std::env::var("BATTLE_PLAN_MAP_DECODE_SELECTION").or_else(|_| std::env::var("HOI4_MAP_DECODE_SELECTION")).as_deref() {
        Ok("map") => vec!["map"],
        Ok("all") => vec!["all"],
        _ => vec!["all", "map"],
    };
    // Interleave dense and sparse targets. Eight retained outputs model an
    // occupied builder pool: dropping each clone immediately hides COW work.
    for round in 0..rounds {
        for &selection in &selections {
            for (stride, behind) in [
                (0, 0),
                (1, 0),
                (1, 2),
                (3, 0),
                (3, 2),
                (6, 0),
                (6, 2),
                (15, 0),
                (15, 2),
            ] {
                if only_stride.is_some_and(|only| only != stride) {
                    continue;
                }
                // Attribute separately from the ordinary wall-time trials:
                // per-table accounting is opt-in and has its own overhead.
                let cpu_before = attribute.then(replay_frame::cpu::snapshot);
                let phase = replay_frame::cpu::scope_named(
                    "decode_measurement",
                    &format!("{selection}:{stride}:{behind}"),
                );
                let started = Instant::now();
                let mut source = Vec::new();
                let mut keyframes = Vec::new();
                let mut deltas = Vec::new();
                let mut materialize = Vec::new();
                let mut retire = Vec::new();
                let mut held = VecDeque::new();
                let mut copies = BTreeMap::new();
                let mut hours = 0_usize;
                for block in &replay.index {
                    let reading = Instant::now();
                    let frames = if selection == "map" {
                        if workers > 1 {
                            replay.read_block_selected_parallel(block, map_tables, &parallel)
                        } else {
                            replay.read_block_selected(block, map_tables)
                        }
                    } else if workers > 1 {
                        replay.read_block_parallel(block, &parallel)
                    } else {
                        replay.read_block(block)
                    }
                    .unwrap();
                    source.push(reading.elapsed().as_secs_f64() * 1000.0);
                    let mut decoder = if selection == "map" {
                        FrameDecoder::for_tables(map_tables)
                    } else {
                        FrameDecoder::new()
                    };
                    for frame in &frames {
                        if attribute {
                            copy_inputs(&decoder, frame, &held, &mut copies);
                        }
                        let applying = Instant::now();
                        let decoded = decoder.advance(frame).unwrap();
                        let elapsed = applying.elapsed().as_secs_f64() * 1000.0;
                        if frame.keyframe {
                            keyframes.push(elapsed);
                        } else {
                            deltas.push(elapsed);
                        }
                        assert_eq!((decoded.branch, decoded.hour), (frame.branch, frame.hour));
                        if stride != 0
                            && (hours.is_multiple_of(stride) || hours % stride + behind >= stride)
                        {
                            let cloning = Instant::now();
                            let current = Current::Frame(Arc::new(decoded.clone()));
                            materialize.push(cloning.elapsed().as_secs_f64() * 1000.0);
                            held.push_back(black_box(current));
                            if held.len() > 8 {
                                let dropping = Instant::now();
                                drop(held.pop_front());
                                retire.push(dropping.elapsed().as_secs_f64() * 1000.0);
                            }
                        }
                        hours += 1;
                    }
                }
                let elapsed = started.elapsed().as_secs_f64();
                let targets = if stride == 0 {
                    0
                } else {
                    hours.div_ceil(stride)
                };
                let mut report = serde_json::json!({"round": round, "stride": stride, "cacheBehind": behind,
                    "deliveredTargets": targets, "nativeHoursPerSecond": hours as f64 / elapsed,
                    "targetsPerSecond": targets as f64 / elapsed,
                    "meanMsPerNativeHour": elapsed * 1000.0 / hours as f64,
                    "meanMsPerTarget": (targets != 0).then(|| elapsed * 1000.0 / targets as f64),
                    "selection": selection, "parseWorkers": workers,
                    "retainedOutputs": if stride == 0 {0} else {8}, "hours": hours,
                    "elapsedSeconds": elapsed,
                    "sourceReadInflateParse": timings(&source),
                    "keyframeApply": timings(&keyframes), "deltaApply": timings(&deltas),
                    "materialize": timings(&materialize), "retire": timings(&retire),
                    "debugAssertions": cfg!(debug_assertions), "attribution": attribute});
                drop(phase);
                if let Some(before) = cpu_before {
                    report["copyInputs"] = serde_json::Value::Array(
                        copies
                            .into_iter()
                            .map(|(table, count)| {
                                let mut value = serde_json::to_value(count).unwrap();
                                value["table"] = serde_json::json!(format!("{table:?}"));
                                value
                            })
                            .collect(),
                    );
                    report["cpuBefore"] = before;
                    report["cpuAfter"] = replay_frame::cpu::snapshot();
                }
                println!("DECODE_MEASUREMENT {report}");
                reports.push(report);
                if let Some(evidence) = std::env::var_os("BATTLE_PLAN_MAP_DECODE_EVIDENCE").or_else(|| std::env::var_os("HOI4_MAP_DECODE_EVIDENCE")) {
                    std::fs::write(evidence, serde_json::to_vec_pretty(&reports).unwrap()).unwrap();
                }
            }
        }
    }
}

#[test]
#[ignore = "CPU benchmark needs BATTLE_PLAN_MAP_BENCH_REPLAY and BATTLE_PLAN_GAME_DIR"]
fn selected_source_read_measurement() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let path = std::env::var_os("BATTLE_PLAN_MAP_BENCH_REPLAY").or_else(|| std::env::var_os("HOI4_MAP_BENCH_REPLAY")).expect("BATTLE_PLAN_MAP_BENCH_REPLAY");
    let replay = Replay::open_path(Path::new(&path)).unwrap();
    let mut selected = dashboard_base::prepared_map::selected_tables().to_vec();
    let selection = if let Ok(name) = std::env::var("BATTLE_PLAN_MAP_DECODE_TABLE").or_else(|_| std::env::var("HOI4_MAP_DECODE_TABLE")) {
        let table = selected
            .iter()
            .copied()
            .find(|table| format!("{table:?}") == name)
            .expect("BATTLE_PLAN_MAP_DECODE_TABLE must name a selected map table");
        selected = vec![table];
        name
    } else if std::env::var_os("BATTLE_PLAN_MAP_DECODE_NO_PANEL").or_else(|| std::env::var_os("HOI4_MAP_DECODE_NO_PANEL")).is_some() {
        selected.retain(|table| *table != TableId::DivisionPanelSources);
        "map_no_panel".to_owned()
    } else {
        "map".to_owned()
    };
    let attribute = std::env::var_os("BATTLE_PLAN_MAP_DECODE_ATTRIBUTION").or_else(|| std::env::var_os("HOI4_MAP_DECODE_ATTRIBUTION")).is_some();
    if attribute {
        replay_frame::cpu::enable();
    }
    let background_workers = std::env::var("BATTLE_PLAN_MAP_DECODE_WORKERS").or_else(|_| std::env::var("HOI4_MAP_DECODE_WORKERS")).map_or_else(
        |_| std::thread::available_parallelism().map_or(1, |count| count.get() / 2),
        |value| value.parse::<usize>().unwrap(),
    )
    .max(1);
    let mut reports = Vec::new();
    let rounds =
        std::env::var("BATTLE_PLAN_MAP_DECODE_ROUNDS").or_else(|_| std::env::var("HOI4_MAP_DECODE_ROUNDS")).map_or(3, |value| value.parse::<usize>().unwrap());
    for round in 0..rounds {
        for workers in [1, background_workers] {
            let before = attribute.then(replay_frame::cpu::snapshot);
            let phase =
                replay_frame::cpu::scope_named("source_measurement", &format!("{selection}:{workers}"));
            let parallel = replay::container::Parallel {
                workers,
                on_worker: &|| {},
            };
            let mut source = Vec::new();
            let mut hours = 0;
            for block in &replay.index {
                let reading = Instant::now();
                let frames = if workers > 1 {
                    replay.read_block_selected_parallel(block, &selected, &parallel)
                } else {
                    replay.read_block_selected(block, &selected)
                }
                .unwrap();
                source.push(reading.elapsed().as_secs_f64() * 1000.0);
                hours += frames.len();
                black_box(&frames);
            }
            drop(phase);
            let mut report = serde_json::json!({"round": round, "selection": selection,
                "parseWorkers": workers, "hours": hours, "sourceReadInflateParse": timings(&source),
                "debugAssertions": cfg!(debug_assertions), "attribution": attribute});
            if let Some(before) = before {
                report["cpuBefore"] = before;
                report["cpuAfter"] = replay_frame::cpu::snapshot();
            }
            println!("SOURCE_MEASUREMENT {report}");
            reports.push(report);
            if let Some(evidence) = std::env::var_os("BATTLE_PLAN_MAP_DECODE_EVIDENCE").or_else(|| std::env::var_os("HOI4_MAP_DECODE_EVIDENCE")) {
                std::fs::write(evidence, serde_json::to_vec_pretty(&reports).unwrap()).unwrap();
            }
        }
    }
}
