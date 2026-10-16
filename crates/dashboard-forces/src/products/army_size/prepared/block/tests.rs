use super::test_fixtures::recording;
use super::*;
use dashboard_base::prepared::{
    prepass::extract_alone,
    series::tests::{Fixture, assert_payloads_equal},
};
use replay::container::ReplayStream;
use std::sync::atomic::AtomicBool;

/// Observing every block hour by hour, and folding each block's extract made
/// alone (last block first, each on its own thread), publish and save the
/// same bytes at every `commits` block end; alternate commits continue from
/// the saved state.
fn assert_folds_continue(replay: &Arc<Replay>, commits: &[usize]) -> Vec<Folded> {
    let flag = AtomicBool::new(false);
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
        tracker.invalidate();
        stream
            .next_block_with(
                alone,
                |raw, frame| {
                    let changes = tracker.observe_updates(frame, Arc::new(raw), alone.format);
                    continuous.observe(frame, alone, &changes)
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
                prepared.flush(&mut writer, 1, &flag).unwrap();
                writer.commit().unwrap();
            }
            assert_payloads_equal(
                &fixtures[1].payloads(),
                &fixtures[0].payloads(),
                &format!("folded vs continuous after block {position}"),
            );
            let saved = [
                serde_json::to_value(&continuous).unwrap(),
                serde_json::to_value(&folded).unwrap(),
            ];
            assert_eq!(saved[1], saved[0], "saved state after block {position}");
            if position % 2 == 1 {
                continuous = serde_json::from_value(saved[0].clone()).unwrap();
                folded = serde_json::from_value(saved[1].clone()).unwrap();
            }
        }
    }
    // The scenario publishes several countries' runs and compositions.
    assert_eq!(folded.tags.len(), 3);
    assert!(folded.mix_ids.values().any(|&version| version > 1));
    assert!(
        folded
            .scan
            .tracks
            .values()
            .any(|track| track.runs.iter().any(|run| run.first > 100))
    );
    how
}

#[test]
fn army_blocks_extracted_alone_and_folded_equal_continuous_observation() {
    let replay = recording();
    assert_eq!(
        assert_folds_continue(&replay, &[0, 1, 2, 3, 4]),
        [Folded::Fast; 5]
    );
    assert_eq!(assert_folds_continue(&replay, &[4]), [Folded::Fast; 5]);
}

/// What a block's extract reports when, after the scenario's first hour, one
/// country's composition of `parts` parts changes at each of `hours` hours:
/// the scan keeps each composition run and the extract records each
/// composition's serialized parts for its fold, as the observer does.
fn reported_with_compositions(parts: usize, hours: i32) -> u64 {
    let replay = recording();
    let first = super::test_fixtures::scenario(100);
    let mut extract = BlockExtract {
        prepared: Box::default(),
        recording: Recording::default(),
        first: None,
    };
    extract
        .observe(&replay, &first, &FrameChanges::alone(&first, replay.format))
        .unwrap();
    let prepared = &mut extract.prepared;
    for hour in 101..101 + hours {
        let mix = RawMixRun {
            first: hour,
            last: hour,
            parts: [
                Some(
                    (0..parts)
                        .map(|part| (format!("part_{part:04}"), hour as i64 + part as i64))
                        .collect(),
                ),
                None,
            ],
        };
        let publication = Publication {
            tag: "origin".into(),
            key: 0,
            hour,
            parts: serde_json::to_vec(&mix.parts).unwrap(),
            starts: false,
        };
        prepared
            .recorded
            .as_mut()
            .expect("the extract records after its first hour")
            .push(publication);
        prepared
            .scan
            .tracks
            .entry("origin".into())
            .or_default()
            .mix
            .push(mix);
    }
    extract.bytes()
}

/// A block of K-part compositions at each of H hours holds every part of
/// every composition twice (its run's map and its serialized publication),
/// so the extract reports at least that, growing with K×H.
#[test]
fn extract_bytes_grow_with_parts_times_hours() {
    // A part's map entry and name, and its serialized name, at least.
    let part = (size_of::<(String, i64)>() + 2 * "part_0000".len()) as u64;
    let (small, wide, long, both) = (
        reported_with_compositions(8, 4),
        reported_with_compositions(256, 4),
        reported_with_compositions(8, 64),
        reported_with_compositions(256, 64),
    );
    assert!(wide - small >= (256 - 8) * 4 * part, "{small} {wide}");
    assert!(long - small >= 8 * (64 - 4) * part, "{small} {long}");
    assert!(both - wide >= 256 * (64 - 4) * part, "{wide} {both}");
    assert!(both - long >= (256 - 8) * 64 * part, "{long} {both}");
}
