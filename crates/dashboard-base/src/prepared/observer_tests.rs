//! Shared checks over real observer callbacks, not replacement scans/replies.
use super::frame_changes::{FrameChangeTracker, FrameChanges};
use super::series::tests::{Fixture, assert_payloads_equal};
use replay_data::format::FormatVersion;
use replay_frame::frame::DecodedFrame;
use serde::Serialize;
use std::io;

/// Cumulative counters incremented where a loader, row visit or emission occurs.
/// Clock/frontier updates are deliberately separate from native/history work.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Work {
    pub content_loads: usize,
    pub row_visits: usize,
    pub history_visits: usize,
    pub emitted: usize,
    pub interpreter_calls: usize,
    /// Current templated faction name revisions driven by a dictionary change/read.
    pub dictionary_interpretations: usize,
}
impl Work {
    fn since(self, before: Self) -> Self {
        Self {
            content_loads: self.content_loads - before.content_loads,
            row_visits: self.row_visits - before.row_visits,
            history_visits: self.history_visits - before.history_visits,
            emitted: self.emitted - before.emitted,
            interpreter_calls: self.interpreter_calls - before.interpreter_calls,
            dictionary_interpretations: self.dictionary_interpretations
                - before.dictionary_interpretations,
        }
    }
}

/// Feed shared table views at adjacent hours after the family's real configure
/// and first accepted observation. Each callback observes and snapshots its
/// counters. Publication/reload/oracle assertions stay with the real fixture.
pub fn unchanged_frames(
    configured: Work,
    first: &DecodedFrame,
    hours: i32,
    format: FormatVersion,
    mut observe: impl FnMut(&DecodedFrame, &FrameChanges) -> io::Result<Work>,
) -> io::Result<Work> {
    assert!(hours > 0);
    let mut frame = first.clone(); // SharedViews, not owned native-row snapshots.
    let mut work = configured;
    let mut tracker = FrameChangeTracker::new();
    tracker.observe(first, format);
    for offset in 1..=hours {
        frame.hour = first.hour.checked_add(offset).expect("fixture hour range");
        let changes = tracker.observe(&frame, format);
        work = observe(&frame, &changes)?;
        assert_eq!(
            work.content_loads, configured.content_loads,
            "content loaded after configuration at hour {}",
            frame.hour
        );
    }
    Ok(work.since(configured))
}

/// Use the same real fixture at N/10N unchanged hours or H/10H closed history.
/// Return measured work so families can retain genuine clock/provenance work
/// and assert flat row/history/emission counts for their unchanged inputs.
pub fn scaled_work(
    size: usize,
    mut measure: impl FnMut(usize) -> io::Result<Work>,
) -> io::Result<[Work; 2]> {
    assert!(size > 0);
    Ok([
        measure(size)?,
        measure(size.checked_mul(10).expect("fixture scale"))?,
    ])
}

/// Clock/session interpretation may grow; native inputs, retained history and
/// unchanged emissions must not. Families select their uncharged fixture.
pub fn assert_flat(work: [Work; 2]) {
    let inputs = |work: Work| {
        (
            work.content_loads,
            work.row_visits,
            work.history_visits,
            work.emitted,
        )
    };
    assert_eq!(
        inputs(work[0]),
        inputs(work[1]),
        "work grew with hours or retained history"
    );
}

/// Compare real prepared and oracle reply types through their public JSON shape,
/// preserving exact integers/strings, array order, duplicates and diagnostics.
pub fn assert_reply_parity(prepared: &impl Serialize, oracle: &impl Serialize) -> io::Result<()> {
    assert_eq!(
        serde_json::to_value(prepared)?,
        serde_json::to_value(oracle)?
    );
    Ok(())
}

/// Observes `blocks` with two copies of one family in lockstep, as preparation
/// does: frame changes are invalidated at every block start and every block
/// ends at a commit. After each commit the first copy continues in memory and
/// the second is restored from what it published. Every published payload,
/// including each saved continuation, must stay byte-identical.
pub fn assert_restore_continues<T>(
    format: FormatVersion,
    blocks: &[&[DecodedFrame]],
    mut new: impl FnMut() -> T,
    mut observe: impl FnMut(&mut T, &DecodedFrame, &FrameChanges, &Fixture),
    mut publish: impl FnMut(&mut T, &mut store::Writer, usize),
    mut restore: impl FnMut(&Fixture) -> T,
) {
    let fixtures = [Fixture::new(), Fixture::new()];
    let mut observers = [new(), new()];
    let mut tracker = FrameChangeTracker::new();
    for (ordinal, block) in blocks.iter().enumerate() {
        tracker.invalidate();
        for frame in *block {
            let changes = tracker.observe(frame, format);
            for (observer, fixture) in observers.iter_mut().zip(&fixtures) {
                observe(observer, frame, &changes, fixture);
            }
        }
        for (observer, fixture) in observers.iter_mut().zip(&fixtures) {
            let mut writer = fixture.writer();
            publish(observer, &mut writer, ordinal);
            writer.commit().unwrap();
        }
        assert_payloads_equal(
            &fixtures[1].payloads(),
            &fixtures[0].payloads(),
            &format!("restored vs continuous after commit {ordinal}"),
        );
        observers[1] = restore(&fixtures[1]);
    }
}
