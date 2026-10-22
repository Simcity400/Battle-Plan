//! Selected preparation with an accepted block cursor and decoder accumulator.

use super::*;
use crate::frame::{FrameSource, RecordedFrame};
use std::sync::{Arc, mpsc};

#[cfg(test)]
type ReconstructedHook = Arc<dyn Fn(&DecodedFrame) + Send + Sync>;
#[cfg(test)]
type ReadHook = Arc<dyn Fn(&Frame) + Send + Sync>;

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
    #[cfg(test)]
    on_read: Option<ReadHook>,
}

impl ReplayStream {
    pub fn new(tables: &[TableId]) -> io::Result<Self> {
        let mut selected = engine_allocation::collect_vec(tables.iter().copied())?;
        if !selected.contains(&TableId::Campaign) {
            engine_allocation::reserve(&mut selected, 1)?;
            selected.push(TableId::Campaign);
        }
        Ok(Self {
            cursor: ReplayCursor::default(),
            decoder: FrameDecoder::for_tables(&selected),
            tables: selected,
            #[cfg(test)]
            on_reconstructed: None,
            #[cfg(test)]
            on_read: None,
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
        self.next_block_recorded_with(replay, |_, decoded| visit(decoded), || Ok(()))
    }

    /// Check cancellation during admission and reconstruction, once per payload.
    #[cfg(any(test, feature = "test-support"))]
    pub fn next_block_with(
        &mut self,
        replay: &Replay,
        mut visit: impl FnMut(Frame, &DecodedFrame) -> io::Result<()>,
        check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<bool> {
        self.next_block_recorded_with(
            replay,
            |raw, frame| visit(raw.into_owned_frame()?, frame),
            check,
        )
    }
    /// The selected stream with immutable recorded updates, shared between readers.
    pub fn next_block_recorded_with(
        &mut self,
        replay: &Replay,
        visit: impl FnMut(RecordedFrame, &DecodedFrame) -> io::Result<()>,
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<bool> {
        check()?;
        self.check_prefix(replay)?;
        let Some(index) = replay.index.get(self.cursor.next_block) else {
            return Ok(false);
        };
        let tables = &self.tables;
        let current = &self.decoder;
        let decoder = reconstruct_recorded_block(
            replay,
            index,
            tables,
            || {
                current
                    .current()
                    .cloned()
                    .map(|frame| FrameDecoder::resume_for_tables(tables, frame))
                    .transpose()
                    .map(|decoder| decoder.unwrap_or_else(|| FrameDecoder::for_tables(tables)))
            },
            visit,
            check,
        )?;
        self.decoder = decoder;
        self.cursor.next_block += 1;
        self.cursor.previous = Some(index.clone());
        Ok(true)
    }

    /// Read on one worker; reconstruct and observe here, in order. Each
    /// observed hour is released before the next is reconstructed, so every
    /// delta applies to rows no observer still holds, in place.
    /// Decoder state and cursor move only after the whole block succeeds.
    #[cfg(test)]
    pub fn next_block_parallel_with(
        &mut self,
        replay: &Arc<Replay>,
        mut visit: impl FnMut(Frame, &DecodedFrame) -> io::Result<()>,
        check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<bool> {
        self.next_block_recorded_parallel_with(
            replay,
            |raw, frame| visit(raw.into_owned_frame()?, frame),
            check,
        )
    }
    /// Parallel admission with shared immutable updates and local reconstruction.
    pub fn next_block_recorded_parallel_with(
        &mut self,
        replay: &Arc<Replay>,
        mut visit: impl FnMut(RecordedFrame, &DecodedFrame) -> io::Result<()>,
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<bool> {
        if std::thread::available_parallelism().map_or(1, usize::from) == 1 {
            return self.next_block_recorded_with(replay, visit, check);
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
        #[cfg(test)]
        let on_read = self.on_read.clone();
        std::thread::scope(|scope| -> io::Result<()> {
            // The receiver belongs to the scope body: unwinding drops it before
            // joining a producer blocked on a full frame channel.
            let (sender, receiver) = mpsc::sync_channel(1);
            let priority = job_priority::Caller::current();
            let worker = std::thread::Builder::new().spawn_scoped(scope, move || {
                priority.enter();
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
                    // The observer synchronously waits for these reads: its
                    // decoder and table helpers run at the observer's job level.
                    let caller = job_priority::Caller::current();
                    let enter = move || caller.clone().enter();
                    let parallel = Parallel {
                        workers: super::pack::workers(),
                        on_worker: &enter,
                    };
                    Some(replay.read_block_recorded_selected_parallel(index, tables, &parallel)?)
                } else {
                    // Admission verifies a malformed tail before observers see it.
                    replay.visit_block(index, Some(tables), |_| {
                        check()?;
                        Ok(true)
                    })?;
                    None
                };
                check()?;
                let send = |frame: RecordedFrame| {
                    check()?;
                    #[cfg(test)]
                    if let Some(hook) = &on_read {
                        hook(&frame.to_owned_frame()?);
                    }
                    sender
                        .send(frame)
                        .map_err(|_| io::Error::from(io::ErrorKind::Interrupted))?;
                    Ok(true)
                };
                match frames {
                    Some(frames) => {
                        for frame in frames {
                            send(frame)?;
                        }
                    }
                    None => replay.visit_block(index, Some(tables), |frame| send(frame.into()))?,
                }
                Ok(())
            })?;
            let observed = (|| -> io::Result<()> {
                while let Ok(frame) = receiver.recv() {
                    let decoded = decoder.advance(&frame)?;
                    validate_campaign(decoded, &replay.campaign_id)?;
                    #[cfg(test)]
                    if let Some(hook) = &on_reconstructed {
                        hook(decoded);
                    }
                    check()?;
                    visit(frame, decoded)?;
                }
                check()
            })();
            if observed.is_err() {
                stopped.store(true, Ordering::Release);
            }
            drop(receiver);
            let read = worker
                .join()
                .map_err(|_| io::Error::other("block decode failed"));
            observed?;
            read?
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

impl Replay {
    /// One block's selected frames reconstructed from its keyframe alone,
    /// without any earlier block's decoder state, so blocks can be read in any
    /// order and on any thread. Every block starts at a keyframe, where the
    /// decoder keeps nothing of the hour before, so each hour equals what a
    /// stream continuing from the previous block reconstructs: rows,
    /// availability, sample hours, metrics and which hours share a table's
    /// allocation. Anchored keyframes are read from the earlier blocks they
    /// name, as every read does. Campaign is reconstructed and checked as the
    /// stream does; other tables are unavailable.
    pub fn read_block_alone(
        &self,
        index: &BlockIndex,
        tables: &[TableId],
        visit: impl FnMut(Frame, &DecodedFrame) -> io::Result<()>,
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        check()?;
        let mut selected = engine_allocation::collect_vec(tables.iter().copied())?;
        if !selected.contains(&TableId::Campaign) {
            engine_allocation::reserve(&mut selected, 1)?;
            selected.push(TableId::Campaign);
        }
        reconstruct_block(
            self,
            index,
            &selected,
            || Ok(FrameDecoder::for_tables(&selected)),
            visit,
            check,
        )
        .map(|_| ())
    }

    /// Reconstruct shared recorded updates without materializing owned wire frames.
    pub fn read_block_recorded_alone(
        &self,
        index: &BlockIndex,
        tables: &[TableId],
        mut visit: impl FnMut(Arc<RecordedFrame>, &DecodedFrame) -> io::Result<()>,
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        check()?;
        let grouped = if self.is_file_backed() {
            self.held_grouped(index)?
        } else {
            self.grouped_block(index)?.is_some()
        };
        if !grouped {
            return self.read_block_alone(
                index,
                tables,
                |raw, frame| visit(Arc::new(raw.into()), frame),
                check,
            );
        }
        let mut selected = engine_allocation::collect_vec(tables.iter().copied())?;
        if !selected.contains(&TableId::Campaign) {
            engine_allocation::reserve(&mut selected, 1)?;
            selected.push(TableId::Campaign);
        }
        let frames = self.read_block_recorded_selected(index, &selected)?;
        let mut decoder = FrameDecoder::for_tables(&selected);
        for frame in frames {
            check()?;
            let frame = Arc::new(frame);
            let decoded = decoder.advance(&frame)?;
            validate_campaign(decoded, &self.campaign_id)?;
            visit(frame, decoded)?;
        }
        Ok(())
    }

    /// [`Replay::read_block_alone`] of a block already read: `frames` are the
    /// block's frames as [`Replay::read_block_selected`] returned them for a
    /// selection holding `tables`, here reconstructed with `tables` (and
    /// Campaign) alone from the block's keyframe and handed to `visit` with
    /// their recorded updates, so one read of a block serves readers of
    /// several selections within it. A selected read decodes each table's
    /// updates on their own, so a selected table's updates and every frame's
    /// hour and metrics are those a read of `tables` returns; the decoder
    /// reconstructs `tables` alone, so each hour, and which hours share a
    /// table's allocation, are `read_block_alone`'s. The frames' other tables'
    /// updates are not reconstructed. The one check that reads every table,
    /// that a keyframe holds only updates needing no earlier hour, would see
    /// them: when one fails it the block is not reconstructed and nothing is
    /// visited (false), and the caller reads it alone. A prefix of a block's
    /// frames reconstructs its hours through the prefix's last.
    pub fn reconstruct_alone<F: FrameSource>(
        &self,
        frames: &[F],
        tables: &[TableId],
        mut visit: impl FnMut(&F, &DecodedFrame) -> io::Result<()>,
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<bool> {
        check()?;
        let mut selected = engine_allocation::collect_vec(tables.iter().copied())?;
        if !selected.contains(&TableId::Campaign) {
            engine_allocation::reserve(&mut selected, 1)?;
            selected.push(TableId::Campaign);
        }
        let mut chosen = [false; TABLE_COUNT];
        for table in &selected {
            chosen[table.index()] = true;
        }
        // A read of `tables` alone holds an unselected table as unavailable,
        // which a keyframe may hold.
        let unselected_dependent = frames.iter().any(|frame| {
            frame.keyframe()
                && TableId::ALL.iter().any(|table| {
                    !chosen[table.index()]
                        && !crate::frame::keyframe_update(frame.update(table.index()))
                })
        });
        if unselected_dependent {
            return Ok(false);
        }
        let mut decoder = FrameDecoder::for_tables(&selected);
        for frame in frames {
            check()?;
            let decoded = decoder.advance(frame)?;
            validate_campaign(decoded, &self.campaign_id)?;
            visit(frame, decoded)?;
        }
        Ok(true)
    }
}

fn reconstruct_recorded_block(
    replay: &Replay,
    index: &BlockIndex,
    tables: &[TableId],
    decoder: impl FnOnce() -> io::Result<FrameDecoder>,
    mut visit: impl FnMut(RecordedFrame, &DecodedFrame) -> io::Result<()>,
    mut check: impl FnMut() -> io::Result<()>,
) -> io::Result<FrameDecoder> {
    let grouped = if replay.is_file_backed() {
        replay.held_grouped(index)?
    } else {
        replay.grouped_block(index)?.is_some()
    };
    if !grouped {
        return reconstruct_block(
            replay,
            index,
            tables,
            decoder,
            |raw, frame| visit(raw.into(), frame),
            check,
        );
    }
    let frames = replay.read_block_recorded_selected(index, tables)?;
    check()?;
    let mut decoder = decoder()?;
    for frame in frames {
        check()?;
        let decoded = decoder.advance(&frame)?;
        validate_campaign(decoded, &replay.campaign_id)?;
        visit(frame, decoded)?;
    }
    Ok(decoder)
}

/// Read a block's selected frames and reconstruct them with the decoder
/// `decoder` returns once admission succeeded, handing each to `visit`.
fn reconstruct_block(
    replay: &Replay,
    index: &BlockIndex,
    tables: &[TableId],
    decoder: impl FnOnce() -> io::Result<FrameDecoder>,
    mut visit: impl FnMut(Frame, &DecodedFrame) -> io::Result<()>,
    mut check: impl FnMut() -> io::Result<()>,
) -> io::Result<FrameDecoder> {
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
        Some(replay.read_block_selected(index, tables)?)
    } else {
        replay.visit_block(index, Some(tables), |_| {
            check()?;
            Ok(true)
        })?;
        None
    };
    check()?;
    let mut decoder = decoder()?;
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
        None => replay.visit_block(index, Some(tables), |frame| {
            check()?;
            reconstruct(frame)
        })?,
    }
    Ok(decoder)
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

    /// Each hour the worker reads, as it queues the frame.
    fn reads(stream: &mut ReplayStream) -> mpsc::Receiver<i32> {
        let (sender, receiver) = mpsc::channel();
        stream.on_read = Some(Arc::new(move |frame| {
            let _ = sender.send(frame.hour);
        }));
        receiver
    }

    fn await_read(receiver: &mpsc::Receiver<i32>, position: i32) {
        loop {
            if receiver
                .recv_timeout(Duration::from_secs(10))
                .expect("future frame must be read during observation")
                == position
            {
                return;
            }
        }
    }

    #[test]
    #[cfg(windows)]
    fn parallel_stream_reads_at_the_observers_job_priority() {
        use job_priority::JobPriority;
        use windows_sys::Win32::System::Threading::{GetCurrentThread, GetThreadPriority};
        for background in [false, true] {
            JobPriority::run(Arc::new(AtomicBool::new(background)), || {
                let expected = unsafe { GetThreadPriority(GetCurrentThread()) };
                let replay = Arc::new(recording());
                let mut stream = ReplayStream::new(&TableId::ALL).unwrap();
                stream.on_read = Some(Arc::new(move |_| {
                    assert_eq!(unsafe { GetThreadPriority(GetCurrentThread()) }, expected);
                }));
                while stream
                    .next_block_parallel_with(&replay, |_, _| Ok(()), || Ok(()))
                    .unwrap()
                {}
            });
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

    /// `recording` with branch 1 continued after branch 2 by an appended
    /// segment: its first hour follows branch 1's last block, and its keyframe
    /// is written against the keyframes of earlier blocks.
    fn continued() -> Replay {
        let mut segment = records(1, &[1]);
        segment.truncate(2);
        let mut encoder = FrameEncoder::new();
        for hour in [1444, 1445, 1446] {
            let mut frame = source(1, hour);
            frame.metrics.table_ns[TableId::Clock.index()] = 1;
            if hour == 1445 {
                frame.metrics.table_ns[TableId::WorldTension.index()] = 1;
                frame.tables[TableId::SunkShips.index()] = TableView::Truncated {
                    rows: TableRows::SunkShips(vec![SunkShipsRow::default()]),
                    missing: 2,
                }
                .into();
            }
            segment.push(Record::Frame(encoder.encode(frame, false).unwrap()));
        }
        recording().append(&[segment]).unwrap()
    }

    #[test]
    fn blocks_read_alone_equal_the_continuous_stream_in_any_order() {
        use crate::frame::SharedView;
        let memory = continued();
        assert_eq!(memory.index.len(), 5);
        assert_eq!(
            (memory.index[4].branch, memory.index[4].first_hour),
            (1, 1444)
        );
        let interleaved = interleaved_copy(&memory);
        let path =
            std::env::temp_dir().join(format!("replay-alone-{}.battleplan", std::process::id()));
        std::fs::write(&path, memory.bytes()).unwrap();
        let held = Replay::open_path(&path).unwrap();
        for replay in [&memory, &interleaved, &held] {
            for tables in [
                &TableId::ALL[..],
                &[TableId::Clock, TableId::WorldTension, TableId::SunkShips][..],
            ] {
                // Every block alone on its own thread, nothing read before it.
                let mut alone: Vec<Vec<(Frame, DecodedFrame)>> = std::thread::scope(|scope| {
                    let reads: Vec<_> = replay
                        .index
                        .iter()
                        .rev()
                        .map(|index| {
                            scope.spawn(move || {
                                let mut frames = Vec::new();
                                replay
                                    .read_block_alone(
                                        index,
                                        tables,
                                        |raw, frame| {
                                            frames.push((raw, frame.clone()));
                                            Ok(())
                                        },
                                        || Ok(()),
                                    )
                                    .unwrap();
                                frames
                            })
                        })
                        .collect();
                    reads.into_iter().map(|read| read.join().unwrap()).collect()
                });
                alone.reverse();
                let mut continuous = ReplayStream::new(tables).unwrap();
                for block in &alone {
                    let mut position = 0;
                    let mut previous: Option<DecodedFrame> = None;
                    continuous
                        .next_block_with(
                            replay,
                            |raw, frame| {
                                let (alone_raw, alone_frame) = &block[position];
                                assert_eq!(&raw, alone_raw);
                                assert_frame(alone_frame, frame);
                                if let Some(previous) = &previous {
                                    let alone_previous = &block[position - 1].1;
                                    for table in TableId::ALL {
                                        let index = table.index();
                                        assert_eq!(
                                            SharedView::ptr_eq(
                                                &alone_previous.tables[index],
                                                &alone_frame.tables[index]
                                            ),
                                            SharedView::ptr_eq(
                                                &previous.tables[index],
                                                &frame.tables[index]
                                            ),
                                            "{table:?} shares the hour before's allocation in one reader only"
                                        );
                                    }
                                }
                                previous = Some(frame.clone());
                                position += 1;
                                Ok(())
                            },
                            || Ok(()),
                        )
                        .unwrap();
                    assert_eq!(position, block.len());
                }
                assert_eq!(continuous.cursor().next_block, replay.index.len());
            }
        }
        drop(held);
        std::fs::remove_file(path).unwrap();
    }

    /// One read of a block for a selection reconstructs each selection
    /// within it exactly as that selection's own read alone: every hour, its
    /// recorded updates of the selected tables and its metrics, and which
    /// hours share each table's allocation (the change facts readers derive).
    /// A keyframe holding an unselected table's dependent update, which a
    /// read of the selection alone holds as unavailable, is refused unvisited.
    #[test]
    fn a_wider_read_reconstructs_each_selection_within_it_as_its_own_read() {
        use crate::frame::{SharedView, TableUpdate};
        let memory = continued();
        let interleaved = interleaved_copy(&memory);
        let path =
            std::env::temp_dir().join(format!("replay-wider-{}.battleplan", std::process::id()));
        std::fs::write(&path, memory.bytes()).unwrap();
        let held = Replay::open_path(&path).unwrap();
        let wide = [
            TableId::Campaign,
            TableId::Clock,
            TableId::WorldTension,
            TableId::SunkShips,
        ];
        let narrow: [&[TableId]; 3] = [
            &[TableId::Clock],
            &[TableId::WorldTension, TableId::SunkShips],
            &wide,
        ];
        for replay in [&memory, &interleaved, &held] {
            for tables in narrow {
                for index in &replay.index {
                    let mut alone = Vec::new();
                    replay
                        .read_block_alone(
                            index,
                            tables,
                            |raw, frame| {
                                alone.push((raw, frame.clone()));
                                Ok(())
                            },
                            || Ok(()),
                        )
                        .unwrap();
                    let read: Vec<Arc<Frame>> = replay
                        .read_block_selected(index, &wide)
                        .unwrap()
                        .into_iter()
                        .map(Arc::new)
                        .collect();
                    let mut position = 0;
                    let mut previous: Option<DecodedFrame> = None;
                    let reconstructed = replay
                        .reconstruct_alone(
                            &read,
                            tables,
                            |raw, frame| {
                                let (alone_raw, alone_frame) = &alone[position];
                                assert_eq!(
                                    (raw.branch, raw.hour, raw.keyframe),
                                    (alone_raw.branch, alone_raw.hour, alone_raw.keyframe)
                                );
                                assert_eq!(raw.metrics, alone_raw.metrics);
                                for table in tables.iter().chain(std::iter::once(&TableId::Campaign))
                                {
                                    assert_eq!(
                                        raw.tables[table.index()],
                                        alone_raw.tables[table.index()],
                                        "{table:?}'s recorded update"
                                    );
                                }
                                assert_frame(frame, alone_frame);
                                if let Some(previous) = &previous {
                                    let alone_previous = &alone[position - 1].1;
                                    for table in TableId::ALL {
                                        let at = table.index();
                                        assert_eq!(
                                            SharedView::ptr_eq(&previous.tables[at], &frame.tables[at]),
                                            SharedView::ptr_eq(
                                                &alone_previous.tables[at],
                                                &alone_frame.tables[at]
                                            ),
                                            "{table:?} shares the hour before's allocation in one read only"
                                        );
                                    }
                                }
                                previous = Some(frame.clone());
                                position += 1;
                                Ok(())
                            },
                            || Ok(()),
                        )
                        .unwrap();
                    assert!(reconstructed);
                    assert_eq!(position, alone.len());
                }
            }
        }
        // A prefix reconstructs its hours alone.
        let index = &memory.index[0];
        let read = memory.read_block_selected(index, &wide).unwrap();
        let mut hours = Vec::new();
        assert!(
            memory
                .reconstruct_alone(
                    &read[..2],
                    &[TableId::Clock],
                    |raw, frame| {
                        assert_eq!(raw.hour, frame.hour);
                        hours.push(frame.hour);
                        Ok(())
                    },
                    || Ok(()),
                )
                .unwrap()
        );
        assert_eq!(hours, [read[0].hour, read[1].hour]);
        // An unselected table's dependent update at a keyframe.
        let mut read = read;
        assert!(read[0].keyframe);
        read[0].tables[TableId::SunkShips.index()] = TableUpdate::Unchanged {
            baseline: read[0].hour - 1,
        };
        let mut visited = 0;
        assert!(
            !memory
                .reconstruct_alone(
                    &read,
                    &[TableId::Clock],
                    |_, _| {
                        visited += 1;
                        Ok(())
                    },
                    || Ok(()),
                )
                .unwrap()
        );
        assert_eq!(visited, 0);
        drop(held);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn reading_overlaps_observation_which_reconstructs_each_hour_after_the_last() {
        let parallel = std::thread::available_parallelism().map_or(1, usize::from) > 1;
        for replay in [recording(), interleaved_copy(&recording())] {
            let replay = Arc::new(replay);
            let mut stream = ReplayStream::new(&TableId::ALL).unwrap();
            let mut serial = ReplayStream::new(&TableId::ALL).unwrap();
            let read = reads(&mut stream);
            let (sender, reconstructed) = mpsc::channel();
            stream.on_reconstructed = Some(Arc::new(move |frame| {
                let _ = sender.send((frame.hour, std::thread::current().id()));
            }));
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
                            if parallel {
                                // Reconstructed here and only once the previous
                                // hour was released, so no observer holds the
                                // rows a delta applies to.
                                assert_eq!(
                                    reconstructed.try_recv(),
                                    Ok((frame.hour, std::thread::current().id()))
                                );
                                assert!(reconstructed.try_recv().is_err());
                            }
                            if block == 0 && position == 0 && parallel {
                                await_read(&read, 3);
                                assert!(
                                    read.try_recv().is_err(),
                                    "one queued frame must stop reading advancing again"
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
        let read = reads(&mut stream);
        let cancelled = AtomicBool::new(false);
        let error = stream
            .next_block_parallel_with(
                &replay,
                |_, _| {
                    if std::thread::available_parallelism().map_or(1, usize::from) > 1 {
                        await_read(&read, 723);
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
        stream.on_read = Some(Arc::new(|frame| {
            assert_ne!(frame.hour, 1, "synthetic read worker panic");
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
        let read = reads(&mut stream);
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = stream.next_block_parallel_with(
                &replay,
                |_, _| {
                    if std::thread::available_parallelism().map_or(1, usize::from) > 1 {
                        await_read(&read, 3);
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
