//! Block-end flushes on several threads publish exactly what serial flushing
//! publishes, and run each step at its serial refusal state.
use super::family_lanes::{assert_payloads_equal, payloads};
use super::*;
use std::{collections::HashSet, thread::ThreadId};

type Steps = Vec<(usize, i32, ThreadId)>;

struct Flushed {
    payloads: BTreeMap<(String, u64), Vec<u8>>,
    refused: BTreeMap<u64, Refused>,
    steps: Steps,
}

/// One fresh run of the fixture's recording on `parallelism` processors (two
/// or fewer prepare serially), every family in the hourly walk, cleared
/// afterwards.
fn flushed(fixture: &mut Fixture, parallelism: usize) -> Flushed {
    independent::with_test_hourly(|| registered_flushed(fixture, parallelism))
}

/// [`flushed`] with the families prepared on their own the registry names,
/// which the walk's flush skips on every thread.
fn registered_flushed(fixture: &mut Fixture, parallelism: usize) -> Flushed {
    let source = fixture.source("parallel-flush");
    let (result, steps) = flush::with_recorded_steps(|| {
        lanes::with_test_lanes(parallelism, None, || {
            run(&source, None, &AtomicBool::new(false), |_| {})
        })
    });
    result.unwrap();
    let (store, key) = super::super::super::files(&source, None).unwrap();
    let refused = checkpoint(&store, &key, None).unwrap().unwrap().0.refused;
    let payloads = payloads(&source, None);
    store.clear().unwrap();
    drop(source);
    Flushed {
        payloads,
        refused,
        steps,
    }
}

/// Each flush hour's step positions, in the order the steps started.
fn positions(steps: &Steps) -> BTreeMap<i32, Vec<usize>> {
    let mut result = BTreeMap::<i32, Vec<usize>>::new();
    for &(position, hour, _) in steps {
        result.entry(hour).or_default().push(position);
    }
    result
}

fn threads(steps: &Steps) -> usize {
    steps
        .iter()
        .map(|&(_, _, thread)| thread)
        .collect::<HashSet<_>>()
        .len()
}

/// Serial flushing runs each flush's steps in order on one thread; the
/// parallel flush runs the same steps on several.
fn assert_same_steps(serial: &Steps, parallel: &Steps) -> BTreeMap<i32, Vec<usize>> {
    let expected = positions(serial);
    assert_eq!(threads(serial), 1, "{serial:?}");
    for steps in expected.values() {
        assert!(steps.windows(2).all(|pair| pair[0] < pair[1]), "{steps:?}");
    }
    let mut actual = positions(parallel);
    for steps in actual.values_mut() {
        steps.sort_unstable();
    }
    assert_eq!(actual, expected);
    assert!(threads(parallel) > 1, "{parallel:?}");
    expected
}

#[test]
fn block_end_flushes_run_on_several_threads_and_publish_the_serial_bytes() {
    let mut fixture = Fixture::new(&recording());
    let serial = flushed(&mut fixture, 2);
    let parallel = flushed(&mut fixture, 4);
    assert_payloads_equal(
        &parallel.payloads,
        &serial.payloads,
        "parallel vs serial flush",
    );
    assert_eq!(parallel.refused, serial.refused);
    let steps = assert_same_steps(&serial.steps, &parallel.steps);
    // Both branches' flushes ran every family's every step: the same
    // positions, from the first to the last with none skipped.
    assert_eq!(steps.keys().copied().collect::<Vec<_>>(), [11, 21]);
    let every = (0..steps[&11].len()).collect::<Vec<_>>();
    assert!(every.len() > 40, "{every:?}");
    for positions in steps.values() {
        assert_eq!(*positions, every);
    }
    // With the registry's families prepared on their own, the helpers skip
    // the same steps as serial flushing: none diverges.
    let serial = registered_flushed(&mut fixture, 2);
    let parallel = registered_flushed(&mut fixture, 4);
    assert_payloads_equal(
        &parallel.payloads,
        &serial.payloads,
        "registered: parallel vs serial flush",
    );
    assert_eq!(parallel.refused, serial.refused);
    let steps = assert_same_steps(&serial.steps, &parallel.steps);
    for positions in steps.values() {
        assert!(
            positions.len() < 48,
            "registered families flush on their own"
        );
    }
}

#[test]
fn refusals_raised_by_flushes_skip_the_serial_steps_and_publish_the_serial_bytes() {
    // Each refuses a family in its flush only: identities before event_feed's
    // and territory's flushes, which it reaches; groups and the ledger before
    // their own later steps.
    let refusals = [("identities", 11), ("groups", 11), ("politics_ledger", 21)];
    flush::with_flush_refusals(&refusals, || {
        let mut fixture = Fixture::new(&recording());
        let serial = flushed(&mut fixture, 2);
        let parallel = flushed(&mut fixture, 4);
        assert_payloads_equal(
            &parallel.payloads,
            &serial.payloads,
            "flush refusals: parallel vs serial",
        );
        assert_eq!(parallel.refused, serial.refused);
        let steps = assert_same_steps(&serial.steps, &parallel.steps);
        // Every step position, the last of which no refusal here skips.
        let every = 0..=steps.values().flatten().copied().max().unwrap();
        let skipped = |hour: i32| {
            every
                .clone()
                .filter(|position| !steps[&hour].contains(position))
                .collect::<Vec<_>>()
        };
        // Branch 1: event_feed (32) and territory (42) by identities (26),
        // the groups write (33) by group changes (28).
        assert_eq!(skipped(11), [32, 33, 42]);
        // Branch 2: the ledger's state write (40) by its flush (39).
        assert_eq!(skipped(21), [40]);
        let first = &serial.refused[&1].0;
        for (family, from) in [
            ("identities", None),
            ("territory", Some("identities")),
            ("event_feed", Some("identities")),
            ("groups", None),
            ("air_superiority", Some("groups")),
        ] {
            assert_eq!(first[family].hour, 11, "{family}");
            assert_eq!(first[family].from.as_deref(), from, "{family}");
        }
        assert_eq!(serial.refused[&2].0["politics_ledger"].hour, 21);
    });
}

/// The manifest's section names, in their stored order.
fn section_order(source: &Source) -> Vec<String> {
    let path = std::fs::canonicalize(source.path()).unwrap();
    let hash = blake3::hash(path.as_os_str().as_encoded_bytes());
    let id = uuid::Uuid::from_bytes(hash.as_bytes()[..16].try_into().unwrap());
    let directory = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap())
        .join("HOI4Replay/prepared")
        .join(id.to_string());
    store::Listing::open(&directory.join("manifest"))
        .unwrap()
        .unwrap()
        .sections()
        .map(|(name, _)| name.to_owned())
        .collect()
}

/// Synthetic tasks: one family writes at positions 0 and 2, another at 1.
fn interleaved<'a>() -> Vec<flush::Task<'a>> {
    vec![
        flush::Task::new(&[(0, "army"), (2, "army")], |step, writer, _| {
            writer.write(&format!("army-{step}"), &b"army"[..])?;
            Ok(Some(Ok(())))
        }),
        flush::Task::new(&[(1, "navy")], |_, writer, _| {
            writer.write("navy", &b"navy"[..])?;
            Ok(Some(Ok(())))
        }),
    ]
}

#[test]
fn parallel_flush_adopts_in_serial_order_and_its_first_serial_error_wins() {
    let mut fixture = Fixture::new(&recording());
    let source = fixture.source("flush-order");
    let (store, key) = super::super::super::files(&source, None).unwrap();
    let mut orders = Vec::new();
    for threads in [1, 2] {
        let mut writer = store.replace(key.clone()).unwrap();
        flush::run(
            interleaved(),
            threads,
            &mut writer,
            &mut None,
            &mut None,
            &mut Refused::default(),
            7,
        )
        .unwrap();
        writer.commit().unwrap();
        orders.push(section_order(&source));
    }
    assert_eq!(orders[0], ["army-0", "navy", "army-1"]);
    assert_eq!(orders[1], orders[0]);

    // A later step's panic or error never overtakes an earlier error; an
    // earlier panic is raised again on the coordinator.
    let failing = |panics_first: bool| {
        vec![
            flush::Task::new(&[(0, "army")], move |_, _, _| {
                if panics_first {
                    panic!("earliest panic");
                }
                Ok(Some(Err(io::Error::other("earliest"))))
            }),
            flush::Task::new(&[(1, "navy")], |_, _, _| panic!("later panic")),
            flush::Task::new(&[(2, "air")], |_, _, _| Err(io::Error::other("later"))),
        ]
    };
    for threads in [1, 3] {
        let mut writer = store.replace(key.clone()).unwrap();
        let mut refused = Refused::default();
        let error = flush::run(
            failing(false),
            threads,
            &mut writer,
            &mut None,
            &mut None,
            &mut refused,
            7,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "earliest", "{threads} threads");
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            flush::run(
                failing(true),
                threads,
                &mut writer,
                &mut None,
                &mut None,
                &mut refused,
                7,
            )
        }))
        .unwrap_err();
        assert_eq!(
            panicked.downcast_ref::<&str>(),
            Some(&"earliest panic"),
            "{threads} threads"
        );
    }
    store.clear().unwrap();
}

#[test]
fn a_refusal_reaching_another_task_is_refused_rather_than_published() {
    // Identities' refusal reaches territory. Serially territory's later step is
    // skipped; in separate tasks it would already have run, so the parallel
    // flush stops instead of publishing what serial flushing would not.
    let mut fixture = Fixture::new(&recording());
    let source = fixture.source("flush-diverged");
    let (store, key) = super::super::super::files(&source, None).unwrap();
    let tasks = || {
        vec![
            flush::Task::new(&[(0, "identities")], |_, _, _| {
                Ok(Some(Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "identities refuse",
                ))))
            }),
            flush::Task::new(&[(1, "territory")], |_, writer, _| {
                writer.write("territory", &b"territory"[..])?;
                Ok(Some(Ok(())))
            }),
        ]
    };
    let mut refused = Refused::default();
    let mut writer = store.replace(key.clone()).unwrap();
    flush::run(
        tasks(),
        1,
        &mut writer,
        &mut None,
        &mut None,
        &mut refused,
        7,
    )
    .unwrap();
    assert_eq!(refused.0["territory"].from.as_deref(), Some("identities"));
    assert_eq!(writer.bytes_written(), 0);
    drop(writer);
    let mut writer = store.replace(key.clone()).unwrap();
    let error = flush::run(
        tasks(),
        2,
        &mut writer,
        &mut None,
        &mut None,
        &mut Refused::default(),
        7,
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert!(error.to_string().contains("territory diverged"), "{error}");
    assert_eq!(writer.bytes_written(), 0);
    drop(writer);
    store.clear().unwrap();
}
