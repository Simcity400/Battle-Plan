use super::*;
use crate::prepared::{
    coverage,
    driver::{
        plan,
        registry::{self, Descriptor},
    },
    pipeline,
    prefix::Prefix,
};
use dashboard_base::prepared::gates;
use std::{
    io::Read,
    path::PathBuf,
    sync::{
        LazyLock,
        atomic::{AtomicUsize, Ordering as Atomic},
    },
};

const BRANCH: u64 = 1;

/// A family of the tests' own, gating its product.
static STEADY: Descriptor = Descriptor {
    queries: &["steady-product"],
    gates: &["steady-product"],
    sections: &["steady"],
    ..registry::family("steady")
};
/// Another, for a loss that ends its branch.
static ENDING: Descriptor = Descriptor {
    queries: &["ending-product"],
    gates: &["ending-product"],
    sections: &["ending"],
    ..registry::family("ending")
};

/// Preparation's plan, with the tests' families prepared beside the registry's.
static PLAN: LazyLock<&'static plan::Plan> = LazyLock::new(|| {
    let mut native = crate::prepared::driver::native::names();
    native.extend(["steady", "ending"]);
    plan::test_native(&[&STEADY, &ENDING], &native).unwrap()
});

/// `query` answered under the tests' plan.
fn planned<T>(query: impl FnOnce() -> T) -> T {
    plan::with_test_native(*PLAN, query)
}

/// A store of its own, bound to a synthetic recording.
struct Fixture {
    root: PathBuf,
    store: store::Store,
    key: store::SourceKey,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("hoi4-publication-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let recording = root.join("recording");
        std::fs::write(&recording, b"synthetic").unwrap();
        Self {
            key: store::SourceKey::capture(&recording, &[]).unwrap(),
            store: store::Store::local(uuid::Uuid::new_v4()).unwrap(),
            root,
        }
    }

    fn open(&self, families: &[Family]) -> (Publisher<'_>, Resume) {
        Publisher::open(
            &self.store,
            self.key.clone(),
            binding(),
            *PLAN,
            families,
            &|_, _, _| true,
        )
        .unwrap()
    }

    /// The gates a query pins now, decoded afresh.
    fn gates(&self) -> gates::Published {
        gates::decode(&self.store, None)
            .unwrap()
            .expect("a publication")
    }

    fn count(&self, section: &str) -> Option<u64> {
        self.store.range_count(&self.key, section).unwrap()
    }

    fn bytes(&self, section: &str, range: u64) -> Option<Vec<u8>> {
        self.store
            .read(&self.key, section, range)
            .unwrap()
            .map(|mut file| {
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes).unwrap();
                bytes
            })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.store.clear();
        let _ = std::fs::remove_dir_all(self.store.directory());
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn binding() -> Binding {
    Binding::new(b"synthetic recording", b"synthetic content")
}

fn family(name: &str) -> Family {
    Family::planned(*PLAN, &[name]).unwrap()[0]
}

/// Segment `index` of run 0: ordinals and hours 10 apart, hours from 1000.
fn segment(index: u32) -> Hours {
    hours(index * 10, index * 10 + 9, true, false)
}

fn hours(first: u32, last: u32, ends_segment: bool, ends_branch: bool) -> Hours {
    Hours {
        branch: BRANCH,
        run: 0,
        first,
        last,
        first_hour: 1000 + first as i32,
        last_hour: 1000 + last as i32,
        ends_segment,
        ends_branch,
        proof: [last as u8; 32],
    }
}

/// Writes one page of `section` for `hours`, then records them as `made`; whether the batch
/// ends after them.
fn process(stage: &mut Stage<'_, '_>, section: &str, hours: Hours, made: Made) -> bool {
    stage
        .pages()
        .unwrap()
        .write(section, &hours.last_hour.to_le_bytes()[..])
        .unwrap();
    stage.processed(hours, made, None, None).unwrap()
}

/// Processes `segments` as prepared, submitting wherever the batch ends; the segments ending a
/// committed batch, counted from 1.
fn prepare(stage: &mut Stage<'_, '_>, section: &str, segments: std::ops::Range<u32>) -> Vec<u32> {
    let mut committed = Vec::new();
    for index in segments {
        if process(stage, section, segment(index), Made::Prepared) {
            stage.submit().unwrap();
            committed.push(index + 1);
        }
    }
    committed
}

#[test]
fn batches_end_after_the_first_second_fourth_eighth_and_sixteenth_segment_then_every_32nd() {
    let ends: Vec<u64> = (0..=100).filter(|&n| ends_batch(n, 32)).collect();
    assert_eq!(ends, [1, 2, 4, 8, 16, 32, 64, 96]);
    let fixture = Fixture::new();
    let steady = family("steady");
    let mut committed = {
        let (publisher, _) = fixture.open(&[steady]);
        let mut stage = publisher.stage(steady.id).unwrap();
        // The 17th to 20th segments end no batch: interrupted, they are prepared again.
        prepare(&mut stage, "steady-test", 0..20)
    };
    assert_eq!(committed, [1, 2, 4, 8, 16]);
    // A resumed run counts its segments from the branch's first.
    let (publisher, resume) = fixture.open(&[steady]);
    assert_eq!(resume.families[&steady.id][&BRANCH].segments, 16);
    assert_eq!(resume.attach(steady.id, BRANCH, 0, 160, 169), Attach::Whole);
    let mut stage = publisher.stage(steady.id).unwrap();
    committed.extend(prepare(&mut stage, "steady-test", 16..40));
    assert_eq!(committed, [1, 2, 4, 8, 16, 32]);
    let published = publisher.committed(steady.id, BRANCH).unwrap();
    assert_eq!(published.publication.generation, 6);
    assert_eq!(published.frontier.segments, 32);
    assert_eq!(fixture.count("steady-test"), Some(32));
}

#[test]
fn a_query_after_the_first_segment_reads_it_while_later_work_is_held() {
    let fixture = Fixture::new();
    let steady = family("steady");
    let (publisher, _) = fixture.open(&[steady]);
    let mut stage = publisher.stage(steady.id).unwrap();
    assert!(process(
        &mut stage,
        "steady-test",
        segment(0),
        Made::Prepared
    ));
    stage.submit().unwrap();
    // The second segment ends its batch too; its lane holds it.
    assert!(process(
        &mut stage,
        "steady-test",
        segment(1),
        Made::Prepared
    ));
    let gates = fixture.gates();
    let covered = planned(|| pipeline::published_coverage(&gates, BRANCH, "steady-product"))
        .unwrap()
        .unwrap();
    assert_eq!(
        (covered.first_hour, covered.last_hour, covered.complete),
        (1000, 1009, false)
    );
    assert!(
        planned(|| pipeline::published_holes(&gates, BRANCH, "steady-product", None))
            .unwrap()
            .is_empty()
    );
    assert_eq!(fixture.count("steady-test"), Some(1));
    let progress = publisher.progress()[&steady.id].clone();
    assert_eq!(
        (progress.emitted, progress.processed, progress.available),
        (2, 1, 10)
    );
    assert_eq!(progress.phase(), Phase::Available);
}

/// Production through four segments and repairs through one, both suppliers of
/// `dockyard-spending`.
fn fast_and_slow(fixture: &Fixture) -> (Family, Family) {
    let (production, repairs) = (family("production"), family("repairs"));
    let (publisher, _) = fixture.open(&[production, repairs]);
    let mut fast = publisher.stage(production.id).unwrap();
    assert_eq!(prepare(&mut fast, "production-spent-test", 0..4), [1, 2, 4]);
    let mut slow = publisher.stage(repairs.id).unwrap();
    assert_eq!(prepare(&mut slow, "dockyard-repairs-test", 0..1), [1]);
    (production, repairs)
}

#[test]
fn fast_and_slow_families_publish_their_own_frontiers() {
    let fixture = Fixture::new();
    fast_and_slow(&fixture);
    let gates = fixture.gates();
    let last = |product| {
        planned(|| pipeline::published_coverage(&gates, BRANCH, product))
            .unwrap()
            .map(|covered| covered.last_hour)
    };
    assert_eq!(last("production-select"), Some(1039));
    assert_eq!(last("dockyard-spending"), Some(1009));
}

#[test]
fn a_joint_query_reads_every_supplier_from_one_pinned_publication() {
    let fixture = Fixture::new();
    let (production, repairs) = fast_and_slow(&fixture);
    let pinned = fixture.gates();
    let joint = |gates: &gates::Published| {
        planned(|| pipeline::published_coverage(gates, BRANCH, "dockyard-spending"))
            .unwrap()
            .map(|covered| covered.last_hour)
    };
    {
        let (publisher, resume) = fixture.open(&[production, repairs]);
        assert_eq!(resume.attach(repairs.id, BRANCH, 0, 10, 19), Attach::Whole);
        let mut slow = publisher.stage(repairs.id).unwrap();
        assert_eq!(prepare(&mut slow, "dockyard-repairs-test", 1..2), [2]);
    }
    assert_eq!(
        joint(&pinned),
        Some(1009),
        "a pinned publication never moves"
    );
    let current = gates::decode(&fixture.store, Some(&pinned))
        .unwrap()
        .expect("a publication");
    assert_eq!(joint(&current), Some(1019));
    assert_ne!(
        planned(|| pipeline::query_pages(&pinned, BRANCH, "dockyard-spending")).unwrap(),
        planned(|| pipeline::query_pages(&current, BRANCH, "dockyard-spending")).unwrap(),
        "a publication to a supplier moves the query's identity"
    );
}

#[test]
fn a_refusal_is_published_with_every_later_hour_unanswered() {
    let fixture = Fixture::new();
    let steady = family("steady");
    {
        let (publisher, _) = fixture.open(&[steady]);
        let mut stage = publisher.stage(steady.id).unwrap();
        assert!(process(
            &mut stage,
            "steady-test",
            segment(0),
            Made::Prepared
        ));
        let refusal = Made::Refused {
            hour: 1015,
            reason: "Refused for the test".into(),
            from: None,
        };
        assert!(process(&mut stage, "steady-test", segment(1), refusal));
        stage.submit().unwrap();
        assert!(
            stage
                .processed(segment(2), Made::Prepared, None, None)
                .is_err(),
            "nothing of a refused branch goes on"
        );
    }
    let gates = fixture.gates();
    let refusal = planned(|| pipeline::refusal_from(&gates, BRANCH, "steady-product")).unwrap();
    // Queries before it answer, those across it refuse (`publication_gates`).
    assert_eq!(refusal, Some((1015, "Refused for the test".into())));
    let holes = |through| {
        planned(|| pipeline::published_holes(&gates, BRANCH, "steady-product", Some(through)))
            .unwrap()
    };
    assert!(holes(1014).is_empty());
    let across = holes(1030);
    assert_eq!(across.first().map(|hole| hole.first_hour), Some(1015));
    assert_eq!(across.last().map(|hole| hole.last_hour), Some(1030));
    assert!(
        across
            .iter()
            .all(|hole| hole.unavailable == coverage::Unavailable::Refused)
    );
    let (_, resume) = fixture.open(&[steady]);
    assert_eq!(resume.attach(steady.id, BRANCH, 0, 20, 29), Attach::Skip);
}

#[test]
fn no_value_is_carried_or_summed_across_a_hole() {
    let fixture = Fixture::new();
    let (steady, ending) = (family("steady"), family("ending"));
    let (publisher, _) = fixture.open(&[steady, ending]);
    let lost = |ends| Made::Lost {
        reason: "A damaged unit".into(),
        ends,
    };
    let mut stage = publisher.stage(steady.id).unwrap();
    for (index, made) in [Made::Prepared, lost(false), Made::Prepared]
        .into_iter()
        .enumerate()
    {
        process(&mut stage, "steady-test", segment(index as u32), made);
        stage.submit().unwrap();
    }
    let gates = fixture.gates();
    let covered = planned(|| pipeline::published_coverage(&gates, BRANCH, "steady-product"))
        .unwrap()
        .unwrap();
    assert_eq!((covered.first_hour, covered.last_hour), (1000, 1029));
    let holes =
        planned(|| pipeline::published_holes(&gates, BRANCH, "steady-product", None)).unwrap();
    assert_eq!(
        holes
            .iter()
            .map(|hole| (hole.first_hour, hole.last_hour, hole.unavailable))
            .collect::<Vec<_>>(),
        [(1010, 1019, coverage::Unavailable::Lost)]
    );
    // A reply needing history answers nothing from the hole on, and leaves none of it pending.
    let prefix = Prefix {
        coverage: covered,
        pending: Vec::new(),
    }
    .before(&holes);
    assert_eq!(
        (prefix.coverage.last_hour, prefix.coverage.complete),
        (1009, false)
    );
    assert_eq!(prefix.through(Some(1029)), Some(1009));
    assert!(prefix.pending.is_empty());
    // Its lost hours are never counted as prepared ones.
    assert_eq!(publisher.progress()[&steady.id].available, 20);
    // A loss its family cannot go on from ends its branch: every later hour is a hole.
    let mut stage = publisher.stage(ending.id).unwrap();
    process(&mut stage, "ending-test", segment(0), lost(true));
    stage.submit().unwrap();
    assert!(
        stage
            .processed(segment(1), Made::Prepared, None, None)
            .is_err()
    );
    let gates = fixture.gates();
    let holes = planned(|| pipeline::published_holes(&gates, BRANCH, "ending-product", Some(1100)))
        .unwrap();
    assert_eq!(holes.first().map(|hole| hole.first_hour), Some(1000));
    assert_eq!(holes.last().map(|hole| hole.last_hour), Some(1100));
}

#[test]
fn a_batch_that_does_not_follow_rolls_back_alone_and_its_group_commits() {
    let fixture = Fixture::new();
    let (production, repairs) = (family("production"), family("repairs"));
    let (publisher, _) = fixture.open(&[production, repairs]);
    let mut first = publisher.stage(production.id).unwrap();
    process(
        &mut first,
        "production-spent-test",
        segment(0),
        Made::Prepared,
    );
    // A second lane of the same family stages the same hours before the first commits.
    let mut stale = publisher.stage(production.id).unwrap();
    process(
        &mut stale,
        "production-spent-test",
        segment(0),
        Made::Prepared,
    );
    let refused = stale
        .pages()
        .unwrap()
        .write("dockyard-repairs-test", &b"not production's"[..])
        .unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied);
    first.submit().unwrap();
    let mut other = publisher.stage(repairs.id).unwrap();
    process(
        &mut other,
        "dockyard-repairs-test",
        segment(0),
        Made::Prepared,
    );
    // Both batches are queued while the coordinator commits, so one group commits them.
    publisher.sealing(2, || {
        publisher
            .coordinate(|_| {
                stale.send().unwrap();
                other.send().unwrap();
                while publisher.lock().queue.len() < 2 {
                    std::thread::yield_now();
                }
                Ok(())
            })
            .unwrap();
        other.settle().unwrap();
        assert_eq!(
            stale.settle().unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    });
    assert_eq!(
        publisher
            .committed(production.id, BRANCH)
            .unwrap()
            .publication
            .generation,
        1
    );
    assert!(publisher.committed(repairs.id, BRANCH).is_some());
    assert_eq!(fixture.count("production-spent-test"), Some(1));
    assert_eq!(fixture.count("dockyard-repairs-test"), Some(1));
}

#[test]
fn an_interruption_around_adoption_or_root_publication_leaves_a_whole_root() {
    let fixture = Fixture::new();
    let steady = family("steady");
    // Staged and never adopted.
    {
        let (publisher, _) = fixture.open(&[steady]);
        let mut stage = publisher.stage(steady.id).unwrap();
        process(&mut stage, "steady-test", segment(0), Made::Prepared);
    }
    let (_, resume) = fixture.open(&[steady]);
    assert!(resume.families.is_empty());
    assert!(!fixture.count("steady-test").is_some_and(|count| count > 0));
    // Adopted, then gone before its root.
    {
        let (publisher, _) = fixture.open(&[steady]);
        publisher.fail_next_commit(Fault::BeforeRoot);
        let mut stage = publisher.stage(steady.id).unwrap();
        process(&mut stage, "steady-test", segment(0), Made::Prepared);
        assert!(stage.submit().is_err());
        assert!(matches!(publisher.stopped(), Some(Stopped::Uncertain(_))));
        assert!(publisher.committed(steady.id, BRANCH).is_none());
    }
    let (_, resume) = fixture.open(&[steady]);
    assert!(resume.families.is_empty());
    // Its root published, then an error: what is read back stands, and nothing else goes on.
    let commits = std::sync::Arc::new(AtomicUsize::new(0));
    {
        let counted = std::sync::Arc::clone(&commits);
        let (publisher, _) = fixture.open(&[steady]);
        let publisher = publisher.with_commits(move || {
            counted.fetch_add(1, Atomic::SeqCst);
        });
        publisher.fail_next_commit(Fault::AfterRoot);
        let mut stage = publisher.stage(steady.id).unwrap();
        process(&mut stage, "steady-test", segment(0), Made::Prepared);
        stage.submit().unwrap();
        assert!(matches!(publisher.stopped(), Some(Stopped::Uncertain(_))));
        assert_eq!(
            publisher
                .committed(steady.id, BRANCH)
                .unwrap()
                .frontier
                .last_hour,
            1009
        );
        let error = stage
            .processed(segment(1), Made::Prepared, None, None)
            .unwrap_err();
        assert!(halted(&error));
    }
    assert_eq!(commits.load(Atomic::SeqCst), 1);
    let (_, resume) = fixture.open(&[steady]);
    assert_eq!(resume.families[&steady.id][&BRANCH].last_hour, 1009);
    assert_eq!(fixture.count("steady-test"), Some(1));
}

#[test]
fn every_stop_refuses_processed_hours_before_the_next_cut() {
    for stopped in [
        Stopped::Cancelled,
        Stopped::Cleared,
        Stopped::Uncertain("An uncertain commit".into()),
        Stopped::Failed("A failed publication".into()),
    ] {
        for pages in [false, true] {
            let fixture = Fixture::new();
            let steady = family("steady");
            let (publisher, _) = fixture.open(&[steady]);
            let mut stage = publisher.stage(steady.id).unwrap();
            assert_eq!(prepare(&mut stage, "steady-test", 0..2), [1, 2]);
            if pages {
                stage
                    .pages()
                    .unwrap()
                    .write("steady-test", &b"uncommitted"[..])
                    .unwrap();
            }
            let progress = publisher.progress();
            std::thread::scope(|scope| {
                let stopping = scope.spawn(|| publisher.stop(stopped.clone()));
                let mut inner = publisher.lock();
                while inner.stopped.is_none() {
                    inner = publisher.wait_changed(inner);
                }
                drop(inner);
                let error = stage
                    .processed(segment(2), Made::Prepared, None, None)
                    .unwrap_err();
                assert!(halted(&error));
                assert_eq!(error.kind(), stopped.error().kind());
                assert_eq!(error.to_string(), stopped.error().to_string());
                assert!(stage.steps.is_empty());
                assert!(!stage.ends());
                assert_eq!(publisher.progress(), progress);
                drop(stage);
                stopping.join().unwrap();
            });
            assert_eq!(fixture.count("steady-test"), Some(2));
            assert_eq!(
                publisher
                    .committed(steady.id, BRANCH)
                    .unwrap()
                    .frontier
                    .last_hour,
                1019
            );
        }
    }
}

#[test]
fn processing_a_segment_leaves_the_previous_batch_with_its_sealer() {
    let fixture = Fixture::new();
    let steady = family("steady");
    let (publisher, _) = fixture.open(&[steady]);
    let mut stage = publisher.stage(steady.id).unwrap();
    assert!(
        stage
            .processed(segment(0), Made::Prepared, None, None)
            .unwrap()
    );
    // Keep the job pending regardless of pressure on the shared series budget.
    stage.handed = Some(publisher.hand(stage.cut().unwrap()));
    let handed = stage.handed;
    assert!(handed.is_some());
    assert!(
        stage
            .processed(segment(1), Made::Prepared, None, None)
            .unwrap()
    );
    assert_eq!(stage.handed, handed);
    assert!(stage.writer.is_none());
    assert_eq!(publisher.lock().jobs.len(), 1);
    assert!(publisher.committed(steady.id, BRANCH).is_none());
    stage.submit().unwrap();
    assert_eq!(publisher.progress()[&steady.id].processed, 2);
    assert_eq!(publisher.progress()[&steady.id].available, 20);
}

#[test]
fn each_family_resumes_from_its_own_committed_segment() {
    let fixture = Fixture::new();
    let (production, repairs) = fast_and_slow(&fixture);
    let (publisher, resume) = fixture.open(&[production, repairs]);
    assert!(resume.again.is_empty());
    let next = |family: Family| resume.families[&family.id][&BRANCH].next;
    assert_eq!(
        next(production),
        Cursor {
            run: 0,
            ordinal: 40
        }
    );
    assert_eq!(
        next(repairs),
        Cursor {
            run: 0,
            ordinal: 10
        }
    );
    for (family, index, attach) in [
        (production, 1, Attach::Skip),
        (repairs, 1, Attach::Whole),
        (production, 4, Attach::Whole),
        (repairs, 4, Attach::Whole),
    ] {
        let seg = segment(index);
        assert_eq!(
            resume.attach(family.id, BRANCH, seg.run, seg.first, seg.last),
            attach
        );
    }
    assert_eq!(resume.attach(production.id, 2, 0, 0, 9), Attach::Whole);
    let mut fast = publisher.stage(production.id).unwrap();
    assert!(
        fast.processed(segment(1), Made::Prepared, None, None)
            .is_err()
    );
    let mut slow = publisher.stage(repairs.id).unwrap();
    assert!(
        slow.processed(segment(1), Made::Prepared, None, None)
            .unwrap()
    );
}

#[test]
fn the_open_tail_publishes_each_arrival_and_an_append_goes_on_after_its_last_hour() {
    let fixture = Fixture::new();
    let steady = family("steady");
    let state = |hour: i32| Some(hour.to_le_bytes().to_vec());
    {
        let (publisher, _) = fixture.open(&[steady]);
        let mut stage = publisher.stage(steady.id).unwrap();
        assert_eq!(prepare(&mut stage, "steady-test", 0..1), [1]);
        // Each arrival of the open tail's committed hours ends a batch, and publishes within
        // its segment: the publication's sequence moves on, its completed segments do not.
        for (sequence, (first, last)) in [(2, (10, 14)), (3, (15, 16))] {
            let tail = hours(first, last, false, true);
            assert!(
                stage
                    .processed(tail, Made::Prepared, state(tail.last_hour), state(-1))
                    .unwrap()
            );
            stage.submit().unwrap();
            let published = publisher.committed(steady.id, BRANCH).unwrap();
            assert_eq!(published.publication.segment, sequence);
            assert_eq!(published.frontier.segments, 1);
            assert_eq!(
                published.frontier.next,
                Cursor {
                    run: 0,
                    ordinal: last + 1
                }
            );
            let covered = planned(|| {
                pipeline::published_coverage(&fixture.gates(), BRANCH, "steady-product")
            })
            .unwrap()
            .unwrap();
            assert_eq!(
                (covered.last_hour, covered.complete),
                (1000 + last as i32, false)
            );
        }
    }
    let (publisher, resume) = fixture.open(&[steady]);
    assert_eq!(
        resume.attach(steady.id, BRANCH, 0, 10, 19),
        Attach::From(17)
    );
    let published = publisher.committed(steady.id, BRANCH).unwrap();
    assert_eq!(published.frontier.segments, 1);
    let before = fixture
        .store
        .session(&fixture.key)
        .unwrap()
        .unwrap()
        .snapshot();
    let mut view = &before;
    assert_eq!(
        roots::state(&mut view, "steady", BRANCH, &published).unwrap(),
        state(1016)
    );
    assert_eq!(
        roots::carry(&mut view, "steady", BRANCH, &published).unwrap(),
        state(-1)
    );
    let mut stage = publisher.stage(steady.id).unwrap();
    assert!(
        stage
            .processed(hours(15, 19, true, true), Made::Prepared, None, None)
            .is_err(),
        "no hour is published twice"
    );
    assert!(
        stage
            .processed(hours(17, 19, true, true), Made::Prepared, None, None)
            .unwrap()
    );
    stage.submit().unwrap();
    let published = publisher.committed(steady.id, BRANCH).unwrap();
    assert_eq!(published.frontier.segments, 2);
    assert!(published.frontier.complete);
    let snapshot = fixture
        .store
        .session(&fixture.key)
        .unwrap()
        .unwrap()
        .snapshot();
    let mut view = &snapshot;
    let ledger =
        roots::ledger(&mut view, "steady", BRANCH, &published, i32::MIN, i32::MAX).unwrap();
    assert_eq!(
        ledger
            .iter()
            .map(|interval| (interval.first_hour, interval.last_hour, interval.held))
            .collect::<Vec<_>>(),
        [(1000, 1019, roots::Held::Prepared)]
    );
    assert_eq!(publisher.progress()[&steady.id].available, 20);
    let covered =
        planned(|| pipeline::published_coverage(&fixture.gates(), BRANCH, "steady-product"))
            .unwrap()
            .unwrap();
    assert_eq!((covered.last_hour, covered.complete), (1019, true));
}

#[test]
fn a_clear_waits_for_pending_stages_and_none_of_them_publish() {
    let fixture = Fixture::new();
    let (production, repairs) = (family("production"), family("repairs"));
    let (publisher, _) = fixture.open(&[production, repairs]);
    let mut queued = publisher.stage(production.id).unwrap();
    process(
        &mut queued,
        "production-spent-test",
        segment(0),
        Made::Prepared,
    );
    queued.send().unwrap();
    let mut open = publisher.stage(repairs.id).unwrap();
    process(
        &mut open,
        "dockyard-repairs-test",
        segment(0),
        Made::Prepared,
    );
    std::thread::scope(|scope| {
        let clearing = scope.spawn(|| publisher.stop(Stopped::Cleared));
        while publisher.stopped().is_none() {
            std::thread::yield_now();
        }
        // The open stage learns of the Clear and lets its handles go.
        let error = open.submit().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        clearing.join().unwrap();
    });
    assert_eq!(
        queued.settle().unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
    fixture.store.clear().unwrap();
    assert_eq!(fixture.count("production-spent-test"), None);
    assert_eq!(fixture.count("dockyard-repairs-test"), None);
    drop(queued);
    drop(open);
    drop(publisher);
    let (_, resume) = fixture.open(&[production, repairs]);
    assert!(resume.families.is_empty());
}

#[test]
fn bumping_a_family_s_version_prepares_that_family_again_alone() {
    let fixture = Fixture::new();
    let (production, repairs) = fast_and_slow(&fixture);
    let pages = |section: &str| {
        (0..fixture.count(section).unwrap_or(0))
            .map(|range| fixture.bytes(section, range))
            .collect::<Vec<_>>()
    };
    let kept = (
        pages("dockyard-repairs-test"),
        pages(roots::root_section("repairs", BRANCH).as_str()),
        pages(roots::ledger_section("repairs", BRANCH).as_str()),
    );
    let bumped = Family {
        version: production.version + 1,
        stamp: production.stamp ^ 1,
        ..production
    };
    let (publisher, resume) = fixture.open(&[bumped, repairs]);
    assert_eq!(resume.again, [production.id]);
    assert!(!resume.families.contains_key(&production.id));
    assert_eq!(resume.families[&repairs.id][&BRANCH].segments, 1);
    for section in [
        "production-spent-test".to_owned(),
        roots::root_section("production", BRANCH),
        roots::ledger_section("production", BRANCH),
        crate::prepared::driver::registry::coverage_section("production", BRANCH),
    ] {
        assert_eq!(fixture.count(&section), Some(0), "{section}");
    }
    assert_eq!(
        (
            pages("dockyard-repairs-test"),
            pages(roots::root_section("repairs", BRANCH).as_str()),
            pages(roots::ledger_section("repairs", BRANCH).as_str()),
        ),
        kept
    );
    let gates = fixture.gates();
    assert!(
        planned(|| pipeline::published_coverage(&gates, BRANCH, "production-select"))
            .unwrap()
            .is_none()
    );
    // The bumped family prepares from its branch's first segment, under its new stamp.
    let mut again = publisher.stage(production.id).unwrap();
    assert_eq!(prepare(&mut again, "production-spent-test", 0..1), [1]);
    let published = publisher.committed(production.id, BRANCH).unwrap();
    assert_eq!(published.identity.schema, bumped.stamp);
    assert_eq!(published.frontier.version, bumped.version);
    assert_eq!(published.publication.generation, 1);
}

#[test]
fn state_and_carry_are_found_by_their_content_wherever_compaction_moves_them() {
    let fixture = Fixture::new();
    let steady = family("steady");
    let state = vec![7_u8; 4096];
    let moved = {
        let (publisher, _) = fixture.open(&[steady]);
        let mut stage = publisher.stage(steady.id).unwrap();
        // A large page overwritten in the same stage leaves its pack mostly dead: the commit
        // empties the pack's live ranges, state and carry among them, into a new one.
        let pages = stage.pages().unwrap();
        pages
            .write_at("steady-filler", 0, &vec![0_u8; 1 << 20][..])
            .unwrap();
        pages.write_at("steady-filler", 0, &b"kept"[..]).unwrap();
        let staged = pages
            .range_id("steady-filler", 0)
            .unwrap()
            .unwrap()
            .cache_key();
        stage
            .processed(
                segment(0),
                Made::Prepared,
                Some(state.clone()),
                Some(b"carry".to_vec()),
            )
            .unwrap();
        stage.submit().unwrap();
        let committed = fixture
            .store
            .range_id(&fixture.key, "steady-filler", 0)
            .unwrap()
            .unwrap()
            .cache_key();
        staged != committed
    };
    assert!(moved, "the commit moved the stage's ranges into a new pack");
    let (publisher, resume) = fixture.open(&[steady]);
    assert!(resume.again.is_empty());
    let published = publisher.committed(steady.id, BRANCH).unwrap();
    let snapshot = fixture
        .store
        .session(&fixture.key)
        .unwrap()
        .unwrap()
        .snapshot();
    let mut view = &snapshot;
    assert_eq!(
        roots::state(&mut view, "steady", BRANCH, &published).unwrap(),
        Some(state)
    );
    assert_eq!(
        roots::carry(&mut view, "steady", BRANCH, &published).unwrap(),
        Some(b"carry".to_vec())
    );
}

#[test]
fn a_frontier_summarises_its_holes_and_its_ledger_is_read_page_by_page() {
    let fixture = Fixture::new();
    let steady = family("steady");
    let (publisher, _) = fixture.open(&[steady]);
    let mut stage = publisher.stage(steady.id).unwrap();
    // One-hour segments alternating prepared and lost: 1,300 intervals over six ledger pages.
    let mut sizes = Vec::new();
    for index in 0..1_300_u32 {
        let made = if index % 2 == 0 {
            Made::Prepared
        } else {
            Made::Lost {
                reason: "A damaged unit".into(),
                ends: false,
            }
        };
        stage
            .processed(hours(index, index, true, false), made, None, None)
            .unwrap();
        if index % 100 == 99 {
            stage.submit().unwrap();
            let frontier = publisher.committed(steady.id, BRANCH).unwrap().frontier;
            sizes.push(serde_json::to_vec(&frontier).unwrap().len());
        }
    }
    let published = publisher.committed(steady.id, BRANCH).unwrap();
    assert_eq!(published.frontier.ledger_pages, 6);
    assert_eq!(published.frontier.prepared, 650);
    assert_eq!(
        published
            .frontier
            .first_hole
            .as_ref()
            .map(|hole| (hole.first_hour, hole.last_hour)),
        Some((1001, 1001))
    );
    // The frontier does not grow with the ledger it counts.
    assert!(
        sizes.iter().max().unwrap() - sizes.iter().min().unwrap() < 160,
        "{sizes:?}"
    );
    // Decoding the gates reads no ledger page, and holds the first hole alone.
    let reads = roots::LEDGER_READS.with(std::cell::Cell::get);
    let gates = fixture.gates();
    assert_eq!(roots::LEDGER_READS.with(std::cell::Cell::get), reads);
    let holes =
        planned(|| pipeline::published_holes(&gates, BRANCH, "steady-product", None)).unwrap();
    assert_eq!(
        holes
            .iter()
            .map(|hole| (hole.first_hour, hole.last_hour))
            .collect::<Vec<_>>(),
        [(1001, 1001)]
    );
    // A detailed range read halves its way to the pages it needs.
    let snapshot = fixture
        .store
        .session(&fixture.key)
        .unwrap()
        .unwrap()
        .snapshot();
    let mut view = &snapshot;
    let reads = roots::LEDGER_READS.with(std::cell::Cell::get);
    let found = roots::ledger(&mut view, "steady", BRANCH, &published, 1_900, 1_903).unwrap();
    assert!(roots::LEDGER_READS.with(std::cell::Cell::get) - reads <= 5);
    assert_eq!(
        found
            .iter()
            .map(|interval| (interval.first_hour, interval.held))
            .collect::<Vec<_>>(),
        [
            (1_900, roots::Held::Prepared),
            (1_901, roots::Held::Lost),
            (1_902, roots::Held::Prepared),
            (1_903, roots::Held::Lost),
        ]
    );
}
