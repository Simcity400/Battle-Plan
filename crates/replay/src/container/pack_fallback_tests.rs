use super::*;
use crate::{
    container::{
        Replay,
        tests::{CAMPAIGN, records},
        units,
    },
    spool::{self, FrameMetadata, Recovered, Recovery},
};
use std::{
    fs::File,
    io::Read,
    path::PathBuf,
    sync::{Arc, Barrier, atomic::AtomicU64},
};

struct Fixture(PathBuf);

impl Fixture {
    fn new(bytes: &[u8]) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "replay-budgeted-fallback-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, bytes).unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct NoLegacyLanes(Option<units::Watch>);

impl NoLegacyLanes {
    fn enter() -> Self {
        Self(units::WATCH.with_borrow_mut(|watch| {
            watch.replace(Arc::new(|_| {
                panic!("budgeted saves must not start legacy lanes")
            }))
        }))
    }
}

impl Drop for NoLegacyLanes {
    fn drop(&mut self) {
        units::WATCH.with_borrow_mut(|watch| *watch = self.0.take());
    }
}

const HOURS: [i32; 6] = [1, 2, 721, 722, 1441, 1442];

fn input() -> (Vec<u8>, Replay) {
    let records = records(1, &HOURS);
    let bytes = records
        .iter()
        .flat_map(|record| spool::encode_record(record).unwrap())
        .collect();
    let previous = WORKERS.replace(Some(1));
    let replay = Replay::from_records(CAMPAIGN.into(), &[records]).unwrap();
    WORKERS.set(previous);
    assert_eq!(replay.index.len(), 3);
    (bytes, replay)
}

fn recover(input: impl Read, length: usize) -> Recovery<Recovered> {
    spool::recover_reader(input, length, Recovered::keep, |_| Ok(())).unwrap()
}

fn frame_bytes(recovery: &Recovery<Recovered>) -> u64 {
    recovery
        .records
        .iter()
        .filter_map(|record| match record {
            Recovered::Frame(frame) => {
                Some((spool::RECORD_HEADER_BYTES + frame.placement.length) as u64)
            }
            _ => None,
        })
        .sum()
}

#[test]
fn background_save_without_ahead_accounts_for_recovery_frame_reads_and_publication_once() {
    let (bytes, expected) = input();
    let spool = Fixture::new(&bytes);
    let destination = Fixture::new(b"old revision");
    let packing = Packing::with_workers(4).unwrap();
    let _legacy = NoLegacyLanes::enter();
    packing.run(|| {
        let recovered = spool::recover_reader(
            Budgeted::new(File::open(&spool.0).unwrap()),
            bytes.len(),
            |record, placement| {
                assert!(Packing::active());
                assert!(job_priority::is_background_thread());
                Recovered::keep(record, placement)
            },
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(packing.io_bytes(), bytes.len() as u64);
        let reads = frame_bytes(&recovered);
        let restored = Mutex::new(Vec::new());
        let mut progressed = 0;
        let mut observed = Vec::new();
        let replay = Replay::from_recovered_with_reader(
            CAMPAIGN.into(),
            &[(7, recovered)],
            |number, frame| {
                assert_eq!(number, 7);
                assert!(Packing::active());
                assert!(job_priority::is_background_thread());
                lock(&restored).push(frame.hour);
                spool::read_frame_at(
                    &mut Budgeted::new(File::open(&spool.0)?),
                    frame.placement,
                    bytes.len(),
                )
            },
            || {
                progressed += 1;
                Ok(())
            },
            |hour| {
                observed.push(hour.hour);
                Ok(())
            },
            None,
        )
        .unwrap();
        assert_eq!(replay.bytes(), expected.bytes());
        assert_eq!(*lock(&restored), HOURS);
        assert_eq!(progressed, HOURS.len());
        assert_eq!(observed, HOURS);
        assert_eq!(packing.io_bytes(), bytes.len() as u64 + reads);
        replay.write_atomic(&destination.0).unwrap();
        assert_eq!(
            packing.io_bytes(),
            bytes.len() as u64 + reads + replay.saved_length()
        );
    });
    assert_eq!(std::fs::read(&destination.0).unwrap(), expected.bytes());
    assert!(!Packing::active());
}

#[test]
fn stop_promotes_a_save_without_ahead_at_its_next_frame_io_without_repeating_work() {
    let (bytes, expected) = input();
    let spool = Fixture::new(&bytes);
    let recovered = recover(io::Cursor::new(&bytes), bytes.len());
    let reads = frame_bytes(&recovered);
    let packing = Arc::new(Packing::with_workers(4).unwrap());
    let arrived = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let running = packing.clone();
    let first = arrived.clone();
    let blocked = release.clone();
    let saving = thread::spawn(move || {
        let _legacy = NoLegacyLanes::enter();
        running.run(|| {
            let restored = Mutex::new(Vec::new());
            let mut progressed = 0;
            let mut observed = Vec::new();
            let replay = Replay::from_recovered_with_reader(
                CAMPAIGN.into(),
                &[(9, recovered)],
                |number, frame: &FrameMetadata| {
                    assert_eq!(number, 9);
                    assert!(Packing::active());
                    if frame.hour == HOURS[0] {
                        assert!(job_priority::is_background_thread());
                        first.wait();
                        blocked.wait();
                    } else {
                        assert!(!job_priority::is_background_thread());
                    }
                    let restored_frame = spool::read_frame_at(
                        &mut Budgeted::new(File::open(&spool.0)?),
                        frame.placement,
                        bytes.len(),
                    )?;
                    assert!(!job_priority::is_background_thread());
                    lock(&restored).push(frame.hour);
                    Ok(restored_frame)
                },
                || {
                    progressed += 1;
                    Ok(())
                },
                |hour| {
                    observed.push(hour.hour);
                    Ok(())
                },
                None,
            )
            .unwrap();
            assert_eq!(*lock(&restored), HOURS);
            assert_eq!(progressed, HOURS.len());
            assert_eq!(observed, HOURS);
            replay
        })
    });
    arrived.wait();
    packing.promote();
    release.wait();
    assert_eq!(saving.join().unwrap().bytes(), expected.bytes());
    assert_eq!(packing.io_bytes(), reads);
}

#[test]
fn saturated_pool_finishes_blocks_ordered_batches_and_nested_helpers_in_both_priorities() {
    for foreground in [2, 4] {
        for promote in [false, true] {
            let packing = Arc::new(Packing::with_workers(foreground).unwrap());
            let identities = packing.pool.broadcast(|_| thread::current().id());
            let count = identities.len();
            let arrived = Arc::new(Barrier::new(count + 1));
            let release = Arc::new(Barrier::new(count + 1));
            let running = packing.clone();
            let first = arrived.clone();
            let blocked = release.clone();
            let saturating = thread::spawn(move || {
                running.pool.broadcast(|_| {
                    assert!(job_priority::is_background_thread());
                    first.wait();
                    blocked.wait();
                    let coordinator = thread::current().id();
                    let (_, packed) = blocks(
                        workers(),
                        |job, cores| {
                            assert_eq!(thread::current().id(), coordinator);
                            assert_eq!(job_priority::is_background_thread(), !promote);
                            let mut next = 0_usize..8;
                            let mut total = 0;
                            ordered(
                                workers(),
                                || next.next(),
                                |item| {
                                    each(cores, vec![0, 1, 2], |leaf| {
                                        assert!(Packing::active());
                                        assert_eq!(job_priority::is_background_thread(), !promote);
                                        let mut byte = [0];
                                        Budgeted::new(io::Cursor::new([1]))
                                            .read_exact(&mut byte)?;
                                        Ok(job + item + leaf)
                                    })
                                    .unwrap()
                                    .into_iter()
                                    .sum::<usize>()
                                },
                                |value| {
                                    total += value;
                                    true
                                },
                            );
                            Ok(total)
                        },
                        |packer| {
                            for job in 0_usize..3 {
                                packer.submit(job)?;
                            }
                            Ok(())
                        },
                    )
                    .unwrap();
                    assert_eq!(packed, [108, 132, 156]);
                })
            });
            arrived.wait();
            if promote {
                packing.promote();
            }
            release.wait();
            assert_eq!(saturating.join().unwrap().len(), count);
            assert_eq!(packing.io_bytes(), (count * 3 * 8 * 3) as u64);
            assert_eq!(
                identities,
                packing.pool.broadcast(|_| thread::current().id())
            );
        }
    }
}

#[test]
fn stop_reaches_busy_helpers_and_the_single_credit_inline_path_at_the_next_boundary() {
    for count in [1, 4] {
        let packing = Arc::new(Packing::with_workers(4).unwrap());
        let arrived = Arc::new(Barrier::new(count + 1));
        let release = Arc::new(Barrier::new(count + 1));
        let running = packing.clone();
        let first = arrived.clone();
        let blocked = release.clone();
        let saving = thread::spawn(move || {
            running.run(|| {
                each(&Cores::new(count), (0..2 * count).collect(), |item| {
                    assert!(Packing::active());
                    if item < count {
                        assert!(job_priority::is_background_thread());
                        first.wait();
                        blocked.wait();
                        return Ok(item);
                    }
                    assert!(!job_priority::is_background_thread());
                    let mut byte = [0];
                    Budgeted::new(io::Cursor::new([1])).read_exact(&mut byte)?;
                    assert!(!job_priority::is_background_thread());
                    Ok(item)
                })
                .unwrap()
            })
        });
        arrived.wait();
        packing.promote();
        release.wait();
        assert_eq!(saving.join().unwrap(), (0..2 * count).collect::<Vec<_>>());
        assert_eq!(packing.io_bytes(), count as u64);
    }
}

#[test]
fn budgeted_ordered_batches_keep_panic_acceptance_order_and_bound_read_ahead() {
    let packing = Packing::with_workers(2).unwrap();
    packing.run(|| {
        for threads in [1, 2, 4] {
            let mut read = 0;
            let mut taken = Vec::new();
            ordered(
                threads,
                || {
                    read += 1;
                    Some(read)
                },
                |item| {
                    assert_ne!(item, 6, "later panic");
                    item
                },
                |item| {
                    taken.push(item);
                    item < 5
                },
            );
            assert_eq!(taken, [1, 2, 3, 4, 5]);
            assert!(read <= 5 + 2 * threads);
            let mut next = 0..8;
            let mut taken = Vec::new();
            ordered(
                threads,
                || next.next(),
                |item| {
                    assert_ne!(item, 3, "panic after the first error");
                    if item == 2 {
                        Err(io::Error::other("two"))
                    } else {
                        Ok(item)
                    }
                },
                |result| match result {
                    Ok(item) => {
                        taken.push(item);
                        true
                    }
                    Err(error) => {
                        assert_eq!(error.to_string(), "two");
                        false
                    }
                },
            );
            assert_eq!(taken, [0, 1]);
            let mut next = 0..8;
            let mut taken = Vec::new();
            let panic = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                ordered(
                    threads,
                    || next.next(),
                    |item| {
                        assert_ne!(item, 3, "ordered panic");
                        item
                    },
                    |item| {
                        taken.push(item);
                        true
                    },
                )
            }));
            assert!(panic.is_err());
            assert_eq!(taken, [0, 1, 2]);
        }
    });
}

#[test]
fn budgeted_blocks_preserve_submission_order_and_the_first_error_or_panic() {
    for promote in [false, true] {
        let packing = Packing::with_workers(4).unwrap();
        if promote {
            packing.promote();
        }
        packing.run(|| {
            for panics in [false, true] {
                let calls = Mutex::new(Vec::new());
                let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                    blocks(
                        workers(),
                        |job, cores| {
                            lock(&calls).push(job);
                            if job == 2 {
                                assert!(!panics, "first packing panic");
                                return Err(io::Error::other("first packing error"));
                            }
                            each(cores, vec![job], Ok)
                        },
                        |packer| {
                            for job in 0..5 {
                                packer.submit(job)?;
                            }
                            Err::<(), _>(io::Error::other("later writer error"))
                        },
                    )
                }));
                assert_eq!(*lock(&calls), [0, 1, 2]);
                if panics {
                    assert!(result.is_err());
                } else {
                    assert_eq!(
                        result.unwrap().unwrap_err().to_string(),
                        "first packing error"
                    );
                }
            }
        });
    }
}

#[test]
fn catch_up_promotes_busy_helpers_at_the_next_item_without_adding_cpu_credits() {
    let packing = Arc::new(Packing::with_workers(8).unwrap());
    let arrived = Arc::new(Barrier::new(5));
    let release = Arc::new(Barrier::new(5));
    let running = packing.clone();
    let first = arrived.clone();
    let blocked = release.clone();
    let saving = thread::spawn(move || {
        running.run(|| {
            each(&Cores::new(workers()), (0..8).collect(), |item| {
                if item < 4 {
                    assert!(job_priority::is_background_thread());
                    first.wait();
                    blocked.wait();
                } else {
                    assert!(!job_priority::is_background_thread());
                    assert_eq!(workers(), 4);
                }
                Ok(item)
            })
            .unwrap()
        })
    });
    arrived.wait();
    packing.catch_up(true);
    release.wait();
    assert_eq!(saving.join().unwrap(), (0..8).collect::<Vec<_>>());
    packing.run(|| {
        assert_eq!(packing.mode(), "catch_up_normal");
        assert!(!job_priority::is_background_thread());
        assert_eq!(workers(), 4);
        let mut bytes = [0; 128 << 10];
        let mut input = Budgeted::new(io::Cursor::new(vec![1; bytes.len()]));
        assert_eq!(input.read(&mut bytes).unwrap(), 64 << 10);
        packing.catch_up(false);
        assert_eq!(packing.mode(), "background");
        assert!(job_priority::is_background_thread());
        assert_eq!(workers(), 4);
        packing.promote();
        packing.catch_up(false);
        assert_eq!(packing.mode(), "foreground");
        assert!(!job_priority::is_background_thread());
        assert_eq!(workers(), 8);
        let mut input = Budgeted::new(io::Cursor::new(vec![1; bytes.len()]));
        assert_eq!(input.read(&mut bytes).unwrap(), bytes.len());
        assert_eq!(packing.io_bytes(), (192 << 10) as u64);
    });
    assert!(!Packing::active());
}

#[test]
fn the_active_work_stall_guard_is_deterministic_and_cannot_undo_stop() {
    use std::time::Duration;
    let packing = Packing::with_workers(8).unwrap();
    let elapsed = Duration::from_nanos;
    assert!(!stalled(&packing.budget, elapsed(100)));
    packing.budget.progress_due_ns.store(10, Ordering::Release);
    assert!(!stalled(&packing.budget, elapsed(9)));
    assert_eq!(packing.mode(), "background");
    assert!(stalled(&packing.budget, elapsed(10)));
    assert_eq!(packing.mode(), "catch_up_normal");
    assert!(!stalled(&packing.budget, elapsed(11)));
    packing.watch_progress(false);
    packing.catch_up(false);
    assert!(!stalled(&packing.budget, elapsed(100)));
    packing.promote();
    packing.budget.progress_due_ns.store(10, Ordering::Release);
    assert!(!stalled(&packing.budget, elapsed(100)));
    assert_eq!(packing.mode(), "foreground");
}
