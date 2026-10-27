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
        None,
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
            None,
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

/// Hours a write reports: hour, plain sizes and how each table was recorded.
type Hours = Vec<(i32, [usize; TABLE_COUNT], [UpdateKind; TABLE_COUNT])>;

/// `records` finalized, with blocks first written ahead from each prefix in `steps`, stopping
/// at every `stop_every`th check; the bytes, the hours reported ahead, then by finalization, and
/// the frames finalization wrote and reported progress for.
fn finalized_ahead(
    records: &[Record],
    existing: Option<&Replay>,
    steps: &[usize],
    stop_every: usize,
    workers: usize,
) -> (Vec<u8>, Hours, usize, usize) {
    let segments = [(
        0,
        Recovery {
            format: Some(FormatVersion::CURRENT),
            records: records.to_vec(),
            valid_bytes: 0,
            damage: None,
        },
    )];
    let kept = kept(&segments);
    let read = |number, frame: &crate::spool::FrameMetadata| restore(&segments, number, frame);
    let mut hours = Vec::new();
    let mut ahead =
        Ahead::new(existing.map(|replay| Replay::open(replay.bytes().to_vec()).unwrap())).unwrap();
    let checks = std::sync::atomic::AtomicUsize::new(0);
    let stop =
        || checks.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % stop_every == stop_every - 1;
    for &step in steps {
        // A stopped advance resumes where it stopped.
        for _ in 0..4 {
            assert!(
                ahead
                    .advance(
                        CAMPAIGN,
                        &[(0, &kept[0].1.records[..step])],
                        read,
                        |hour| {
                            hours.push((hour.hour, hour.sizes, hour.updates));
                            Ok(())
                        },
                        &stop,
                    )
                    .unwrap()
            );
        }
    }
    assert!(ahead.continues(&[(0, kept[0].1.records.as_slice())]));
    // Stop takes over the same encoder even when only one CPU credit is available.
    let frames = records
        .iter()
        .filter(|record| matches!(record, Record::Frame(_)))
        .count();
    let written = frames - ahead.written();
    pack::WORKERS.set(Some(workers));
    let mut observe = |hour: &Observed| {
        hours.push((hour.hour, hour.sizes, hour.updates));
        Ok(())
    };
    let mut progress = 0;
    let counting = || {
        progress += 1;
        Ok(())
    };
    let replay = match existing {
        Some(existing) => {
            existing.append_recovered_with_reader(&kept, read, counting, &mut observe, Some(ahead))
        }
        None => Replay::from_recovered_with_reader(
            CAMPAIGN.into(),
            &kept,
            read,
            counting,
            &mut observe,
            Some(ahead),
        ),
    };
    pack::WORKERS.set(None);
    (replay.unwrap().bytes().to_vec(), hours, written, progress)
}

#[test]
fn blocks_written_ahead_finalize_to_the_same_bytes_and_hours() {
    let records = scaled_records(200, 4, 6);
    let whole = Replay::from_records(CAMPAIGN.into(), std::slice::from_ref(&records))
        .unwrap()
        .bytes()
        .to_vec();
    let segments = [(
        0,
        Recovery {
            format: Some(FormatVersion::CURRENT),
            records: records.clone(),
            valid_bytes: 0,
            damage: None,
        },
    )];
    let mut hours = Vec::new();
    Replay::from_recovered_with(CAMPAIGN.into(), &segments, |hour| {
        hours.push((hour.hour, hour.sizes, hour.updates));
        Ok(())
    })
    .unwrap();
    let frames = |count| {
        records
            .iter()
            .scan(0, |frames, record| {
                *frames += usize::from(matches!(record, Record::Frame(_)));
                Some(*frames)
            })
            .position(|frames| frames == count)
            .map_or(records.len(), |at| at + 1)
    };
    // Mid-block, at block ends, past the last keyframe and the whole spool.
    let steps = [
        frames(3),
        frames(6),
        frames(7),
        frames(13),
        frames(19),
        records.len(),
    ];
    for workers in [1, 3] {
        for stop_every in [1_000_000, 7, 2] {
            for end in 1..=steps.len() {
                let ahead = finalized_ahead(&records, None, &steps[..end], stop_every, workers);
                assert!(ahead.0 == whole, "{workers} {stop_every} {end}");
                assert_eq!(ahead.1, hours, "{workers} {stop_every} {end}");
                assert_eq!(ahead.3, ahead.2, "{workers} {stop_every} {end}");
            }
        }
    }
}

#[test]
fn blocks_written_ahead_append_to_the_same_bytes() {
    let records = scaled_records(100, 3, 4);
    let existing = Replay::from_records(CAMPAIGN.into(), &[records[..9].to_vec()]).unwrap();
    let mut later = vec![
        Record::SessionStart(session(2)),
        Record::BranchStart(branch(2, 2)),
    ];
    let mut encoder = FrameEncoder::new();
    for at in 0..9 {
        let mut frame = scaled(at + 20, 100);
        frame.branch = 2;
        later.push(Record::Frame(encoder.encode(frame, at % 4 == 0).unwrap()));
    }
    let whole = existing.append(std::slice::from_ref(&later)).unwrap();
    for workers in [1, 3] {
        let (bytes, _, written, progress) =
            finalized_ahead(&later, Some(&existing), &[6, 9], 5, workers);
        assert!(bytes == whole.bytes(), "{workers}");
        assert_eq!(progress, written, "{workers}");
    }
}

#[test]
fn records_that_change_under_blocks_written_ahead_are_not_continued() {
    let records = scaled_records(50, 2, 3);
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
    let mut ahead = Ahead::new(None).unwrap();
    let read = |number, frame: &crate::spool::FrameMetadata| restore(&segments, number, frame);
    assert!(
        ahead
            .advance(
                CAMPAIGN,
                &[(0, &kept[0].1.records[..5])],
                read,
                |_| Ok(()),
                &|| false
            )
            .unwrap()
    );
    let mut changed = kept[0].1.records.clone();
    changed.remove(3);
    assert!(!ahead.continues(&[(0, changed.as_slice())]));
    assert!(!ahead.continues(&[(1, kept[0].1.records.as_slice())]));
    assert!(ahead.continues(&[(0, kept[0].1.records.as_slice())]));
    assert!(
        !ahead
            .advance(CAMPAIGN, &[(0, &changed)], read, |_| Ok(()), &|| false)
            .unwrap()
    );
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
            None,
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

/// An hour of [`scaled`] on `branch` whose first `missing` of six division supply footers are not
/// recorded, the one gap a current recording keeps.
fn with_supply_gaps(branch: u64, hour: i32, missing: u64) -> DecodedFrame {
    let mut frame = scaled(hour, 20);
    frame.branch = branch;
    *frame.tables[TableId::DivisionStatus.index()] =
        TableView::Complete(TableRows::DivisionStatus(
            (0..6_u64)
                .map(|id| crate::rows::DivisionStatusRow {
                    division: id << 32 | 1,
                    supply: if id < missing {
                        crate::rows::Recorded::NotRecorded
                    } else {
                        Default::default()
                    },
                    ..Default::default()
                })
                .collect(),
        ));
    frame
}

/// Two sessions: a branch whose gaps change, stay unchanged after a change, after a keyframe and
/// across a gap, a reload onto an earlier hour, then a restart.
fn supply_gap_segments() -> [Vec<Record>; 2] {
    let mut encoder = FrameEncoder::new();
    let mut frame = |branch, hour, missing, keyframe| {
        let decoded = with_supply_gaps(branch, hour, missing);
        Record::Frame(encoder.encode(decoded, keyframe).unwrap())
    };
    let mut first = vec![
        Record::SessionStart(session(1)),
        Record::BranchStart(branch(1, 1)),
    ];
    for (hour, missing) in [(1, 2), (2, 2), (3, 3), (4, 3), (5, 3), (6, 0)] {
        first.push(frame(1, hour, missing, hour % 4 == 1));
    }
    first.push(Record::Gap(Gap {
        branch: 1,
        first_hour: 7,
        last_hour: 7,
        reason: "queue full".into(),
    }));
    for (hour, missing) in [(8, 0), (9, 1), (10, 1), (11, 4)] {
        first.push(frame(1, hour, missing, hour % 4 == 1));
    }
    first.push(Record::BranchStart(BranchStart {
        id: 2,
        session: 1,
        loaded_hour: Some(3),
        first_hour: 4,
        parent: Parentage::Unknown,
        reason: BranchReason::LoadSignal,
    }));
    for (hour, missing) in [(4, 5), (5, 5), (6, 1), (7, 1)] {
        first.push(frame(2, hour, missing, hour == 4));
    }
    let mut second = vec![
        Record::SessionStart(session(2)),
        Record::BranchStart(BranchStart {
            id: 3,
            session: 2,
            loaded_hour: Some(19),
            first_hour: 20,
            parent: Parentage::Unknown,
            reason: BranchReason::SessionStart,
        }),
    ];
    for (hour, missing) in [(20, 6), (21, 6), (22, 2), (23, 2), (24, 2), (25, 2)] {
        second.push(frame(3, hour, missing, hour % 4 == 0));
    }
    [first, second]
}

/// What a hour's observation says beyond its sizes, which writers measure.
type Seen = (
    BranchId,
    Hour,
    [UpdateKind; TABLE_COUNT],
    Vec<(TableId, Incomplete)>,
    Vec<(TableId, &'static str, usize)>,
);

fn seen(hour: &Observed) -> Seen {
    (
        hour.branch,
        hour.hour,
        hour.updates,
        hour.incomplete.clone(),
        hour.field_gaps.clone(),
    )
}

/// Every frame of `segments` observed with each table's gaps counted afresh.
fn scanned(segments: &[&[Record]]) -> Vec<Seen> {
    let mut decoder = FrameDecoder::new();
    segments
        .iter()
        .flat_map(|segment| segment.iter())
        .filter_map(|record| match record {
            Record::Frame(frame) => Some(frame),
            _ => None,
        })
        .map(|frame| {
            let decoded = decoder.advance(frame).unwrap();
            seen(&Observed::new(frame, decoded, [0; TABLE_COUNT], None).unwrap())
        })
        .collect()
}

#[test]
fn an_unchanged_table_keeps_the_gaps_a_full_scan_counts() {
    let hour = |branch, hour, missing| {
        let decoded = with_supply_gaps(branch, hour, missing);
        let frame = FrameEncoder::new().encode(decoded.clone(), true).unwrap();
        (frame, decoded)
    };
    let (mut frame, decoded) = hour(1, 5, 2);
    let scan = Observed::new(&frame, &decoded, [0; TABLE_COUNT], None).unwrap();
    let supply = (TableId::DivisionStatus, "supply", 2);
    assert!(scan.field_gaps.contains(&supply));
    frame.tables[TableId::DivisionStatus.index()] = TableUpdate::Unchanged { baseline: 4 };
    // Only the baseline hour on the same branch lends its gaps.
    let mut previous = scan.clone();
    previous.field_gaps = vec![(TableId::DivisionStatus, "supply", 9)];
    for (branch, at) in [(1, 3), (2, 4)] {
        (previous.branch, previous.hour) = (branch, at);
        let observed = Observed::new(&frame, &decoded, [0; TABLE_COUNT], Some(&previous)).unwrap();
        assert_eq!(observed.field_gaps, scan.field_gaps, "{branch} {at}");
    }
    (previous.branch, previous.hour) = (1, 4);
    let observed = Observed::new(&frame, &decoded, [0; TABLE_COUNT], Some(&previous)).unwrap();
    assert_eq!(
        observed.field_gaps,
        [(TableId::DivisionStatus, "supply", 9)]
    );
}

#[test]
fn hours_observed_by_every_writer_match_a_full_scan() {
    let [first, second] = supply_gap_segments();
    let expected = scanned(&[&first, &second]);
    // The fixture exercises the copy: gaps on hours whose division table is unchanged.
    let at = TableId::DivisionStatus.index();
    assert!(
        expected
            .iter()
            .any(|hour| hour.2[at] == UpdateKind::Unchanged
                && hour.4.iter().any(|gap| gap.0 == TableId::DivisionStatus))
    );
    assert!(expected.iter().any(|hour| hour.2[at] == UpdateKind::Delta));
    let whole = Replay::from_records(CAMPAIGN.into(), &[first.clone(), second.clone()]).unwrap();
    for workers in [1, 2, 3, 8] {
        let mut hours = Vec::new();
        pack::WORKERS.set(Some(workers));
        let replay = Replay::from_record_slices(
            CAMPAIGN.into(),
            &[&first, &second],
            Observer::Hours(&mut |hour| {
                hours.push(seen(hour));
                Ok(())
            }),
            Frames::Kept,
            MAX_BLOCK_RAW_BYTES,
            None,
        );
        pack::WORKERS.set(None);
        assert!(
            replay.unwrap().bytes() == whole.bytes(),
            "{workers} workers"
        );
        assert_eq!(hours, expected, "{workers} workers");
    }
    // A block split by size stops the parallel writer, and the sequential one repeats the run,
    // observing only the hours not yet reported; a split's keyframe records every table afresh.
    let mut split = first.clone();
    for (hour, byte) in [(9, 1_u8), (10, 2)] {
        let frame = split
            .iter_mut()
            .find_map(|record| match record {
                Record::Frame(frame) if frame.hour == hour => Some(frame),
                _ => None,
            })
            .unwrap();
        frame.tables[TableId::LexicalTokens.index()] =
            TableUpdate::Snapshot(TableRows::LexicalTokens(vec![
                crate::rows::LexicalTokenRow {
                    token: 0,
                    literal: vec![byte; 200 * 1024],
                },
            ]));
    }
    let gaps = |hours: Vec<Seen>| -> Vec<_> {
        hours
            .into_iter()
            .map(|(branch, hour, _, incomplete, gaps)| (branch, hour, incomplete, gaps))
            .collect()
    };
    let expected = gaps(scanned(&[&split, &second]));
    for workers in [1, 3, 8] {
        let mut hours = Vec::new();
        pack::WORKERS.set(Some(workers));
        let replay = Replay::from_record_slices(
            CAMPAIGN.into(),
            &[&split, &second],
            Observer::Hours(&mut |hour| {
                hours.push(seen(hour));
                Ok(())
            }),
            Frames::Kept,
            300 * 1024,
            None,
        )
        .unwrap();
        pack::WORKERS.set(None);
        assert_eq!(
            replay.index.len(),
            whole.index.len() + 1,
            "{workers} workers"
        );
        assert_eq!(gaps(hours), expected, "split, {workers} workers");
    }
    // Appending the restart observes its hours as a full scan of them does.
    let existing = Replay::from_records(CAMPAIGN.into(), std::slice::from_ref(&first)).unwrap();
    let whole = existing.append(std::slice::from_ref(&second)).unwrap();
    let segments = [(
        0,
        Recovery {
            format: Some(FormatVersion::CURRENT),
            records: second.clone(),
            valid_bytes: 0,
            damage: None,
        },
    )];
    for workers in [1, 3] {
        pack::WORKERS.set(Some(workers));
        let mut hours = Vec::new();
        let appended = existing
            .append_recovered_with_reader(
                &kept(&segments),
                |number, frame| restore(&segments, number, frame),
                || Ok(()),
                |hour| {
                    hours.push(seen(hour));
                    Ok(())
                },
                None,
            )
            .unwrap();
        pack::WORKERS.set(None);
        assert!(appended.bytes() == whole.bytes(), "{workers} workers");
        assert_eq!(hours, scanned(&[&second]), "{workers} workers");
    }
}

#[test]
fn hours_observed_ahead_and_after_match_a_full_scan() {
    let [first, _] = supply_gap_segments();
    let expected = scanned(&[&first]);
    let segments = [(
        0,
        Recovery {
            format: Some(FormatVersion::CURRENT),
            records: first.clone(),
            valid_bytes: 0,
            damage: None,
        },
    )];
    let kept = kept(&segments);
    let read = |number, frame: &crate::spool::FrameMetadata| restore(&segments, number, frame);
    // Mid-block, at a block end and past the reload.
    for steps in [&[5][..], &[9, 12], &[14, first.len()]] {
        for workers in [1, 3] {
            let mut hours = Vec::new();
            let mut ahead = Ahead::new(None).unwrap();
            for &step in steps {
                assert!(
                    ahead
                        .advance(
                            CAMPAIGN,
                            &[(0, &kept[0].1.records[..step])],
                            read,
                            |hour| {
                                hours.push(seen(hour));
                                Ok(())
                            },
                            &|| false,
                        )
                        .unwrap()
                );
            }
            pack::WORKERS.set(Some(workers));
            Replay::from_recovered_with_reader(
                CAMPAIGN.into(),
                &kept,
                read,
                || Ok(()),
                |hour| {
                    hours.push(seen(hour));
                    Ok(())
                },
                Some(ahead),
            )
            .unwrap();
            pack::WORKERS.set(None);
            assert_eq!(hours, expected, "{steps:?} {workers} workers");
        }
    }
}

#[test]
fn retained_stop_does_not_restore_any_encoded_frame_twice_even_on_one_core() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let records = scaled_records(1_500, 2, 3);
    let segments = [(
        0,
        Recovery {
            format: Some(FormatVersion::CURRENT),
            records,
            valid_bytes: 0,
            damage: None,
        },
    )];
    let metadata = kept(&segments);
    let reads: Vec<_> = segments[0]
        .1
        .records
        .iter()
        .map(|_| AtomicUsize::new(0))
        .collect();
    let read = |number, frame: &crate::spool::FrameMetadata| {
        assert_eq!(
            reads[frame.placement.offset].fetch_add(1, Ordering::Relaxed),
            0
        );
        restore(&segments, number, frame)
    };
    let mut ahead = Ahead::new(None).unwrap();
    ahead
        .advance(
            CAMPAIGN,
            &[(0, &metadata[0].1.records[..6])],
            read,
            |_| Ok(()),
            &|| false,
        )
        .unwrap();
    pack::WORKERS.set(Some(1));
    let result = Replay::from_recovered_with_reader(
        CAMPAIGN.into(),
        &metadata,
        read,
        || Ok(()),
        |_| Ok(()),
        Some(ahead),
    );
    pack::WORKERS.set(None);
    let result = result.unwrap();
    assert_eq!(result.index.len(), 2);
    for (record, count) in segments[0].1.records.iter().zip(reads) {
        assert_eq!(
            count.load(Ordering::Relaxed),
            usize::from(matches!(record, Record::Frame(_)))
        );
    }
}

#[test]
fn retained_raw_size_splits_match_sequential_bytes_without_restoring_frames_twice() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let records = scaled_records(50, 3, 5);
    let largest = records
        .iter()
        .filter_map(|record| match record {
            Record::Frame(frame) => {
                Some(*frame.table_encoded_sizes().unwrap().iter().max().unwrap())
            }
            _ => None,
        })
        .max()
        .unwrap();
    let limit = largest + largest / 2;
    let expected = packed_on(1, &records, &mut |_| Ok(()), limit).unwrap();
    let segments = [(
        0,
        Recovery {
            format: Some(FormatVersion::CURRENT),
            records,
            valid_bytes: 0,
            damage: None,
        },
    )];
    let metadata = kept(&segments);
    let reads: Vec<_> = segments[0]
        .1
        .records
        .iter()
        .map(|_| AtomicUsize::new(0))
        .collect();
    let read = |number, frame: &crate::spool::FrameMetadata| {
        assert_eq!(
            reads[frame.placement.offset].fetch_add(1, Ordering::Relaxed),
            0
        );
        restore(&segments, number, frame)
    };
    let mut ahead = Ahead::new(None).unwrap();
    ahead.limit_for_test(limit);
    let mut observed = Vec::new();
    ahead
        .advance(
            CAMPAIGN,
            &[(0, &metadata[0].1.records[..7])],
            read,
            |hour| {
                observed.push(seen(hour));
                Ok(())
            },
            &|| false,
        )
        .unwrap();
    let result = Replay::from_recovered_with_reader(
        CAMPAIGN.into(),
        &metadata,
        read,
        || Ok(()),
        |hour| {
            observed.push(seen(hour));
            Ok(())
        },
        Some(ahead),
    )
    .unwrap();
    assert!(result.index.len() > 3);
    assert_eq!(result.bytes(), expected.bytes());
    assert_eq!(observed, scanned(&[&segments[0].1.records]));
}
