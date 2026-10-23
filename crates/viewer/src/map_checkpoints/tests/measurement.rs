//! Lead-run read-only measurements of the production shared map reader.
use super::*;
use replay::container::ReplayStream;
use std::{
    collections::BTreeSet,
    fs::File,
    io::{BufWriter, Write},
    path::Path,
    time::Instant,
};

mod aircraft_resolve;
mod attribution;
mod prefixes;
mod recording;
mod resources;

/// A prepared manifest entry: the range's pack, offset and length.
const MANIFEST_ENTRY_BYTES: u64 = 24;

fn micros(started: Instant) -> u64 {
    started.elapsed().as_micros().try_into().unwrap()
}

fn timings(values: &[u64]) -> serde_json::Value {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    if sorted.is_empty() {
        return serde_json::json!({"count": 0});
    }
    let percentile = |percent| sorted[(sorted.len() - 1) * percent / 100];
    serde_json::json!({"count": sorted.len(), "p50Us": percentile(50), "p95Us": percentile(95),
        "p99Us": percentile(99), "maxUs": sorted.last().unwrap(),
        "within1ms": sorted.iter().filter(|&&time| time <= 1_000).count(),
        "within50ms": sorted.iter().filter(|&&time| time <= 50_000).count(),
        "within2s": sorted.iter().filter(|&&time| time <= 2_000_000).count()})
}

fn equal_frames(actual: &DecodedFrame, expected: &DecodedFrame, selected: &[TableId]) {
    assert_eq!(
        (actual.branch, actual.hour),
        (expected.branch, expected.hour)
    );
    assert_eq!(actual.metrics, expected.metrics);
    for &table in selected {
        assert!(
            actual.tables[table.index()] == expected.tables[table.index()],
            "{table:?} differs at branch {} hour {}",
            actual.branch,
            actual.hour
        );
        assert_eq!(
            actual.tables[table.index()].sampled_at(),
            expected.tables[table.index()].sampled_at(),
            "{table:?} supplying read differs"
        );
    }
}

fn equal_current(actual: &Current, expected: &SeekResult, selected: &[TableId]) {
    match (actual, expected) {
        (Current::Frame(actual), SeekResult::Frame(expected)) => {
            equal_frames(actual, expected, selected)
        }
        (
            Current::Gap {
                branch: ab,
                hour: ah,
                reason: ar,
            },
            SeekResult::Gap {
                branch: eb,
                hour: eh,
                reason: er,
            },
        ) => assert_eq!((ab, ah, ar), (eb, eh, er)),
        _ => panic!("map frame/gap differs from recording"),
    }
}

/// One invocation publishes independent growing-prefix rows, then the burn-in.
#[test]
#[ignore = "lead-run release measurement; set BATTLE_PLAN_MAP_BENCH_REPLAY and BATTLE_PLAN_MAP_BENCH_EVIDENCE"]
fn checkpoint_disk_seek_measurement() {
    if cfg!(debug_assertions) {
        panic!("measure with --release");
    }
    let affinity =
        resources::affinity().expect("invalid processor affinity or unsupported platform");
    engine_executable::executable_tables::initialize_for_tests().expect("set BATTLE_PLAN_GAME_DIR");
    let recording = PathBuf::from(
        std::env::var_os("BATTLE_PLAN_MAP_BENCH_REPLAY").or_else(|| std::env::var_os("HOI4_MAP_BENCH_REPLAY")).expect("set BATTLE_PLAN_MAP_BENCH_REPLAY"),
    );
    let evidence = PathBuf::from(
        std::env::var_os("BATTLE_PLAN_MAP_BENCH_EVIDENCE").or_else(|| std::env::var_os("HOI4_MAP_BENCH_EVIDENCE")).expect("set BATTLE_PLAN_MAP_BENCH_EVIDENCE"),
    );
    std::fs::create_dir_all(&evidence).unwrap();
    let evidence = evidence.canonicalize().unwrap();
    // Test binaries may be built or launched from the shared measurement
    // checkout. Never require the compile-time worker worktree to exist.
    let mut worktree = std::env::current_dir().unwrap();
    while !worktree.join(".git").exists() && worktree.pop() {}
    if worktree.join(".git").exists() {
        assert!(
            !evidence.starts_with(worktree.canonicalize().unwrap()),
            "evidence must be outside Git"
        );
    }
    let local = PathBuf::from(std::env::var_os("LOCALAPPDATA").expect("set private LOCALAPPDATA"));
    std::fs::create_dir_all(&local).unwrap();
    assert!(
        local.canonicalize().unwrap().starts_with(&evidence),
        "LOCALAPPDATA must be inside evidence"
    );
    let hours = prefixes::parse(
        &std::env::var("BATTLE_PLAN_MAP_BENCH_PREFIX_HOURS").or_else(|_| std::env::var("HOI4_MAP_BENCH_PREFIX_HOURS")).expect("set BATTLE_PLAN_MAP_BENCH_PREFIX_HOURS"),
    );
    let burnin = PathBuf::from(
        std::env::var_os("BATTLE_PLAN_MAP_BENCH_BURNIN_REPLAY").or_else(|| std::env::var_os("HOI4_MAP_BENCH_BURNIN_REPLAY")).expect("set BATTLE_PLAN_MAP_BENCH_BURNIN_REPLAY"),
    );
    let mut jobs: Vec<_> = hours
        .into_iter()
        .map(|hours| (recording.clone(), Some(hours), format!("prefix-{hours}")))
        .collect();
    jobs.push((burnin, None, "burnin".into()));
    let mut completed = File::create(evidence.join("prefixes.jsonl")).unwrap();
    let mut summary = File::create(evidence.join("stride-summary.csv")).unwrap();
    writeln!(summary, "case,requested_prefix_hours,recorded_hours,stride,total_store_bytes,bytes_per_1000_hours,checkpoint_bytes,resume_bytes,seek_p50_us,seek_p95_us,step_p50_us,step_p95_us,retained_step_p95_us,block_load_p95_us,peak_private_bytes,parity_differences,disk_met,seek_met").unwrap();
    let mut rows = Vec::<serde_json::Value>::new();
    for (recording, hours, case) in jobs {
        let directory = evidence.join(&case);
        std::fs::create_dir_all(&directory).unwrap();
        measure_recording(&recording, &directory, &local, hours, |mut row| {
            row["case"] = serde_json::json!(case);
            row["processorAffinity"] = serde_json::json!(affinity);
            row["growthFromPreviousPrefix"] = prefixes::growth(&row, hours.and(rows.last()));
            let mut file = File::create(directory.join("report.json")).unwrap();
            serde_json::to_writer_pretty(&mut file, &row).unwrap();
            file.sync_all().unwrap();
            serde_json::to_writer(&mut completed, &row).unwrap();
            writeln!(completed).unwrap();
            completed.sync_all().unwrap();
            eprintln!("map prefix row committed: {case}");
            for candidate in row["candidates"].as_array().unwrap() {
                writeln!(
                    summary,
                    "{case},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
                    row["requestedPrefixHours"],
                    row["recordedFrames"],
                    candidate["stride"],
                    candidate["totalMapStoreBytes"],
                    candidate["mapBytesPer1000RecordedHours"],
                    candidate["checkpointBytes"],
                    candidate["resumeBytes"],
                    candidate["seekEmptyDecoder"]["p50Us"],
                    candidate["seekEmptyDecoder"]["p95Us"],
                    candidate["step"]["p50Us"],
                    candidate["step"]["p95Us"],
                    candidate["stepRetainedBlock"]["p95Us"],
                    candidate["stepBlockLoad"]["p95Us"],
                    row["peakPrivateBytes"],
                    candidate["parityDifferences"],
                    candidate["diskBudgetMet"],
                    candidate["seekP95BudgetMet"]
                )
                .unwrap();
            }
            summary.sync_all().unwrap();
            rows.push(row);
        });
    }
    let mut file = File::create(evidence.join("report.json")).unwrap();
    serde_json::to_writer_pretty(&mut file, &serde_json::json!({"rows": rows, "scope": "measured growing 50-year-recording prefixes plus independent burn-in continuity; completed rows are synced to prefixes.jsonl before Store cleanup"})).unwrap();
    file.sync_all().unwrap();
}

fn measure_recording(
    recording: &Path,
    evidence: &Path,
    local: &Path,
    hours: Option<u32>,
    publish: impl FnOnce(serde_json::Value),
) {
    let memory = resources::Peak::start();
    let source_before = SourceKey::capture(recording, &[]).unwrap();
    let replay = Arc::new(Replay::open_path(recording).unwrap());
    let blocks = prefixes::blocks(&replay.index, hours);
    let first_hour = replay.index[0].first_hour;
    let through_hour = replay.index[blocks - 1].last_hour;
    let ranges = prefixes::ranges(&replay.index[..blocks]);
    let gaps: Vec<_> = replay
        .gaps
        .iter()
        .filter_map(|gap| {
            let &(first, last) = ranges.get(&gap.branch)?;
            if gap.first_hour > last || gap.last_hour < first {
                return None;
            }
            let mut gap = gap.clone();
            gap.first_hour = gap.first_hour.max(first);
            gap.last_hour = gap.last_hour.min(last);
            Some(gap)
        })
        .collect();
    let selected = dashboard_base::prepared_map::selected_tables();
    let interval = dashboard_base::prepared_map::CHECKPOINT_STRIDE;
    let strides = BTreeSet::from([interval, 360, 720, 1440]);
    let fixtures: Vec<_> = strides
        .iter()
        .map(|_| Fixture::source(recording.to_path_buf()))
        .collect();
    let mut builders: Vec<_> = strides
        .iter()
        .map(|&stride| {
            let mut builder = Builder::new(
                NonZeroU32::new(stride).unwrap(),
                selected,
                Progress {
                    ranges: ranges.clone(),
                    ..Default::default()
                },
            )
            .unwrap();
            if hours.is_none() {
                builder.set_ranges(&replay);
            }
            builder
        })
        .collect();
    let mut writers: Vec<_> = fixtures
        .iter()
        .map(|fixture| fixture.store.replace(fixture.key.clone()).unwrap())
        .collect();
    // The integrated pipeline decodes product inputs together. Persist that
    // full resume state separately from selected interactive checkpoints.
    let mut stream = ReplayStream::new(&TableId::ALL).unwrap();
    let mut frames = 0u64;
    let mut last_position = None;
    let mut builder_us = vec![0u64; builders.len()];
    let mut checkpoint_accept_us = vec![Vec::new(); builders.len()];
    let mut update_accept_us = vec![Vec::new(); builders.len()];
    let mut accepts = BufWriter::new(File::create(evidence.join("map-accept.csv")).unwrap());
    writeln!(accepts, "stride,block,branch,hour,checkpoint,accept_us").unwrap();
    let mut attribution = attribution::Attribution::default();
    let started = Instant::now();
    loop {
        let block = stream.cursor().next_block;
        if block >= blocks {
            break;
        }
        let mut count = 0;
        if !stream
            .next_block_with(
                &replay,
                |source, decoded| {
                    count += 1;
                    frames += 1;
                    let position = Position {
                        block,
                        next_frame: count,
                    };
                    // Candidate clones are test-only: production moves to one builder.
                    let mut remaining = Some(source);
                    let last = builders.len() - 1;
                    for (ordinal, (builder, writer)) in
                        builders.iter_mut().zip(&mut writers).enumerate()
                    {
                        let update = if ordinal == last {
                            remaining.take().unwrap()
                        } else {
                            remaining.as_ref().unwrap().clone()
                        };
                        let checkpoint = checkpoint_due(builder, decoded.branch);
                        let started = Instant::now();
                        builder.accept(writer, &update, decoded, position)?;
                        let elapsed = micros(started);
                        builder_us[ordinal] += elapsed;
                        if checkpoint {
                            checkpoint_accept_us[ordinal].push(elapsed);
                        } else {
                            update_accept_us[ordinal].push(elapsed);
                        }
                        if checkpoint {
                            attribution.observe_checkpoint(
                                builder.progress().stride,
                                decoded,
                                selected,
                            );
                        }
                        writeln!(
                            accepts,
                            "{},{block},{},{},{checkpoint},{elapsed}",
                            builder.progress().stride,
                            decoded.branch,
                            decoded.hour
                        )?;
                    }
                    last_position = Some(position);
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap()
        {
            break;
        }
        assert!(
            stream.cursor().next_block > block,
            "source stream made no progress"
        );
        accepts.flush().unwrap();
        eprintln!("map candidates: accepted block {block}, frames={frames}");
    }
    assert_eq!(
        stream.cursor().next_block,
        blocks,
        "preparation must stop exactly at the rounded prefix boundary"
    );
    let walk_us = micros(started);
    let frame = stream.accumulator().unwrap();
    for (builder, writer) in builders.iter_mut().zip(&mut writers) {
        builder
            .save_resume(writer, frame, last_position.unwrap(), &TableId::ALL)
            .unwrap();
        builder.save_gaps(writer, &gaps).unwrap();
    }
    let progresses: Vec<_> = builders
        .iter()
        .map(|builder| builder.progress().clone())
        .collect();
    for writer in writers {
        writer.commit().unwrap();
    }
    drop(stream);
    attribution.source(&replay, blocks, selected);
    let attribution = attribution.report(evidence, selected);
    let mut checkpoints = BufWriter::new(File::create(evidence.join("checkpoints.csv")).unwrap());
    let mut seeks = BufWriter::new(File::create(evidence.join("seeks.csv")).unwrap());
    let mut steps = BufWriter::new(File::create(evidence.join("steps.csv")).unwrap());
    writeln!(checkpoints, "stride,branch,ordinal,hour,bytes,load_us").unwrap();
    writeln!(seeks, "stride,branch,hour,state,seek_us,applied_frames,checkpoint_headers,checkpoint_bytes,source_reads,source_block_bytes_upper_bound,source_decoded_frames,source_load_us,oracle_us").unwrap();
    writeln!(steps, "stride,branch,hour,step_us,applied_frames,checkpoint_headers,checkpoint_bytes,source_reads,source_block_bytes_upper_bound,source_decoded_frames,source_load_us").unwrap();
    let mut reports = Vec::new();
    let targets = prefixes::targets(&replay.index[..blocks]);
    let mut report = serde_json::Value::Null;
    for (candidate, (fixture, progress)) in fixtures.iter().zip(&progresses).enumerate() {
        let stride = progress.stride;
        let mut checkpoint_bytes = 0u64;
        let mut initial_checkpoint_bytes = 0u64;
        let mut load_times = Vec::new();
        let mut checkpoint_hours = BTreeSet::new();
        for (&branch, state) in &progress.branches {
            let mut probe = fixture.reader(progress.clone(), Arc::clone(&replay), selected);
            for ordinal in 0..state.checkpoints {
                let started = Instant::now();
                let file = fixture
                    .store
                    .read(&fixture.key, &section(branch), ordinal)
                    .unwrap()
                    .unwrap();
                let bytes = file.len();
                let (_, checkpoint_hour) = header(file).unwrap();
                checkpoint_hours.insert((branch, checkpoint_hour));
                let checkpoint = probe.seek(branch, checkpoint_hour).unwrap().unwrap();
                let load_us = micros(started);
                assert_eq!(probe.applied_frames, 0);
                drop(checkpoint);
                checkpoint_bytes += bytes;
                if ordinal == 0 {
                    initial_checkpoint_bytes += bytes;
                }
                load_times.push(load_us);
                writeln!(
                    checkpoints,
                    "{stride},{branch},{ordinal},{},{bytes},{load_us}",
                    checkpoint_hour
                )
                .unwrap();
            }
        }
        checkpoints.flush().unwrap();
        let mut cold_times = Vec::new();
        let mut repeated_times = Vec::new();
        let mut advanced_times = Vec::new();
        let mut max_applied = 0;
        let mut max_source_decoded = 0;
        for &(branch, hour) in &targets {
            let mut map = fixture.reader(progress.clone(), Arc::clone(&replay), selected);
            let started = Instant::now();
            let actual = map.seek(branch, hour).unwrap().unwrap();
            let cold_us = micros(started);
            max_applied = max_applied.max(map.applied_frames);
            assert!(map.applied_frames < u64::from(stride));
            let started = Instant::now();
            let expected = replay.seek_selected(branch, hour, Some(selected)).unwrap();
            let oracle_us = micros(started);
            equal_current(&actual, &expected, selected);
            cold_times.push(cold_us);
            max_source_decoded = max_source_decoded.max(map.source_decoded_frames);
            writeln!(
                seeks,
                "{stride},{branch},{hour},empty,{cold_us},{},{},{},{},{},{},{},{oracle_us}",
                map.applied_frames,
                map.checkpoint_reads,
                map.checkpoint_bytes,
                map.source_reads,
                map.source_block_bytes,
                map.source_decoded_frames,
                map.source_load_us
            )
            .unwrap();
            drop(actual);
            let started = Instant::now();
            let actual = map.seek(branch, hour).unwrap().unwrap();
            let repeat_us = micros(started);
            equal_current(&actual, &expected, selected);
            repeated_times.push(repeat_us);
            writeln!(
                seeks,
                "{stride},{branch},{hour},repeated,{repeat_us},{},{},{},{},{},{},{},{oracle_us}",
                map.applied_frames,
                map.checkpoint_reads,
                map.checkpoint_bytes,
                map.source_reads,
                map.source_block_bytes,
                map.source_decoded_frames,
                map.source_load_us
            )
            .unwrap();
            drop(actual);
            // Prime an earlier hour. This measures native state and selected
            // source block reuse; priming is outside the measurement.
            if let Some(start) = hour
                .checked_sub((stride / 2) as i32)
                .filter(|start| *start >= progress.ranges[&branch].0)
            {
                drop(map.seek(branch, start).unwrap());
                let started = Instant::now();
                let actual = map.advance(branch, hour, |_| {}).unwrap().unwrap();
                let retained_us = micros(started);
                equal_current(&actual, &expected, selected);
                advanced_times.push(retained_us);
                writeln!(seeks, "{stride},{branch},{hour},earlier_decoder,{retained_us},{},{},{},{},{},{},{},{oracle_us}", map.applied_frames, map.checkpoint_reads, map.checkpoint_bytes, map.source_reads, map.source_block_bytes, map.source_decoded_frames, map.source_load_us).unwrap();
            }
            seeks.flush().unwrap();
            eprintln!("map stride {stride}: measured seek at branch {branch}, hour {hour}");
        }
        let mut map = fixture.reader(progress.clone(), Arc::clone(&replay), selected);
        let mut oracle = FrameDecoder::for_tables(selected);
        let mut previous: Option<(BranchId, Hour)> = None;
        let mut step_times = Vec::new();
        let mut keyframe_step_times = Vec::new();
        let mut coarse_step_times = Vec::new();
        let mut block_load_times = Vec::new();
        let mut retained_step_times = Vec::new();
        let mut source_load_times = Vec::new();
        let mut delta_apply_times = Vec::new();
        for index in replay.index.iter().take(blocks) {
            for frame in replay.read_block_selected(index, selected).unwrap() {
                let started = Instant::now();
                let expected = oracle.advance(&frame).unwrap();
                let apply_us = micros(started);
                if !frame.keyframe {
                    delta_apply_times.push(apply_us);
                }
                if previous.is_some_and(|(branch, hour)| {
                    branch == frame.branch && hour.checked_add(1) == Some(frame.hour)
                }) {
                    let started = Instant::now();
                    let actual = map
                        .advance(frame.branch, frame.hour, |_| {})
                        .unwrap()
                        .unwrap();
                    let step_us = micros(started);
                    match &actual {
                        Current::Frame(actual) => equal_frames(actual, expected, selected),
                        _ => panic!("step lost a recorded frame"),
                    }
                    assert_eq!(map.applied_frames, 1);
                    assert_eq!(map.checkpoint_reads, 0);
                    assert_eq!(
                        map.checkpoint_bytes, 0,
                        "steps never restore a coarse checkpoint"
                    );
                    if frame.keyframe {
                        keyframe_step_times.push(step_us);
                    }
                    if checkpoint_hours.contains(&(frame.branch, frame.hour)) {
                        coarse_step_times.push(step_us);
                    }
                    step_times.push(step_us);
                    if map.source_reads == 0 {
                        retained_step_times.push(step_us);
                    } else {
                        block_load_times.push(step_us);
                        source_load_times.push(map.source_load_us);
                    }
                    assert!(
                        map.source_reads <= 1,
                        "one-hour steps load at most one block"
                    );
                    writeln!(
                        steps,
                        "{stride},{},{},{step_us},{},{},{},{},{},{},{}",
                        frame.branch,
                        frame.hour,
                        map.applied_frames,
                        map.checkpoint_reads,
                        map.checkpoint_bytes,
                        map.source_reads,
                        map.source_block_bytes,
                        map.source_decoded_frames,
                        map.source_load_us
                    )
                    .unwrap();
                } else {
                    let actual = map.seek(frame.branch, frame.hour).unwrap().unwrap();
                    match &actual {
                        Current::Frame(actual) => equal_frames(actual, expected, selected),
                        _ => panic!("seek lost a recorded frame"),
                    }
                }
                previous = Some((frame.branch, frame.hour));
            }
            steps.flush().unwrap();
            eprintln!(
                "map stride {stride}: measured steps through hour {}",
                index.last_hour
            );
        }
        let resume_bytes = fixture
            .store
            .read(&fixture.key, "map.resume", 0)
            .unwrap()
            .unwrap()
            .len();
        let gap_bytes: u64 = progress
            .gaps
            .iter()
            .map(|(&branch, page)| {
                fixture
                    .store
                    .read(&fixture.key, &format!("map.gaps.{branch}"), page.range)
                    .unwrap()
                    .unwrap()
                    .len()
            })
            .sum();
        let directory = local
            .join("HOI4Replay/prepared")
            .join(fixture.id.to_string());
        let total_bytes: u64 = std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().metadata().unwrap().len())
            .sum();
        let manifest_bytes = progress
            .branches
            .values()
            .map(|state| state.checkpoints)
            .sum::<u64>()
            * MANIFEST_ENTRY_BYTES;
        let scalable = checkpoint_bytes + gap_bytes + manifest_bytes;
        let fixed_base_bytes =
            initial_checkpoint_bytes + progress.branches.len() as u64 * MANIFEST_ENTRY_BYTES;
        let projected = (scalable - fixed_base_bytes) as f64 / frames as f64 * 438_000.0
            + (total_bytes - scalable + fixed_base_bytes) as f64;
        let direct = if stride == interval {
            recording::measure(
                fixture, progress, &replay, selected, &targets, evidence, blocks,
            )
        } else {
            serde_json::Value::Null
        };
        let checkpoint_only = total_bytes;
        let recording_keyframes_only = total_bytes - checkpoint_bytes - manifest_bytes;
        let mut candidate_report = serde_json::Map::new();
        candidate_report.insert("stride".into(), serde_json::json!((stride)));
        candidate_report.insert(
            "builderUs".into(),
            serde_json::json!((builder_us[candidate])),
        );
        candidate_report.insert(
            "checkpointAccept".into(),
            serde_json::json!((timings(&checkpoint_accept_us[candidate]))),
        );
        candidate_report.insert(
            "ordinaryAccept".into(),
            serde_json::json!((timings(&update_accept_us[candidate]))),
        );
        candidate_report.insert(
            "checkpointAcceptTotalUs".into(),
            serde_json::json!((checkpoint_accept_us[candidate].iter().sum::<u64>())),
        );
        candidate_report.insert(
            "ordinaryAcceptTotalUs".into(),
            serde_json::json!((update_accept_us[candidate].iter().sum::<u64>())),
        );
        candidate_report.insert(
            "mapAcceptUsPerSourceBlock".into(),
            serde_json::json!((builder_us[candidate] as f64 / blocks as f64)),
        );
        candidate_report.insert("acceptTimingScope".into(), serde_json::json!(("whole production map accept call, including native checkpoint encode, zstd and Store write; ordinary accepts write no data")));
        candidate_report.insert(
            "checkpointCount".into(),
            serde_json::json!(
                (progress
                    .branches
                    .values()
                    .map(|state| state.checkpoints)
                    .sum::<u64>())
            ),
        );
        candidate_report.insert(
            "checkpointBytes".into(),
            serde_json::json!((checkpoint_bytes)),
        );
        candidate_report.insert(
            "initialCheckpointBytes".into(),
            serde_json::json!((initial_checkpoint_bytes)),
        );
        candidate_report.insert("updateBytes".into(), serde_json::json!(0));
        candidate_report.insert("resumeBytes".into(), serde_json::json!((resume_bytes)));
        candidate_report.insert("gapBytes".into(), serde_json::json!((gap_bytes)));
        candidate_report.insert(
            "totalMapStoreBytes".into(),
            serde_json::json!((total_bytes)),
        );
        candidate_report.insert(
            "mapBytesPer1000RecordedHours".into(),
            serde_json::json!((total_bytes as f64 / frames as f64 * 1000.0)),
        );
        candidate_report.insert(
            "scalableBytesPerRecordedHour".into(),
            serde_json::json!(((scalable - fixed_base_bytes) as f64 / frames as f64)),
        );
        candidate_report.insert(
            "projectedMapBytes438000Hours".into(),
            serde_json::json!((projected)),
        );
        candidate_report.insert(
            "checkpointLoad".into(),
            serde_json::json!((timings(&load_times))),
        );
        candidate_report.insert(
            "seekEmptyDecoder".into(),
            serde_json::json!((timings(&cold_times))),
        );
        candidate_report.insert(
            "seekRepeated".into(),
            serde_json::json!((timings(&repeated_times))),
        );
        candidate_report.insert(
            "seekEarlierDecoder".into(),
            serde_json::json!((timings(&advanced_times))),
        );
        candidate_report.insert("step".into(), serde_json::json!((timings(&step_times))));
        candidate_report.insert(
            "stepSourceKeyframe".into(),
            serde_json::json!((timings(&keyframe_step_times))),
        );
        candidate_report.insert(
            "stepCoarseCheckpointHour".into(),
            serde_json::json!((timings(&coarse_step_times))),
        );
        candidate_report.insert(
            "nativeDeltaApply".into(),
            serde_json::json!((timings(&delta_apply_times))),
        );
        candidate_report.insert(
            "maxSeekAppliedFrames".into(),
            serde_json::json!((max_applied)),
        );
        candidate_report.insert("parityDifferences".into(), serde_json::json!((0)));
        candidate_report.insert("storageRepresentation".into(), serde_json::json!(("independent sparse native checkpoints, zstd9 per range; selected hourly updates read only from recording")));
        candidate_report.insert(
            "diskBudgetMet".into(),
            serde_json::json!((total_bytes as f64 / frames as f64 * 1000.0 <= 6_000_000.0)),
        );
        candidate_report.insert(
            "seekP95BudgetMet".into(),
            serde_json::json!(
                (timings(&cold_times)["p95Us"]
                    .as_u64()
                    .is_some_and(|us| us <= 2_000_000))
            ),
        );
        candidate_report.insert(
            "checkpointOnlyStoreBytes".into(),
            serde_json::json!((checkpoint_only)),
        );
        candidate_report.insert(
            "checkpointOnlyBytesPer1000RecordedHours".into(),
            serde_json::json!((checkpoint_only as f64 / frames as f64 * 1000.0)),
        );
        candidate_report.insert(
            "projectedCheckpointOnlyBytes438000Hours".into(),
            serde_json::json!(
                ((scalable - fixed_base_bytes) as f64 / frames as f64 * 438_000.0
                    + (total_bytes - scalable + fixed_base_bytes) as f64)
            ),
        );
        candidate_report.insert(
            "recordingKeyframesOnlyStoreBytes".into(),
            serde_json::json!((recording_keyframes_only)),
        );
        candidate_report.insert(
            "recordingKeyframesOnlyBytesPer1000RecordedHours".into(),
            serde_json::json!((recording_keyframes_only as f64 / frames as f64 * 1000.0)),
        );
        candidate_report.insert(
            "checkpointBytesPer1000RecordedHours".into(),
            serde_json::json!((checkpoint_bytes as f64 / frames as f64 * 1000.0)),
        );
        candidate_report.insert(
            "updateBytesPer1000RecordedHours".into(),
            serde_json::json!(0),
        );
        candidate_report.insert(
            "projectedRecordingKeyframesOnlyBytes438000Hours".into(),
            serde_json::json!((gap_bytes as f64 / frames as f64 * 438_000.0 + resume_bytes as f64)),
        );
        candidate_report.insert("stepRetainedBlock".into(), timings(&retained_step_times));
        candidate_report.insert("stepBlockLoad".into(), timings(&block_load_times));
        candidate_report.insert("selectedSourceLoad".into(), timings(&source_load_times));
        candidate_report.insert(
            "maxSeekSourceDecodedFrames".into(),
            serde_json::json!(max_source_decoded),
        );
        candidate_report.insert("recordingReadCandidate".into(), serde_json::json!((direct)));
        let mut tables = attribution["checkpointCandidates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["stride"] == stride)
            .unwrap()["tables"]
            .as_array()
            .unwrap()
            .clone();
        let independent_bytes: u64 = tables
            .iter()
            .map(|table| table["independentNativeRowZstd9Bytes"].as_u64().unwrap())
            .sum();
        for table in &mut tables {
            let weight = table["independentNativeRowZstd9Bytes"].as_u64().unwrap() as f64
                / independent_bytes as f64;
            table["estimatedStoreCheckpointBytes"] =
                serde_json::json!(weight * checkpoint_bytes as f64);
            table["estimatedStoreCheckpointBytesPer1000Hours"] =
                serde_json::json!(weight * checkpoint_bytes as f64 / frames as f64 * 1000.0);
        }
        candidate_report.insert("tableAttribution".into(), serde_json::json!({"tables": tables,
            "scope": "actual whole checkpoint bytes allocated proportionally to independently compressed native row payloads; per-table shares are estimates"}));
        reports.push(serde_json::Value::Object(candidate_report));
        let mut prefix_report = serde_json::Map::new();
        prefix_report.insert("recording".into(), serde_json::json!((recording)));
        prefix_report.insert(
            "recordingBytes".into(),
            serde_json::json!((source_before.recording_size())),
        );
        prefix_report.insert("requestedPrefixHours".into(), serde_json::json!((hours)));
        prefix_report.insert("firstHour".into(), serde_json::json!((first_hour)));
        prefix_report.insert("throughHour".into(), serde_json::json!((through_hour)));
        prefix_report.insert(
            "roundedElapsedHours".into(),
            serde_json::json!((i64::from(through_hour) - i64::from(first_hour) + 1)),
        );
        prefix_report.insert(
            "prefixRecordedBlockBytes".into(),
            serde_json::json!(
                (replay
                    .index
                    .iter()
                    .take(blocks)
                    .map(|block| block.length)
                    .sum::<u64>())
            ),
        );
        prefix_report.insert(
            "format".into(),
            serde_json::json!((replay_data::FORMAT_VERSION)),
        );
        prefix_report.insert("recordedFrames".into(), serde_json::json!((frames)));
        prefix_report.insert("blocks".into(), serde_json::json!((blocks)));
        prefix_report.insert(
            "totalRecordingBlocks".into(),
            serde_json::json!((replay.index.len())),
        );
        prefix_report.insert(
            "selectedTables".into(),
            serde_json::json!(
                (selected
                    .iter()
                    .map(|table| format!("{table:?}"))
                    .collect::<Vec<_>>())
            ),
        );
        prefix_report.insert("productionStride".into(), serde_json::json!((interval)));
        prefix_report.insert("preparedStepCheckpointLoads".into(), serde_json::json!((0)));
        prefix_report.insert(
            "sharedWalkAndAllCandidateWritesUs".into(),
            serde_json::json!((walk_us)),
        );
        prefix_report.insert("candidates".into(), serde_json::json!((reports)));
        prefix_report.insert("tableAttribution".into(), serde_json::json!((attribution)));
        prefix_report.insert("parityDifferences".into(), serde_json::json!((0)));
        prefix_report.insert("seekTargets".into(), serde_json::json!((targets)));
        prefix_report.insert("seekTargetPolicy".into(), serde_json::json!(("41 uniformly spaced clock-hour targets per represented branch, including endpoints and 90/92.5/95/97.5/100 percent of the prefix; gaps stay targets")));
        prefix_report.insert("timingScope".into(), serde_json::json!(("decoded Current only; OS cache warm; empty decoder/block, repeated seek with retained block and earlier decoder advance are distinct; excludes hour builders, GPU and presentation")));
        prefix_report.insert("diskScope".into(), serde_json::json!(("actual sparse native checkpoints, full pipeline resume, gap metadata and Store manifest/stamps; no stored updates or seeds; recording-keyframe alternative subtracts checkpoints and their manifest entries; other products require integrated acceptance; projection scales later checkpoints/gaps and their range entries, holds initial checkpoints, resume and metadata once; measured prefix growth is the evidence, projections are estimates")));
        report = serde_json::Value::Object(prefix_report);
        serde_json::to_writer_pretty(
            File::create(evidence.join("partial-report.json")).unwrap(),
            &report,
        )
        .unwrap();
        eprintln!("map candidate {stride} complete: {total_bytes} bytes");
    }
    assert_eq!(
        source_before,
        source_before.recapture().unwrap(),
        "recording changed during measurement"
    );
    report["memory"] = memory.finish();
    report["peakPrivateBytes"] = report["memory"]["peakPrivateBytes"].clone();
    publish(report);
    eprintln!(
        "map measurement report={}",
        evidence.join("report.json").display()
    );
}

fn checkpoint_due(builder: &Builder, branch: BranchId) -> bool {
    builder
        .progress()
        .branches
        .get(&branch)
        .is_none_or(|progress| {
            progress.checkpoints == 0 || progress.since_checkpoint >= builder.progress().stride - 1
        })
}
