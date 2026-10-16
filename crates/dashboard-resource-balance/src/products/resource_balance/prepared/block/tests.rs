use super::test_fixtures::recording;
use super::*;
use dashboard_base::prepared::{
    prepass::extract_alone,
    series::tests::{Fixture, assert_payloads_equal},
};
use dashboard_core::config::LoadContext;
use replay::container::ReplayStream;
use std::sync::atomic::AtomicBool;

fn first_difference(left: &[u8], right: &[u8]) -> String {
    let at = left
        .iter()
        .zip(right)
        .position(|(left, right)| left != right)
        .unwrap_or(left.len().min(right.len()));
    let window = |bytes: &[u8]| {
        String::from_utf8_lossy(&bytes[at.saturating_sub(80)..(at + 80).min(bytes.len())])
            .into_owned()
    };
    format!("at byte {at}:\n  {}\n  {}", window(left), window(right))
}

/// Prepare every block of `replay` twice: observed hour by hour as
/// preparation did, and folded from extracts each made alone (last block
/// first, on its own thread). At every `commits` block end both publish and
/// must have published, and must save, the same bytes; alternate commits
/// continue from the saved state. Returns how each block folded.
fn assert_folds_continue(replay: &Arc<Replay>, commits: &[usize]) -> Vec<Folded> {
    let flag = AtomicBool::new(false);
    let source = dashboard_base::source::Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "resource-blocks".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::clone(replay),
        &flag,
    )
    .unwrap();
    let context = LoadContext { content: None };
    let alone: &Replay = replay;
    let mut outcomes: Vec<_> = std::thread::scope(|scope| {
        let extracts: Vec<_> = alone
            .index
            .iter()
            .rev()
            .map(|index| scope.spawn(move || extract_alone(alone, index, &FAMILY)))
            .collect();
        extracts
            .into_iter()
            .map(|extract| extract.join().unwrap())
            .collect()
    });
    outcomes.reverse();
    let fixtures = [Fixture::new(), Fixture::new()];
    let mut continuous = Prepared::default();
    let mut folded = Prepared::default();
    let mut stream = ReplayStream::new(TABLES).unwrap();
    let mut tracker = FrameChangeTracker::new();
    let mut how = Vec::new();
    for (position, outcome) in outcomes.into_iter().enumerate() {
        // Preparation starts every block without the change facts of the last.
        tracker.invalidate();
        stream
            .next_block_with(
                alone,
                |raw, frame| {
                    let changes = tracker.observe_updates(frame, Arc::new(raw), alone.format);
                    continuous.observe_changed(frame, &changes)
                },
                || Ok(()),
            )
            .unwrap();
        let Ok(facts) = outcome else {
            panic!("block {position} was not extracted");
        };
        how.push(
            folded
                .fold(
                    facts,
                    alone,
                    &alone.index[position],
                    &|_, result| result,
                    &mut || Ok(()),
                )
                .map_err(|failure| failure.error)
                .unwrap(),
        );
        assert_eq!(
            folded.bytes(),
            continuous.bytes(),
            "batch bytes after block {position}"
        );
        if commits.contains(&position) {
            for (prepared, fixture) in
                [(&mut continuous, &fixtures[0]), (&mut folded, &fixtures[1])]
            {
                let mut writer = fixture.writer();
                prepared
                    .flush(&mut writer, &source, &context, 1, &flag)
                    .unwrap();
                writer.commit().unwrap();
            }
            assert_payloads_equal(
                &fixtures[1].payloads(),
                &fixtures[0].payloads(),
                &format!("folded vs continuous after block {position}"),
            );
            let saved = [
                serde_json::to_vec(&continuous).unwrap(),
                serde_json::to_vec(&folded).unwrap(),
            ];
            assert!(
                saved[0] == saved[1],
                "saved state after block {position} differs {}",
                first_difference(&saved[1], &saved[0])
            );
            if position % 2 == 1 {
                continuous = serde_json::from_slice(&saved[0]).unwrap();
                folded = serde_json::from_slice(&saved[1]).unwrap();
            }
        }
    }
    how
}

#[test]
fn resource_blocks_extracted_alone_and_folded_equal_continuous_observation() {
    let replay = recording(None);
    assert_eq!(
        assert_folds_continue(&replay, &[0, 1, 2, 3, 4]),
        [Folded::Fast; 5]
    );
    // Publishing only at the end carries every block's state in memory.
    assert_eq!(assert_folds_continue(&replay, &[4]), [Folded::Fast; 5]);
}

#[test]
fn resource_fold_observes_again_when_a_keyframe_loses_a_learned_name() {
    let replay = recording(Some(114));
    let how = assert_folds_continue(&replay, &[0, 2, 3, 4]);
    assert_eq!(
        how,
        [
            Folded::Fast,
            Folded::Fast,
            Folded::Fast,
            Folded::Fallback,
            Folded::Fast
        ]
    );
    // Folding that block fast regardless publishes something else.
    UNCHECKED.with(|unchecked| unchecked.set(true));
    let fast = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assert_folds_continue(&replay, &[3, 4])
    }));
    UNCHECKED.with(|unchecked| unchecked.set(false));
    assert!(
        fast.is_err(),
        "the fast fold is wrong here and must not be taken"
    );
}
