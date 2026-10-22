//! Recordings packed on several threads: the sequential writer's bytes and
//! errors at scale, and what packing on one thread costs against many.

use super::*;
use crate::rows::{CountryRow, DivisionRow, ProvinceRow};
use std::time::{Duration, Instant};

/// One hour of a campaign of `countries` countries, a province and two
/// divisions each: steady and irregular numeric changes, edited text,
/// divisions destroyed and raised again, provinces changing owner, and
/// Provinces unavailable some hours.
fn scaled(hour: i32, countries: u32) -> DecodedFrame {
    let mut frame = source(1, hour);
    let at = i64::from(hour);
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (0..countries)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                command_power_raw: if id % 7 == 0 {
                    (at * at + i64::from(id)) % 97
                } else {
                    i64::from(id) + 3 * at
                },
                tag: format!("T{id:04}-{}", (hour as u32 + id) / 11).into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::Provinces.index()] = if hour % 9 == 4 {
        TableView::Unavailable {
            reason: "synthetic guarded read failure".into(),
        }
    } else {
        TableView::Complete(TableRows::Provinces(
            (0..countries)
                .map(|id| ProvinceRow {
                    id,
                    state: id / 4,
                    owner: if (id + hour as u32).is_multiple_of(53) {
                        (id + 1) % countries
                    } else {
                        id
                    },
                    controller: id,
                    victory_points: (id % 5) as i32,
                    ..Default::default()
                })
                .collect(),
        ))
    };
    // A different one in thirteen divisions is missing each hour.
    *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(
        (0..u64::from(countries) * 2)
            .filter(|id| !(id + hour as u64).is_multiple_of(13))
            .map(|id| DivisionRow {
                id: id << 32 | 1,
                roster_owner: (id / 2) as u32,
                province: (id / 2) as u32,
                // A third of a step more each hour: 1, 1, 2, 1, ... steps.
                organisation: 10_000 * (1_000 + at + at / 3 + id as i64 % 5),
                hit_points: 10_000 * (90_000 - (id as i64 * at) % 1_000),
                ..Default::default()
            })
            .collect(),
    ));
    frame
}

/// One session of `blocks` spool keyframes, each followed by `hours - 1`
/// hours of [`scaled`].
fn scaled_records(countries: u32, blocks: usize, hours: usize) -> Vec<Record> {
    let mut encoder = FrameEncoder::new();
    let mut records = vec![
        Record::SessionStart(session(1)),
        Record::BranchStart(branch(1, 1)),
    ];
    for at in 0..blocks * hours {
        let frame = scaled(at as i32 + 1, countries);
        records.push(Record::Frame(
            encoder.encode(frame, at % hours == 0).unwrap(),
        ));
    }
    records
}

/// `records` written, their blocks packed on `workers` threads.
fn packed_on(
    workers: usize,
    records: &[Record],
    observe: &mut HourObserver<'_>,
    limit: usize,
) -> io::Result<Replay> {
    pack::WORKERS.set(Some(workers));
    let replay = Replay::from_record_slices(
        CAMPAIGN.into(),
        &[records],
        Observer::Hours(observe),
        Frames::Kept,
        limit,
    );
    pack::WORKERS.set(None);
    replay
}

const WORKERS: [usize; 3] = [2, 3, 8];

#[test]
fn countries_pack_to_the_sequential_bytes_on_any_worker_count() {
    let records = scaled_records(50, 3, 5);
    // A bound that splits every five-hour block by size too: its divisions
    // take about one and a half snapshots, a split keyframe one.
    let mut largest = 0;
    for record in &records {
        if let Record::Frame(frame) = record {
            largest = largest.max(*frame.table_encoded_sizes().unwrap().iter().max().unwrap());
        }
    }
    for limit in [MAX_BLOCK_RAW_BYTES, largest + largest / 2] {
        let sequential = packed_on(1, &records, &mut |_| Ok(()), limit).unwrap();
        if limit == MAX_BLOCK_RAW_BYTES {
            assert_eq!(sequential.index.len(), 3);
        } else {
            assert!(sequential.index.len() > 3, "{}", sequential.index.len());
        }
        for workers in WORKERS {
            let packed = packed_on(workers, &records, &mut |_| Ok(()), limit).unwrap();
            assert!(
                packed.bytes() == sequential.bytes(),
                "{workers} workers, limit {limit}"
            );
        }
        let mut decoded = Vec::new();
        for block in &sequential.index {
            let mut decoder = FrameDecoder::new();
            for frame in sequential.read_block(block).unwrap() {
                decoded.push(decoder.advance(&frame).unwrap().clone());
            }
        }
        assert_eq!(decoded.len(), 15);
        let mut last = scaled(15, 50);
        last.metrics = last.metrics.stored();
        assert_eq!(decoded.last(), Some(&last));
    }
}

#[test]
fn a_writer_error_after_packed_blocks_is_the_sequential_one() {
    let records = scaled_records(50, 4, 3);
    // The observer refuses an hour in the third block.
    let observed = |workers| {
        packed_on(
            workers,
            &records,
            &mut |hour| match hour.hour {
                8 => Err(io::Error::other("observer refused hour 8")),
                _ => Ok(()),
            },
            MAX_BLOCK_RAW_BYTES,
        )
        .err()
        .unwrap()
        .to_string()
    };
    // A frame in the last block repeats an earlier hour.
    let mut repeated = records.clone();
    let Some(Record::Frame(frame)) = repeated.last_mut() else {
        unreachable!()
    };
    frame.hour = 10;
    let structural = |workers| {
        packed_on(workers, &repeated, &mut |_| Ok(()), MAX_BLOCK_RAW_BYTES)
            .err()
            .unwrap()
            .to_string()
    };
    assert_eq!(observed(1), "observer refused hour 8");
    // The writer's segment check refuses the repeated hour as the record
    // arrives, after the earlier blocks went to the packer.
    assert!(
        structural(1).contains("frame hours do not increase within branch"),
        "{}",
        structural(1)
    );
    for workers in WORKERS {
        assert_eq!(observed(workers), observed(1));
        assert_eq!(structural(workers), structural(1));
    }
}

/// This process's CPU time, all threads together.
#[cfg(windows)]
fn process_cpu() -> Duration {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetCurrentProcess, GetProcessTimes},
    };
    // SAFETY: FILETIME is plain data.
    let mut times: [FILETIME; 4] = unsafe { std::mem::zeroed() };
    let [creation, exit, kernel, user] = &mut times;
    // SAFETY: the current process's pseudo-handle and four owned FILETIMEs.
    let read = unsafe { GetProcessTimes(GetCurrentProcess(), creation, exit, kernel, user) };
    assert_ne!(read, 0, "GetProcessTimes failed");
    let ticks =
        |time: &FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    Duration::from_nanos((ticks(kernel) + ticks(user)) * 100)
}

#[cfg(not(windows))]
fn process_cpu() -> Duration {
    Duration::ZERO
}

/// Packs a recording of `PACK_BENCH_BLOCKS` blocks (two per default worker)
/// of `PACK_BENCH_HOURS` hours (24) with `PACK_BENCH_COUNTRIES` countries
/// (5,000) on one thread and on the default workers, and reports each run's
/// wall time and cores used (process CPU time over wall time):
/// `cargo test -p replay --release --lib pack_bench -- --ignored --nocapture`.
#[test]
#[ignore = "release bench"]
fn pack_bench_one_against_default_workers() {
    let setting = |name: &str, default: usize| {
        std::env::var(name)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(default)
    };
    let workers = pack::workers();
    let countries = setting("PACK_BENCH_COUNTRIES", 5_000);
    let blocks = setting("PACK_BENCH_BLOCKS", 2 * workers);
    let hours = setting("PACK_BENCH_HOURS", 24);
    let records = scaled_records(countries as u32, blocks, hours);
    let mut written = Vec::new();
    for workers in [1, workers] {
        let (started, cpu) = (Instant::now(), process_cpu());
        let replay = packed_on(workers, &records, &mut |_| Ok(()), MAX_BLOCK_RAW_BYTES).unwrap();
        let (wall, cpu) = (started.elapsed(), process_cpu() - cpu);
        println!(
            "pack_bench workers={workers} countries={countries} blocks={} hours={} wall_s={:.3} cores_used={:.2} ms_per_hour={:.3} bytes={}",
            replay.index.len(),
            blocks * hours,
            wall.as_secs_f64(),
            cpu.as_secs_f64() / wall.as_secs_f64(),
            wall.as_secs_f64() * 1e3 / (blocks * hours) as f64,
            replay.bytes().len(),
        );
        written.push((wall, replay.bytes().to_vec()));
    }
    println!(
        "pack_bench speedup={:.2}",
        written[0].0.as_secs_f64() / written[1].0.as_secs_f64()
    );
    assert!(written[0].1 == written[1].1);
}

#[test]
fn recovered_frames_read_on_several_threads_write_the_sequential_bytes() {
    let records = scaled_records(300, 5, 4);
    let segments = [(
        0,
        Recovery {
            format: Some(FormatVersion::CURRENT),
            records: records.clone(),
            valid_bytes: 0,
            damage: None,
        },
    )];
    let kept = kept(&segments);
    let written = |workers| {
        pack::WORKERS.set(Some(workers));
        let mut hours = Vec::new();
        let mut progress = 0;
        let replay = Replay::from_recovered_with_reader(
            CAMPAIGN.into(),
            &kept,
            |number, frame| restore(&segments, number, frame),
            || {
                progress += 1;
                Ok(())
            },
            |hour| {
                hours.push((hour.hour, hour.sizes, hour.updates));
                Ok(())
            },
        );
        pack::WORKERS.set(None);
        (replay.unwrap().bytes().to_vec(), hours, progress)
    };
    let sequential = written(1);
    assert_eq!(sequential.1.len(), 20);
    assert_eq!(sequential.2, 20);
    assert!(
        sequential.0
            == Replay::from_records(CAMPAIGN.into(), &[records])
                .unwrap()
                .bytes()
    );
    for workers in WORKERS {
        let parallel = written(workers);
        assert!(parallel.0 == sequential.0, "{workers} workers");
        assert_eq!(parallel.1, sequential.1, "{workers} workers");
        assert_eq!(parallel.2, sequential.2, "{workers} workers");
    }
}

#[test]
fn a_fallback_after_partial_progress_reports_each_frame_once() {
    use crate::rows::LexicalTokenRow;
    let mut records = scaled_records(50, 4, 5);
    // The last block's second and third hours re-send a large table, so only it splits by size.
    for (back, byte) in [(4, 1_u8), (3, 2)] {
        let at = records.len() - back;
        let Record::Frame(frame) = &mut records[at] else {
            unreachable!()
        };
        frame.tables[TableId::LexicalTokens.index()] =
            TableUpdate::Snapshot(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 0,
                literal: vec![byte; 200 * 1024],
            }]));
    }
    let segments = [(
        0,
        Recovery {
            format: Some(FormatVersion::CURRENT),
            records: records.clone(),
            valid_bytes: 0,
            damage: None,
        },
    )];
    let kept = kept(&segments);
    let kept_records = [kept[0].1.records.as_slice()];
    let frames = records
        .iter()
        .filter(|record| matches!(record, Record::Frame(_)))
        .count();
    // The splitting hour is read only once progress was reported, so the fallback follows some.
    let splitting = records.len() - 3;
    let reported = (std::sync::Mutex::new(false), std::sync::Condvar::new());
    let read = |_: usize, frame: &crate::spool::FrameMetadata| {
        if frame.placement.offset == splitting {
            let mut any = reported.0.lock().unwrap();
            while !*any {
                any = reported.1.wait(any).unwrap();
            }
        }
        restore(&segments, 0, frame)
    };
    let written = |workers| {
        pack::WORKERS.set(Some(workers));
        *reported.0.lock().unwrap() = false;
        let mut progress = 0;
        let mut counting = || {
            progress += 1;
            *reported.0.lock().unwrap() = true;
            reported.1.notify_all();
            if progress > frames {
                return Err(io::Error::other("progress past the last frame"));
            }
            Ok(())
        };
        let mut hours = Vec::new();
        let replay = Replay::from_record_slices(
            CAMPAIGN.into(),
            &kept_records,
            Observer::Hours(&mut |hour| {
                hours.push(hour.hour);
                Ok(())
            }),
            Frames::Shared(&read, &mut counting),
            300 * 1024,
        );
        pack::WORKERS.set(None);
        (replay.unwrap().bytes().to_vec(), hours, progress)
    };
    let sequential = written(1);
    assert_eq!(sequential.2, frames);
    for workers in WORKERS {
        let parallel = written(workers);
        assert!(parallel.0 == sequential.0, "{workers} workers");
        assert_eq!(parallel.1, sequential.1, "{workers} workers");
        assert_eq!(parallel.2, frames, "{workers} workers");
    }
}

#[test]
fn parallel_blocks_held_from_admission_to_packing_stay_within_the_sequential_writers() {
    use crate::container::units::{HELD, Step, WATCH};
    use std::sync::{Arc, Condvar, Mutex};
    let blocks = 8;
    let records = scaled_records(50, blocks, 4);
    let expected = packed_on(1, &records, &mut |_| Ok(()), MAX_BLOCK_RAW_BYTES).unwrap();
    #[derive(Default)]
    struct Held {
        /// The writer's count at each admission.
        admitted: Vec<usize>,
        /// Admissions less packings, which each of the writer's counts must match.
        counted: usize,
        matched: bool,
        drained: bool,
        finished: bool,
        over: bool,
    }
    // Two workers at least, so a begun block always finds a worker this test does not hold.
    for workers in [3, 8] {
        let held = Arc::new((
            Mutex::new(Held {
                matched: true,
                ..Held::default()
            }),
            Condvar::new(),
        ));
        let watched = Arc::clone(&held);
        WATCH.set(Some(Arc::new(move |step: Step| {
            let (state, changed) = &*watched;
            let mut state = state.lock().unwrap();
            match step {
                Step::Admitted(writer) => {
                    state.counted += 1;
                    state.matched &= writer == state.counted;
                    state.admitted.push(writer);
                    state.drained = false;
                    // Past the bound nothing is held back, so the write ends and the test fails.
                    state.over |= writer.max(state.counted) > HELD;
                }
                Step::Waiting { drained } => state.drained = drained,
                Step::Finished => state.finished = true,
                // Packed only once the keyframe thread cannot go on without it, or admits no more.
                Step::Packing => {
                    while !(state.drained || state.finished || state.over) {
                        state = changed.wait(state).unwrap();
                    }
                }
                Step::Packed(writer) => {
                    state.counted -= 1;
                    state.matched &= writer == state.counted;
                }
            }
            changed.notify_all();
        })));
        pack::WORKERS.set(Some(workers));
        let replay = Replay::from_records(CAMPAIGN.into(), &[records.clone()]);
        pack::WORKERS.set(None);
        WATCH.set(None);
        assert!(
            replay.unwrap().bytes() == expected.bytes(),
            "{workers} workers"
        );
        let state = held.0.lock().unwrap();
        assert!(state.matched, "{workers} workers");
        assert_eq!(state.admitted.len(), blocks, "{workers} workers");
        assert_eq!(
            state.admitted.iter().max(),
            Some(&HELD),
            "{workers} workers"
        );
        assert_eq!(
            (state.counted, state.finished),
            (0, true),
            "{workers} workers"
        );
    }
}
