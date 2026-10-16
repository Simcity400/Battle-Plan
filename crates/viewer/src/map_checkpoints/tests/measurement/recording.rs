//! Test-only comparison: canonical recording keyframes and selected prefix reads.
use super::*;
use replay::{container::Parallel, frame::Frame};
use std::{collections::BTreeMap, io};

pub(super) struct Recording<'a> {
    fixture: &'a Fixture,
    progress: &'a Progress,
    replay: &'a Replay,
    selected: &'a [TableId],
    workers: usize,
    end_block: usize,
    decoder: FrameDecoder,
    block: Option<(usize, Hour, Vec<Frame>)>,
    cursor: Position,
    pub(super) block_reads: u64,
    pub(super) decoded_frames: u64,
    pub(super) applied: u64,
}

impl<'a> Recording<'a> {
    pub(super) fn new(
        fixture: &'a Fixture,
        progress: &'a Progress,
        replay: &'a Replay,
        selected: &'a [TableId],
        workers: usize,
    ) -> Self {
        Self {
            fixture,
            progress,
            replay,
            selected,
            workers,
            end_block: progress
                .branches
                .values()
                .filter_map(|branch| branch.last_block)
                .max()
                .map_or(replay.index.len(), |block| block + 1),
            decoder: FrameDecoder::for_tables(selected),
            block: None,
            cursor: Position {
                block: 0,
                next_frame: 0,
            },
            block_reads: 0,
            decoded_frames: 0,
            applied: 0,
        }
    }

    fn seek_keyframe(&mut self, branch: BranchId, hour: Hour) -> io::Result<Current> {
        self.block_reads = 0;
        self.decoded_frames = 0;
        self.applied = 0;
        self.block = None;
        self.decoder = FrameDecoder::for_tables(self.selected);
        // Test-only metadata lookup. Production's indexed replay-block lookup
        // is a separate concern if this representation wins the measurements.
        let Some(block) = self
            .replay
            .index
            .iter()
            .take(self.end_block)
            .position(|index| {
                index.branch == branch && index.first_hour <= hour && hour <= index.last_hour
            })
        else {
            return self.gap(branch, hour);
        };
        self.cursor = Position {
            block,
            next_frame: 0,
        };
        self.advance_through(branch, hour, false)
    }

    pub(super) fn advance(&mut self, branch: BranchId, hour: Hour) -> io::Result<Current> {
        self.advance_through(branch, hour, true)
    }

    fn advance_through(
        &mut self,
        branch: BranchId,
        hour: Hour,
        preload: bool,
    ) -> io::Result<Current> {
        self.block_reads = 0;
        self.decoded_frames = 0;
        self.applied = 0;
        if self
            .decoder
            .current()
            .is_some_and(|frame| frame.branch == branch && frame.hour == hour)
        {
            return Ok(Current::Frame(Arc::new(
                self.decoder.current().unwrap().clone(),
            )));
        }
        while let Some(index) = self
            .replay
            .index
            .get(self.cursor.block)
            .filter(|_| self.cursor.block < self.end_block)
        {
            if index.branch != branch {
                self.cursor.block += 1;
                self.cursor.next_frame = 0;
                continue;
            }
            // Cold seeks stop column decoding at the target. Step trials load
            // the complete block once and report that stall separately.
            let through = if preload {
                index.last_hour
            } else {
                hour.min(index.last_hour)
            };
            if self
                .block
                .as_ref()
                .is_none_or(|(block, end, _)| *block != self.cursor.block || *end < through)
            {
                // Keep just one parsed source block, as ordinary playback does.
                let parallel = Parallel {
                    workers: self.workers,
                    on_worker: &|| {},
                };
                let frames = self.replay.read_block_selected_through(
                    index,
                    Some(self.selected),
                    through,
                    (self.workers > 1).then_some(&parallel),
                )?;
                self.decoded_frames += frames.len() as u64;
                self.block = Some((self.cursor.block, through, frames));
                self.block_reads += 1;
            }
            let (_, end, frames) = self.block.as_ref().unwrap();
            while let Some(frame) = frames.get(self.cursor.next_frame) {
                if frame.hour > hour {
                    return self.gap(branch, hour);
                }
                let decoded = self.decoder.advance(frame)?;
                self.applied += 1;
                self.cursor.next_frame += 1;
                if decoded.hour == hour {
                    return Ok(Current::Frame(Arc::new(decoded.clone())));
                }
            }
            if *end < index.last_hour {
                return self.gap(branch, hour);
            }
            self.cursor.block += 1;
            self.cursor.next_frame = 0;
            self.block = None;
        }
        self.gap(branch, hour)
    }

    fn gap(&self, branch: BranchId, hour: Hour) -> io::Result<Current> {
        Ok(Current::Gap {
            branch,
            hour,
            reason: dashboard::prepared_map::gap_reason(
                &self.fixture.store,
                &self.fixture.key,
                self.progress,
                branch,
                hour,
            )?,
        })
    }
}

pub(super) fn measure(
    fixture: &Fixture,
    progress: &Progress,
    replay: &Arc<Replay>,
    selected: &[TableId],
    targets: &BTreeSet<(BranchId, Hour)>,
    evidence: &std::path::Path,
    blocks: usize,
) -> serde_json::Value {
    let maximum = std::thread::available_parallelism()
        .map_or(1, |count| count.get() / 2)
        .max(1);
    let mut reports = Vec::new();
    let mut seeks = BufWriter::new(
        File::create(evidence.join(format!("recording-seeks-{}.csv", progress.stride))).unwrap(),
    );
    let mut steps = BufWriter::new(
        File::create(evidence.join(format!("recording-steps-{}.csv", progress.stride))).unwrap(),
    );
    writeln!(
        seeks,
        "stride,workers,selection,branch,hour,seek_us,applied_frames,block_reads,source_decoded_frames"
    )
    .unwrap();
    writeln!(
        steps,
        "stride,workers,branch,hour,step_us,applied_frames,block_reads,source_decoded_frames"
    )
    .unwrap();
    // The production reader was compared with the ordinary recording oracle above.
    // Reuse those exact selected outputs instead of repeating slow oracle reads.
    let mut reference = fixture.reader(progress.clone(), Arc::clone(replay), selected);
    for workers in BTreeSet::from([1, maximum]) {
        let mut keyframe_cold = Vec::new();
        let mut max_decoded = 0;
        let mut max_keyframe_applied = 0;
        for &(branch, hour) in targets {
            let mut reader = Recording::new(fixture, progress, replay, selected, workers);
            let started = Instant::now();
            let actual = reader.seek_keyframe(branch, hour).unwrap();
            let elapsed = micros(started);
            let expected = reference.seek(branch, hour).unwrap().unwrap();
            match (&actual, &expected) {
                (Current::Frame(actual), Current::Frame(expected)) => {
                    equal_frames(actual, expected, selected)
                }
                _ => assert_eq!(actual, expected),
            }
            max_decoded = max_decoded.max(reader.decoded_frames);
            max_keyframe_applied = max_keyframe_applied.max(reader.applied);
            keyframe_cold.push(elapsed);
            writeln!(
                seeks,
                "{},{workers},recording-keyframe,{branch},{hour},{elapsed},{},{},{}",
                progress.stride, reader.applied, reader.block_reads, reader.decoded_frames
            )
            .unwrap();
            seeks.flush().unwrap();
            eprintln!(
                "map source workers {workers}: measured seek at branch {branch}, hour {hour}"
            );
        }
        let mut reader = Recording::new(fixture, progress, replay, selected, workers);
        let mut oracle = FrameDecoder::for_tables(selected);
        let mut previous: Option<(BranchId, Hour)> = None;
        let mut steady = Vec::new();
        let mut boundary = Vec::new();
        for index in replay.index.iter().take(blocks) {
            for frame in replay.read_block_selected(index, selected).unwrap() {
                let expected = oracle.advance(&frame).unwrap();
                if previous.is_some_and(|(branch, hour)| {
                    branch == frame.branch && hour.checked_add(1) == Some(frame.hour)
                }) {
                    let started = Instant::now();
                    let actual = reader.advance(frame.branch, frame.hour).unwrap();
                    let elapsed = micros(started);
                    let Current::Frame(actual) = actual else {
                        panic!("recorded step became a gap")
                    };
                    equal_frames(&actual, expected, selected);
                    assert_eq!(reader.applied, 1);
                    assert!(reader.block_reads <= 1);
                    if reader.block_reads == 0 {
                        steady.push(elapsed);
                    } else {
                        boundary.push(elapsed);
                    }
                    writeln!(
                        steps,
                        "{},{workers},{},{},{elapsed},{},{},{}",
                        progress.stride,
                        frame.branch,
                        frame.hour,
                        reader.applied,
                        reader.block_reads,
                        reader.decoded_frames
                    )
                    .unwrap();
                } else {
                    let actual = reader.seek_keyframe(frame.branch, frame.hour).unwrap();
                    let Current::Frame(actual) = actual else {
                        panic!("recorded hour became a gap")
                    };
                    equal_frames(&actual, expected, selected);
                }
                previous = Some((frame.branch, frame.hour));
            }
            steps.flush().unwrap();
            eprintln!(
                "map source stride {}, workers {workers}: measured steps through hour {}",
                progress.stride, index.last_hour
            );
        }
        reports.push(serde_json::json!({"workers": workers, "decodeSelection": "selected",
            "maxSeekSourceDecodedFrames": max_decoded,
            "seekRecordingKeyframe": timings(&keyframe_cold), "maxRecordingKeyframeAppliedFrames": max_keyframe_applied,
            "stepRetainedBlock": timings(&steady), "stepBlockLoad": timings(&boundary), "parityDifferences": 0}));
    }
    serde_json::json!({"trials": reports, "scope": "test-only canonical recording keyframe plus selected source prefix through target hour; one parsed block; sequential and parallel selected reads; steps load through the block's last hour; no read-ahead; block-load stalls including extension after seek reported separately"})
}

#[test]
fn recording_prefix_does_not_decode_a_source_block_after_its_accepted_boundary() {
    let replay = status_replay(&[100, 101, 821, 823]);
    assert_eq!(replay.index.len(), 2);
    let fixture = Fixture::new(&replay);
    let progress = Progress {
        branches: BTreeMap::from([(
            1,
            dashboard::prepared_map::BranchProgress {
                last_block: Some(0),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let mut reader = Recording::new(&fixture, &progress, &replay, SELECTED, 2);
    let actual = reader.seek_keyframe(1, 101).unwrap();
    equal_current(
        &actual,
        &replay.seek_selected(1, 101, Some(SELECTED)).unwrap(),
        SELECTED,
    );
    assert_eq!(reader.decoded_frames, 2);
    assert!(matches!(
        reader.seek_keyframe(1, 821).unwrap(),
        Current::Gap { hour: 821, .. }
    ));
    assert_eq!(reader.block_reads, 0);
    assert_eq!(reader.decoded_frames, 0);
}

#[test]
fn recording_keyframe_seek_needs_no_prepared_checkpoint_and_keeps_holes() {
    let replay = status_replay(&[100, 101, 103, 111, 112, 113]);
    let fixture = Fixture::new(&replay);
    // No prepared ranges exist. The source keyframe supplies all selected rows.
    let progress = Progress::default();
    let mut reader = Recording::new(&fixture, &progress, &replay, SELECTED, 2);
    for hour in [103, 111, 112, 113] {
        let actual = reader.seek_keyframe(1, hour).unwrap();
        equal_current(
            &actual,
            &replay.seek_selected(1, hour, Some(SELECTED)).unwrap(),
            SELECTED,
        );
        assert_eq!(reader.block_reads, 1);
        assert_eq!(reader.applied, reader.decoded_frames);
    }
    let actual = reader.seek_keyframe(1, 110).unwrap();
    equal_current(
        &actual,
        &replay.seek_selected(1, 110, Some(SELECTED)).unwrap(),
        SELECTED,
    );
    assert_eq!(reader.decoded_frames, 3);
    assert_eq!(reader.applied, 3);
}

#[test]
fn source_prefix_extends_for_steps_without_skipping_native_states() {
    let replay = status_replay(&(100..=120).collect::<Vec<_>>());
    let fixture = Fixture::new(&replay);
    let progress = prepare(&fixture, &replay, 8, SELECTED);
    for workers in [1, 2] {
        let mut reader = Recording::new(&fixture, &progress, &replay, SELECTED, workers);
        let actual = reader.seek_keyframe(1, 110).unwrap();
        equal_current(
            &actual,
            &replay.seek_selected(1, 110, Some(SELECTED)).unwrap(),
            SELECTED,
        );
        assert_eq!(
            reader.decoded_frames, 11,
            "seek decodes only its source prefix"
        );
        assert_eq!(reader.block_reads, 1);
        for hour in [111, 112, 113] {
            let actual = reader.advance(1, hour).unwrap();
            equal_current(
                &actual,
                &replay.seek_selected(1, hour, Some(SELECTED)).unwrap(),
                SELECTED,
            );
            assert_eq!(reader.applied, 1);
            assert_eq!(reader.block_reads, u64::from(hour == 111));
            assert_eq!(reader.decoded_frames, if hour == 111 { 21 } else { 0 });
        }
    }
}
