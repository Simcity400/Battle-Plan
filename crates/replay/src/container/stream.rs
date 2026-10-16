//! Selected preparation with an accepted block cursor and decoder accumulator.

use super::*;
use std::sync::{Arc, mpsc};

#[cfg(test)]
type ReconstructedHook = Arc<dyn Fn(&DecodedFrame) + Send + Sync>;

/// Accepted block boundary. Appends preserve offsets and append independent blocks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReplayCursor {
    pub next_block: usize,
    pub previous: Option<BlockIndex>,
}

/// Preparation keeps one selected decoder accumulator and an accepted block cursor.
pub struct ReplayStream {
    cursor: ReplayCursor,
    tables: Vec<TableId>,
    decoder: FrameDecoder,
    #[cfg(test)]
    on_reconstructed: Option<ReconstructedHook>,
}

impl ReplayStream {
    pub fn new(tables: &[TableId]) -> io::Result<Self> {
        let mut selected = engine::allocation::collect_vec(tables.iter().copied())?;
        if !selected.contains(&TableId::Campaign) {
            engine::allocation::reserve(&mut selected, 1)?;
            selected.push(TableId::Campaign);
        }
        Ok(Self {
            cursor: ReplayCursor::default(),
            decoder: FrameDecoder::for_tables(&selected),
            tables: selected,
            #[cfg(test)]
            on_reconstructed: None,
        })
    }

    pub fn resume(
        tables: &[TableId],
        cursor: ReplayCursor,
        accumulator: Option<DecodedFrame>,
    ) -> io::Result<Self> {
        let mut stream = Self::new(tables)?;
        if let Some(frame) = accumulator {
            stream.decoder = FrameDecoder::resume_for_tables(&stream.tables, frame)?;
        }
        stream.cursor = cursor;
        Ok(stream)
    }

    pub fn cursor(&self) -> &ReplayCursor {
        &self.cursor
    }
    pub fn accumulator(&self) -> Option<&DecodedFrame> {
        self.decoder.current()
    }

    /// Returns false at the current tail; the same stream can visit an appended revision.
    /// Visitor side effects are caller-owned; only successful blocks move accepted state.
    pub fn next_block(
        &mut self,
        replay: &Replay,
        mut visit: impl FnMut(&DecodedFrame) -> io::Result<()>,
    ) -> io::Result<bool> {
        self.next_block_with(replay, |_, decoded| visit(decoded), || Ok(()))
    }

    /// Check cancellation during admission and reconstruction, once per payload.
    pub fn next_block_with(
        &mut self,
        replay: &Replay,
        mut visit: impl FnMut(Frame, &DecodedFrame) -> io::Result<()>,
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<bool> {
        check()?;
        self.check_prefix(replay)?;
        let Some(index) = replay.index.get(self.cursor.next_block) else {
            return Ok(false);
        };
        // Grouped selected reads return only after the native reader's selected
        // decoding and first-read section verification succeed. Keep those
        // admitted frames for one reconstruction pass. Interleaved blocks keep
        // their bounded payload streaming and admission pass.
        let grouped = if replay.is_file_backed() {
            replay.held_grouped(index)?
        } else {
            replay.grouped_block(index)?.is_some()
        };
        let frames = if grouped {
            Some(replay.read_block_selected(index, &self.tables)?)
        } else {
            replay.visit_block(index, Some(&self.tables), |_| {
                check()?;
                Ok(true)
            })?;
            None
        };
        check()?;
        let mut decoder = self
            .decoder
            .current()
            .cloned()
            .map(|frame| FrameDecoder::resume_for_tables(&self.tables, frame))
            .transpose()?
            .unwrap_or_else(|| FrameDecoder::for_tables(&self.tables));
        let mut reconstruct = |frame: Frame| {
            let decoded = decoder.advance(&frame)?;
            validate_campaign(decoded, &replay.campaign_id)?;
            visit(frame, decoded)?;
            Ok(true)
        };
        match frames {
            Some(frames) => {
                for frame in frames {
                    check()?;
                    reconstruct(frame)?;
                }
            }
            None => replay.visit_block(index, Some(&self.tables), |frame| {
                check()?;
                reconstruct(frame)
            })?,
        }
        self.decoder = decoder;
        self.cursor.next_block += 1;
        self.cursor.previous = Some(index.clone());
        Ok(true)
    }

    /// Reconstruct on one worker and observe in order through one queued frame.
    /// Decoder state and cursor move only after the whole block succeeds.
    pub fn next_block_parallel_with(
        &mut self,
        replay: &Arc<Replay>,
        mut visit: impl FnMut(Frame, &DecodedFrame) -> io::Result<()>,
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<bool> {
        if std::thread::available_parallelism().map_or(1, usize::from) == 1 {
            return self.next_block_with(replay, visit, check);
        }
        check()?;
        self.check_prefix(replay)?;
        let Some(index) = replay.index.get(self.cursor.next_block) else {
            return Ok(false);
        };
        let mut decoder = self
            .decoder
            .current()
            .cloned()
            .map(|frame| FrameDecoder::resume_for_tables(&self.tables, frame))
            .transpose()?
            .unwrap_or_else(|| FrameDecoder::for_tables(&self.tables));
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stopped = Arc::clone(&stopped);
        let tables = &self.tables;
        #[cfg(test)]
        let on_reconstructed = self.on_reconstructed.clone();
        let decoder = std::thread::scope(|scope| -> io::Result<FrameDecoder> {
            // The receiver belongs to the scope body: unwinding drops it before
            // joining a producer blocked on a full frame channel.
            let (sender, receiver) = mpsc::sync_channel(1);
            let worker = std::thread::Builder::new().spawn_scoped(scope, move || {
                #[cfg(windows)]
                unsafe {
                    use windows_sys::Win32::System::Threading::{
                        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
                    };
                    SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
                }
                let check = || {
                    if worker_stopped.load(Ordering::Acquire) {
                        Err(io::Error::from(io::ErrorKind::Interrupted))
                    } else {
                        Ok(())
                    }
                };
                check()?;
                let grouped = if replay.is_file_backed() {
                    replay.held_grouped(index)?
                } else {
                    replay.grouped_block(index)?.is_some()
                };
                let frames = if grouped {
                    Some(replay.read_block_selected(index, tables)?)
                } else {
                    // Admission verifies a malformed tail before observers see it.
                    replay.visit_block(index, Some(tables), |_| {
                        check()?;
                        Ok(true)
                    })?;
                    None
                };
                check()?;
                {
                    let mut reconstruct = |frame: Frame| {
                        check()?;
                        let decoded = decoder.advance(&frame)?;
                        validate_campaign(decoded, &replay.campaign_id)?;
                        #[cfg(test)]
                        if let Some(hook) = &on_reconstructed {
                            hook(decoded);
                        }
                        sender
                            .send((frame, decoded.clone()))
                            .map_err(|_| io::Error::from(io::ErrorKind::Interrupted))?;
                        Ok(true)
                    };
                    match frames {
                        Some(frames) => {
                            for frame in frames {
                                reconstruct(frame)?;
                            }
                        }
                        None => replay.visit_block(index, Some(tables), &mut reconstruct)?,
                    }
                }
                Ok(decoder)
            })?;
            let observed = (|| -> io::Result<()> {
                while let Ok((updates, frame)) = receiver.recv() {
                    check()?;
                    visit(updates, &frame)?;
                }
                check()
            })();
            if observed.is_err() {
                stopped.store(true, Ordering::Release);
            }
            drop(receiver);
            let reconstructed = worker
                .join()
                .map_err(|_| io::Error::other("block decode failed"));
            observed?;
            reconstructed?
        })?;
        self.decoder = decoder;
        self.cursor.next_block += 1;
        self.cursor.previous = Some(index.clone());
        Ok(true)
    }

    fn check_prefix(&self, replay: &Replay) -> io::Result<()> {
        if self.cursor.next_block > replay.index.len()
            || self
                .cursor
                .next_block
                .checked_sub(1)
                .map(|position| replay.index.get(position))
                != self.cursor.previous.as_ref().map(Some)
        {
            return Err(invalid("preparation cursor differs from recording prefix"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{CAMPAIGN, interleaved_copy, records, source};
    use super::*;
    use crate::{frame::FrameEncoder, rows::SunkShipsRow};
    use std::sync::mpsc;
    use std::time::Duration;

    fn recording() -> Replay {
        let mut captured = records(1, &[1]);
        captured.pop();
        let mut encoder = FrameEncoder::new();
        for hour in [1, 2, 3, 4, 721, 722, 723, 1441, 1442, 1443] {
            let mut frame = source(1, hour);
            frame.metrics.table_ns[TableId::Clock.index()] = 1;
            if hour % 720 == 3 {
                frame.metrics.table_ns[TableId::WorldTension.index()] = 1;
            }
            if hour == 2 {
                frame.tables[TableId::SunkShips.index()] = TableView::Truncated {
                    rows: TableRows::SunkShips(vec![SunkShipsRow::default()]),
                    missing: 1,
                }
                .into();
            }
            captured.push(Record::Frame(encoder.encode(frame, false).unwrap()));
        }
        Replay::from_records(CAMPAIGN.into(), &[captured, records(2, &[10, 11])]).unwrap()
    }

    fn checkpoint(frame: &DecodedFrame) -> Vec<u8> {
        let mut bytes = Vec::new();
        frame.encode_checkpoint(&mut bytes).unwrap();
        bytes
    }

    fn assert_frame(actual: &DecodedFrame, expected: &DecodedFrame) {
        assert_eq!(actual, expected);
        assert_eq!(checkpoint(actual), checkpoint(expected));
        for table in TableId::ALL {
            assert_eq!(
                actual.tables[table.index()].sampled_at(),
                expected.tables[table.index()].sampled_at()
            );
        }
    }

    fn completions(stream: &mut ReplayStream) -> mpsc::Receiver<i32> {
        let (sender, receiver) = mpsc::channel();
        stream.on_reconstructed = Some(Arc::new(move |frame| {
            let _ = sender.send(frame.hour);
        }));
        receiver
    }

    fn await_decoded(receiver: &mpsc::Receiver<i32>, position: i32) {
        loop {
            if receiver
                .recv_timeout(Duration::from_secs(10))
                .expect("future frame must reconstruct during observation")
                == position
            {
                return;
            }
        }
    }

    #[test]
    fn independent_blocks_equal_continuous_frames_checkpoints_and_samples() {
        let memory = recording();
        let interleaved = interleaved_copy(&memory);
        let path = std::env::temp_dir().join(format!(
            "replay-independent-{}.battleplan",
            std::process::id()
        ));
        std::fs::write(&path, memory.bytes()).unwrap();
        let held = Replay::open_path(&path).unwrap();
        for replay in [&memory, &interleaved, &held] {
            for tables in [
                &TableId::ALL[..],
                &[
                    TableId::Campaign,
                    TableId::Clock,
                    TableId::WorldTension,
                    TableId::SunkShips,
                ][..],
            ] {
                let mut continuous = ReplayStream::new(tables).unwrap();
                for index in &replay.index {
                    let mut decoder = FrameDecoder::for_tables(tables);
                    let independent: Vec<_> = replay
                        .read_block_selected(index, tables)
                        .unwrap()
                        .iter()
                        .map(|frame| decoder.advance(frame).unwrap().clone())
                        .collect();
                    let mut position = 0;
                    continuous
                        .next_block(replay, |frame| {
                            assert_frame(&independent[position], frame);
                            position += 1;
                            Ok(())
                        })
                        .unwrap();
                    assert_eq!(position, independent.len());
                }
            }
        }
        drop(held);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn reconstruction_overlaps_ordered_observation_with_one_queued_frame() {
        for replay in [recording(), interleaved_copy(&recording())] {
            let replay = Arc::new(replay);
            let mut stream = ReplayStream::new(&TableId::ALL).unwrap();
            let mut serial = ReplayStream::new(&TableId::ALL).unwrap();
            let completed = completions(&mut stream);
            for block in 0..replay.index.len() {
                let raw = replay
                    .read_block_selected(&replay.index[block], &TableId::ALL)
                    .unwrap();
                let mut expected = Vec::new();
                serial
                    .next_block(&replay, |frame| {
                        expected.push(frame.clone());
                        Ok(())
                    })
                    .unwrap();
                let mut position = 0;
                stream
                    .next_block_parallel_with(
                        &replay,
                        |updates, frame| {
                            assert_eq!((updates.branch, updates.hour), (frame.branch, frame.hour));
                            assert_eq!(updates, raw[position], "the coordinator owns the admitted update, not a reconstructed placeholder");
                            if block == 0
                                && position == 0
                                && std::thread::available_parallelism().map_or(1, usize::from) > 1
                            {
                                await_decoded(&completed, 3);
                                assert!(
                                    completed.try_recv().is_err(),
                                    "one queued frame must stop reconstruction advancing again"
                                );
                            }
                            assert_frame(frame, &expected[position]);
                            position += 1;
                            Ok(())
                        },
                        || Ok(()),
                    )
                    .unwrap();
                assert_eq!(position, expected.len());
                assert_eq!(stream.cursor(), serial.cursor());
                assert_frame(stream.accumulator().unwrap(), serial.accumulator().unwrap());
            }
        }
    }

    #[test]
    fn cancellation_discards_queued_frames_and_resume_observes_only_tail() {
        let replay = Arc::new(recording());
        let mut stream = ReplayStream::new(&TableId::ALL).unwrap();
        stream
            .next_block_parallel_with(&replay, |_, _| Ok(()), || Ok(()))
            .unwrap();
        let cursor = stream.cursor().clone();
        let accumulator = stream.accumulator().cloned();
        let completed = completions(&mut stream);
        let cancelled = AtomicBool::new(false);
        let error = stream
            .next_block_parallel_with(
                &replay,
                |_, _| {
                    if std::thread::available_parallelism().map_or(1, usize::from) > 1 {
                        await_decoded(&completed, 723);
                    }
                    cancelled.store(true, Ordering::Release);
                    Ok(())
                },
                || {
                    if cancelled.load(Ordering::Acquire) {
                        Err(io::Error::from(io::ErrorKind::Interrupted))
                    } else {
                        Ok(())
                    }
                },
            )
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert_eq!(stream.cursor(), &cursor);
        assert_frame(stream.accumulator().unwrap(), accumulator.as_ref().unwrap());
        let mut resumed =
            ReplayStream::resume(&TableId::ALL, cursor.clone(), accumulator.clone()).unwrap();
        let mut serial = ReplayStream::resume(&TableId::ALL, cursor, accumulator).unwrap();
        while serial.cursor().next_block < replay.index.len() {
            let mut expected = Vec::new();
            serial
                .next_block(&replay, |frame| {
                    expected.push(frame.clone());
                    Ok(())
                })
                .unwrap();
            let mut position = 0;
            resumed
                .next_block_parallel_with(
                    &replay,
                    |updates, frame| {
                        assert_eq!((updates.branch, updates.hour), (frame.branch, frame.hour));
                        assert_frame(frame, &expected[position]);
                        position += 1;
                        Ok(())
                    },
                    || Ok(()),
                )
                .unwrap();
            assert_eq!(position, expected.len());
        }
        assert_eq!(resumed.cursor(), serial.cursor());
    }

    #[test]
    fn appended_revision_and_branch_changes_preserve_order_and_prefix() {
        let replay = Arc::new(recording());
        let appended = Arc::new(replay.append(&[records(3, &[20, 21])]).unwrap());
        let mut stream = ReplayStream::new(&TableId::ALL).unwrap();
        stream
            .next_block_parallel_with(&replay, |_, _| Ok(()), || Ok(()))
            .unwrap();
        let mut serial = ReplayStream::resume(
            &TableId::ALL,
            stream.cursor().clone(),
            stream.accumulator().cloned(),
        )
        .unwrap();
        while serial.cursor().next_block < appended.index.len() {
            let mut expected = Vec::new();
            serial
                .next_block(&appended, |frame| {
                    expected.push(frame.clone());
                    Ok(())
                })
                .unwrap();
            let mut position = 0;
            stream
                .next_block_parallel_with(
                    &appended,
                    |updates, frame| {
                        assert_eq!((updates.branch, updates.hour), (frame.branch, frame.hour));
                        assert_frame(frame, &expected[position]);
                        position += 1;
                        Ok(())
                    },
                    || Ok(()),
                )
                .unwrap();
            assert_eq!(position, expected.len());
        }
        assert_eq!(stream.cursor(), serial.cursor());
    }
    #[test]
    fn queued_malformed_block_refuses_at_its_original_boundary() {
        let original =
            Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2, 721, 722])]).unwrap();
        let tail = original.index.last().unwrap();
        let mut bytes = original.bytes().to_vec();
        bytes[(tail.offset + tail.length - 1) as usize] ^= 1;
        let replay = Arc::new(Replay::open(bytes).unwrap());
        let mut stream = ReplayStream::new(&[TableId::Clock]).unwrap();
        let mut serial = ReplayStream::new(&[TableId::Clock]).unwrap();
        stream
            .next_block_parallel_with(&replay, |_, _| Ok(()), || Ok(()))
            .unwrap();
        serial.next_block(&replay, |_| Ok(())).unwrap();
        let cursor = stream.cursor().clone();
        let accumulator = stream.accumulator().cloned().unwrap();
        let error = stream
            .next_block_parallel_with(&replay, |_, _| panic!("bad block observed"), || Ok(()))
            .unwrap_err();
        let expected = serial
            .next_block(&replay, |_| panic!("bad block observed"))
            .unwrap_err();
        assert_eq!(error.kind(), expected.kind());
        assert_eq!(error.to_string(), expected.to_string());
        assert_eq!(stream.cursor(), &cursor);
        assert_frame(stream.accumulator().unwrap(), &accumulator);
    }

    #[test]
    fn precancelled_stream_admits_no_workers_or_observations() {
        let replay = Arc::new(recording());
        let mut stream = ReplayStream::new(&TableId::ALL).unwrap();
        let completed = completions(&mut stream);
        let error = stream
            .next_block_parallel_with(
                &replay,
                |_, _| panic!("cancelled visitor"),
                || Err(io::Error::from(io::ErrorKind::Interrupted)),
            )
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert_eq!(stream.cursor(), &ReplayCursor::default());
        assert!(stream.accumulator().is_none());
        assert!(completed.try_recv().is_err());
    }

    #[test]
    fn cancellation_during_blocking_delivery_discards_the_completed_block() {
        let replay = Arc::new(recording());
        let mut stream = ReplayStream::new(&TableId::ALL).unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        stream.on_reconstructed = Some(Arc::new(move |frame| {
            if frame.hour == 1 {
                worker_cancelled.store(true, Ordering::Release);
            }
        }));
        let error = stream
            .next_block_parallel_with(
                &replay,
                |_, _| {
                    if std::thread::available_parallelism().map_or(1, usize::from) > 1 {
                        panic!("cancelled delivery was observed");
                    }
                    Err(io::Error::from(io::ErrorKind::Interrupted))
                },
                || {
                    if cancelled.load(Ordering::Acquire) {
                        Err(io::Error::from(io::ErrorKind::Interrupted))
                    } else {
                        Ok(())
                    }
                },
            )
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert_eq!(stream.cursor(), &ReplayCursor::default());
        assert!(stream.accumulator().is_none());
    }

    #[test]
    fn worker_panic_disconnects_ordered_delivery_and_preserves_the_cursor() {
        if std::thread::available_parallelism().map_or(1, usize::from) == 1 {
            return;
        }
        let replay = Arc::new(recording());
        let mut stream = ReplayStream::new(&TableId::ALL).unwrap();
        stream.on_reconstructed = Some(Arc::new(|frame| {
            assert_ne!(frame.hour, 1, "synthetic decode worker panic");
        }));
        let error = stream
            .next_block_parallel_with(&replay, |_, _| panic!("failed worker observed"), || Ok(()))
            .unwrap_err();
        assert_eq!(error.to_string(), "block decode failed");
        assert_eq!(stream.cursor(), &ReplayCursor::default());
        assert!(stream.accumulator().is_none());
    }

    #[test]
    fn observer_panic_drops_receiver_before_joining_a_blocked_producer() {
        let replay = Arc::new(recording());
        let mut stream = ReplayStream::new(&TableId::ALL).unwrap();
        let completed = completions(&mut stream);
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = stream.next_block_parallel_with(
                &replay,
                |_, _| {
                    if std::thread::available_parallelism().map_or(1, usize::from) > 1 {
                        await_decoded(&completed, 3);
                    }
                    panic!("synthetic observer panic while producer is blocked");
                },
                || Ok(()),
            );
        }));
        assert!(panic.is_err());
        assert_eq!(stream.cursor(), &ReplayCursor::default());
        assert!(stream.accumulator().is_none());
    }
}
