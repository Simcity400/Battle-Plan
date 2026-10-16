//! Publication batches end where the recording puts them (`Batches`): a
//! preparation publishes the same ranges however long its blocks take, its
//! batches end at branch ends and block budgets counted from each branch's
//! first block, and a family prepared on its own ends its batches where the
//! hourly walk does.
use super::family_lanes::{assert_payloads_equal, payloads};
use super::independent_groups::{self, Payloads};
use super::prepass_parity;
use super::*;
use crate::prepared::prepass;

/// A loaded machine's block: longer than ten seconds of work.
const STALL: Duration = Duration::from_millis(10_500);

/// A pre-pass hook that stalls one extract once: `family`'s first hour in
/// `hours`, as a loaded machine would. `stalled` records that it did.
fn stall(
    family: &'static str,
    hours: std::ops::RangeInclusive<i32>,
    stalled: Arc<AtomicBool>,
) -> prepass::Hook {
    Arc::new(move |name, frame| {
        if name == family && hours.contains(&frame.hour) && !stalled.swap(true, Ordering::SeqCst) {
            std::thread::sleep(STALL);
        }
        prepass::Fault::Continue
    })
}

/// What one preparation published, and where its batches ended: the hourly
/// walk's committed cursors and the country groups' own committed progress.
struct Prepared {
    payloads: Payloads,
    walk: Vec<usize>,
    groups: Vec<usize>,
}

/// Note a committed position: each once, in order, past the start.
fn push(points: &mut Vec<usize>, next: usize) {
    if next > 0 && points.last() != Some(&next) {
        points.push(next);
    }
}

/// Prepare `fixture`'s recording from nothing with one pre-pass worker and
/// `hook`, then clear its store.
fn prepare(fixture: &mut Fixture, name: &str, hook: Option<prepass::Hook>) -> Prepared {
    let source = fixture.source(name);
    let (mut walk, mut groups) = (Vec::new(), Vec::new());
    prepass::with_test_prepass(1, hook, || {
        run(&source, None, &AtomicBool::new(false), |accepted| {
            push(&mut walk, accepted.next_block);
            // Status rows count a family prepared on its own by its
            // committed progress.
            if let Some(row) = accepted
                .sections
                .iter()
                .find(|row| row.id == "country-groups")
            {
                push(&mut groups, row.done.parse().unwrap());
            }
        })
    })
    .unwrap();
    let payloads = payloads(&source, None);
    clear(source);
    Prepared {
        payloads,
        walk,
        groups,
    }
}

fn clear(source: Arc<Source>) {
    super::super::super::files(&source, None)
        .unwrap()
        .0
        .clear()
        .unwrap();
}

/// A block that takes longer than ten seconds (its extract stalls) changes
/// nothing the hourly walk publishes: its batches still end only at its
/// branches' ends, where an unstalled run's do.
#[test]
fn hourly_batches_end_where_the_recording_puts_them_however_long_a_block_takes() {
    // Branch 1's five blocks (hours 100 to 121), then branch 2's one.
    let replay = prepass_parity::resources(5, true);
    let mut fixture = Fixture::new(&replay);
    let unstalled = prepare(&mut fixture, "commit-points-walk", None);
    let stalled = Arc::new(AtomicBool::new(false));
    // Inside branch 1's second to fourth blocks, which its batch goes on from.
    let hook = stall("resources", 104..=117, Arc::clone(&stalled));
    let slow = prepare(&mut fixture, "commit-points-walk", Some(hook));
    assert!(
        stalled.load(Ordering::SeqCst),
        "the resources extract stalled inside the branch"
    );
    assert_eq!(unstalled.walk, [5, 6], "at the branches' ends");
    assert_eq!(slow.walk, unstalled.walk, "the stalled run's batches");
    assert_payloads_equal(&slow.payloads, &unstalled.payloads, "stalled walk");
}

/// With a two-block budget, the walk commits after every second block of a
/// branch and at each branch's end; a run resumed after a commit at the
/// branch's first block counts from the branch's first block, not from where
/// it resumed, so its batches end where a fresh run's do.
#[test]
fn hourly_batches_end_at_block_budgets_counted_from_each_branch_start() {
    let replay = prepass_parity::resources(5, true);
    let mut fixture = Fixture::new(&replay);
    independent::with_test_hourly(|| {
        with_test_batch_blocks(2, || {
            let fresh = prepare(&mut fixture, "commit-points-budget", None);
            // Branch 1's blocks 0 to 4 end batches after its second and
            // fourth and at its end; branch 2's one block at its own.
            assert_eq!(fresh.walk, [2, 4, 5, 6], "a fresh run's batches");

            let source = fixture.source("commit-points-budget");
            let flag = AtomicBool::new(false);
            // A measured run commits the first block alone.
            run_measured(&source, None, &flag, true, Duration::ZERO, |_| {}).unwrap();
            let mut resumed = Vec::new();
            run(&source, None, &flag, |accepted| {
                push(&mut resumed, accepted.next_block);
            })
            .unwrap();
            assert_eq!(
                resumed,
                [1, 2, 4, 5, 6],
                "resumed at block 1, its batches end where a fresh run's do"
            );
            clear(source);
        });
    });
}

/// Country groups prepared on their own end their batches where the hourly
/// walk ends its own inside a branch (a two-block budget), with the pages and
/// saved state hourly preparation publishes there; and a stalled extract,
/// longer than ten seconds, changes nothing they publish.
#[test]
fn own_batches_end_where_the_hourly_walk_does_however_long_a_block_takes() {
    // The groups alone prepared on their own; every other family hourly.
    independent::with_test_only(&["groups"], || {
        // Branch 1's six blocks (hours 100 to 125), then branch 2's one.
        let replay = independent_groups::scenario(6, true);
        let mut fixture = Fixture::new(&replay);
        with_test_batch_blocks(2, || {
            let hourly =
                independent::with_test_hourly(|| prepare(&mut fixture, "commit-points-own", None));
            let own = prepare(&mut fixture, "commit-points-own", None);
            assert_eq!(hourly.walk, [2, 4, 6, 7], "the hourly walk's batches");
            assert_eq!(own.groups, hourly.walk, "the groups' own batches");
            assert_eq!(own.walk, hourly.walk, "the walk's batches beside them");
            independent_groups::assert_same(
                &own.payloads,
                &hourly.payloads,
                "a two-block budget",
                true,
            );
        });
        let unstalled = prepare(&mut fixture, "commit-points-own", None);
        let stalled = Arc::new(AtomicBool::new(false));
        // Inside branch 1's second and third blocks (B turns human at 104, C
        // leaves its faction at 106 and joins another at 109).
        let hook = stall("groups", 104..=111, Arc::clone(&stalled));
        let slow = prepare(&mut fixture, "commit-points-own", Some(hook));
        assert!(
            stalled.load(Ordering::SeqCst),
            "the groups' extract stalled inside the branch"
        );
        assert_eq!(unstalled.groups, [6, 7], "at the branches' ends");
        assert_eq!(slow.groups, unstalled.groups, "the stalled run's batches");
        assert_payloads_equal(&slow.payloads, &unstalled.payloads, "stalled groups");
    });
}
