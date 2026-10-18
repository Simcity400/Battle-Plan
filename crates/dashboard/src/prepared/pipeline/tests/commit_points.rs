//! Publication batches end where the recording puts them (`Batches`): a
//! preparation publishes the same ranges however long its blocks take, its
//! batches end at branch ends, early after a branch's 1st, 2nd, 4th, 8th and
//! 16th blocks, and at block budgets counted from each branch's first block,
//! and a family prepared on its own ends its batches where the hourly walk
//! does.
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
/// walk's committed cursors and the country groups' own committed progress;
/// and the country groups' replies on the branches asked for.
struct Prepared {
    payloads: Payloads,
    walk: Vec<usize>,
    groups: Vec<usize>,
    replies: Vec<serde_json::Value>,
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
    prepare_answering(fixture, name, hook, &[])
}

/// [`prepare`], answering the country groups on `branches` before the store
/// is cleared.
fn prepare_answering(
    fixture: &mut Fixture,
    name: &str,
    hook: Option<prepass::Hook>,
    branches: &[&str],
) -> Prepared {
    let source = fixture.source(name);
    let mut walk = Vec::new();
    let groups = Arc::new(std::sync::Mutex::new(Vec::new()));
    let published = Arc::clone(&groups);
    // Status may coalesce several family commits. Observe each actual
    // publication when checking the family's exact batch boundaries.
    independent::with_test_on_publish(
        move |family, next| {
            if family == "groups" {
                push(&mut published.lock().unwrap(), next);
            }
        },
        || {
            prepass::with_test_prepass(1, hook, || {
                run(&source, None, &AtomicBool::new(false), |accepted| {
                    push(&mut walk, accepted.next_block);
                })
            })
        },
    )
    .unwrap();
    let groups = std::mem::take(&mut *groups.lock().unwrap());
    let payloads = payloads(&source, None);
    let replies = branches
        .iter()
        .map(|branch| independent_groups::query(&source, "country-groups", branch).unwrap())
        .collect();
    clear(source);
    Prepared {
        payloads,
        walk,
        groups,
        replies,
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
/// nothing the hourly walk publishes: its batches still end after the
/// branch's 1st, 2nd and 4th blocks and at its branches' ends, where an
/// unstalled run's do.
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
    assert_eq!(
        unstalled.walk,
        [1, 2, 4, 5, 6],
        "early, then at the branches' ends"
    );
    assert_eq!(slow.walk, unstalled.walk, "the stalled run's batches");
    assert_payloads_equal(&slow.payloads, &unstalled.payloads, "stalled walk");
}

/// With a two-block budget, the walk commits after a branch's first block,
/// after every second block of it and at each branch's end; a run resumed
/// after a commit at the branch's first block counts from the branch's first
/// block, not from where it resumed (which would end batches after blocks 2,
/// 3 and 5), so its batches end where a fresh run's do.
#[test]
fn hourly_batches_end_at_block_budgets_counted_from_each_branch_start() {
    let replay = prepass_parity::resources(5, true);
    let mut fixture = Fixture::new(&replay);
    independent::with_test_hourly(|| {
        with_test_batch_blocks(2, || {
            let fresh = prepare(&mut fixture, "commit-points-budget", None);
            // Branch 1's blocks 0 to 4 end batches after its first, second
            // and fourth and at its end; branch 2's one block at its own.
            assert_eq!(fresh.walk, [1, 2, 4, 5, 6], "a fresh run's batches");

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
            assert_eq!(hourly.walk, [1, 2, 4, 6, 7], "the hourly walk's batches");
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
        assert_eq!(
            unstalled.groups,
            [1, 2, 4, 6, 7],
            "early, then at the branches' ends"
        );
        assert_eq!(slow.groups, unstalled.groups, "the stalled run's batches");
        assert_payloads_equal(&slow.payloads, &unstalled.payloads, "stalled groups");
    });
}

/// Batch ends by a branch's block ordinal: with the production budget, after
/// a branch's 1st, 2nd, 4th, 8th, 16th and 32nd blocks, then after every 32nd,
/// each branch counted from its own first block; a smaller budget keeps the
/// powers of two below it, then ends at its every multiple.
#[test]
fn batches_end_early_then_at_every_budget_counted_per_branch() {
    // A 100-block branch, then a 40-block one.
    let ends = |blocks: usize, ordinals: Vec<usize>| -> Vec<usize> {
        let batches = Batches { ordinals, blocks };
        (0..batches.ordinals.len())
            .filter(|&block| batches.full(block))
            .collect()
    };
    assert_eq!(batch_blocks(), BATCH_BLOCKS, "no test budget");
    assert_eq!(
        ends(batch_blocks(), (0..100).chain(0..40).collect()),
        [0, 1, 3, 7, 15, 31, 63, 95, 100, 101, 103, 107, 115, 131],
        "the production budget"
    );
    assert_eq!(
        ends(3, (0..10).collect()),
        [0, 1, 2, 5, 8],
        "a three-block budget"
    );
    assert_eq!(
        ends(1, (0..4).collect()),
        [0, 1, 2, 3],
        "a one-block budget"
    );
}

/// `blocks` blocks of four hours of branch 1's country groups (hours 100 on),
/// then branch 2's one block.
fn long_groups(blocks: usize) -> Replay {
    let frames = |block: usize| -> Vec<DecodedFrame> {
        let first = 100 + 4 * i32::try_from(block).unwrap();
        (first..first + 4)
            .map(|hour| independent_groups::groups_frame(1, hour))
            .collect()
    };
    let mut replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames(0))]).unwrap();
    for block in 1..blocks {
        replay = replay
            .append(&[continuation_records(&replay, 1, frames(block))])
            .unwrap();
    }
    replay
        .append(&[records(
            2,
            (20..=23)
                .map(|hour| independent_groups::groups_frame(2, hour))
                .collect(),
        )])
        .unwrap()
}

/// Country groups prepared on their own over a 40-block branch publish after
/// its first block, then after its 2nd, 4th, 8th, 16th and 32nd blocks and at
/// its end, where the hourly walk does in either mode, with the pages hourly
/// preparation publishes there; and their replies are those of an hourly
/// walk whose batches end elsewhere (every third block).
#[test]
fn own_batches_publish_after_a_branch_first_block_whatever_their_commit_points() {
    independent::with_test_only(&["groups"], || {
        let replay = long_groups(40);
        let mut fixture = Fixture::new(&replay);
        let name = "commit-points-early";
        let own = prepare_answering(&mut fixture, name, None, &["1", "2"]);
        assert_eq!(
            own.groups,
            [1, 2, 4, 8, 16, 32, 40, 41],
            "the groups' own batches"
        );
        assert_eq!(own.walk, own.groups, "the walk's batches beside them");
        let hourly = independent::with_test_hourly(|| prepare(&mut fixture, name, None));
        assert_eq!(hourly.walk, own.groups, "the hourly walk's batches");
        independent_groups::assert_same(&own.payloads, &hourly.payloads, "early batches", true);
        let elsewhere = with_test_batch_blocks(3, || {
            independent::with_test_hourly(|| {
                prepare_answering(&mut fixture, name, None, &["1", "2"])
            })
        });
        assert_eq!(
            elsewhere.walk,
            [
                1, 2, 3, 6, 9, 12, 15, 18, 21, 24, 27, 30, 33, 36, 39, 40, 41
            ],
            "the hourly walk's batches with a three-block budget"
        );
        assert!(
            own.replies.iter().all(|reply| !reply.is_null()),
            "both branches answer"
        );
        assert_eq!(own.replies, elsewhere.replies, "batches ending elsewhere");
    });
}
